"""Configuration file parser for Ulanzi Manager"""

import yaml
import logging
from pathlib import Path
from typing import Dict, List, Optional, Any
from dataclasses import dataclass

logger = logging.getLogger(__name__)


@dataclass
class ButtonConfig:
    """Button configuration"""
    index: int
    image: Optional[str]
    label: str
    action_type: str  # 'command', 'obs', 'app', 'key'
    action_params: Dict[str, Any]
    state: int = 0
    icon_spec: Optional[Dict[str, Any]] = None


@dataclass
class Config:
    """Main configuration"""
    brightness: int = 100
    label_style: Dict[str, Any] = None
    buttons: List[ButtonConfig] = None
    obs_host: str = "localhost"
    obs_port: int = 4444
    obs_password: Optional[str] = None
    dials: Dict[int, Dict[str, Any]] = None
    sleep_timeout: int = 10
    sleep_brightness: int = 0
    hide_labels: bool = False

    def __post_init__(self):
        if self.label_style is None:
            self.label_style = {}
        if self.buttons is None:
            self.buttons = []
        if self.dials is None:
            self.dials = {}


class ConfigParser:
    """Parse YAML configuration files"""

    @staticmethod
    def load(config_path: str) -> Config:
        """Load configuration from YAML file"""
        config_file = Path(config_path)
        if not config_file.exists():
            raise FileNotFoundError(f"Config file not found: {config_path}")

        with open(config_file, 'r') as f:
            data = yaml.safe_load(f) or {}

        config = ConfigParser._parse_config(data, config_file.parent)
        ConfigParser._generate_icons(config, config_file.parent)
        return config

    @staticmethod
    def _parse_config(data: Dict, base_path: Path) -> Config:
        """Parse configuration dictionary"""
        config = Config()

        # Global settings
        if 'brightness' in data:
            config.brightness = int(data['brightness'])

        if 'sleep_timeout' in data:
            config.sleep_timeout = int(data['sleep_timeout'])

        if 'sleep_brightness' in data:
            config.sleep_brightness = int(data['sleep_brightness'])

        if 'hide_labels' in data:
            config.hide_labels = bool(data['hide_labels'])

        if 'label_style' in data:
            config.label_style = data['label_style']

        # OBS settings
        if 'obs' in data:
            obs_config = data['obs']
            config.obs_host = obs_config.get('host', 'localhost')
            config.obs_port = obs_config.get('port', 4444)
            config.obs_password = obs_config.get('password')

        # Parse buttons
        buttons = []
        if 'buttons' in data:
            for idx, button_data in enumerate(data['buttons']):
                if button_data is None:
                    continue

                button = ConfigParser._parse_button(idx, button_data, base_path)
                buttons.append(button)

        # Parse dials
        dials = {}
        if 'dials' in data and isinstance(data['dials'], dict):
            for dial_idx_str, dial_data in data['dials'].items():
                try:
                    dial_idx = int(dial_idx_str)
                    if dial_data:
                        dials[dial_idx] = dial_data
                except ValueError:
                    logger.warning(f"Invalid dial index: {dial_idx_str}")

        config.buttons = buttons
        config.dials = dials
        logger.info(f"Loaded config with {len(buttons)} button(s) and {len(dials)} dial(s)")
        return config

    @staticmethod
    def _parse_button(index: int, data: Dict, base_path: Path) -> ButtonConfig:
        """Parse button configuration"""
        # Resolve image path relative to config file
        image = data.get('image')
        if image:
            image_path = Path(image)
            if not image_path.is_absolute():
                image_path = base_path / image_path
            image = str(image_path)

        label = data.get('label', f'Button {index}')
        action_type = data.get('action', 'command')
        action_params = data.get('params', {})
        state = data.get('state', 0)
        icon_spec = data.get('icon_spec')

        return ButtonConfig(
            index=index,
            image=image,
            label=label,
            action_type=action_type,
            action_params=action_params,
            state=state,
            icon_spec=icon_spec
        )

    @staticmethod
    def _validate_action(action_type: str, action_params: Dict[str, Any], context: str) -> List[str]:
        """Validate action configuration properties"""
        errors = []
        if action_type not in ['command', 'obs', 'app', 'key', 'volume', 'media']:
            errors.append(f"{context}: invalid action type: {action_type}")
            return errors

        # We treat missing or empty parameters as "unset" or "no-op" actions, 
        # allowing the daemon to run on a best-effort basis without halting.
        
        if action_type == 'command':
            pass

        elif action_type == 'obs':
            action = action_params.get('action')
            if action:
                if action == 'toggle_scene':
                    if not action_params.get('scene1') or not action_params.get('scene2'):
                        errors.append(f"{context}: 'toggle_scene' action requires 'scene1' and 'scene2' parameters")
                elif action == 'set_scene':
                    if not action_params.get('scene'):
                        errors.append(f"{context}: 'set_scene' action requires 'scene' parameter")
                elif action == 'toggle_source':
                    if not action_params.get('scene') or not action_params.get('source'):
                        errors.append(f"{context}: 'toggle_source' action requires 'scene' and 'source' parameters")

        elif action_type == 'app':
            pass

        elif action_type == 'key':
            pass

        elif action_type == 'volume':
            operation = action_params.get('operation')
            if operation:
                if operation not in ['up', 'down', 'mute']:
                    errors.append(f"{context}: 'volume' action requires 'operation' parameter ('up', 'down', 'mute')")

        elif action_type == 'media':
            control = action_params.get('control')
            if control:
                if control not in ['play_pause', 'next', 'previous', 'stop']:
                    errors.append(f"{context}: 'media' action requires 'control' parameter ('play_pause', 'next', 'previous', 'stop')")

        return errors

    @staticmethod
    def _generate_icons(config: Config, base_path: Path) -> None:
        """Generate icons from specs and update image paths"""
        try:
            from .icon_generator import IconGenerator
        except ImportError:
            logger.warning("Pillow not installed, skipping icon generation")
            return

        icon_dir = base_path / 'icons'
        icon_dir.mkdir(exist_ok=True)
        generator = IconGenerator(cache_dir=icon_dir)

        for button in config.buttons:
            if button and button.icon_spec:
                try:
                    logger.info(f"Generating icon for button {button.index}")
                    # Use specific filename and always regenerate
                    icon_path = generator.generate_from_dict(button.icon_spec, button_index=button.index, force=True)
                    button.image = str(icon_path)
                except Exception as e:
                    logger.error(f"Failed to generate icon for button {button.index}: {e}")

    @staticmethod
    def validate(config: Config) -> List[str]:
        """Validate configuration and return list of errors"""
        errors = []

        if config.brightness < 0 or config.brightness > 100:
            errors.append("brightness must be between 0 and 100")

        if config.obs_port < 1 or config.obs_port > 65535:
            errors.append("obs.port must be between 1 and 65535")

        for button in config.buttons:
            if button is None:
                continue

            if button.image and not Path(button.image).exists():
                errors.append(f"Button {button.index}: image file not found: {button.image}")

            # Validate icon_spec if present
            if button.icon_spec:
                try:
                    from .icon_generator import IconSpec
                    spec = IconSpec(button.icon_spec)
                    spec_errors = spec.validate()
                    for error in spec_errors:
                        errors.append(f"Button {button.index}: icon_spec error: {error}")
                except Exception as e:
                    errors.append(f"Button {button.index}: icon_spec error: {str(e)}")

            errors.extend(
                ConfigParser._validate_action(
                    button.action_type,
                    button.action_params,
                    f"Button {button.index}"
                )
            )

        if config.dials:
            for dial_idx, dial_config in config.dials.items():
                if dial_idx not in (17, 18, 19):
                    errors.append(f"Dial {dial_idx}: invalid dial index. Expected 17, 18, or 19.")
                    continue

                for event_name in ('click', 'left', 'right'):
                    if event_name in dial_config:
                        event_action = dial_config[event_name]
                        if not isinstance(event_action, dict):
                            errors.append(f"Dial {dial_idx} event '{event_name}' must be an action configuration dictionary")
                            continue

                        action_type = event_action.get('action')
                        action_params = event_action.get('params', {})

                        if not action_type:
                            errors.append(f"Dial {dial_idx} event '{event_name}' requires 'action' field")
                            continue

                        errors.extend(
                            ConfigParser._validate_action(
                                action_type,
                                action_params,
                                f"Dial {dial_idx} event '{event_name}'"
                            )
                        )

        return errors
