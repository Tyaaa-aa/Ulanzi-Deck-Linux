"""CLI tool for Ulanzi Manager"""

import sys
import argparse
import logging
from pathlib import Path
from PIL import Image

from ulanzi_manager.device import UlanziDevice
from ulanzi_manager.config import ConfigParser

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)


class UlanziCLI:
    """Command-line interface for Ulanzi Manager"""

    def __init__(self):
        """Initialize CLI"""
        self.device = None

    def connect(self):
        """Connect to device"""
        try:
            self.device = UlanziDevice()
            logger.info("Connected to device")
        except Exception as e:
            logger.error(f"Failed to connect: {e}")
            sys.exit(1)

    def disconnect(self):
        """Disconnect from device"""
        if self.device:
            self.device.close()

    def cmd_status(self, args):
        """Show device status"""
        self.connect()
        try:
            logger.info("Device connected and ready")
        finally:
            self.disconnect()

    def cmd_brightness(self, args):
        """Set brightness"""
        self.connect()
        try:
            self.device.set_brightness(args.level)
            logger.info(f"Brightness set to {args.level}%")
        finally:
            self.disconnect()

    def cmd_configure(self, args):
        """Configure device from config file"""
        self.connect()
        try:
            config = ConfigParser.load(args.config)

            # Validate
            errors = ConfigParser.validate(config)
            if errors:
                logger.error("Configuration errors:")
                for error in errors:
                    logger.error(f"  - {error}")
                sys.exit(1)

            # Set brightness
            self.device.set_brightness(config.brightness)
            logger.info(f"Set brightness to {config.brightness}%")

            # Set label style
            if config.label_style is not None:
                style = dict(config.label_style)
                if getattr(config, 'hide_labels', False):
                    style['ShowTitle'] = False
                else:
                    style.setdefault('ShowTitle', True)
                self.device.set_label_style(style)
                logger.info("Set label style")

            # Set clock/small window mode
            clock_mode = getattr(config, 'clock_mode', 1)
            try:
                self.device.set_small_window_data({
                    'mode': clock_mode,
                    'cpu': 0,
                    'mem': 0,
                    'gpu': 0,
                    'time': '00:00:00'
                })
                logger.info(f"Set small window mode to {clock_mode}")
            except Exception as e:
                logger.warning(f"Failed to set initial clock mode: {e}")

            # Set buttons
            button_dict = {}
            for button in config.buttons:
                button_dict[button.index] = {
                    'image': button.image,
                    'label': button.label,
                    'state': button.state
                }

            if button_dict:
                self.device.set_buttons(button_dict)
                logger.info(f"Configured {len(button_dict)} button(s)")

            logger.info("Device configured successfully")

        except Exception as e:
            logger.error(f"Configuration failed: {e}")
            sys.exit(1)
        finally:
            self.disconnect()

    def cmd_test_image(self, args):
        """Test image on a button"""
        self.connect()
        try:
            # Validate image
            image_path = Path(args.image)
            if not image_path.exists():
                logger.error(f"Image not found: {args.image}")
                sys.exit(1)

            # Check image size
            img = Image.open(image_path)
            if img.size != (196, 196):
                logger.warning(f"Image size is {img.size}, expected (196, 196)")

            # Send to device
            button_dict = {
                args.button: {
                    'image': str(image_path),
                    'label': args.label or f'Button {args.button}',
                    'state': 0
                }
            }
            self.device.set_buttons(button_dict)
            logger.info(f"Sent image to button {args.button}")

        except Exception as e:
            logger.error(f"Test failed: {e}")
            sys.exit(1)
        finally:
            self.disconnect()

    def cmd_daemon(self, args):
        """Start daemon"""
        from ulanzi_manager.daemon import UlanziDaemon

        daemon = UlanziDaemon(args.config)
        daemon.run()

    def cmd_debug(self, args):
        """Debug mode - show button presses"""
        self.connect()
        try:
            import time
            logger.info("Debug mode: Press buttons to see their index")
            logger.info("Button layout:")
            logger.info("  0  1  2  3  4")
            logger.info("  5  6  7  8  9")
            logger.info(" 10 11 12")
            logger.info(" 13 (Clock/Big Button)")
            logger.info("")
            logger.info("Waiting for button presses (Ctrl+C to exit)...")
            logger.info("")

            while True:
                button = self.device.read_button_press()
                if button:
                    button_name = "Clock" if button.index == 13 else f"Button {button.index}"
                    logger.info(f">>> {button_name} PRESSED (index={button.index}, state={button.state}) <<<")
                time.sleep(0.05)

        except KeyboardInterrupt:
            logger.info("Debug mode stopped")
        finally:
            self.disconnect()

    def cmd_validate(self, args):
        """Validate configuration file"""
        try:
            config = ConfigParser.load(args.config)
            errors = ConfigParser.validate(config)

            if errors:
                logger.error("Configuration errors:")
                for error in errors:
                    logger.error(f"  - {error}")
                sys.exit(1)
            else:
                logger.info("Configuration is valid")

        except Exception as e:
            logger.error(f"Validation failed: {e}")
            sys.exit(1)

    def cmd_generate_config(self, args):
        """Generate example configuration file"""
        example_config = """# Ulanzi D200 Configuration

# Global settings
brightness: 100

# Label styling
label_style:
  Align: bottom
  Color: 0xFFFFFF
  FontName: Roboto
  ShowTitle: true
  Size: 10
  Weight: 80

# OBS Studio settings (optional)
obs:
  host: localhost
  port: 4444
  password: null

# Button definitions (13 buttons total, 0-12)
buttons:
  # Button 0 - Launch Firefox
  - image: ./icons/firefox.png
    label: Firefox
    action: app
    params:
      name: firefox

  # Button 1 - Toggle OBS scene
  - image: ./icons/obs.png
    label: Scene
    action: obs
    params:
      action: toggle_scene
      scene1: "Scene 1"
      scene2: "Scene 2"

  # Button 2 - Execute command
  - image: ./icons/terminal.png
    label: Terminal
    action: command
    params:
      cmd: "gnome-terminal"

  # Button 3 - Keyboard shortcut
  - image: ./icons/keyboard.png
    label: Shortcut
    action: key
    params:
      keys: "ctrl+alt+t"

  # Button 4 - Toggle recording
  - image: ./icons/record.png
    label: Record
    action: obs
    params:
      action: toggle_recording

  # Buttons 5-12 - Empty (optional)
  - null
  - null
  - null
  - null
  - null
  - null
  - null
  - null

# Dial/Rotary Encoder definitions (Ulanzi D200X only, indexed 17-19)
# dials:
#   17:
#     click:
#       action: command
#       params:
#         cmd: "amixer set Master toggle"
#     left:
#       action: command
#       params:
#         cmd: "amixer set Master 5%-"
#     right:
#       action: command
#       params:
#         cmd: "amixer set Master 5%+"
#
#   18:
#     left:
#       action: command
#       params:
#         cmd: "brightnessctl set 10%-"
#     right:
#       action: command
#       params:
#         cmd: "brightnessctl set +10%"
#
#   19:
#     click:
#       action: app
#       params:
#         name: firefox
"""
        output_path = Path(args.output)
        output_path.parent.mkdir(parents=True, exist_ok=True)
        output_path.write_text(example_config)
        logger.info(f"Example configuration written to {output_path}")


def main():
    """Main entry point"""
    parser = argparse.ArgumentParser(
        description='Ulanzi D200 Manager - Control your StreamDeck device',
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  ulanzi-manager status                          # Check device status
  ulanzi-manager brightness 80                   # Set brightness to 80%
  ulanzi-manager configure config.yaml           # Configure device from file
  ulanzi-manager test-image 0 icon.png           # Test image on button 0
  ulanzi-manager validate config.yaml            # Validate configuration
  ulanzi-manager generate-config config.yaml     # Generate example config
  ulanzi-manager debug                           # Debug mode - show button presses
  ulanzi-daemon config.yaml                      # Start background daemon
        """
    )

    subparsers = parser.add_subparsers(dest='command', help='Command to execute')

    # Status command
    subparsers.add_parser('status', help='Show device status')

    # Brightness command
    brightness_parser = subparsers.add_parser('brightness', help='Set brightness')
    brightness_parser.add_argument('level', type=int, help='Brightness level (0-100)')

    # Configure command
    configure_parser = subparsers.add_parser('configure', help='Configure device from file')
    configure_parser.add_argument('config', nargs='?', default=str(Path.home() / '.config' / 'ulanzi' / 'config.yaml'), help='Path to configuration file')

    # Test image command
    test_parser = subparsers.add_parser('test-image', help='Test image on button')
    test_parser.add_argument('button', type=int, help='Button index (0-12)')
    test_parser.add_argument('image', help='Path to image file')
    test_parser.add_argument('--label', help='Button label')

    # Validate command
    validate_parser = subparsers.add_parser('validate', help='Validate configuration')
    validate_parser.add_argument('config', nargs='?', default=str(Path.home() / '.config' / 'ulanzi' / 'config.yaml'), help='Path to configuration file')

    # Generate config command
    generate_parser = subparsers.add_parser('generate-config', help='Generate example configuration')
    generate_parser.add_argument('output', help='Output file path')

    # Daemon command
    daemon_parser = subparsers.add_parser('daemon', help='Start background daemon')
    daemon_parser.add_argument('config', nargs='?', default=str(Path.home() / '.config' / 'ulanzi' / 'config.yaml'), help='Path to configuration file')

    # Debug command
    debug_parser = subparsers.add_parser('debug', help='Debug mode - show button presses')

    args = parser.parse_args()

    if not args.command:
        parser.print_help()
        sys.exit(1)

    cli = UlanziCLI()

    # Execute command
    command_method = getattr(cli, f'cmd_{args.command.replace("-", "_")}', None)
    if command_method:
        command_method(args)
    else:
        logger.error(f"Unknown command: {args.command}")
        sys.exit(1)


if __name__ == '__main__':
    main()
