import os
import signal
import subprocess
from pathlib import Path
from PyQt6.QtWidgets import (QMainWindow, QWidget, QVBoxLayout, QHBoxLayout, 
                             QGridLayout, QPushButton, QLabel, QComboBox, 
                             QLineEdit, QTabWidget, QFormLayout, QFrame, 
                             QMessageBox, QScrollArea, QStyle, QDialog,
                             QApplication, QProgressDialog, QSplitter, QSpinBox,
                             QStyleOptionButton, QSystemTrayIcon, QMenu, QCheckBox)
from PyQt6.QtGui import QIcon, QPixmap, QColor, QPainter, QDrag, QPen, QAction
from PyQt6.QtCore import Qt, QSize, QTimer, QRect, QThread, pyqtSignal, QMimeData

from ulanzi_manager.config import ConfigParser, Config, ButtonConfig
from ulanzi_gui.app_picker import AppPicker
from ulanzi_gui.icon_picker import IconPicker
from ulanzi_gui.volume_picker import VolumePicker
from ulanzi_gui.soundboard import SoundboardPicker
from ulanzi_gui.key_recorder import KeyShortcutBuilder
from ulanzi_manager.device import UlanziDevice, hid, VENDOR_ID, PRODUCT_ID

# Layout dimensions
GRID_COLS = 5
GRID_ROWS = 3  # 0-12 buttons
BUTTON_COUNT = 13
SMALL_BUTTON_INDEXES = [15, 16]  # Small buttons beside the dials
DIAL_COUNT = 3  # 17, 18, 19
DIAL_HARDWARE_INDEXES = [17, 18, 19]

class ApplyWorker(QThread):
    """
    Runs stop-daemon → ulanzi-manager configure → restart-daemon
    in a background thread so the Qt main thread never blocks.

    Stop strategy is thorough:
    - systemd unit (if active) is stopped first
    - ALL ulanzi-daemon processes are killed by name via pkill
    - We verify the HID device node is free before running configure
    """
    progress = pyqtSignal(str)        # label text for progress dialog
    finished = pyqtSignal(bool, str)  # (success, error_message)

    CONFIGURE_TIMEOUT = 30            # seconds before giving up on configure
    DEVICE_FREE_TIMEOUT = 6           # seconds to wait for device fd to free

    def __init__(self, config_path, was_running, is_systemd, pid):
        super().__init__()
        self.config_path = config_path
        self.was_running = was_running
        self.is_systemd  = is_systemd
        self.pid         = pid

    # ── helpers ───────────────────────────────────────────────────────────
    @staticmethod
    def _kill_all_daemons():
        """Kill every ulanzi-daemon process on the system, not just the one we know about."""
        import os, signal
        my_pid = os.getpid()
        patterns = ['ulanzi-daemon', 'ulanzi_manager.daemon', '--daemon']
        pids_to_kill = set()
        
        # Kill by PID file first
        pid_file = Path.home() / '.local/share/ulanzi/daemon.pid'
        if pid_file.exists():
            try:
                pid = int(pid_file.read_text().strip())
                if pid != my_pid:
                    pids_to_kill.add(pid)
            except Exception:
                pass

        # Look up other matching processes
        for pattern in patterns:
            try:
                res = subprocess.run(['pgrep', '-f', pattern], capture_output=True, text=True)
                for line in res.stdout.splitlines():
                    try:
                        pid = int(line.strip())
                        if pid != my_pid:
                            pids_to_kill.add(pid)
                    except ValueError:
                        pass
            except Exception:
                pass

        # Send SIGTERM
        for pid in pids_to_kill:
            try:
                os.kill(pid, signal.SIGTERM)
            except Exception:
                pass

        if pids_to_kill:
            import time; time.sleep(0.8)
            # Send SIGKILL to those still alive
            for pid in pids_to_kill:
                try:
                    os.kill(pid, signal.SIGKILL)
                except Exception:
                    pass

    @staticmethod
    def _hid_device_path():
        """Return the /dev/hidrawN path held by the Ulanzi device, or None."""
        try:
            import glob, re
            # Find the hidraw node for VID 2207
            for sys_path in glob.glob('/sys/bus/hid/devices/*/hidraw/hidraw*'):
                # e.g. /sys/bus/hid/devices/0003:2207:...
                if '2207' in sys_path:
                    node = '/dev/' + sys_path.split('/')[-1]
                    if os.path.exists(node):
                        return node
            # Fallback: read /proc/bus/input/devices
            for entry in glob.glob('/sys/class/hidraw/hidraw*'):
                real = os.path.realpath(entry)
                if '2207' in real:
                    return '/dev/' + os.path.basename(entry)
        except Exception:
            pass
        return None

    @staticmethod
    def _device_is_free(dev_path):
        """Return True if no process has dev_path open."""
        if dev_path is None:
            return True  # can't check, optimistically continue
        try:
            import glob
            for fd_link in glob.glob('/proc/*/fd/*'):
                try:
                    if os.readlink(fd_link) == dev_path:
                        return False
                except (OSError, PermissionError):
                    pass
        except Exception:
            pass
        return True

    # ── main thread body ──────────────────────────────────────────────────
    def run(self):
        import time

        # 1 ── Stop ALL daemon instances ───────────────────────────────────
        self.progress.emit("Stopping background daemon…")
        if self.is_systemd:
            try:
                subprocess.run(['systemctl', '--user', 'stop', 'ulanzi-daemon'],
                               timeout=8, capture_output=True)
            except Exception:
                pass

        # Always pkill by name to catch any orphaned processes
        self._kill_all_daemons()

        # 2 ── Wait until the HID device node is actually free ─────────────
        self.progress.emit("Waiting for device to become available…")
        dev_path = self._hid_device_path()
        deadline = time.monotonic() + self.DEVICE_FREE_TIMEOUT
        while time.monotonic() < deadline:
            if self._device_is_free(dev_path):
                break
            time.sleep(0.2)
        else:
            # Force-kill anything still hanging on
            subprocess.run(['pkill', '-KILL', '-f', 'ulanzi-daemon'],
                           capture_output=True)
            time.sleep(0.5)

        # Small grace period for kernel to flush fd table
        time.sleep(0.3)

        # 3 ── Write configuration ─────────────────────────────────────────
        self.progress.emit("Writing configuration to device screen…")
        apply_success = False
        apply_error   = ""
        try:
            import sys
            if getattr(sys, 'frozen', False):
                cmd = [sys.executable, '--cli', 'configure', self.config_path]
                env = None
            else:
                cmd = [sys.executable, '-m', 'ulanzi_manager.cli', 'configure', self.config_path]
                env = os.environ.copy()
                project_root = str(Path(__file__).resolve().parent.parent)
                env['PYTHONPATH'] = project_root + os.pathsep + env.get('PYTHONPATH', '')

            res = subprocess.run(
                cmd,
                capture_output=True, text=True,
                timeout=self.CONFIGURE_TIMEOUT,
                env=env
            )
            apply_success = (res.returncode == 0)
            if not apply_success:
                apply_error = (res.stderr or res.stdout).strip()
        except subprocess.TimeoutExpired:
            apply_error = (
                f"Configure timed out after {self.CONFIGURE_TIMEOUT} s.\n"
                "The device may be unresponsive. Try unplugging and re-plugging it."
            )
        except Exception as e:
            apply_error = str(e)

        # 4 ── Restart daemon (only if configure succeeded) ───────────────
        if self.was_running:
            self.progress.emit("Restarting background daemon…")
            if self.is_systemd:
                try:
                    subprocess.run(['systemctl', '--user', 'start', 'ulanzi-daemon'],
                                   timeout=8, capture_output=True)
                except Exception:
                    pass
            else:
                try:
                    import sys
                    if getattr(sys, 'frozen', False):
                        cmd = [sys.executable, '--daemon', self.config_path]
                        env = None
                    else:
                        cmd = [sys.executable, '-m', 'ulanzi_manager.daemon', self.config_path]
                        env = os.environ.copy()
                        project_root = str(Path(__file__).resolve().parent.parent)
                        env['PYTHONPATH'] = project_root + os.pathsep + env.get('PYTHONPATH', '')
                    subprocess.Popen(cmd, env=env)
                except Exception:
                    pass

        self.finished.emit(apply_success, apply_error)




class KeyButton(QPushButton):
    """
    Stream-deck style hardware key button.
    Draws the icon centered in the upper area and the label text
    elided at the very bottom — no overflow.
    Supports Drag & Drop swapping.
    """
    def __init__(self, text="", parent=None):
        super().__init__(text, parent)
        self.setAcceptDrops(True)
        self.drag_start_pos = None

    def mousePressEvent(self, event):  # noqa: N802
        if event.button() == Qt.MouseButton.LeftButton:
            self.drag_start_pos = event.position().toPoint()
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event):  # noqa: N802
        if not (event.buttons() & Qt.MouseButton.LeftButton):
            super().mouseMoveEvent(event)
            return
        if self.drag_start_pos is None:
            super().mouseMoveEvent(event)
            return
            
        distance = (event.position().toPoint() - self.drag_start_pos).manhattanLength()
        if distance < QApplication.startDragDistance():
            super().mouseMoveEvent(event)
            return

        idx = self.property("index")
        if idx is None:
            super().mouseMoveEvent(event)
            return

        drag = QDrag(self)
        mime_data = QMimeData()
        mime_data.setData("application/x-ulanzi-button-index", str(idx).encode('utf-8'))
        drag.setMimeData(mime_data)

        # Set visual drag pixmap
        pixmap = self.grab()
        drag.setPixmap(pixmap)
        drag.setHotSpot(event.position().toPoint())

        # Reset button pressed state visual when dragging starts
        self.setDown(False)

        # Execute drag operation
        drag.exec(Qt.DropAction.MoveAction)
        self.drag_start_pos = None

    def dragEnterEvent(self, event):  # noqa: N802
        if event.mimeData().hasFormat("application/x-ulanzi-button-index"):
            src_idx = int(event.mimeData().data("application/x-ulanzi-button-index").data().decode('utf-8'))
            dest_idx = self.property("index")
            if src_idx != dest_idx:
                event.acceptProposedAction()
                self.setProperty("dragOver", True)
                self.style().polish(self)
                self.update()
                return
        event.ignore()

    def dragLeaveEvent(self, event):  # noqa: N802
        self.setProperty("dragOver", False)
        self.style().polish(self)
        self.update()
        super().dragLeaveEvent(event)

    def dropEvent(self, event):  # noqa: N802
        self.setProperty("dragOver", False)
        self.style().polish(self)
        self.update()

        mime = event.mimeData()
        if mime.hasFormat("application/x-ulanzi-button-index"):
            src_idx = int(mime.data("application/x-ulanzi-button-index").data().decode('utf-8'))
            dest_idx = self.property("index")
            if src_idx != dest_idx:
                # Walk up parent chain to find MainWindow
                parent = self.parentWidget()
                while parent:
                    if hasattr(parent, 'swap_buttons'):
                        parent.swap_buttons(src_idx, dest_idx)
                        event.acceptProposedAction()
                        return
                    parent = parent.parentWidget()
        event.ignore()

    def paintEvent(self, event):  # noqa: N802
        opt = QStyleOptionButton()
        self.initStyleOption(opt)

        painter = QPainter(self)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)

        # Draw the frame / background (handles checked, hover, pressed states)
        self.style().drawControl(
            QStyle.ControlElement.CE_PushButtonBevel, opt, painter, self
        )

        r = self.rect().adjusted(3, 3, -3, -3)   # inner padding
        text = self.text()

        # Reserve bottom strip for text
        fm = painter.fontMetrics()
        text_h = (fm.height() + 6) if text else 0

        # ── Icon: centred in the space above the text strip ─────────────
        icon = self.icon()
        if not icon.isNull():
            isz = self.iconSize()
            icon_area_h = r.height() - text_h
            ix = r.x() + (r.width() - isz.width()) // 2
            iy = r.y() + max(0, (icon_area_h - isz.height()) // 2)
            mode = QIcon.Mode.Selected if self.isChecked() else QIcon.Mode.Normal
            state = QIcon.State.On if self.isChecked() else QIcon.State.Off
            icon.paint(painter, ix, iy, isz.width(), isz.height(), Qt.AlignmentFlag.AlignCenter, mode, state)

        # ── Text: elided, centred at the very bottom ────────────────────
        if text:
            color = QColor("#3a86f0") if self.isChecked() else QColor("#c0c0ca")
            painter.setPen(color)
            text_rect = QRect(r.x(), r.bottom() - text_h + 2, r.width(), text_h)
            elided = fm.elidedText(text, Qt.TextElideMode.ElideRight, text_rect.width() - 2)
            painter.drawText(text_rect, Qt.AlignmentFlag.AlignHCenter | Qt.AlignmentFlag.AlignTop, elided)

        # ── Drag and Drop Hover Outline ──────────────────────────────────
        if self.property("dragOver"):
            painter.save()
            painter.setPen(QPen(QColor("#10b981"), 2, Qt.PenStyle.DashLine))
            idx = self.property("index")
            r_val = 8 if (isinstance(idx, int) and idx >= 15) else 12
            painter.drawRoundedRect(self.rect().adjusted(1, 1, -1, -1), r_val, r_val)
            painter.restore()

        painter.end()


class MainWindow(QMainWindow):
    """Main window of the Ulanzi D200X Manager GUI"""
    
    def __init__(self, config_path: str):
        super().__init__()
        self.config_path = config_path
        self.config: Config = None
        self.selected_type = None  # 'button' or 'dial'
        self.selected_index = None # 0-12, 13 (clock), or 17-19 (dials)
        self.selected_dial_tab = "click" # 'click', 'left', 'right'
        self.is_applying = False
        
        self.setWindowTitle("Ulanzi D200X Manager")
        self.setMinimumSize(980, 640)
        self.setStyleSheet("""
            QMainWindow {
                background-color: #08080a;
                color: #e2e2e5;
                font-family: 'Segoe UI', -apple-system, BlinkMacSystemFont, Roboto, sans-serif;
            }
            QLabel {
                color: #8e8e9f;
                font-size: 13px;
            }
            QLabel#inspector_title {
                font-size: 15px;
                font-weight: 700;
                color: #ffffff;
                padding-bottom: 6px;
            }
            QFrame#inspector {
                background-color: #0f0f14;
                border-left: 1px solid #1a1a24;
            }
            QFrame#device_frame {
                background-color: #121217;
                border: 2px solid #22222e;
                border-radius: 20px;
            }
            
            /* Custom Settings Cards */
            QFrame#settings_card {
                background-color: #15151f;
                border: 1px solid #232332;
                border-radius: 10px;
            }
            
            QFrame#placeholder_card {
                background-color: #15151f;
                border: 1px dashed #2a2a3e;
                border-radius: 12px;
            }
            
            /* Inputs */
            QLineEdit, QComboBox, QSpinBox {
                background-color: #1c1c28;
                border: 1px solid #2e2e3e;
                border-radius: 8px;
                padding: 8px 12px;
                color: #ffffff;
                font-size: 13px;
            }
            QLineEdit:hover, QComboBox:hover, QSpinBox:hover {
                border: 1px solid #3e3e54;
            }
            QLineEdit:focus, QComboBox:focus, QSpinBox:focus {
                border: 2px solid #3a86f0;
                background-color: #181c28;
            }
            
            QComboBox::drop-down {
                border: none;
                padding-right: 12px;
            }
            QComboBox::down-arrow {
                image: none;
                border-left: 5px solid transparent;
                border-right: 5px solid transparent;
                border-top: 5px solid #8a8a93;
                width: 0;
                height: 0;
            }
            
            /* Checkboxes */
            QCheckBox {
                spacing: 8px;
                color: #c5c5ca;
                font-size: 13px;
            }
            QCheckBox::indicator {
                width: 18px;
                height: 18px;
                border: 1px solid #2e2e3e;
                border-radius: 4px;
                background-color: #1c1c28;
            }
            QCheckBox::indicator:hover {
                border-color: #3e3e54;
            }
            QCheckBox::indicator:checked {
                background-color: #3a86f0;
                border-color: #3a86f0;
            }
            
            /* Tab Widget */
            QTabWidget::pane {
                border: 1px solid #1f1f2e;
                border-radius: 8px;
                background-color: #0f0f14;
                top: -1px;
            }
            QTabBar::tab {
                background-color: #08080a;
                border: 1px solid #1f1f2e;
                border-bottom: none;
                padding: 8px 16px;
                border-top-left-radius: 6px;
                border-top-right-radius: 6px;
                color: #8a8a93;
                font-weight: 600;
                font-size: 12px;
                margin-right: 4px;
            }
            QTabBar::tab:hover {
                background-color: #15151f;
                color: #c5c5ca;
            }
            QTabBar::tab:selected {
                background-color: #0f0f14;
                border: 1px solid #1f1f2e;
                border-bottom: 1px solid #0f0f14;
                color: #3a86f0;
            }
            
            /* Splitter */
            QSplitter::handle {
                background-color: #1a1a24;
            }
            QSplitter::handle:horizontal {
                width: 5px;
            }
            
            /* Scrollbars */
            QScrollArea {
                background: transparent;
                border: none;
            }
            QScrollBar:vertical {
                border: none;
                background: #08080a;
                width: 8px;
                margin: 0px;
                border-radius: 4px;
            }
            QScrollBar::handle:vertical {
                background: #232332;
                min-height: 20px;
                border-radius: 4px;
            }
            QScrollBar::handle:vertical:hover {
                background: #3e3e54;
            }
            QScrollBar::add-line:vertical, QScrollBar::sub-line:vertical {
                border: none;
                background: none;
                height: 0px;
            }
            
            /* Standard Action Buttons */
            QPushButton {
                background-color: #1a1a24;
                border: 1px solid #2d2d3d;
                border-radius: 6px;
                padding: 8px 16px;
                color: #e2e2e5;
                font-weight: bold;
                font-size: 12px;
            }
            QPushButton:hover {
                background-color: #232332;
                border-color: #3e3e54;
            }
            QPushButton:pressed {
                background-color: #1c1c28;
            }
            
            /* Apply Button */
            QPushButton#apply_btn {
                background-color: #10b981;
                border: none;
                color: #ffffff;
                font-size: 13px;
                padding: 10px 24px;
                border-radius: 6px;
            }
            QPushButton#apply_btn:hover {
                background-color: #059669;
            }
            QPushButton#apply_btn:pressed {
                background-color: #047857;
            }
            
            /* Toggle Daemon Button overrides */
            QPushButton#daemon_toggle_btn {
                background-color: #252535;
                border: 1px solid #333345;
            }
            
            /* Hardware Mockup Buttons (keys) - KeyButton draws icon+text in paintEvent */
            QPushButton[type="button"] {
                background-color: #14141c;
                border: 2px solid #272736;
                border-radius: 12px;
                font-size: 9px;
                font-weight: 600;
            }
            QPushButton[type="button"]:hover {
                background-color: #1c1c28;
                border-color: #404058;
                color: #ffffff;
            }
            QPushButton[type="button"]:checked {
                border: 2px solid #3a86f0;
                background-color: #131c2e;
                color: #3a86f0;
            }
            
            /* Clock Button */
            QPushButton[type="clock"] {
                background-color: #0c0c18;
                border: 2px solid #272736;
                border-radius: 10px;
                color: #3a86f0;
                font-weight: 700;
                font-size: 12px;
                letter-spacing: 2px;
            }
            QPushButton[type="clock"]:hover {
                background-color: #12122a;
                border-color: #405070;
            }
            QPushButton[type="clock"]:checked {
                border: 2px solid #3a86f0;
                background-color: #0d1525;
            }
            
            /* Dial Buttons */
            QPushButton[type="dial"] {
                background-color: #1a1a22;
                border: 3px solid #383848;
                border-radius: 36px;
                color: #8a8a9a;
                font-size: 10px;
                font-weight: 700;
            }
            QPushButton[type="dial"]:hover {
                border-color: #5a5a78;
                background-color: #22223a;
                color: #e0e0ea;
            }
            QPushButton[type="dial"]:checked {
                border: 3px solid #3a86f0;
                color: #ffffff;
                background-color: #101828;
            }
            
            /* Small Side Buttons (SB15, SB16) */
            QPushButton[type="button"][index="15"],
            QPushButton[type="button"][index="16"] {
                background-color: #18181e;
                border: 2px solid #2a2a38;
                border-radius: 8px;
                color: #606070;
                font-size: 8px;
                font-weight: 700;
            }
            QPushButton[type="button"][index="15"]:hover,
            QPushButton[type="button"][index="16"]:hover {
                border-color: #3a86f0;
                color: #a0a0b8;
            }
            QPushButton[type="button"][index="15"]:checked,
            QPushButton[type="button"][index="16"]:checked {
                border: 2px solid #3a86f0;
                background-color: #0d1525;
                color: #3a86f0;
            }
        """)
        
        self.load_configuration()
        self.init_ui()
        
        # Periodic daemon status check
        self.status_timer = QTimer(self)
        self.status_timer.timeout.connect(self.update_status_indicators)
        self.status_timer.start(2000) # Check every 2s
        
        # Setup System Tray Icon
        self.setup_system_tray()

    def load_configuration(self):
        """Load configuration from config.yaml, create default if missing"""
        config_file = Path(self.config_path)
        if not config_file.exists():
            # Generate default configuration
            config_file.parent.mkdir(parents=True, exist_ok=True)
            self.config = Config()
            self.save_configuration_to_file()
        else:
            try:
                self.config = ConfigParser.load(self.config_path)
            except Exception as e:
                QMessageBox.warning(self, "Load Warning", f"Failed to load config.yaml:\n{e}\n\nUsing blank config.")
                self.config = Config()

    def save_configuration_to_file(self):
        """Save the current Config dataclass state to config.yaml"""
        import yaml
        
        # Capture OBS credentials from the inputs if they exist
        if hasattr(self, 'obs_host_input'):
            self.config.obs_host = self.obs_host_input.text().strip()
        if hasattr(self, 'obs_port_spin'):
            self.config.obs_port = int(self.obs_port_spin.value())
        if hasattr(self, 'obs_password_input'):
            self.config.obs_password = self.obs_password_input.text().strip() or None

        # Structure it nicely for serialization
        data = {
            'brightness': self.config.brightness,
            'sleep_timeout': getattr(self.config, 'sleep_timeout', 10),
            'sleep_brightness': getattr(self.config, 'sleep_brightness', 0),
            'hide_labels': getattr(self.config, 'hide_labels', False),
            'clock_mode': getattr(self.config, 'clock_mode', 1),
            'label_style': self.config.label_style,
            'obs': {
                'host': self.config.obs_host,
                'port': self.config.obs_port,
                'password': self.config.obs_password
            },
            'buttons': [None] * 17,  # indices 0-16 (13=clock placeholder, 14=unused, 15-16=small btns)
            'dials': {}
        }
        
        # Populate buttons
        for btn in self.config.buttons:
            if btn.index < 17:
                data['buttons'][btn.index] = {
                    'image': btn.image,
                    'label': btn.label,
                    'action': btn.action_type,
                    'params': btn.action_params,
                    'state': btn.state
                }
                if getattr(btn, 'icon_spec', None):
                    data['buttons'][btn.index]['icon_spec'] = btn.icon_spec
                
        # Populate dials
        for dial_idx, dial_cfg in self.config.dials.items():
            data['dials'][dial_idx] = dial_cfg
            
        try:
            with open(self.config_path, 'w') as f:
                yaml.dump(data, f, default_flow_style=False, sort_keys=False)
            return True
        except Exception as e:
            QMessageBox.warning(self, "Save Failed", f"Failed to save configuration:\n{e}")
            return False

    def swap_buttons(self, src_idx: int, dest_idx: int):
        """Swap the configurations of two buttons and update the UI"""
        src_cfg = None
        dest_cfg = None
        for b in self.config.buttons:
            if b.index == src_idx:
                src_cfg = b
            elif b.index == dest_idx:
                dest_cfg = b

        if src_cfg and dest_cfg:
            # Both configurations exist, swap their property fields
            src_cfg.image, dest_cfg.image = dest_cfg.image, src_cfg.image
            src_cfg.label, dest_cfg.label = dest_cfg.label, src_cfg.label
            src_cfg.action_type, dest_cfg.action_type = dest_cfg.action_type, src_cfg.action_type
            src_cfg.action_params, dest_cfg.action_params = dest_cfg.action_params, src_cfg.action_params
            src_cfg.state, dest_cfg.state = dest_cfg.state, src_cfg.state
        elif src_cfg:
            # Only source exists, move it to destination
            src_cfg.index = dest_idx
        elif dest_cfg:
            # Only destination exists, move it to source
            dest_cfg.index = src_idx

        # Save to configuration file
        self.save_configuration_to_file()

        # Update the visual mockup elements
        self.render_device_mockups()

        # Re-select the destination button in the UI
        btn = self.btn_widgets.get(dest_idx)
        if btn:
            btn.setChecked(True)
            self.selected_type = btn.property("type")
            self.selected_index = dest_idx
            
            # Deselect all other interactive elements
            for idx, b in self.btn_widgets.items():
                if b != btn:
                    b.setChecked(False)
            for idx, dial in self.dial_widgets.items():
                dial.setChecked(False)
                
            self.build_inspector()

        # Display a status notification
        self._show_notification(f"Moved button configuration from {src_idx} to {dest_idx}")

    def init_ui(self):
        main_widget = QWidget()
        main_layout = QHBoxLayout(main_widget)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)
        
        # 1. Left Side: Device Mockup Grid
        device_section = QVBoxLayout()
        device_section.setContentsMargins(20, 20, 20, 20)
        device_section.setSpacing(15)
        
        title_label = QLabel("<h2>Ulanzi D200X Console</h2>")
        title_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        device_section.addWidget(title_label)

        # Create Status Card (Connection & Daemon status upfront)
        self.status_card = QFrame()
        self.status_card.setObjectName("status_card")
        self.status_card.setStyleSheet("""
            QFrame#status_card {
                background-color: #121217;
                border: 1px solid #22222e;
                border-radius: 10px;
            }
            QLabel {
                font-size: 12px;
            }
        """)
        status_card_layout = QHBoxLayout(self.status_card)
        status_card_layout.setContentsMargins(15, 8, 15, 8)
        status_card_layout.setSpacing(15)
        
        # 1. Connection Status Section
        conn_layout = QHBoxLayout()
        self.conn_dot = QLabel("●")
        self.conn_dot.setStyleSheet("color: #e74c3c; font-size: 16px;")
        self.conn_text = QLabel("Device: Disconnected")
        self.conn_text.setStyleSheet("font-weight: bold; color: #ffffff;")
        
        self.conn_retry_btn = QPushButton("Retry")
        self.conn_retry_btn.setToolTip("Retry connection to device")
        self.conn_retry_btn.clicked.connect(self.manual_retry_connection)
        self.conn_retry_btn.setStyleSheet("padding: 3px 8px; font-size: 11px; font-weight: normal;")
        
        self.conn_fix_btn = QPushButton("Fix Rules")
        self.conn_fix_btn.setToolTip("Fix USB permission rules (requires sudo)")
        self.conn_fix_btn.clicked.connect(self.fix_permissions_gui)
        self.conn_fix_btn.setVisible(False)
        self.conn_fix_btn.setStyleSheet("""
            QPushButton { padding: 3px 8px; font-size: 11px; font-weight: normal; background-color: #3a86f0; color: white; border: none; }
            QPushButton:hover { background-color: #2a76e0; }
        """)
        
        conn_layout.addWidget(self.conn_dot)
        conn_layout.addWidget(self.conn_text)
        conn_layout.addWidget(self.conn_retry_btn)
        conn_layout.addWidget(self.conn_fix_btn)
        
        # 2. Daemon Status Section
        daemon_layout_row = QHBoxLayout()
        self.daemon_dot = QLabel("●")
        self.daemon_dot.setStyleSheet("color: #e74c3c; font-size: 16px;")
        self.daemon_status_lbl = QLabel("Daemon: Inactive")
        self.daemon_status_lbl.setStyleSheet("font-weight: bold; color: #ffffff;")
        
        self.daemon_control_btn = QPushButton("Start Daemon")
        self.daemon_control_btn.clicked.connect(self.toggle_daemon)
        self.daemon_control_btn.setStyleSheet("padding: 3px 8px; font-size: 11px; font-weight: normal;")
        
        daemon_layout_row.addWidget(self.daemon_dot)
        daemon_layout_row.addWidget(self.daemon_status_lbl)
        daemon_layout_row.addWidget(self.daemon_control_btn)
        
        status_card_layout.addLayout(conn_layout)
        status_card_layout.addStretch(1)
        status_card_layout.addLayout(daemon_layout_row)
        
        device_section.addWidget(self.status_card)
        
        # Device Shell Frame
        self.device_frame = QFrame()
        self.device_frame.setObjectName("device_frame")
        self.device_frame.setFixedSize(550, 430)
        
        device_frame_layout = QVBoxLayout(self.device_frame)
        device_frame_layout.setContentsMargins(25, 25, 25, 25)
        device_frame_layout.setSpacing(20)
        
        # Grid layout for buttons
        grid_layout = QGridLayout()
        grid_layout.setSpacing(12)
        
        self.btn_widgets = {}
        
        # Create buttons 0-12
        for idx in range(BUTTON_COUNT):
            row = idx // GRID_COLS
            col = idx % GRID_COLS
            
            btn = KeyButton()
            btn.setFixedSize(88, 88)
            btn.setProperty("index", idx)
            btn.setProperty("type", "button")
            btn.clicked.connect(self.on_device_element_clicked)
            btn.setCheckable(True)
            self.btn_widgets[idx] = btn
            grid_layout.addWidget(btn, row, col)
            
        # Add clock slot (index 13)
        clock_btn = QPushButton("CLOCK")
        clock_btn.setFixedSize(188, 88)  # spans 2 columns
        clock_btn.setProperty("index", 13)
        clock_btn.setProperty("type", "clock")
        clock_btn.setCheckable(True)
        clock_btn.clicked.connect(self.on_device_element_clicked)
        self.btn_widgets[13] = clock_btn
        grid_layout.addWidget(clock_btn, 2, 3, 1, 2)  # Row 2, Col 3, span 1 row, 2 cols
        
        device_frame_layout.addLayout(grid_layout)
        
        # Dials Row: [SB15] [SB16]  [DIAL1]  [DIAL2]  [DIAL3]
        dials_layout = QHBoxLayout()
        dials_layout.setContentsMargins(10, 0, 10, 0)
        dials_layout.setSpacing(10)
        
        self.dial_widgets = {}
        
        # Small buttons 15 & 16 on the left of the dials
        for idx in SMALL_BUTTON_INDEXES:
            sb = KeyButton(f"SB{idx - 14}")
            sb.setFixedSize(42, 42)
            sb.setProperty("index", idx)
            sb.setProperty("type", "button")
            sb.setCheckable(True)
            sb.clicked.connect(self.on_device_element_clicked)
            sb.setToolTip(f"Small Button {idx - 14} (index {idx})")
            self.btn_widgets[idx] = sb
            dials_layout.addWidget(sb)
        
        dials_layout.addStretch(1)
        
        # Three rotary dials
        for idx in DIAL_HARDWARE_INDEXES:
            dial_widget = QPushButton(f"◎\nDIAL {idx-16}")
            dial_widget.setFixedSize(72, 72)
            dial_widget.setProperty("index", idx)
            dial_widget.setProperty("type", "dial")
            dial_widget.setCheckable(True)
            dial_widget.clicked.connect(self.on_device_element_clicked)
            dial_widget.setToolTip(f"Dial {idx - 16} — click to configure rotate & press actions")
            self.dial_widgets[idx] = dial_widget
            dials_layout.addWidget(dial_widget)
            dials_layout.addStretch(1)
            
        device_frame_layout.addLayout(dials_layout)
        
        # Center mockup frame vertically and horizontally in left panel
        device_section.addStretch(1)
        device_section.addWidget(self.device_frame, alignment=Qt.AlignmentFlag.AlignCenter)
        device_section.addStretch(1)
        
        # Control & Status Footer
        footer_layout = QHBoxLayout()
        footer_layout.setContentsMargins(20, 10, 20, 15)
        
        self.footer_status_text = QLabel("Console Connected")
        self.footer_status_text.setStyleSheet("color: #7a7a85; font-size: 12px;")
        footer_layout.addWidget(self.footer_status_text)
        
        footer_layout.addStretch()
        
        self.notification_label = QLabel("")
        self.notification_label.setStyleSheet("color: #10b981; font-weight: bold; margin-right: 15px; font-size: 13px;")
        footer_layout.addWidget(self.notification_label)
        
        # Save & Apply Button
        self.apply_btn = QPushButton("Apply to Device")
        self.apply_btn.setObjectName("apply_btn")
        self.apply_btn.clicked.connect(self.apply_configuration)
        footer_layout.addWidget(self.apply_btn)
        
        device_section.addLayout(footer_layout)
        
        # Wrap device_section in a widget for splitter
        device_widget = QWidget()
        device_widget.setLayout(device_section)
        
        # Create resizable split layout
        splitter = QSplitter(Qt.Orientation.Horizontal)
        splitter.addWidget(device_widget)
        
        # 2. Right Side: Inspector Panel (Tabbed)
        self.inspector_frame = QFrame()
        self.inspector_frame.setObjectName("inspector")
        self.inspector_layout = QVBoxLayout(self.inspector_frame)
        self.inspector_layout.setContentsMargins(10, 15, 10, 15)
        self.inspector_layout.setSpacing(10)
        
        self.inspector_tabs = QTabWidget()
        self.inspector_tabs.setObjectName("inspector_tabs")
        
        # Tab 1: Element Actions
        self.element_tab = QWidget()
        element_tab_layout = QVBoxLayout(self.element_tab)
        element_tab_layout.setContentsMargins(5, 10, 5, 5)
        element_tab_layout.setSpacing(10)
        
        self.inspector_title = QLabel("No Element Selected")
        self.inspector_title.setObjectName("inspector_title")
        element_tab_layout.addWidget(self.inspector_title)
        
        self.scroll_area = QScrollArea()
        self.scroll_area.setWidgetResizable(True)
        self.scroll_area.setFrameShape(QFrame.Shape.NoFrame)
        self.scroll_widget = QWidget()
        self.scroll_layout = QVBoxLayout(self.scroll_widget)
        self.scroll_layout.setContentsMargins(0, 0, 0, 0)
        self.scroll_layout.setSpacing(15)
        self.scroll_area.setWidget(self.scroll_widget)
        element_tab_layout.addWidget(self.scroll_area)
        
        self.inspector_tabs.addTab(self.element_tab, "Element Actions")
        
        # Tab 2: Global Settings
        self.global_tab = QWidget()
        global_tab_layout = QVBoxLayout(self.global_tab)
        global_tab_layout.setContentsMargins(5, 10, 5, 5)
        
        self.global_scroll = QScrollArea()
        self.global_scroll.setWidgetResizable(True)
        self.global_scroll.setFrameShape(QFrame.Shape.NoFrame)
        self.global_scroll_widget = QWidget()
        self.global_scroll_layout = QVBoxLayout(self.global_scroll_widget)
        self.global_scroll_layout.setContentsMargins(0, 0, 0, 0)
        self.global_scroll_layout.setSpacing(15)
        self.global_scroll.setWidget(self.global_scroll_widget)
        
        global_tab_layout.addWidget(self.global_scroll)
        self.build_global_settings_tab()
        
        self.inspector_tabs.addTab(self.global_tab, "Device Settings")
        
        self.inspector_layout.addWidget(self.inspector_tabs)
        splitter.addWidget(self.inspector_frame)
        
        # Stretches: Left console (3), Right inspector (2)
        splitter.setStretchFactor(0, 3)
        splitter.setStretchFactor(1, 2)
        
        main_layout.addWidget(splitter)
        
        self.setCentralWidget(main_widget)
        self.render_device_mockups()
        self.show_placeholder_inspector()

    def build_global_settings_tab(self):
        """Build the global configuration panel inside the Device Settings tab"""
        layout = self.global_scroll_layout
        
        # 1. Display & Power Card
        disp_card = QFrame()
        disp_card.setObjectName("settings_card")
        disp_layout = QFormLayout(disp_card)
        disp_layout.setContentsMargins(15, 15, 15, 15)
        disp_layout.setSpacing(10)
        
        disp_title = QLabel("Display & Power")
        disp_title.setStyleSheet("font-size: 14px; font-weight: bold; color: #ffffff; margin-bottom: 5px;")
        disp_layout.addRow(disp_title)
        
        self.brightness_combo = QComboBox()
        self.brightness_combo.addItems([f"{x}%" for x in range(10, 110, 10)])
        self.brightness_combo.setCurrentText(f"{self.config.brightness}%")
        self.brightness_combo.currentTextChanged.connect(self.on_brightness_changed)
        disp_layout.addRow("Global Brightness:", self.brightness_combo)
        
        self.sleep_timeout_combo = QComboBox()
        self.sleep_timeout_combo.addItems(["Never", "1 min", "5 min", "10 min", "30 min", "60 min"])
        timeout_val = getattr(self.config, 'sleep_timeout', 10)
        timeout_map = {0: "Never", 1: "1 min", 5: "5 min", 10: "10 min", 30: "30 min", 60: "60 min"}
        self.sleep_timeout_combo.setCurrentText(timeout_map.get(timeout_val, "10 min"))
        self.sleep_timeout_combo.currentTextChanged.connect(self.on_sleep_timeout_changed)
        disp_layout.addRow("Sleep Timeout:", self.sleep_timeout_combo)
        
        self.sleep_brightness_combo = QComboBox()
        self.sleep_brightness_combo.addItems(["0%", "10%", "20%", "30%"])
        brightness_val = getattr(self.config, 'sleep_brightness', 0)
        self.sleep_brightness_combo.setCurrentText(f"{brightness_val}%")
        self.sleep_brightness_combo.currentTextChanged.connect(self.on_sleep_brightness_changed)
        disp_layout.addRow("Sleep Brightness:", self.sleep_brightness_combo)
        
        # Hide Labels Toggle
        self.hide_labels_checkbox = QCheckBox()
        self.hide_labels_checkbox.setChecked(getattr(self.config, 'hide_labels', False))
        self.hide_labels_checkbox.toggled.connect(self.on_hide_labels_toggled)
        disp_layout.addRow("Hide Button Labels:", self.hide_labels_checkbox)
        
        layout.addWidget(disp_card)
        
        # 2. OBS WebSocket Connection Card
        obs_card = QFrame()
        obs_card.setObjectName("settings_card")
        obs_layout = QFormLayout(obs_card)
        obs_layout.setContentsMargins(15, 15, 15, 15)
        obs_layout.setSpacing(10)
        
        obs_title = QLabel("OBS Studio WebSockets")
        obs_title.setStyleSheet("font-size: 14px; font-weight: bold; color: #ffffff; margin-bottom: 5px;")
        obs_layout.addRow(obs_title)
        
        self.obs_host_input = QLineEdit(self.config.obs_host or "localhost")
        self.obs_host_input.setPlaceholderText("e.g. localhost")
        obs_layout.addRow("Host / Address:", self.obs_host_input)
        
        self.obs_port_spin = QSpinBox()
        self.obs_port_spin.setRange(1, 65535)
        self.obs_port_spin.setValue(self.config.obs_port or 4444)
        obs_layout.addRow("Port:", self.obs_port_spin)
        
        pw_layout = QHBoxLayout()
        self.obs_password_input = QLineEdit(self.config.obs_password or "")
        self.obs_password_input.setEchoMode(QLineEdit.EchoMode.Password)
        self.obs_password_input.setPlaceholderText("Leave blank if no password")
        pw_layout.addWidget(self.obs_password_input)
        
        show_pw_btn = QPushButton("Show")
        show_pw_btn.setCheckable(True)
        show_pw_btn.setFixedWidth(50)
        def toggle_pw_visibility(checked):
            if checked:
                self.obs_password_input.setEchoMode(QLineEdit.EchoMode.Normal)
                show_pw_btn.setText("Hide")
            else:
                self.obs_password_input.setEchoMode(QLineEdit.EchoMode.Password)
                show_pw_btn.setText("Show")
        show_pw_btn.toggled.connect(toggle_pw_visibility)
        pw_layout.addWidget(show_pw_btn)
        
        obs_layout.addRow("Password:", pw_layout)
        
        layout.addWidget(obs_card)
        
        # 3. Daemon Control Card
        daemon_card = QFrame()
        daemon_card.setObjectName("settings_card")
        daemon_layout = QVBoxLayout(daemon_card)
        daemon_layout.setContentsMargins(15, 15, 15, 15)
        daemon_layout.setSpacing(12)
        
        daemon_title = QLabel("Background Daemon")
        daemon_title.setStyleSheet("font-size: 14px; font-weight: bold; color: #ffffff;")
        daemon_layout.addWidget(daemon_title)
        
        status_row = QHBoxLayout()
        self.daemon_status_dot = QLabel("●")
        self.daemon_status_dot.setStyleSheet("color: #e74c3c; font-size: 16px;")
        status_row.addWidget(self.daemon_status_dot)
        
        self.daemon_status_text = QLabel("Daemon: Inactive")
        self.daemon_status_text.setStyleSheet("font-weight: 500;")
        status_row.addWidget(self.daemon_status_text)
        status_row.addStretch()
        
        daemon_layout.addLayout(status_row)
        
        self.daemon_toggle_btn = QPushButton("Start Daemon")
        self.daemon_toggle_btn.setObjectName("daemon_toggle_btn")
        self.daemon_toggle_btn.clicked.connect(self.toggle_daemon)
        daemon_layout.addWidget(self.daemon_toggle_btn)
        
        layout.addWidget(daemon_card)
        layout.addStretch()

    def show_placeholder_inspector(self):
        """Displays a clean placeholder card when no console element is selected"""
        self._clear_layout(self.scroll_layout)
        
        placeholder = QFrame()
        placeholder.setObjectName("placeholder_card")
        layout = QVBoxLayout(placeholder)
        placeholder_margins = 20
        layout.setContentsMargins(placeholder_margins, placeholder_margins, placeholder_margins, placeholder_margins)
        layout.setSpacing(15)
        layout.setAlignment(Qt.AlignmentFlag.AlignCenter)
        
        icon_label = QLabel("⚡")
        icon_label.setStyleSheet("font-size: 48px; color: #3a86f0; margin-bottom: 10px;")
        icon_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(icon_label)
        
        title_label = QLabel("No Element Selected")
        title_label.setStyleSheet("font-size: 16px; font-weight: bold; color: #ffffff;")
        title_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(title_label)
        
        desc_label = QLabel(
            "Click any key, dial, or the clock on the console mockup "
            "to configure its label, icon, and triggered actions."
        )
        desc_label.setWordWrap(True)
        desc_label.setStyleSheet("color: #7a7a85; font-size: 13px; line-height: 1.5;")
        desc_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(desc_label)
        
        self.scroll_layout.addWidget(placeholder)
        self.scroll_layout.addStretch()

    def render_device_mockups(self):
        """Render labels and icon previews in grid and small buttons"""
        all_btn_indexes = list(range(BUTTON_COUNT)) + SMALL_BUTTON_INDEXES
        for idx in all_btn_indexes:
            btn = self.btn_widgets.get(idx)
            if btn is None:
                continue
            
            btn_cfg = None
            for b in self.config.buttons:
                if b.index == idx:
                    btn_cfg = b
                    break
            
            is_small = idx in SMALL_BUTTON_INDEXES
            
            # Special rendering for clock slot (index 13) depending on mode
            if idx == 13:
                clock_mode = getattr(self.config, 'clock_mode', 1)
                if clock_mode == 0:
                    btn.setIcon(QIcon())
                    btn.setText("STATS")
                    btn.update()
                    continue
                elif clock_mode == 1:
                    if btn_cfg and btn_cfg.image and Path(btn_cfg.image).is_file():
                        pixmap = QPixmap(btn_cfg.image).scaled(
                            80, 54,
                            Qt.AspectRatioMode.KeepAspectRatio,
                            Qt.TransformationMode.SmoothTransformation
                        )
                        btn.setIcon(QIcon(pixmap))
                        btn.setIconSize(QSize(80, 54))
                    else:
                        btn.setIcon(QIcon())
                    btn.setText("CLOCK")
                    btn.update()
                    continue
                elif clock_mode == 3:
                    if btn_cfg and btn_cfg.image and Path(btn_cfg.image).is_file():
                        pixmap = QPixmap(btn_cfg.image).scaled(
                            80, 54,
                            Qt.AspectRatioMode.KeepAspectRatio,
                            Qt.TransformationMode.SmoothTransformation
                        )
                        btn.setIcon(QIcon(pixmap))
                        btn.setIconSize(QSize(80, 54))
                    else:
                        btn.setIcon(QIcon())
                    btn.setText("MEDIA")
                    btn.update()
                    continue
            
            if btn_cfg and btn_cfg.image and Path(btn_cfg.image).is_file():
                sz = 28 if is_small else 54
                pixmap = QPixmap(btn_cfg.image).scaled(
                    sz, sz,
                    Qt.AspectRatioMode.KeepAspectRatio,
                    Qt.TransformationMode.SmoothTransformation
                )
                btn.setIcon(QIcon(pixmap))
                btn.setIconSize(QSize(sz, sz))
            else:
                btn.setIcon(QIcon())
            
            # Set the text label (respect global hide_labels toggle)
            if getattr(self.config, 'hide_labels', False):
                btn.setText("")
            elif is_small:
                btn.setText(btn_cfg.label if btn_cfg else f"SB{idx - 14}")
            else:
                btn.setText(btn_cfg.label if btn_cfg else f"Btn {idx}")
            
            btn.update()  # force repaint


    def on_device_element_clicked(self):
        sender = self.sender()
        self.selected_type = sender.property("type")
        self.selected_index = sender.property("index")
        
        # Deselect all other interactive elements
        for idx, btn in self.btn_widgets.items():
            if btn != sender:
                btn.setChecked(False)
        for idx, dial in self.dial_widgets.items():
            if dial != sender:
                dial.setChecked(False)
                
        self.build_inspector()

    def _clear_layout(self, layout):
        """Recursively clear all widgets and sub-layouts from a layout"""
        if layout is not None:
            while layout.count():
                item = layout.takeAt(0)
                widget = item.widget()
                if widget is not None:
                    widget.setParent(None)
                    widget.deleteLater()
                else:
                    self._clear_layout(item.layout())

    def build_inspector(self):
        # Clear inspector scroll layout
        self._clear_layout(self.scroll_layout)
        
        # Switch to Element Actions tab automatically
        if hasattr(self, 'inspector_tabs'):
            self.inspector_tabs.setCurrentIndex(0)
            
        if self.selected_type == 'button':
            if self.selected_index in SMALL_BUTTON_INDEXES:
                self.inspector_title.setText(f"Side Button {self.selected_index - 14}")
            else:
                self.inspector_title.setText(f"Button {self.selected_index}")
            self.build_button_inspector()
        elif self.selected_type == 'dial':
            self.inspector_title.setText(f"Dial {self.selected_index - 16}")
            self.selected_dial_tab = "click"  # always start on Press zone
            self.build_dial_inspector()
        elif self.selected_type == 'clock':
            self.inspector_title.setText("Clock Display")
            self.build_clock_inspector()

    def build_button_inspector(self):
        # Find or create config
        btn_cfg = None
        for b in self.config.buttons:
            if b.index == self.selected_index:
                btn_cfg = b
                break
        
        is_small = self.selected_index in SMALL_BUTTON_INDEXES
        default_label = f"Side Btn {self.selected_index - 14}" if is_small else f"Button {self.selected_index}"
        
        if not btn_cfg:
            btn_cfg = ButtonConfig(index=self.selected_index, image="", label=default_label, action_type="command", action_params={})
            self.config.buttons.append(btn_cfg)
        
        # --- Identity Card ---
        identity_card = QFrame()
        identity_card.setObjectName("settings_card")
        identity_layout = QFormLayout(identity_card)
        identity_layout.setContentsMargins(15, 15, 15, 15)
        identity_layout.setSpacing(10)
        
        card_title = QLabel("Appearance")
        card_title.setStyleSheet("font-size: 13px; font-weight: bold; color: #a0a0b0; letter-spacing: 1px; text-transform: uppercase;")
        identity_layout.addRow(card_title)
            
        # Label Input
        self.label_input = QLineEdit(btn_cfg.label)
        self.label_input.textChanged.connect(lambda t: self.update_button_property('label', t))
        identity_layout.addRow("Text Label:", self.label_input)
        
        # Icon Row
        icon_layout = QHBoxLayout()
        self.icon_preview = QLabel()
        self.icon_preview.setFixedSize(52, 52)
        self.icon_preview.setStyleSheet("border: 1px solid #2e2e3e; background-color: #1c1c28; border-radius: 8px;")
        self.icon_preview.setAlignment(Qt.AlignmentFlag.AlignCenter)
        if btn_cfg.image and Path(btn_cfg.image).is_file():
            self.icon_preview.setPixmap(QPixmap(btn_cfg.image).scaled(44, 44, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation))
            
        icon_layout.addWidget(self.icon_preview)
        icon_layout.setSpacing(10)
        
        # Vertical button container
        btn_vbox = QVBoxLayout()
        btn_vbox.setSpacing(4)
        
        btn_hbox = QHBoxLayout()
        btn_hbox.setSpacing(6)
        
        self.choose_icon_btn = QPushButton("Icon...")
        self.choose_icon_btn.clicked.connect(self.open_icon_picker)
        btn_hbox.addWidget(self.choose_icon_btn)
        
        self.choose_file_btn = QPushButton("File...")
        self.choose_file_btn.clicked.connect(self.open_image_file_picker)
        btn_hbox.addWidget(self.choose_file_btn)
        
        btn_vbox.addLayout(btn_hbox)
        
        self.clear_icon_btn = QPushButton("Remove Icon")
        self.clear_icon_btn.clicked.connect(self.clear_button_icon)
        self.clear_icon_btn.setStyleSheet("""
            QPushButton {
                background-color: #3e2727;
                color: #ff8888;
                border: 1px solid #5a3c3c;
            }
            QPushButton:hover {
                background-color: #5c3232;
                color: #ffaaaa;
            }
        """)
        btn_vbox.addWidget(self.clear_icon_btn)
        
        icon_layout.addLayout(btn_vbox)
        
        identity_layout.addRow("Button Icon:", icon_layout)
        self.scroll_layout.addWidget(identity_card)
        
        # --- Action Card ---
        action_card = QFrame()
        action_card.setObjectName("settings_card")
        action_layout_outer = QVBoxLayout(action_card)
        action_layout_outer.setContentsMargins(15, 15, 15, 15)
        action_layout_outer.setSpacing(10)
        
        action_title = QLabel("Action")
        action_title.setStyleSheet("font-size: 13px; font-weight: bold; color: #a0a0b0; letter-spacing: 1px;")
        action_layout_outer.addWidget(action_title)
        
        # Action Dropdown
        self.action_combo = QComboBox()
        self.action_combo.addItems(["Launch Application", "Simulate Shortcut", "Run Command", "Play Sound (Soundboard)", "Control OBS Studio", "Change Audio Volume", "Media Controls"])
        
        # Map back from action type string to UI index
        action_map = {
            "app": "Launch Application",
            "key": "Simulate Shortcut",
            "command": "Run Command",
            "obs": "Control OBS Studio",
            "volume": "Change Audio Volume",
            "media": "Media Controls"
        }
        
        # Check if command matches soundboard structure
        curr_action_ui = action_map.get(btn_cfg.action_type, "Run Command")
        if btn_cfg.action_type == "command" and btn_cfg.action_params.get("cmd", "").startswith(("pw-play", "paplay", "play", "aplay")):
            curr_action_ui = "Play Sound (Soundboard)"
            
        self.action_combo.setCurrentText(curr_action_ui)
        self.action_combo.currentTextChanged.connect(self.on_action_type_changed)
        
        action_form = QFormLayout()
        action_form.setSpacing(8)
        action_form.addRow("Action Type:", self.action_combo)
        action_layout_outer.addLayout(action_form)
        
        # Reusable Parameter Widget Container (inside action card)
        self.param_widget_container = QWidget()
        self.param_widget_layout = QVBoxLayout(self.param_widget_container)
        self.param_widget_layout.setContentsMargins(0, 5, 0, 0)
        action_layout_outer.addWidget(self.param_widget_container)
        
        self.scroll_layout.addWidget(action_card)
        self.scroll_layout.addStretch()
        
        # Build current action params view
        self.load_action_parameter_form(btn_cfg.action_type, btn_cfg.action_params)

    def update_button_property(self, prop, val):
        for btn in self.config.buttons:
            if btn.index == self.selected_index:
                setattr(btn, prop, val)
                break
        self.render_device_mockups()

    def open_icon_picker(self):
        # 1. Try to find or parse existing custom icon settings from the button's image filename
        btn_cfg = None
        for b in self.config.buttons:
            if b.index == self.selected_index:
                btn_cfg = b
                break
                
        current_icon_name = None
        current_bg = None
        current_fg = None
        
        if btn_cfg and btn_cfg.image:
            filename = Path(btn_cfg.image).name
            if "_bg_" in filename and "_fg_" in filename:
                try:
                    parts = filename.split("_bg_")
                    current_icon_name = parts[0]
                    color_parts = parts[1].split("_fg_")
                    current_bg = "#" + color_parts[0]
                    current_fg = "#" + color_parts[1].replace(".png", "")
                except Exception:
                    pass

        # 2. Open the customized picker
        picker = IconPicker(
            self,
            current_icon_name=current_icon_name,
            current_bg_color=current_bg,
            current_fg_color=current_fg
        )
        if picker.exec() == QDialog.DialogCode.Accepted and picker.selected_icon_path:
            self.icon_preview.setPixmap(QPixmap(picker.selected_icon_path).scaled(44, 44, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation))
            self.update_button_property('image', picker.selected_icon_path)
            
            # Save icon_spec
            if picker.selected_icon_name:
                icon_spec = {
                    'type': 'icon',
                    'name': picker.selected_icon_name,
                    'bg_color': picker.bg_color,
                    'fg_color': picker.fg_color
                }
                for btn in self.config.buttons:
                    if btn.index == self.selected_index:
                        btn.icon_spec = icon_spec
                        break

    def open_image_file_picker(self):
        from PyQt6.QtWidgets import QFileDialog
        from PIL import Image
        import re
        
        file_path, _ = QFileDialog.getOpenFileName(
            self, "Select Image File", "",
            "Images (*.png *.jpg *.jpeg *.svg *.gif *.webp);;All Files (*)"
        )
        if file_path:
            dest_dir = Path.home() / '.local/share/ulanzi/icons'
            dest_dir.mkdir(parents=True, exist_ok=True)
            
            src_path = Path(file_path)
            clean_name = re.sub(r'[^a-zA-Z0-9_]', '_', src_path.stem.lower())
            dest_path = dest_dir / f"custom_{clean_name}.png"
            
            try:
                # Open, resize and standardize to 196x196 PNG
                with Image.open(file_path) as img:
                    img_resized = img.resize((196, 196), Image.Resampling.LANCZOS)
                    if img_resized.mode in ('RGBA', 'LA') or (img_resized.mode == 'P' and 'transparency' in img_resized.info):
                        img_resized.save(dest_path, "PNG")
                    else:
                        img_resized.convert("RGB").save(dest_path, "PNG")
                        
                self.icon_preview.setPixmap(QPixmap(str(dest_path)).scaled(
                    44, 44, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation
                ))
                self.update_button_property('image', str(dest_path))
                # Clear icon_spec
                for btn in self.config.buttons:
                    if btn.index == self.selected_index:
                        btn.icon_spec = None
                        break
            except Exception as e:
                QMessageBox.critical(self, "Error", f"Failed to process and save image file:\n{e}")

    def clear_button_icon(self):
        self.icon_preview.setPixmap(QPixmap())
        self.update_button_property('image', '')
        # Clear icon_spec
        for btn in self.config.buttons:
            if btn.index == self.selected_index:
                btn.icon_spec = None
                break

    def on_hide_labels_toggled(self, checked):
        self.config.hide_labels = checked
        self.render_device_mockups()

    def on_action_type_changed(self, text):
        # Map UI Selection to internal strings
        ui_map = {
            "Launch Application": "app",
            "Simulate Shortcut": "key",
            "Run Command": "command",
            "Play Sound (Soundboard)": "command", # Soundboard maps to CommandAction internally
            "Control OBS Studio": "obs",
            "Change Audio Volume": "volume",
            "Media Controls": "media"
        }
        action_type = ui_map.get(text, "command")
        
        # Update config
        for btn in self.config.buttons:
            if btn.index == self.selected_index:
                btn.action_type = action_type
                btn.action_params = {}
                break
                
        # Reload param form
        self.load_action_parameter_form(action_type, {})

    def load_action_parameter_form(self, action_type, params):
        # Clear param container layout
        self._clear_layout(self.param_widget_layout)
            
        curr_ui_text = self.action_combo.currentText()
        
        if curr_ui_text == "Launch Application":
            widget = QWidget()
            layout = QHBoxLayout(widget)
            layout.setContentsMargins(0, 0, 0, 0)
            
            exec_input = QLineEdit(params.get("name", ""))
            exec_input.setPlaceholderText("Select app or enter command...")
            exec_input.textChanged.connect(lambda t: self.update_action_params({'name': t}))
            layout.addWidget(exec_input)
            
            pick_btn = QPushButton("Pick...")
            pick_btn.clicked.connect(lambda: self.open_app_picker(exec_input))
            layout.addWidget(pick_btn)
            
            self.param_widget_layout.addWidget(QLabel("Application Executive Command:"))
            self.param_widget_layout.addWidget(widget)
            
        elif curr_ui_text == "Simulate Shortcut":
            widget = QWidget()
            layout = QHBoxLayout(widget)
            layout.setContentsMargins(0, 0, 0, 0)
            
            initial_shortcut = params.get("keys", "")
            recorder_btn = KeyShortcutBuilder(initial_shortcut)
            layout.addWidget(recorder_btn)
            
            clear_btn = QPushButton("Clear")
            clear_btn.setStyleSheet("""
                QPushButton {
                    background-color: #3e3e44;
                    border: 1px solid #444;
                    border-radius: 4px;
                    padding: 6px 12px;
                    color: #e2e2e5;
                }
                QPushButton:hover {
                    background-color: #c0392b;
                    border-color: #e74c3c;
                    color: white;
                }
            """)
            layout.addWidget(clear_btn)
            
            recorder_btn.shortcutChanged.connect(lambda s: self.update_action_params({'keys': s}))
            clear_btn.clicked.connect(recorder_btn.clear_shortcut)
            
            self.param_widget_layout.addWidget(QLabel("Shortcut Keys:"))
            self.param_widget_layout.addWidget(widget)
            
        elif curr_ui_text == "Run Command":
            cmd_input = QLineEdit(params.get("cmd", ""))
            cmd_input.setPlaceholderText("Enter shell command to run...")
            cmd_input.textChanged.connect(lambda t: self.update_action_params({'cmd': t}))
            
            self.param_widget_layout.addWidget(QLabel("Shell Command:"))
            self.param_widget_layout.addWidget(cmd_input)
            
        elif curr_ui_text == "Play Sound (Soundboard)":
            picker = SoundboardPicker()
            picker.set_params(action_type, params)
            picker.changed.connect(lambda p: self.update_action_params(p.get('params', {}), action_type=p.get('action')))
            self.param_widget_layout.addWidget(picker)
            
        elif curr_ui_text == "Control OBS Studio":
            obs_form = QFormLayout()
            
            obs_op = QComboBox()
            obs_op.addItems(["Toggle Scene", "Set Scene", "Toggle Source Visibility", "Toggle Recording", "Toggle Streaming"])
            
            op_map = {
                "toggle_scene": "Toggle Scene",
                "set_scene": "Set Scene",
                "toggle_source": "Toggle Source Visibility",
                "toggle_recording": "Toggle Recording",
                "toggle_streaming": "Toggle Streaming"
            }
            obs_op.setCurrentText(op_map.get(params.get("action", ""), "Toggle Scene"))
            obs_form.addRow("OBS Action:", obs_op)
            
            # Context-sensitive OBS parameter widgets
            obs_sub_container = QWidget()
            obs_sub_layout = QFormLayout(obs_sub_container)
            obs_sub_layout.setContentsMargins(0, 0, 0, 0)
            
            def rebuild_obs_inputs():
                # Clear sub-layout
                for i in reversed(range(obs_sub_layout.count())): 
                    obs_sub_layout.itemAt(i).widget().setParent(None)
                
                selected_op = obs_op.currentText()
                ui_to_op = {
                    "Toggle Scene": "toggle_scene",
                    "Set Scene": "set_scene",
                    "Toggle Source Visibility": "toggle_source",
                    "Toggle Recording": "toggle_recording",
                    "Toggle Streaming": "toggle_streaming"
                }
                
                new_params = {'action': ui_to_op[selected_op]}
                
                if selected_op == "Toggle Scene":
                    s1 = QLineEdit(params.get("scene1", ""))
                    s2 = QLineEdit(params.get("scene2", ""))
                    s1.textChanged.connect(lambda t: self.update_action_params({**new_params, 'scene1': t, 'scene2': s2.text()}))
                    s2.textChanged.connect(lambda t: self.update_action_params({**new_params, 'scene1': s1.text(), 'scene2': t}))
                    obs_sub_layout.addRow("Scene 1 Name:", s1)
                    obs_sub_layout.addRow("Scene 2 Name:", s2)
                    new_params.update({'scene1': s1.text(), 'scene2': s2.text()})
                elif selected_op == "Set Scene":
                    s = QLineEdit(params.get("scene", ""))
                    s.textChanged.connect(lambda t: self.update_action_params({**new_params, 'scene': t}))
                    obs_sub_layout.addRow("Scene Name:", s)
                    new_params.update({'scene': s.text()})
                elif selected_op == "Toggle Source Visibility":
                    s = QLineEdit(params.get("scene", ""))
                    src = QLineEdit(params.get("source", ""))
                    s.textChanged.connect(lambda t: self.update_action_params({**new_params, 'scene': t, 'source': src.text()}))
                    src.textChanged.connect(lambda t: self.update_action_params({**new_params, 'scene': s.text(), 'source': t}))
                    obs_sub_layout.addRow("Scene Name:", s)
                    obs_sub_layout.addRow("Source Name:", src)
                    new_params.update({'scene': s.text(), 'source': src.text()})
                
                self.update_action_params(new_params)

            obs_op.currentTextChanged.connect(rebuild_obs_inputs)
            self.param_widget_layout.addLayout(obs_form)
            self.param_widget_layout.addWidget(obs_sub_container)
            rebuild_obs_inputs()
            
        elif curr_ui_text == "Change Audio Volume":
            picker = VolumePicker()
            picker.set_params(params)
            picker.changed.connect(self.update_action_params)
            self.param_widget_layout.addWidget(picker)
            self.update_action_params(picker.get_params())
            
        elif curr_ui_text == "Media Controls":
            form = QFormLayout()
            
            control_combo = QComboBox()
            control_combo.addItems(["Play / Pause", "Next Track", "Previous Track", "Stop"])
            
            # Map parameters
            ui_to_param = {
                "Play / Pause": "play_pause",
                "Next Track": "next",
                "Previous Track": "previous",
                "Stop": "stop"
            }
            param_to_ui = {v: k for k, v in ui_to_param.items()}
            
            control_combo.setCurrentText(param_to_ui.get(params.get("control", "play_pause"), "Play / Pause"))
            form.addRow("Control:", control_combo)
            
            player_input = QLineEdit(params.get("player", ""))
            player_input.setPlaceholderText("e.g. spotify (leave blank for default)")
            form.addRow("Player name:", player_input)
            
            def on_media_changed():
                self.update_action_params({
                    'control': ui_to_param[control_combo.currentText()],
                    'player': player_input.text().strip()
                }, action_type="media")
                
            control_combo.currentTextChanged.connect(on_media_changed)
            player_input.textChanged.connect(on_media_changed)
            
            self.param_widget_layout.addLayout(form)
            on_media_changed()

    def open_app_picker(self, line_edit):
        picker = AppPicker(self)
        if picker.exec() == QDialog.DialogCode.Accepted and picker.selected_app:
            line_edit.setText(picker.selected_app['exec'])
            if self.selected_type == 'button':
                self.update_button_property('label', picker.selected_app['name'])
                if hasattr(self, "label_input"):
                    self.label_input.setText(picker.selected_app['name'])
                
                # Extract and set system icon
                try:
                    from PyQt6.QtGui import QImage, QPainter
                    import re
                    icon_name = picker.selected_app['icon']
                    if Path(icon_name).is_absolute() and Path(icon_name).exists():
                        icon = QIcon(icon_name)
                    else:
                        icon = QIcon.fromTheme(icon_name)
                    if icon.isNull():
                        icon = QIcon.fromTheme("application-x-executable")
                    
                    pixmap = icon.pixmap(196, 196)
                    if not pixmap.isNull():
                        icons_dir = Path.home() / '.local/share/ulanzi/icons'
                        icons_dir.mkdir(parents=True, exist_ok=True)
                        
                        clean_name = re.sub(r'[^a-zA-Z0-9_]', '_', picker.selected_app['name'].lower())
                        icon_path = icons_dir / f"app_{clean_name}.png"
                        
                        image = QImage(196, 196, QImage.Format.Format_ARGB32)
                        image.fill(Qt.GlobalColor.transparent)
                        painter = QPainter(image)
                        x = (196 - pixmap.width()) // 2
                        y = (196 - pixmap.height()) // 2
                        painter.drawPixmap(x, y, pixmap)
                        painter.end()
                        
                        image.save(str(icon_path), "PNG")
                        self.update_button_property('image', str(icon_path))
                        
                        if hasattr(self, "icon_preview"):
                            self.icon_preview.setPixmap(QPixmap(str(icon_path)).scaled(42, 42, Qt.AspectRatioMode.KeepAspectRatio))
                except Exception as e:
                    logger.error(f"Failed to auto-extract app icon: {e}")

    def update_action_params(self, params_dict, action_type=None):
        """Update active button/dial action configuration"""
        if self.selected_type == 'button':
            for btn in self.config.buttons:
                if btn.index == self.selected_index:
                    if action_type:
                        btn.action_type = action_type
                    btn.action_params = params_dict
                    break
        elif self.selected_type == 'dial':
            dial_cfg = self.config.dials.setdefault(self.selected_index, {})
            event_cfg = dial_cfg.setdefault(self.selected_dial_tab, {})
            if action_type:
                event_cfg['action'] = action_type
            event_cfg['params'] = params_dict

    def build_dial_inspector(self):
        dial_cfg = self.config.dials.setdefault(self.selected_index, {})
        dial_num = self.selected_index - 16

        # ── Visual Dial Zone Selector ──────────────────────────────────
        selector_card = QFrame()
        selector_card.setObjectName("settings_card")
        sel_layout = QVBoxLayout(selector_card)
        sel_layout.setContentsMargins(15, 15, 15, 15)
        sel_layout.setSpacing(10)

        hint = QLabel(f"Dial {dial_num}  ·  Click a zone to configure its action")
        hint.setStyleSheet("font-size: 12px; color: #7a7a90; margin-bottom: 4px;")
        sel_layout.addWidget(hint)

        # Three-zone spatial bar
        zones_row = QHBoxLayout()
        zones_row.setSpacing(6)

        zone_defs = [
            ("left",  "◄",  "Rotate\nLeft",         "#1a1a2e", "#c084fc"),
            ("click", "⊙",  f"Dial {dial_num}\nPress", "#0d1521", "#3a86f0"),
            ("right", "►",  "Rotate\nRight",         "#1a1a2e", "#10b981"),
        ]

        self._dial_zone_btns = {}

        for zone_key, icon, label, bg, accent in zone_defs:
            btn = QPushButton(f"{icon}\n{label}")
            btn.setCheckable(True)
            btn.setFixedHeight(72)
            btn.setProperty("zone_key", zone_key)
            btn.setStyleSheet(f"""
                QPushButton {{
                    background-color: {bg};
                    border: 2px solid #2a2a3a;
                    border-radius: 10px;
                    color: #7a7a90;
                    font-size: 13px;
                    font-weight: bold;
                    padding: 6px;
                }}
                QPushButton:hover {{
                    border-color: {accent};
                    color: #e0e0ea;
                }}
                QPushButton:checked {{
                    border: 2px solid {accent};
                    background-color: rgba(0,0,0,0.35);
                    color: {accent};
                }}
            """)
            self._dial_zone_btns[zone_key] = btn
            zones_row.addWidget(btn)

        sel_layout.addLayout(zones_row)
        self.scroll_layout.addWidget(selector_card)

        # ── Parameter Card (updates when zone selected) ────────────────
        self._dial_param_card = QFrame()
        self._dial_param_card.setObjectName("settings_card")
        self._dial_param_layout = QVBoxLayout(self._dial_param_card)
        self._dial_param_layout.setContentsMargins(15, 15, 15, 15)
        self._dial_param_layout.setSpacing(10)
        self.scroll_layout.addWidget(self._dial_param_card)
        self.scroll_layout.addStretch()

        # Connect zone buttons
        def make_zone_handler(zone_key, all_btns):
            def on_zone_clicked():
                self.selected_dial_tab = zone_key
                for k, b in all_btns.items():
                    b.setChecked(k == zone_key)
                self._load_dial_zone_params(zone_key)
            return on_zone_clicked

        for zone_key in self._dial_zone_btns:
            self._dial_zone_btns[zone_key].clicked.connect(
                make_zone_handler(zone_key, self._dial_zone_btns)
            )

        # Auto-select the last active zone (default: click)
        default_zone = self.selected_dial_tab or "click"
        self._dial_zone_btns[default_zone].setChecked(True)
        self._load_dial_zone_params(default_zone)

    def _load_dial_zone_params(self, zone_key):
        """Rebuild the parameter form for the selected dial zone"""
        self._clear_layout(self._dial_param_layout)

        dial_cfg = self.config.dials.setdefault(self.selected_index, {})
        event_cfg = dial_cfg.setdefault(zone_key, {})
        action_type = event_cfg.get('action', 'command')
        action_params = event_cfg.get('params', {})

        zone_labels = {"left": "◄ Rotate Left", "click": "⊙ Press", "right": "► Rotate Right"}
        zone_colors = {"left": "#c084fc", "click": "#3a86f0", "right": "#10b981"}
        accent = zone_colors.get(zone_key, "#8a8a9a")

        zone_title = QLabel(zone_labels.get(zone_key, zone_key))
        zone_title.setStyleSheet(
            f"font-size: 14px; font-weight: bold; color: {accent}; margin-bottom: 4px;"
        )
        self._dial_param_layout.addWidget(zone_title)

        # Action type dropdown row
        action_row = QHBoxLayout()
        action_lbl = QLabel("Action:")
        action_lbl.setFixedWidth(65)
        self._dial_action_combo = QComboBox()
        self._dial_action_combo.addItems([
            "Launch Application", "Simulate Shortcut", "Run Command",
            "Play Sound (Soundboard)", "Control OBS Studio",
            "Change Audio Volume", "Media Controls"
        ])
        action_map = {
            "app": "Launch Application", "key": "Simulate Shortcut",
            "command": "Run Command", "obs": "Control OBS Studio",
            "volume": "Change Audio Volume", "media": "Media Controls"
        }
        curr_ui = action_map.get(action_type, "Run Command")
        if action_type == "command" and action_params.get("cmd", "").startswith(
            ("pw-play", "paplay", "play", "aplay")
        ):
            curr_ui = "Play Sound (Soundboard)"
        self._dial_action_combo.setCurrentText(curr_ui)
        action_row.addWidget(action_lbl)
        action_row.addWidget(self._dial_action_combo)
        self._dial_param_layout.addLayout(action_row)

        # Sub-param container
        self._dial_sub_container = QWidget()
        self._dial_sub_layout = QVBoxLayout(self._dial_sub_container)
        self._dial_sub_layout.setContentsMargins(0, 4, 0, 0)
        self._dial_sub_layout.setSpacing(8)
        self._dial_param_layout.addWidget(self._dial_sub_container)

        # Load current action's sub-form
        self.load_dial_parameter_form(
            zone_key, action_type, action_params,
            self._dial_action_combo, self._dial_sub_layout
        )

        def on_dial_action_changed(text):
            ui_map = {
                "Launch Application": "app", "Simulate Shortcut": "key",
                "Run Command": "command", "Play Sound (Soundboard)": "command",
                "Control OBS Studio": "obs", "Change Audio Volume": "volume",
                "Media Controls": "media"
            }
            a_type = ui_map.get(text, "command")
            self.update_dial_action_type(zone_key, a_type)
            self.load_dial_parameter_form(
                zone_key, a_type, {},
                self._dial_action_combo, self._dial_sub_layout
            )

        self._dial_action_combo.currentTextChanged.connect(on_dial_action_changed)


    def update_dial_action_type(self, tab_key, action_type):
        dial_cfg = self.config.dials.setdefault(self.selected_index, {})
        event_cfg = dial_cfg.setdefault(tab_key, {})
        event_cfg['action'] = action_type
        event_cfg['params'] = {}

    def load_dial_parameter_form(self, tab_key, action_type, params, combo_widget, container_layout):
        # Clear
        self._clear_layout(container_layout)
            
        curr_ui_text = combo_widget.currentText()
        
        # Bind change listener to update correct tab
        def make_update_handler(t_key, a_type):
            def handler(params_dict, act_type=None):
                self.selected_dial_tab = t_key
                self.update_action_params(params_dict, action_type=act_type or a_type)
            return handler
            
        update_handler = make_update_handler(tab_key, action_type)
        
        if curr_ui_text == "Launch Application":
            widget = QWidget()
            layout = QHBoxLayout(widget)
            layout.setContentsMargins(0, 0, 0, 0)
            
            exec_input = QLineEdit(params.get("name", ""))
            exec_input.setPlaceholderText("Select app or enter command...")
            exec_input.textChanged.connect(lambda t: update_handler({'name': t}))
            layout.addWidget(exec_input)
            
            pick_btn = QPushButton("Pick...")
            pick_btn.clicked.connect(lambda: self.open_app_picker(exec_input))
            layout.addWidget(pick_btn)
            
            container_layout.addWidget(QLabel("Application Executive Command:"))
            container_layout.addWidget(widget)
            
        elif curr_ui_text == "Simulate Shortcut":
            widget = QWidget()
            layout = QHBoxLayout(widget)
            layout.setContentsMargins(0, 0, 0, 0)
            
            initial_shortcut = params.get("keys", "")
            recorder_btn = KeyShortcutBuilder(initial_shortcut)
            layout.addWidget(recorder_btn)
            
            clear_btn = QPushButton("Clear")
            clear_btn.setStyleSheet("""
                QPushButton {
                    background-color: #3e3e44;
                    border: 1px solid #444;
                    border-radius: 4px;
                    padding: 6px 12px;
                    color: #e2e2e5;
                }
                QPushButton:hover {
                    background-color: #c0392b;
                    border-color: #e74c3c;
                    color: white;
                }
            """)
            layout.addWidget(clear_btn)
            
            recorder_btn.shortcutChanged.connect(lambda s: update_handler({'keys': s}))
            clear_btn.clicked.connect(recorder_btn.clear_shortcut)
            
            container_layout.addWidget(QLabel("Shortcut Keys:"))
            container_layout.addWidget(widget)
            
        elif curr_ui_text == "Run Command":
            cmd_input = QLineEdit(params.get("cmd", ""))
            cmd_input.setPlaceholderText("Enter command...")
            cmd_input.textChanged.connect(lambda t: update_handler({'cmd': t}))
            
            container_layout.addWidget(QLabel("Shell Command:"))
            container_layout.addWidget(cmd_input)
            
        elif curr_ui_text == "Play Sound (Soundboard)":
            picker = SoundboardPicker()
            picker.set_params(action_type, params)
            picker.changed.connect(lambda p: update_handler(p.get('params', {}), act_type=p.get('action')))
            container_layout.addWidget(picker)
            
        elif curr_ui_text == "Control OBS Studio":
            obs_form = QFormLayout()
            obs_op = QComboBox()
            obs_op.addItems(["Toggle Scene", "Set Scene", "Toggle Source Visibility", "Toggle Recording", "Toggle Streaming"])
            
            op_map = {
                "toggle_scene": "Toggle Scene",
                "set_scene": "Set Scene",
                "toggle_source": "Toggle Source Visibility",
                "toggle_recording": "Toggle Recording",
                "toggle_streaming": "Toggle Streaming"
            }
            obs_op.setCurrentText(op_map.get(params.get("action", ""), "Toggle Scene"))
            obs_form.addRow("OBS Action:", obs_op)
            
            obs_sub_container = QWidget()
            obs_sub_layout = QFormLayout(obs_sub_container)
            obs_sub_layout.setContentsMargins(0, 0, 0, 0)
            
            def rebuild_obs_inputs():
                self._clear_layout(obs_sub_layout)
                
                selected_op = obs_op.currentText()
                ui_to_op = {
                    "Toggle Scene": "toggle_scene",
                    "Set Scene": "set_scene",
                    "Toggle Source Visibility": "toggle_source",
                    "Toggle Recording": "toggle_recording",
                    "Toggle Streaming": "toggle_streaming"
                }
                new_params = {'action': ui_to_op[selected_op]}
                
                if selected_op == "Toggle Scene":
                    s1 = QLineEdit(params.get("scene1", ""))
                    s2 = QLineEdit(params.get("scene2", ""))
                    s1.textChanged.connect(lambda t: update_handler({**new_params, 'scene1': t, 'scene2': s2.text()}))
                    s2.textChanged.connect(lambda t: update_handler({**new_params, 'scene1': s1.text(), 'scene2': t}))
                    obs_sub_layout.addRow("Scene 1 Name:", s1)
                    obs_sub_layout.addRow("Scene 2 Name:", s2)
                    new_params.update({'scene1': s1.text(), 'scene2': s2.text()})
                elif selected_op == "Set Scene":
                    s = QLineEdit(params.get("scene", ""))
                    s.textChanged.connect(lambda t: update_handler({**new_params, 'scene': t}))
                    obs_sub_layout.addRow("Scene Name:", s)
                    new_params.update({'scene': s.text()})
                elif selected_op == "Toggle Source Visibility":
                    s = QLineEdit(params.get("scene", ""))
                    src = QLineEdit(params.get("source", ""))
                    s.textChanged.connect(lambda t: update_handler({**new_params, 'scene': t, 'source': src.text()}))
                    src.textChanged.connect(lambda t: update_handler({**new_params, 'scene': s.text(), 'source': t}))
                    obs_sub_layout.addRow("Scene Name:", s)
                    obs_sub_layout.addRow("Source Name:", src)
                    new_params.update({'scene': s.text(), 'source': src.text()})
                
                update_handler(new_params)

            obs_op.currentTextChanged.connect(rebuild_obs_inputs)
            container_layout.addLayout(obs_form)
            container_layout.addWidget(obs_sub_container)
            rebuild_obs_inputs()
            
        elif curr_ui_text == "Change Audio Volume":
            picker = VolumePicker()
            picker.set_params(params)
            picker.changed.connect(update_handler)
            container_layout.addWidget(picker)
            update_handler(picker.get_params())
            
        elif curr_ui_text == "Media Controls":
            form = QFormLayout()
            
            control_combo = QComboBox()
            control_combo.addItems(["Play / Pause", "Next Track", "Previous Track", "Stop"])
            
            # Map parameters
            ui_to_param = {
                "Play / Pause": "play_pause",
                "Next Track": "next",
                "Previous Track": "previous",
                "Stop": "stop"
            }
            param_to_ui = {v: k for k, v in ui_to_param.items()}
            
            control_combo.setCurrentText(param_to_ui.get(params.get("control", "play_pause"), "Play / Pause"))
            form.addRow("Control:", control_combo)
            
            player_input = QLineEdit(params.get("player", ""))
            player_input.setPlaceholderText("e.g. spotify (leave blank for default)")
            form.addRow("Player name:", player_input)
            
            def on_media_changed():
                update_handler({
                    'control': ui_to_param[control_combo.currentText()],
                    'player': player_input.text().strip()
                }, act_type="media")
                
            control_combo.currentTextChanged.connect(on_media_changed)
            player_input.textChanged.connect(on_media_changed)
            
            container_layout.addLayout(form)
            on_media_changed()

    def build_clock_inspector(self):
        # Find or create config for button 13
        btn_cfg = None
        for b in self.config.buttons:
            if b.index == 13:
                btn_cfg = b
                break
        if not btn_cfg:
            btn_cfg = ButtonConfig(index=13, image="", label="Clock Button", action_type="command", action_params={})
            self.config.buttons.append(btn_cfg)

        # 1. Mode Selection Card
        mode_card = QFrame()
        mode_card.setObjectName("settings_card")
        mode_layout = QFormLayout(mode_card)
        mode_layout.setContentsMargins(15, 15, 15, 15)
        mode_layout.setSpacing(10)

        card_title = QLabel("Clock Display Configuration")
        card_title.setStyleSheet("font-size: 13px; font-weight: bold; color: #a0a0b0; letter-spacing: 1px; text-transform: uppercase;")
        mode_layout.addRow(card_title)

        self.clock_mode_combo = QComboBox()
        self.clock_mode_combo.addItems([
            "System Performance Monitor (CPU/RAM)",
            "Built-in Clock (Analog & Digital)",
            "Custom Button (Show Image/Label)",
            "Currently Playing Media"
        ])
        current_mode = getattr(self.config, 'clock_mode', 1)
        self.clock_mode_combo.setCurrentIndex(current_mode)
        self.clock_mode_combo.currentIndexChanged.connect(self.on_clock_mode_changed)
        mode_layout.addRow("Display Mode:", self.clock_mode_combo)

        # Help description based on selected mode
        help_lbl = QLabel()
        help_lbl.setWordWrap(True)
        help_lbl.setStyleSheet("color: #7a7a85; font-size: 12px; margin-top: 5px;")
        if current_mode == 0:
            help_lbl.setText("STATS Mode: The big LCD screen will dynamically monitor and display your system's CPU and Memory usage.")
        elif current_mode == 1:
            help_lbl.setText("CLOCK Mode: Displays Ulanzi's built-in analog and digital clocks over your configured custom background image.")
        elif current_mode == 2:
            help_lbl.setText("BACKGROUND Mode: Hides the built-in clock overlays completely, displaying your custom button image and text. Tapping it triggers custom actions.")
        elif current_mode == 3:
            help_lbl.setText("MEDIA Mode: Displays the currently playing track title and artist dynamically. Tapping it triggers custom actions (defaults to play/pause).")
        mode_layout.addRow(help_lbl)

        self.scroll_layout.addWidget(mode_card)

        # Always build the button inspector to allow custom key mapping/action in every mode
        self.build_button_inspector()

    def on_clock_mode_changed(self, index):
        self.config.clock_mode = index
        
        # If switching to MEDIA mode (3), set default action to media -> play_pause if no custom action is set
        if index == 3:
            btn_cfg = None
            for b in self.config.buttons:
                if b.index == 13:
                    btn_cfg = b
                    break
            if btn_cfg:
                is_empty = False
                if not btn_cfg.action_type:
                    is_empty = True
                elif btn_cfg.action_type == 'command' and not btn_cfg.action_params.get('cmd'):
                    is_empty = True
                
                if is_empty:
                    btn_cfg.action_type = "media"
                    btn_cfg.action_params = {"control": "play_pause"}

        self.save_configuration_to_file()
        self.render_device_mockups()
        self.build_inspector()  # rebuild clock inspector to update sub-cards

    def on_brightness_changed(self, text):
        level = int(text.replace("%", ""))
        self.config.brightness = level

    def on_sleep_timeout_changed(self, text):
        timeout_map = {"Never": 0, "1 min": 1, "5 min": 5, "10 min": 10, "30 min": 30, "60 min": 60}
        val = timeout_map.get(text, 10)
        self.config.sleep_timeout = val

    def on_sleep_brightness_changed(self, text):
        val = int(text.replace("%", ""))
        self.config.sleep_brightness = val

    def check_daemon_running(self) -> int:
        """Returns PID if daemon is running, 0 otherwise"""
        pid_file = Path.home() / '.local/share/ulanzi/daemon.pid'
        if pid_file.exists():
            try:
                pid = int(pid_file.read_text().strip())
                # Check process existence
                os.kill(pid, 0)
                # Verify that it is actually our daemon process
                cmdline_path = Path(f'/proc/{pid}/cmdline')
                if cmdline_path.exists():
                    cmdline = cmdline_path.read_text()
                    # Check if 'daemon' or 'ulanzi' is in the cmdline
                    if 'daemon' in cmdline or 'ulanzi' in cmdline:
                        return pid
            except (ValueError, OSError, ProcessLookupError, PermissionError):
                pass
                
        # Fallback to systemd checks
        try:
            res = subprocess.run(['systemctl', '--user', 'is-active', 'ulanzi-daemon'], capture_output=True, text=True)
            if res.stdout.strip() == 'active':
                # Find PID via systemd
                res_pid = subprocess.run(['systemctl', '--user', 'show', 'ulanzi-daemon', '--property', 'MainPID'], capture_output=True, text=True)
                pid_str = res_pid.stdout.strip().split('=')[-1]
                if pid_str and pid_str != '0':
                    return int(pid_str)
        except Exception:
            pass
            
        return 0

    def update_daemon_status(self):
        """Delegates to the unified status indicator update method"""
        self.update_status_indicators()

    def update_status_indicators(self):
        """Update both connection and daemon status widgets"""
        if getattr(self, 'is_applying', False):
            return
            
        # 1. Update Daemon Status
        pid = self.check_daemon_running()
        if pid > 0:
            self.daemon_dot.setStyleSheet("color: #10b981; font-size: 16px;")
            self.daemon_status_lbl.setText(f"Daemon: Active (PID {pid})")
            self.daemon_control_btn.setText("Stop Daemon")
            if hasattr(self, 'daemon_status_dot'):
                self.daemon_status_dot.setStyleSheet("color: #10b981; font-size: 16px;")
                self.daemon_status_text.setText(f"Active (PID {pid})")
                self.daemon_toggle_btn.setText("Stop Daemon")
            if hasattr(self, 'tray_daemon_action'):
                self.tray_daemon_action.setText("Stop Daemon")
        else:
            self.daemon_dot.setStyleSheet("color: #e74c3c; font-size: 16px;")
            self.daemon_status_lbl.setText("Daemon: Inactive")
            self.daemon_control_btn.setText("Start Daemon")
            if hasattr(self, 'daemon_status_dot'):
                self.daemon_status_dot.setStyleSheet("color: #e74c3c; font-size: 16px;")
                self.daemon_status_text.setText("Inactive")
                self.daemon_toggle_btn.setText("Start Daemon")
            if hasattr(self, 'tray_daemon_action'):
                self.tray_daemon_action.setText("Start Daemon")

        # 2. Update Connection Status
        status, reason = self.check_device_connection(pid)
        if status == 'connected':
            self.conn_dot.setStyleSheet("color: #10b981; font-size: 16px;")
            if pid > 0:
                self.conn_text.setText("Device: Connected (In use)")
            else:
                self.conn_text.setText("Device: Connected")
            self.conn_retry_btn.setVisible(False)
            self.conn_fix_btn.setVisible(False)
            self.apply_btn.setEnabled(True)
            self.apply_btn.setToolTip("Apply current configuration to the hardware device")
            self.footer_status_text.setText("Console Connected")
        elif status == 'permission_denied':
            self.conn_dot.setStyleSheet("color: #f59e0b; font-size: 16px;")
            self.conn_text.setText("Device: Permission Denied")
            self.conn_retry_btn.setVisible(True)
            self.conn_fix_btn.setVisible(True)
            self.apply_btn.setEnabled(False)
            self.apply_btn.setToolTip("Cannot apply: permission denied to write to USB device. Click 'Fix Rules' above.")
            self.footer_status_text.setText(f"Permission Denied: {reason}")
        else:
            self.conn_dot.setStyleSheet("color: #e74c3c; font-size: 16px;")
            self.conn_text.setText("Device: Disconnected")
            self.conn_retry_btn.setVisible(True)
            self.conn_fix_btn.setVisible(False)
            self.apply_btn.setEnabled(False)
            self.apply_btn.setToolTip("Cannot apply: device is not connected via USB.")
            self.footer_status_text.setText(f"Not Connected: {reason}")

    def check_device_connection(self, daemon_pid: int = 0) -> tuple[str, str]:
        """
        Returns (status, error_reason)
        status: 'connected', 'permission_denied', 'not_found'
        """
        if hid is None:
            return 'not_found', "python-hidapi bindings are missing."
            
        # 1. If daemon is running, the device is connected and in use.
        if daemon_pid > 0:
            return 'connected', "In use by daemon"
            
        # 2. Check if enumerated
        devices = hid.enumerate(VENDOR_ID, PRODUCT_ID)
        if not devices:
            return 'not_found', "Device not found via USB."
            
        # 3. Check device path access (Linux specific, completely passive)
        try:
            device_info = devices[0]
            path = device_info.get('path')
            if path:
                path_str = path.decode('utf-8') if isinstance(path, bytes) else str(path)
                if os.path.exists(path_str):
                    if os.access(path_str, os.R_OK | os.W_OK):
                        return 'connected', ""
                    else:
                        return 'permission_denied', "USB permission denied. Udev rules need to be installed."
        except Exception:
            pass # Fallback to open check
            
        # 4. Fallback (try to open/close device node)
        try:
            device = UlanziDevice()
            device.close()
            return 'connected', ""
        except Exception as e:
            err_msg = str(e)
            if "open failed" in err_msg.lower() or "permission denied" in err_msg.lower() or "operation not permitted" in err_msg.lower():
                return 'permission_denied', "USB permission denied. Udev rules need to be installed."
            return 'permission_denied', f"Connection failed: {err_msg}"

    def manual_retry_connection(self):
        """Manually trigger connection check and report result if failing"""
        pid = self.check_daemon_running()
        status, reason = self.check_device_connection(pid)
        self.update_status_indicators()
        
        if status == 'connected':
            QMessageBox.information(
                self, "Connected",
                "Successfully connected to the Ulanzi StreamDeck device!"
            )
        elif status == 'permission_denied':
            QMessageBox.warning(
                self, "Permission Denied",
                f"The device was found, but permission was denied:\n{reason}\n\nPlease click 'Fix Rules' to resolve this."
            )
        else:
            QMessageBox.warning(
                self, "Not Connected",
                f"Could not find the device:\n{reason}\n\nMake sure the USB cable is securely connected and the device is powered on."
            )

    def fix_permissions_gui(self):
        """Run pkexec to install udev rules and check connection"""
        import sys
        if getattr(sys, 'frozen', False) and hasattr(sys, '_MEIPASS'):
            rules_path = Path(sys._MEIPASS) / '99-ulanzi.rules'
        else:
            rules_path = Path(__file__).parent.parent / '99-ulanzi.rules'

        if not rules_path.exists():
            QMessageBox.critical(
                self, "Error",
                f"Could not find udev rule file at:\n{rules_path}"
            )
            return

        cmd = (
            f"pkexec bash -c \"cp '{rules_path}' /etc/udev/rules.d/ "
            "&& udevadm control --reload-rules && udevadm trigger\""
        )

        try:
            self._show_notification("Running authentication helper...")
            res = subprocess.run(cmd, shell=True, capture_output=True, text=True)
            if res.returncode == 0:
                self._show_notification("Udev rules installed successfully!")
                QTimer.singleShot(1500, self.update_status_indicators)
            else:
                QMessageBox.warning(
                    self, "Failed",
                    f"Failed to install rules:\n{res.stderr or res.stdout}"
                )
        except Exception as e:
            QMessageBox.critical(self, "Error", f"Error executing authentication helper: {e}")

    def setup_system_tray(self):
        """Initialize the system tray icon and menu options"""
        self.tray_icon = QSystemTrayIcon(self)
        
        icon_path = Path(__file__).parent.parent / 'icons' / 'logo.png'
        if not icon_path.exists():
            icon_path = Path(__file__).parent.parent / 'terminal.png'
            
        if icon_path.exists():
            self.tray_icon.setIcon(QIcon(str(icon_path)))
        else:
            self.tray_icon.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_ComputerIcon))
            
        # Create tray context menu
        self.tray_menu = QMenu(self)
        
        # Add actions
        show_action = QAction("Show Manager", self)
        show_action.triggered.connect(self.show_normal)
        self.tray_menu.addAction(show_action)
        
        self.tray_menu.addSeparator()
        
        # Daemon status display/toggle action in tray
        self.tray_daemon_action = QAction("Start Daemon", self)
        self.tray_daemon_action.triggered.connect(self.toggle_daemon)
        self.tray_menu.addAction(self.tray_daemon_action)
        
        self.tray_menu.addSeparator()
        
        quit_action = QAction("Quit Ulanzi Manager", self)
        quit_action.triggered.connect(self.clean_quit)
        self.tray_menu.addAction(quit_action)
        
        self.tray_icon.setContextMenu(self.tray_menu)
        self.tray_icon.activated.connect(self.on_tray_icon_activated)
        self.tray_icon.show()
        
    def show_normal(self):
        self.show()
        self.activateWindow()
        self.raise_()
        
    def on_tray_icon_activated(self, reason):
        if reason == QSystemTrayIcon.ActivationReason.DoubleClick or reason == QSystemTrayIcon.ActivationReason.Trigger:
            if self.isVisible():
                self.hide()
            else:
                self.show_normal()

    def closeEvent(self, event):
        """Intercept window close to minimize/hide to system tray instead of exiting"""
        if QSystemTrayIcon.isSystemTrayAvailable() and self.tray_icon.isVisible():
            self.hide()
            self._show_notification("Minimized to system tray.")
            event.ignore()
        else:
            self.clean_quit()

    def clean_quit(self):
        """Cleanly disconnect, stop the daemon, and exit application"""
        # Stop daemon
        try:
            pid = self.check_daemon_running()
            if pid > 0:
                # Try systemd stop first
                try:
                    subprocess.run(['systemctl', '--user', 'stop', 'ulanzi-daemon'], timeout=5)
                except Exception:
                    pass
                # Direct kill
                try:
                    os.kill(pid, signal.SIGTERM)
                except Exception:
                    pass
        except Exception:
            pass
            
        # Cleanly disconnect device if open and run cleanup
        try:
            ApplyWorker._kill_all_daemons()
        except Exception:
            pass
            
        # Hide tray icon
        if hasattr(self, 'tray_icon'):
            self.tray_icon.hide()
            
        # Quit Qt Application
        QApplication.quit()

    def _show_notification(self, msg: str, error: bool = False):
        """Show an inline notification that auto-dismisses after 3 seconds"""
        color = "#e74c3c" if error else "#10b981"
        self.notification_label.setStyleSheet(f"color: {color}; font-weight: bold; margin-right: 15px; font-size: 13px;")
        self.notification_label.setText(msg)
        QTimer.singleShot(3000, lambda: self.notification_label.setText(""))

    def toggle_daemon(self):
        pid = self.check_daemon_running()
        
        # Stop daemon
        if pid > 0:
            # Try systemd first
            try:
                res = subprocess.run(['systemctl', '--user', 'is-active', 'ulanzi-daemon'], capture_output=True, text=True)
                if res.stdout.strip() == 'active':
                    subprocess.run(['systemctl', '--user', 'stop', 'ulanzi-daemon'])
                    self.update_daemon_status()
                    self._show_notification("Daemon stopped.")
                    return
            except Exception:
                pass
                
            # Fallback to direct kill
            try:
                os.kill(pid, signal.SIGTERM)
                self._show_notification("Daemon stopped.")
            except Exception as e:
                self._show_notification(f"Failed to stop daemon: {e}", error=True)
                
        # Start daemon
        else:
            # Check if systemd unit is available
            try:
                res_list = subprocess.run(['systemctl', '--user', 'list-unit-files', 'ulanzi-daemon.service'], capture_output=True, text=True)
                if 'ulanzi-daemon.service' in res_list.stdout:
                    subprocess.run(['systemctl', '--user', 'start', 'ulanzi-daemon'])
                    self.update_daemon_status()
                    self._show_notification("Daemon started via systemd.")
                    return
            except Exception:
                pass
                
            # Fallback to direct subprocess start
            try:
                import sys
                if getattr(sys, 'frozen', False):
                    cmd = [sys.executable, '--daemon', self.config_path]
                    env = None
                else:
                    cmd = [sys.executable, '-m', 'ulanzi_manager.daemon', self.config_path]
                    env = os.environ.copy()
                    project_root = str(Path(__file__).resolve().parent.parent)
                    env['PYTHONPATH'] = project_root + os.pathsep + env.get('PYTHONPATH', '')
                subprocess.Popen(cmd, env=env)
                self._show_notification("Daemon started.")
            except Exception as e:
                self._show_notification(f"Failed to launch daemon: {e}", error=True)
                
        self.update_daemon_status()

    def apply_configuration(self):
        """Save config, then run stop→configure→start in a background thread."""
        if not self.save_configuration_to_file():
            return

        self.is_applying = True

        # Snapshot daemon state on the main thread (fast, no I/O)
        pid = self.check_daemon_running()
        was_running = (pid > 0)
        is_systemd  = False
        if was_running:
            try:
                res = subprocess.run(
                    ['systemctl', '--user', 'is-active', 'ulanzi-daemon'],
                    capture_output=True, text=True, timeout=3
                )
                is_systemd = (res.stdout.strip() == 'active')
            except Exception:
                pass

        # Progress dialog — cancelable only with the X button (we re-enable later)
        self._apply_progress = QProgressDialog("Preparing…", None, 0, 0, self)
        self._apply_progress.setWindowTitle("Applying Configuration")
        self._apply_progress.setWindowModality(Qt.WindowModality.WindowModal)
        self._apply_progress.setCancelButton(None)
        self._apply_progress.show()
        QApplication.processEvents()

        self.apply_btn.setEnabled(False)

        # Launch worker thread
        self._apply_worker = ApplyWorker(self.config_path, was_running, is_systemd, pid)
        self._apply_worker.progress.connect(self._apply_progress.setLabelText)
        self._apply_worker.finished.connect(self._on_apply_finished)
        self._apply_worker.start()

    def _on_apply_finished(self, success: bool, error: str):
        """Called on the main thread when ApplyWorker completes."""
        self._apply_progress.close()
        self.apply_btn.setEnabled(True)
        self.is_applying = False
        self.update_daemon_status()

        if success:
            self._show_notification("✓ Configuration applied successfully!")
        else:
            QMessageBox.warning(
                self, "Hardware Apply Failed",
                f"Failed to write configuration to device:\n{error}"
            )
