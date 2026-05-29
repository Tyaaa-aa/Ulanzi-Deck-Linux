"""Background daemon for Ulanzi Manager"""

import sys
import time
import logging
import signal
import os
import re
import subprocess
import urllib.request
import urllib.parse
import hashlib
import threading
from pathlib import Path
from typing import Optional

from ulanzi_manager.device import UlanziDevice, ButtonPress
from ulanzi_manager.config import ConfigParser, Config, ButtonConfig
from ulanzi_manager.actions import ActionExecutor

# Setup logging
log_dir = Path.home() / '.local/share/ulanzi'
try:
    log_dir.mkdir(parents=True, exist_ok=True)
    handlers = [
        logging.FileHandler(log_dir / 'daemon.log'),
        logging.StreamHandler()
    ]
except Exception:
    handlers = [
        logging.StreamHandler()
    ]

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s',
    handlers=handlers
)
logger = logging.getLogger(__name__)


class UlanziDaemon:
    """Background daemon for Ulanzi device"""

    def __init__(self, config_path: str):
        """Initialize daemon"""
        self.config_path = config_path
        self.config: Optional[Config] = None
        self.device: Optional[UlanziDevice] = None
        self.executor: Optional[ActionExecutor] = None
        self.running = False
        self.obs_client = None
        self.last_interaction_time = time.time()
        self.is_sleeping = False
        self.last_button_dict = None
        self._prev_cpu_total = 0
        self._prev_cpu_idle = 0

    def _ensure_button_13(self, config):
        """Ensure Button 13 exists in the configuration"""
        if not config:
            return
        has_btn_13 = any(b.index == 13 for b in config.buttons)
        if not has_btn_13:
            logger.info("Button 13 config not found. Creating default placeholder.")
            config.buttons.append(ButtonConfig(
                index=13,
                image="",
                label="Clock Button",
                action_type="command",
                action_params={}
            ))

    def _get_cpu_temp(self) -> int:
        """Scan system sensors to find the CPU temperature"""
        for hwmon_dir in Path('/sys/class/hwmon').glob('hwmon*'):
            try:
                name_file = hwmon_dir / 'name'
                if name_file.exists():
                    driver_name = name_file.read_text().strip()
                    if driver_name in ('k10temp', 'coretemp', 'cpu_thermal'):
                        temp_files = list(hwmon_dir.glob('temp*_input'))
                        if temp_files:
                            temps = []
                            for tf in temp_files:
                                try:
                                    temps.append(int(tf.read_text().strip()) / 1000)
                                except Exception:
                                    pass
                            if temps:
                                return int(max(temps))
            except Exception:
                pass
        # Fallback to thermal zones
        for zone in Path('/sys/class/thermal').glob('thermal_zone*'):
            try:
                type_file = zone / 'type'
                if type_file.exists() and 'cpu' in type_file.read_text().lower():
                    temp_file = zone / 'temp'
                    if temp_file.exists():
                        return int(int(temp_file.read_text().strip()) / 1000)
            except Exception:
                pass
        # Final fallback
        try:
            t0 = Path('/sys/class/thermal/thermal_zone0/temp')
            if t0.exists():
                return int(int(t0.read_text().strip()) / 1000)
        except Exception:
            pass
        return 0

    def _get_system_stats(self) -> dict:
        """Calculate current CPU and RAM usage percentages"""
        stats = {'cpu': 0, 'ram': 0, 'temp': 0}
        
        # Calculate CPU usage
        try:
            with open('/proc/stat', 'r') as f:
                first_line = f.readline()
                if first_line.startswith('cpu '):
                    parts = [int(x) for x in first_line.split()[1:]]
                    idle = parts[3] + parts[4]
                    non_idle = parts[0] + parts[1] + parts[2] + parts[5] + parts[6] + parts[7]
                    total = idle + non_idle
                    
                    if self._prev_cpu_total > 0:
                        diff_total = total - self._prev_cpu_total
                        diff_idle = idle - self._prev_cpu_idle
                        if diff_total > 0:
                            stats['cpu'] = int((diff_total - diff_idle) / diff_total * 100)
                    
                    self._prev_cpu_total = total
                    self._prev_cpu_idle = idle
        except Exception as e:
            logger.debug(f"Error reading CPU usage: {e}")

        # Calculate RAM usage
        mem_total = 0
        mem_available = 0
        try:
            with open('/proc/meminfo', 'r') as f:
                for line in f:
                    if line.startswith('MemTotal:'):
                        mem_total = int(line.split()[1])
                    elif line.startswith('MemAvailable:'):
                        mem_available = int(line.split()[1])
            if mem_total > 0:
                mem_used = mem_total - mem_available
                stats['ram'] = int((mem_used / mem_total) * 100)
        except Exception as e:
            logger.debug(f"Error reading RAM usage: {e}")

        # Get Temperature
        stats['temp'] = self._get_cpu_temp()
        return stats

    def _get_mpris_media_info(self) -> dict:
        """Retrieve media metadata and player status via busctl"""
        media = {'title': 'No Media', 'artist': 'Unknown', 'status': 'Stopped', 'artUrl': ''}
        try:
            # 1. Find player services
            res = subprocess.run(
                ['busctl', '--user', 'list'],
                capture_output=True, text=True, timeout=1
            )
            players = []
            for line in res.stdout.splitlines():
                parts = line.split()
                if parts and parts[0].startswith('org.mpris.MediaPlayer2.'):
                    players.append(parts[0])

            if not players:
                return media

            # 2. Find active player
            active_player = players[0]
            for player in players:
                res_status = subprocess.run(
                    ['busctl', '--user', 'get-property', player, '/org/mpris/MediaPlayer2', 'org.mpris.MediaPlayer2.Player', 'PlaybackStatus'],
                    capture_output=True, text=True, timeout=1
                )
                if 'Playing' in res_status.stdout:
                    active_player = player
                    media['status'] = 'Playing'
                    break
                elif 'Paused' in res_status.stdout:
                    active_player = player
                    media['status'] = 'Paused'

            # 3. Query metadata from player
            res_meta = subprocess.run(
                ['busctl', '--user', 'get-property', active_player, '/org/mpris/MediaPlayer2', 'org.mpris.MediaPlayer2.Player', 'Metadata'],
                capture_output=True, text=True, timeout=1
            )
            
            # Parse title, artist and artUrl from dbus output dict
            title_match = re.search(r'"xesam:title"\s+s\s+"((?:[^"\\]|\\.)*)"', res_meta.stdout)
            if title_match:
                media['title'] = title_match.group(1).replace('\\"', '"')
                
            artist_match = re.search(r'"xesam:artist"\s+as\s+\d+\s+"((?:[^"\\]|\\.)*)"', res_meta.stdout)
            if not artist_match:
                artist_match = re.search(r'"xesam:artist"\s+s\s+"((?:[^"\\]|\\.)*)"', res_meta.stdout)
            if artist_match:
                media['artist'] = artist_match.group(1).replace('\\"', '"')

            art_match = re.search(r'"mpris:artUrl"\s+s\s+"((?:[^"\\]|\\.)*)"', res_meta.stdout)
            if art_match:
                media['artUrl'] = art_match.group(1).replace('\\"', '"')

        except Exception as e:
            logger.debug(f"Error querying MPRIS: {e}")
            
        return media

    def _resolve_variables(self, label: str, stats: dict, media: dict) -> str:
        """Resolve CPU, RAM, and media variable replacements inside button label text"""
        if not label:
            return ""
        
        variables = {
            'cpu_usage': str(stats.get('cpu', 0)),
            'ram_usage': str(stats.get('ram', 0)),
            'cpu_temp': str(stats.get('temp', 0)),
            'media_title': media.get('title', 'No Media'),
            'media_artist': media.get('artist', 'Unknown'),
            'media_status': media.get('status', 'Stopped'),
        }
        
        for k, v in variables.items():
            placeholder = f"{{{k}}}"
            if placeholder in label:
                label = label.replace(placeholder, v)
        return label

    def _fetch_art_async(self, url: str, cache_dir: Path) -> Optional[str]:
        """Fetch album art from local file:// or background thread download remote http/s"""
        if not url:
            return None
            
        if url.startswith('file://'):
            from urllib.parse import unquote
            path = unquote(url[7:])
            if os.path.exists(path):
                return path
            return None
            
        if not (url.startswith('http://') or url.startswith('https://')):
            if os.path.exists(url):
                return url
            return None
            
        # Cache lookup
        if not hasattr(self, '_art_cache'):
            self._art_cache = {}
        if not hasattr(self, '_downloading_urls'):
            self._downloading_urls = set()
            
        if url in self._art_cache:
            cached_path = self._art_cache[url]
            if os.path.exists(cached_path):
                return cached_path
                
        # Hash the URL for filename
        url_hash = hashlib.md5(url.encode('utf-8')).hexdigest()
        cached_path = cache_dir / f"art_{url_hash}.png"
        if cached_path.exists():
            self._art_cache[url] = str(cached_path)
            return str(cached_path)
            
        # Start background download thread if not already downloading
        if url not in self._downloading_urls:
            self._downloading_urls.add(url)
            def download_thread():
                try:
                    cache_dir.mkdir(parents=True, exist_ok=True)
                    req = urllib.request.Request(
                        url, 
                        headers={'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64)'}
                    )
                    with urllib.request.urlopen(req, timeout=3) as response:
                        data = response.read()
                    cached_path.write_bytes(data)
                    self._art_cache[url] = str(cached_path)
                    logger.info(f"Downloaded album art to cache: {cached_path}")
                except Exception as e:
                    logger.debug(f"Failed to download album art from {url}: {e}")
                finally:
                    self._downloading_urls.discard(url)
                    
            threading.Thread(target=download_thread, daemon=True).start()
            
        return None

    def _generate_media_clock_image(self, title: str, artist: str, art_path: Optional[str], output_path: Path):
        """Generate a 392x196 media overlay canvas with album art and text, squeezed to 196x196"""
        try:
            from PIL import Image, ImageDraw, ImageFont
        except ImportError:
            logger.warning("Pillow not installed, cannot generate media clock image")
            return

        # 1. Create a 392x196 canvas
        bg_color = (19, 28, 46) # Dark background matching Fusion style
        img = Image.new('RGB', (392, 196), color=bg_color)
        draw = ImageDraw.Draw(img)
        
        # 2. Draw album art on the left
        art_drawn = False
        if art_path and os.path.exists(art_path):
            try:
                with Image.open(art_path) as art_img:
                    art_resized = art_img.resize((176, 176), Image.Resampling.LANCZOS)
                    img.paste(art_resized, (10, 10))
                    art_drawn = True
            except Exception as e:
                logger.debug(f"Failed to load album art image: {e}")
                
        if not art_drawn:
            # Draw a fallback CD circle
            cd_center = (98, 98)
            cd_radius = 80
            draw.ellipse([cd_center[0] - cd_radius, cd_center[1] - cd_radius, cd_center[0] + cd_radius, cd_center[1] + cd_radius], fill=(40, 50, 75), outline=(100, 120, 160), width=2)
            draw.ellipse([cd_center[0] - 25, cd_center[1] - 25, cd_center[0] + 25, cd_center[1] + 25], fill=(19, 28, 46), outline=(100, 120, 160), width=2)
            draw.ellipse([cd_center[0] - 8, cd_center[1] - 8, cd_center[0] + 8, cd_center[1] + 8], fill=(100, 120, 160))
            draw.polygon([(cd_center[0] - 4, cd_center[1] - 8), (cd_center[0] + 8, cd_center[1]), (cd_center[0] - 4, cd_center[1] + 8)], fill=(255, 255, 255))
            
        # 3. Draw text on the right (x from 200 to 380, y centered)
        title_font = None
        artist_font = None
        try:
            title_font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf", 24)
            artist_font = ImageFont.truetype("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf", 20)
        except Exception:
            title_font = ImageFont.load_default()
            artist_font = ImageFont.load_default()
            
        max_text_width = 180
        
        def wrap_text(text, font, max_width):
            words = text.split(' ')
            lines = []
            current_line = []
            for word in words:
                test_line = ' '.join(current_line + [word])
                if hasattr(font, 'getbbox'):
                    w = font.getbbox(test_line)[2]
                else:
                    w = font.getsize(test_line)[0]
                if w <= max_width:
                    current_line.append(word)
                else:
                    if current_line:
                        lines.append(' '.join(current_line))
                        current_line = [word]
                    else:
                        lines.append(word)
            if current_line:
                lines.append(' '.join(current_line))
            return lines

        title_lines = wrap_text(title, title_font, max_text_width)
        artist_lines = wrap_text(artist, artist_font, max_text_width)
        
        if len(title_lines) > 2:
            title_lines = title_lines[:2]
            title_lines[-1] += "..."
        if len(artist_lines) > 2:
            artist_lines = artist_lines[:2]
            artist_lines[-1] += "..."
            
        y = 40
        for line in title_lines:
            draw.text((200, y), line, fill=(255, 255, 255), font=title_font)
            if hasattr(title_font, 'getbbox'):
                h = title_font.getbbox(line)[3] - title_font.getbbox(line)[1]
            else:
                h = title_font.getsize(line)[1]
            y += h + 6
            
        y = max(y + 4, 110)
        for line in artist_lines:
            draw.text((200, y), line, fill=(180, 200, 240), font=artist_font)
            if hasattr(artist_font, 'getbbox'):
                h = artist_font.getbbox(line)[3] - artist_font.getbbox(line)[1]
            else:
                h = artist_font.getsize(line)[1]
            y += h + 4
            
        # 4. Resize to 196x196 (squeeze horizontally)
        img_squeezed = img.resize((196, 196), Image.Resampling.LANCZOS)
        
        # 5. Save image
        img_squeezed.save(output_path, 'PNG')

    def start(self):
        """Start the daemon"""
        logger.info("Starting Ulanzi daemon...")

        try:
            # Load configuration
            self.config = ConfigParser.load(self.config_path)
            self._ensure_button_13(self.config)

            # Validate configuration
            errors = ConfigParser.validate(self.config)
            if errors:
                logger.error("Configuration errors:")
                for error in errors:
                    logger.error(f"  - {error}")
                return False

            # Connect to device
            self.device = UlanziDevice()
            self.device.set_button_callback(self._on_button_press)

            # Initialize OBS client if configured
            self._init_obs_client()

            # Initialize action executor
            self.executor = ActionExecutor(self.obs_client)

            # Configure device
            self._configure_device()

            self.running = True
            logger.info("Daemon started successfully")
            
            # Track PID
            pid_file = Path.home() / '.local/share/ulanzi/daemon.pid'
            try:
                pid_file.parent.mkdir(parents=True, exist_ok=True)
                pid_file.write_text(str(os.getpid()))
            except Exception as e:
                logger.warning(f"Could not write PID file: {e}")
                
            return True

        except Exception as e:
            logger.error(f"Failed to start daemon: {e}")
            return False

    def stop(self):
        """Stop the daemon"""
        logger.info("Stopping daemon...")
        self.running = False

        if self.device:
            self.device.close()

        if self.obs_client:
            try:
                self.obs_client.disconnect()
            except:
                pass

        # Remove PID file
        pid_file = Path.home() / '.local/share/ulanzi/daemon.pid'
        if pid_file.exists():
            try:
                pid_file.unlink()
            except Exception:
                pass

        logger.info("Daemon stopped")

    def run(self):
        """Run the daemon main loop"""
        if not self.start():
            return

        # Setup signal handlers
        signal.signal(signal.SIGTERM, lambda s, f: self.stop())
        signal.signal(signal.SIGINT, lambda s, f: self.stop())
        try:
            signal.signal(signal.SIGHUP, lambda s, f: self.reload_config())
        except AttributeError:
            pass

        try:
            last_keepalive = time.time()
            last_stats_update = 0.0
            self.last_interaction_time = time.time()
            self.is_sleeping = False
            self.last_button_dict = None
            
            while self.running:
                # Read button presses (non-blocking)
                self.device.read_button_press()

                now = time.time()
                inactive_time = now - self.last_interaction_time
                
                # Check sleep timeout
                sleep_timeout = getattr(self.config, 'sleep_timeout', 10)
                sleep_brightness = getattr(self.config, 'sleep_brightness', 0)
                
                if sleep_timeout > 0 and inactive_time > sleep_timeout * 60:
                    if not self.is_sleeping:
                        self.is_sleeping = True
                        logger.info("Device entering sleep mode due to inactivity.")
                        try:
                            self.device.set_brightness(sleep_brightness)
                        except Exception as e:
                            logger.error(f"Failed to set sleep brightness: {e}")
                else:
                    # Device should be awake
                    if self.is_sleeping:
                        self.is_sleeping = False
                        logger.info("Device waking up from sleep mode.")
                        try:
                            self.device.set_brightness(self.config.brightness)
                        except Exception as e:
                            logger.error(f"Failed to restore brightness: {e}")
                            
                    # Keepalive: if sleep_timeout is 0 (Never sleep), send periodic brightness commands
                    # every 60 seconds of inactivity to keep USB/screen alive.
                    if sleep_timeout == 0 and now - last_keepalive > 60:
                        last_keepalive = now
                        logger.debug("Sending periodic keepalive brightness command")
                        try:
                            self.device.set_brightness(self.config.brightness)
                        except Exception:
                            pass

                # Periodically update system statistics & dynamic labels
                if now - last_stats_update > 2.0:
                    last_stats_update = now
                    
                    stats = self._get_system_stats()
                    media = self._get_mpris_media_info()
                    
                    # Update status screen according to clock mode
                    clock_mode = getattr(self.config, 'clock_mode', 1)
                    # For hardware, MEDIA mode (3) is sent as BACKGROUND mode (2)
                    hw_mode = 2 if clock_mode == 3 else clock_mode
                    try:
                        self.device.set_small_window_data({
                            'mode': hw_mode,
                            'cpu': stats.get('cpu', 0),
                            'mem': stats.get('ram', 0),
                            'gpu': 0,
                            'time': time.strftime('%H:%M:%S')
                        })
                    except Exception as e:
                        logger.debug(f"Failed to update small screen: {e}")
                        
                    # Rebuild buttons with dynamically resolved labels & icons
                    button_dict = {}
                    for button in self.config.buttons:
                        resolved_label = self._resolve_variables(button.label, stats, media)
                        image_path = button.image
                        
                        # Special handling for clock button (13) in MEDIA mode
                        if button.index == 13 and clock_mode == 3:
                            title = media.get('title', 'No Media')
                            artist = media.get('artist', 'Unknown')
                            status = media.get('status', 'Stopped')
                            art_url = media.get('artUrl', '')
                            
                            # Create a unique hash for the current media state
                            state_str = f"{title}|{artist}|{status}|{art_url}"
                            state_hash = hashlib.md5(state_str.encode('utf-8')).hexdigest()[:12]
                            
                            media_icon_dir = Path.home() / '.local/share/ulanzi/icons'
                            media_icon_dir.mkdir(parents=True, exist_ok=True)
                            image_path = str(media_icon_dir / f"media_clock_{state_hash}.png")
                            
                            # Generate the image if it doesn't exist
                            if not os.path.exists(image_path):
                                # Clean up any existing media_clock_*.png files to prevent leak
                                for old_file in media_icon_dir.glob("media_clock_*.png"):
                                    try:
                                        old_file.unlink()
                                    except Exception:
                                        pass
                                
                                # Fetch album art path (local or cached remote)
                                local_art_path = self._fetch_art_async(art_url, media_icon_dir)
                                self._generate_media_clock_image(title, artist, local_art_path, Path(image_path))
                                
                            resolved_label = ""
                        
                        # Auto-toggle play/pause media buttons if icons are generated/available
                        if button.action_type == 'media' and button.action_params.get('control') == 'play_pause':
                            if media.get('status') == 'Playing':
                                if 'play' in image_path:
                                    pause_path = image_path.replace('play', 'pause')
                                    if Path(pause_path).exists():
                                        image_path = pause_path
                            else:
                                if 'pause' in image_path:
                                    play_path = image_path.replace('pause', 'play')
                                    if Path(play_path).exists():
                                        image_path = play_path
                                        
                        button_dict[button.index] = {
                            'image': image_path,
                            'label': resolved_label,
                            'state': button.state
                        }
                        
                    # Push changes to device only if resolved state has modified
                    if button_dict != self.last_button_dict:
                        try:
                            self.device.set_buttons(button_dict)
                            self.last_button_dict = button_dict
                        except Exception as e:
                            logger.error(f"Failed to update buttons in loop: {e}")

                time.sleep(0.1)

        except KeyboardInterrupt:
            logger.info("Interrupted by user")
        except Exception as e:
            logger.error(f"Daemon error: {e}")
        finally:
            self.stop()

    def reload_config(self):
        """Reload configuration from file without stopping"""
        logger.info("Reloading configuration...")
        try:
            # Load new configuration
            new_config = ConfigParser.load(self.config_path)
            self._ensure_button_13(new_config)
            
            # Validate
            errors = ConfigParser.validate(new_config)
            if errors:
                logger.error("Failed to reload configuration. Validation errors:")
                for error in errors:
                    logger.error(f"  - {error}")
                return False
                
            self.config = new_config
            
            # Reconfigure device settings
            self._configure_device()
            
            # Reinitialize OBS client if config changed
            if self.obs_client:
                try:
                    self.obs_client.disconnect()
                except:
                    pass
                self.obs_client = None
            self._init_obs_client()
            
            # Reinitialize executor with new OBS client
            self.executor = ActionExecutor(self.obs_client)
            
            logger.info("Configuration reloaded successfully")
            return True
        except Exception as e:
            logger.error(f"Failed to reload configuration: {e}")
            return False

    def _init_obs_client(self):
        """Initialize OBS WebSocket client"""
        try:
            import obsws_python as obs

            # Silence obsws_python's internal logger to suppress stack traces on connection refusal
            obs_logger = logging.getLogger('obsws_python')
            old_level = obs_logger.level
            obs_logger.setLevel(logging.CRITICAL)

            try:
                self.obs_client = obs.ReqClient(
                    host=self.config.obs_host,
                    port=self.config.obs_port,
                    password=self.config.obs_password,
                    timeout=3
                )
                logger.info(f"Connected to OBS at {self.config.obs_host}:{self.config.obs_port}")
            finally:
                obs_logger.setLevel(old_level)
        except ImportError:
            logger.warning("obsws-python not installed, OBS features disabled")
        except ConnectionRefusedError:
            logger.warning(f"Could not connect to OBS at {self.config.obs_host}:{self.config.obs_port} - is it running?")
        except Exception as e:
            logger.warning(f"Failed to connect to OBS: {type(e).__name__}: {e}")

    def _configure_device(self):
        """Configure device with settings from config"""
        try:
            # Reset cache so loop re-evaluates immediately
            self.last_button_dict = None
            
            # Set brightness
            self.device.set_brightness(self.config.brightness)

            # Set label style
            if self.config.label_style is not None:
                style = dict(self.config.label_style)
                if getattr(self.config, 'hide_labels', False):
                    style['ShowTitle'] = False
                else:
                    style.setdefault('ShowTitle', True)
                self.device.set_label_style(style)

            # Configure small status display
            clock_mode = getattr(self.config, 'clock_mode', 1)
            try:
                self.device.set_small_window_data({
                    'mode': clock_mode,
                    'cpu': 0,
                    'mem': 0,
                    'gpu': 0,
                    'time': time.strftime('%H:%M:%S')
                })
            except Exception as e:
                logger.debug(f"Failed to set initial clock mode: {e}")

            # Set button images (initially resolved using empty stats to avoid block)
            button_dict = {}
            stats = {'cpu': 0, 'ram': 0, 'temp': 0}
            media = {'title': 'No Media', 'artist': 'Unknown', 'status': 'Stopped'}
            for button in self.config.buttons:
                resolved_label = self._resolve_variables(button.label, stats, media)
                button_dict[button.index] = {
                    'image': button.image,
                    'label': resolved_label,
                    'state': button.state
                }

            if button_dict:
                self.device.set_buttons(button_dict)
                self.last_button_dict = button_dict

            logger.info("Device configured successfully")
        except Exception as e:
            logger.error(f"Failed to configure device: {e}")

    def _on_button_press(self, button: ButtonPress):
        """Handle button press event"""
        self.last_interaction_time = time.time()
        
        # If we were sleeping, wake up!
        if self.is_sleeping:
            self.is_sleeping = False
            logger.info("Waking up device from sleep mode on user interaction")
            try:
                self.device.set_brightness(self.config.brightness)
            except Exception as e:
                logger.error(f"Failed to restore brightness on wake-up: {e}")
                
        # If it is a dial event, handle it separately
        if button.dial_event is not None:
            dial_config = self.config.dials.get(button.index)
            if not dial_config:
                logger.warning(f"No config for dial {button.index}")
                return

            action = None
            event_name = ""
            if button.dial_event == 0:  # click release
                action = dial_config.get('click')
                event_name = "click"
            elif button.dial_event == 1:  # click press
                logger.info(f"Dial {button.index} clicked (pressed)")
                return
            elif button.dial_event == 2:  # turn left
                action = dial_config.get('left')
                event_name = "left"
            elif button.dial_event == 3:  # turn right
                action = dial_config.get('right')
                event_name = "right"

            if action:
                action_type = action.get('action')
                action_params = action.get('params', {})
                logger.info(f"Executing dial {button.index} action on '{event_name}': {action_type}")
                if self.executor:
                    self.executor.execute(action_type, action_params)
            else:
                logger.debug(f"Dial {button.index} event '{event_name}' has no configured action")
            return

        # Handle regular buttons
        logger.info(f"Button {button.index} pressed (state={button.state})")

        # Find button config
        button_config = None
        for btn in self.config.buttons:
            if btn.index == button.index:
                button_config = btn
                break

        if not button_config and button.index != 13:
            logger.warning(f"No config for button {button.index}")
            return

        # Execute action
        if self.executor and not button.pressed:
            has_custom_action = False
            if button_config:
                act_type = button_config.action_type
                params = button_config.action_params or {}
                if act_type == 'command' and params.get('cmd'):
                    has_custom_action = True
                elif act_type == 'app' and params.get('name'):
                    has_custom_action = True
                elif act_type == 'key' and params.get('keys'):
                    has_custom_action = True
                elif act_type == 'obs' and params.get('action'):
                    has_custom_action = True
                elif act_type == 'volume' and params.get('operation'):
                    has_custom_action = True
                elif act_type == 'media' and params.get('control'):
                    has_custom_action = True

            if has_custom_action:
                logger.info(f"Executing custom action: {button_config.action_type} - {button_config.label}")
                self.executor.execute(button_config.action_type, button_config.action_params)
            elif button.index == 13:
                # Default actions for clock button based on mode
                clock_mode = getattr(self.config, 'clock_mode', 1)
                if clock_mode == 3: # MEDIA
                    logger.info("Executing default media action for clock button: play_pause")
                    self.executor.execute('media', {'control': 'play_pause'})
                else:
                    logger.warning(f"No custom action or default action for button {button.index} in mode {clock_mode}")


def main():
    """Main entry point"""
    import argparse

    parser = argparse.ArgumentParser(description='Ulanzi D200 daemon')
    parser.add_argument('config', nargs='?', default=str(Path.home() / '.config' / 'ulanzi' / 'config.yaml'), help='Path to configuration file')
    parser.add_argument('--log-level', default='INFO', help='Logging level')
    args = parser.parse_args()

    # Set log level
    logging.getLogger().setLevel(getattr(logging, args.log_level.upper()))

    # Create and run daemon
    daemon = UlanziDaemon(args.config)
    daemon.run()


if __name__ == '__main__':
    main()
