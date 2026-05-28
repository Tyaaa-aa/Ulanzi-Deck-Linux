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
from PyQt6.QtWidgets import QApplication, QMessageBox
from PyQt6.QtCore import Qt, QLockFile
from ulanzi_gui.main_window import MainWindow

def main():
    parser = argparse.ArgumentParser(description='Ulanzi D200X Graphical Manager')
    parser.add_argument('--config', default=str(Path.home() / '.config' / 'ulanzi' / 'config.yaml'),
                        help='Path to configuration file')
    args = parser.parse_args()
    
    app = QApplication(sys.argv)
    app.setStyle("Fusion") # Dark clean consistent theme
    
    # Enable High DPI scaling
    app.setAttribute(Qt.ApplicationAttribute.AA_UseHighDpiPixmaps) if hasattr(Qt.ApplicationAttribute, "AA_UseHighDpiPixmaps") else None
    
    # Prevent multiple instances using QLockFile
    lock_dir = Path.home() / '.local/share/ulanzi'
    lock_dir.mkdir(parents=True, exist_ok=True)
    lock_file = QLockFile(str(lock_dir / 'gui.lock'))
    if not lock_file.tryLock(100):
        QMessageBox.warning(
            None,
            "Already Running",
            "Ulanzi Manager is already running.\nPlease check your system tray icon."
        )
        sys.exit(0)
            
    # Launch main configuration window
    window = MainWindow(args.config)
    window.show()
    
    # Store lock_file reference to keep it alive during event loop execution
    window._lock_file = lock_file
    
    sys.exit(app.exec())

if __name__ == '__main__':
    main()
