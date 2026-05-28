"""Background daemon for Ulanzi Manager"""

import sys
import time
import logging
import signal
import os
from pathlib import Path
from typing import Optional

from ulanzi_manager.device import UlanziDevice, ButtonPress
from ulanzi_manager.config import ConfigParser, Config
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

    def start(self):
        """Start the daemon"""
        logger.info("Starting Ulanzi daemon...")

        try:
            # Load configuration
            self.config = ConfigParser.load(self.config_path)

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
            self.last_interaction_time = time.time()
            self.is_sleeping = False
            
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

                # Keep-alive
                self.device.set_small_window_data({})

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

            self.obs_client = obs.ReqClient(
                host=self.config.obs_host,
                port=self.config.obs_port,
                password=self.config.obs_password,
                timeout=3
            )
            logger.info(f"Connected to OBS at {self.config.obs_host}:{self.config.obs_port}")
        except ImportError:
            logger.warning("obsws-python not installed, OBS features disabled")
        except ConnectionRefusedError:
            logger.warning(f"Could not connect to OBS at {self.config.obs_host}:{self.config.obs_port} - is it running?")
        except Exception as e:
            logger.warning(f"Failed to connect to OBS: {type(e).__name__}: {e}")

    def _configure_device(self):
        """Configure device with settings from config"""
        try:
            # Set brightness
            self.device.set_brightness(self.config.brightness)

            # Set label style
            if self.config.label_style:
                self.device.set_label_style(self.config.label_style)

            # Set button images
            button_dict = {}
            for button in self.config.buttons:
                button_dict[button.index] = {
                    'image': button.image,
                    'label': button.label,
                    'state': button.state
                }

            if button_dict:
                self.device.set_buttons(button_dict)

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

        if not button_config:
            logger.warning(f"No config for button {button.index}")
            return

        logger.info(f"Executing action: {button_config.action_type} - {button_config.label}")

        # Execute action
        if self.executor and not button.pressed:
            self.executor.execute(button_config.action_type, button_config.action_params)


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
