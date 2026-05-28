"""Action handlers for button presses"""

import subprocess
import logging
from typing import Dict, Any, Optional
from abc import ABC, abstractmethod

logger = logging.getLogger(__name__)


class ActionHandler(ABC):
    """Base class for action handlers"""

    @abstractmethod
    def execute(self, params: Dict[str, Any]):
        """Execute the action"""
        pass


class CommandAction(ActionHandler):
    """Execute shell commands"""

    def execute(self, params: Dict[str, Any]):
        """Execute shell command"""
        cmd = params.get('cmd')
        if not cmd:
            logger.error("Command action requires 'cmd' parameter")
            return

        try:
            subprocess.Popen(cmd, shell=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            logger.info(f"Executed command: {cmd}")
        except Exception as e:
            logger.error(f"Failed to execute command: {e}")


class AppAction(ActionHandler):
    """Launch applications"""

    def execute(self, params: Dict[str, Any]):
        """Launch application"""
        app_name = params.get('name')
        if not app_name:
            logger.error("App action requires 'name' parameter")
            return

        try:
            subprocess.Popen([app_name], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            logger.info(f"Launched application: {app_name}")
        except Exception as e:
            logger.error(f"Failed to launch application: {e}")


class KeyAction(ActionHandler):
    """Simulate keyboard input"""

    def execute(self, params: Dict[str, Any]):
        """Simulate keyboard input"""
        keys = params.get('keys')
        if not keys:
            logger.error("Key action requires 'keys' parameter")
            return

        try:
            # Try xdotool first (most common on Linux)
            subprocess.run(['xdotool', 'key', keys], check=True, capture_output=True)
            logger.info(f"Sent keys: {keys}")
        except FileNotFoundError:
            logger.error("xdotool not found. Install it with: sudo apt install xdotool")
        except Exception as e:
            logger.error(f"Failed to send keys: {e}")


class OBSAction(ActionHandler):
    """Control OBS Studio via WebSocket"""

    def __init__(self, obs_client=None):
        """Initialize OBS action handler"""
        self.obs_client = obs_client

    def execute(self, params: Dict[str, Any]):
        """Execute OBS action"""
        if not self.obs_client:
            logger.error("OBS client not connected")
            return

        action = params.get('action', 'toggle_scene')

        try:
            if action == 'toggle_scene':
                self._toggle_scene(params)
            elif action == 'set_scene':
                self._set_scene(params)
            elif action == 'toggle_source':
                self._toggle_source(params)
            elif action == 'toggle_recording':
                self._toggle_recording(params)
            elif action == 'toggle_streaming':
                self._toggle_streaming(params)
            else:
                logger.error(f"Unknown OBS action: {action}")
        except Exception as e:
            logger.error(f"OBS action failed: {e}")

    def _toggle_scene(self, params: Dict[str, Any]):
        """Toggle between two scenes"""
        scene1 = params.get('scene1')
        scene2 = params.get('scene2')

        if not scene1 or not scene2:
            logger.error("toggle_scene requires 'scene1' and 'scene2' parameters")
            return

        try:
            current_scene = self.obs_client.get_current_program_scene()
            current_name = current_scene.current_program_scene_name

            target_scene = scene2 if current_name == scene1 else scene1
            self.obs_client.set_current_program_scene(target_scene)
            logger.info(f"Switched to scene: {target_scene}")
        except Exception as e:
            logger.error(f"Failed to toggle scene: {e}")

    def _set_scene(self, params: Dict[str, Any]):
        """Set active scene"""
        scene = params.get('scene')
        if not scene:
            logger.error("set_scene requires 'scene' parameter")
            return

        try:
            self.obs_client.set_current_program_scene(scene)
            logger.info(f"Set scene to: {scene}")
        except Exception as e:
            logger.error(f"Failed to set scene: {e}")

    def _toggle_source(self, params: Dict[str, Any]):
        """Toggle source visibility"""
        scene = params.get('scene')
        source = params.get('source')

        if not scene or not source:
            logger.error("toggle_source requires 'scene' and 'source' parameters")
            return

        try:
            # Get current visibility state
            item = self.obs_client.get_scene_item_id(scene, source)
            item_id = item.scene_item_id

            state = self.obs_client.get_scene_item_enabled(scene, item_id)
            enabled = state.scene_item_enabled

            # Toggle visibility
            self.obs_client.set_scene_item_enabled(scene, item_id, not enabled)
            logger.info(f"Toggled source '{source}' in scene '{scene}'")
        except Exception as e:
            logger.error(f"Failed to toggle source: {e}")

    def _toggle_recording(self, params: Dict[str, Any]):
        """Toggle recording"""
        try:
            status = self.obs_client.get_record_status()
            is_recording = status.output_active

            if is_recording:
                self.obs_client.stop_record()
                logger.info("Stopped recording")
            else:
                self.obs_client.start_record()
                logger.info("Started recording")
        except Exception as e:
            logger.error(f"Failed to toggle recording: {e}")

    def _toggle_streaming(self, params: Dict[str, Any]):
        """Toggle streaming"""
        try:
            status = self.obs_client.get_stream_status()
            is_streaming = status.output_active

            if is_streaming:
                self.obs_client.stop_stream()
                logger.info("Stopped streaming")
            else:
                self.obs_client.start_stream()
                logger.info("Started streaming")
        except Exception as e:
            logger.error(f"Failed to toggle streaming: {e}")


class VolumeAction(ActionHandler):
    """Control audio volume, devices, and applications on Linux"""

    def execute(self, params: Dict[str, Any]):
        target = params.get('target', 'default') # 'default', 'device', 'app'
        name = params.get('name', '') # device name or app name
        op = params.get('operation', 'up') # 'up', 'down', 'mute'
        step = params.get('step', 5)
        limit_100 = params.get('limit_100', True)

        import subprocess
        import re

        try:
            if target == 'default':
                if op == 'up':
                    if limit_100:
                        curr_vol = 0.0
                        res = subprocess.run(['wpctl', 'get-volume', '@DEFAULT_AUDIO_SINK@'], capture_output=True, text=True)
                        if res.returncode == 0:
                            match = re.search(r'Volume:\s*([0-9.]+)', res.stdout)
                            if match:
                                curr_vol = float(match.group(1))
                        if curr_vol >= 1.0:
                            return
                    subprocess.run(['wpctl', 'set-volume', '@DEFAULT_AUDIO_SINK@', f'{step}%+'])
                elif op == 'down':
                    subprocess.run(['wpctl', 'set-volume', '@DEFAULT_AUDIO_SINK@', f'{step}%-'])
                elif op == 'mute':
                    subprocess.run(['wpctl', 'set-mute', '@DEFAULT_AUDIO_SINK@', 'toggle'])
                    
            elif target == 'device':
                if not name:
                    logger.error("Device volume action requires 'name' parameter")
                    return
                if op == 'up':
                    if limit_100:
                        curr_vol = 0.0
                        res = subprocess.run(['wpctl', 'get-volume', name], capture_output=True, text=True)
                        if res.returncode == 0:
                            match = re.search(r'Volume:\s*([0-9.]+)', res.stdout)
                            if match:
                                curr_vol = float(match.group(1))
                        if curr_vol >= 1.0:
                            return
                    subprocess.run(['wpctl', 'set-volume', name, f'{step}%+'])
                elif op == 'down':
                    subprocess.run(['wpctl', 'set-volume', name, f'{step}%-'])
                elif op == 'mute':
                    subprocess.run(['wpctl', 'set-mute', name, 'toggle'])
                    
            elif target == 'app':
                if not name:
                    logger.error("App volume action requires 'name' parameter")
                    return
                # Get sink inputs from pactl
                res = subprocess.run(['pactl', 'list', 'sink-inputs'], capture_output=True, text=True)
                output = res.stdout
                
                inputs = re.split(r'Sink Input #', output)
                found = False
                for block in inputs[1:]:
                    lines = block.split('\n')
                    input_id = lines[0].strip()
                    if any(name.lower() in line.lower() for line in lines if 'application.name' in line or 'media.name' in line or 'process.binary' in line):
                        found = True
                        if op == 'up':
                            if limit_100:
                                curr_vol = 0
                                for line in lines:
                                    if 'Volume:' in line:
                                        match = re.search(r'(\d+)%', line)
                                        if match:
                                            curr_vol = int(match.group(1))
                                            break
                                if curr_vol >= 100:
                                    continue
                            subprocess.run(['pactl', 'set-sink-input-volume', input_id, f'+{step}%'])
                        elif op == 'down':
                            subprocess.run(['pactl', 'set-sink-input-volume', input_id, f'-{step}%'])
                        elif op == 'mute':
                            subprocess.run(['pactl', 'set-sink-input-mute', input_id, 'toggle'])
                if not found:
                    logger.warning(f"No running audio stream found for app: {name}")
        except Exception as e:
            logger.error(f"Failed to execute volume action: {e}")


class MediaAction(ActionHandler):
    """Control media playback (play/pause, next, prev, stop)"""

    def execute(self, params: Dict[str, Any]):
        cmd = params.get('control', 'play_pause')
        
        playerctl_map = {
            'play_pause': 'play-pause',
            'next': 'next',
            'previous': 'previous',
            'stop': 'stop'
        }
        
        xdotool_map = {
            'play_pause': 'XF86AudioPlay',
            'next': 'XF86AudioNext',
            'previous': 'XF86AudioPrev',
            'stop': 'XF86AudioStop'
        }
        
        # Try playerctl first
        try:
            player = params.get('player', '')
            cmd_args = ['playerctl']
            if player:
                cmd_args.extend(['--player', player])
            cmd_args.append(playerctl_map.get(cmd, 'play-pause'))
            
            res = subprocess.run(cmd_args, capture_output=True)
            if res.returncode == 0:
                logger.info(f"Executed media control via playerctl: {cmd}")
                return
        except FileNotFoundError:
            pass
            
        # Fallback to xdotool simulating media keys
        try:
            key = xdotool_map.get(cmd, 'XF86AudioPlay')
            subprocess.run(['xdotool', 'key', key], check=True, capture_output=True)
            logger.info(f"Simulated media key fallback: {key}")
        except Exception as e:
            logger.error(f"Failed to execute media control fallback: {e}")


class ActionExecutor:
    """Execute button actions"""

    def __init__(self, obs_client=None):
        """Initialize action executor"""
        self.handlers = {
            'command': CommandAction(),
            'app': AppAction(),
            'key': KeyAction(),
            'obs': OBSAction(obs_client),
            'volume': VolumeAction(),
            'media': MediaAction(),
        }

    def execute(self, action_type: str, params: Dict[str, Any]):
        """Execute action by type"""
        handler = self.handlers.get(action_type)
        if not handler:
            logger.error(f"Unknown action type: {action_type}")
            return

        try:
            handler.execute(params)
        except Exception as e:
            logger.error(f"Action execution failed: {e}")
