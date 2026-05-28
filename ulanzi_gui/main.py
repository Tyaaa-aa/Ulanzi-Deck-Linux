import sys
from pathlib import Path

# Add project root directory to sys.path to allow running directly from cloned source directory
project_root = str(Path(__file__).resolve().parent.parent)
if project_root not in sys.path:
    sys.path.insert(0, project_root)

# Check if we should run as a daemon or CLI/manager command to support standalone/packaged mode
if len(sys.argv) > 1:
    if sys.argv[1] == '--daemon':
        sys.argv.pop(1)
        from ulanzi_manager.daemon import main as daemon_main
        daemon_main()
        sys.exit(0)
    elif sys.argv[1] == '--cli':
        sys.argv.pop(1)
        from ulanzi_manager.cli import main as cli_main
        cli_main()
        sys.exit(0)

# Import PyQt modules afterwards to avoid unnecessarily loading them if running in cli/daemon mode
import argparse
from PyQt6.QtWidgets import QApplication, QDialog
from PyQt6.QtCore import Qt
from ulanzi_gui.main_window import MainWindow
from ulanzi_gui.onboarding import OnboardingDialog
from ulanzi_manager.device import UlanziDevice

def main():
    parser = argparse.ArgumentParser(description='Ulanzi D200X Graphical Manager')
    parser.add_argument('--config', default=str(Path.home() / '.config' / 'ulanzi' / 'config.yaml'),
                        help='Path to configuration file')
    args = parser.parse_args()
    
    app = QApplication(sys.argv)
    app.setStyle("Fusion") # Dark clean consistent theme
    
    # Enable High DPI scaling
    app.setAttribute(Qt.ApplicationAttribute.AA_UseHighDpiPixmaps) if hasattr(Qt.ApplicationAttribute, "AA_UseHighDpiPixmaps") else None
    
    # Try connecting to the device
    device_available = False
    try:
        device = UlanziDevice()
        device.close()
        device_available = True
    except Exception as e:
        # If not connected or permission error, show onboarding setup
        print(f"Initial connection check failed: {e}. Opening onboarding setup...")
        
    if not device_available:
        onboarding = OnboardingDialog()
        if onboarding.exec() != QDialog.DialogCode.Accepted:
            # User canceled/closed onboarding without success
            print("Onboarding setup was not completed. Exiting.")
            sys.exit(0)
            
    # Launch main configuration window
    window = MainWindow(args.config)
    window.show()
    sys.exit(app.exec())

if __name__ == '__main__':
    main()
