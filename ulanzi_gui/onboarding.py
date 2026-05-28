import subprocess
from pathlib import Path
from PyQt6.QtWidgets import (QDialog, QVBoxLayout, QLabel, QPushButton,
                             QHBoxLayout, QMessageBox, QProgressBar)
from PyQt6.QtCore import Qt, QTimer
from ulanzi_manager.device import UlanziDevice, hid, VENDOR_ID, PRODUCT_ID


class OnboardingDialog(QDialog):
    """Dialog to guide user through setting up USB permissions"""

    POLL_INTERVAL_MS = 2000   # retry every 2 s
    POLL_MAX_ATTEMPTS = 20    # give up after 40 s

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Ulanzi Device Setup")
        self.setFixedSize(520, 360)
        self._poll_timer = QTimer(self)
        self._poll_timer.timeout.connect(self._poll_connection)
        self._poll_attempts = 0
        self.init_ui()

    # ──────────────────────────────────────────────────────────────────────
    def init_ui(self):
        layout = QVBoxLayout()
        layout.setSpacing(14)
        layout.setContentsMargins(22, 22, 22, 22)

        # Title
        title = QLabel("Ulanzi D200/D200X Connection Setup")
        title.setStyleSheet("font-size: 16px; font-weight: bold; color: #ff5555;")
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(title)

        # Status / description text
        self.desc_label = QLabel()
        self.desc_label.setWordWrap(True)
        self.desc_label.setStyleSheet("font-size: 13px; line-height: 1.4;")
        layout.addWidget(self.desc_label)

        # Polling progress / countdown
        self.poll_label = QLabel("")
        self.poll_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.poll_label.setStyleSheet("font-size: 12px; color: #3a86f0;")
        self.poll_label.setVisible(False)
        layout.addWidget(self.poll_label)

        self.poll_bar = QProgressBar()
        self.poll_bar.setRange(0, self.POLL_MAX_ATTEMPTS)
        self.poll_bar.setValue(0)
        self.poll_bar.setTextVisible(False)
        self.poll_bar.setFixedHeight(6)
        self.poll_bar.setStyleSheet("""
            QProgressBar { background-color: #1e1e2e; border-radius: 3px; }
            QProgressBar::chunk { background-color: #3a86f0; border-radius: 3px; }
        """)
        self.poll_bar.setVisible(False)
        layout.addWidget(self.poll_bar)

        # Action Buttons
        btn_layout = QHBoxLayout()
        btn_layout.setSpacing(10)

        self.fix_btn = QPushButton("Fix Permissions Automatically")
        self.fix_btn.setStyleSheet("""
            QPushButton {
                background-color: #3a86f0; color: white;
                font-weight: bold; padding: 8px 15px; border-radius: 6px;
            }
            QPushButton:hover { background-color: #2a76e0; }
            QPushButton:disabled { background-color: #2a2a38; color: #555; }
        """)
        self.fix_btn.clicked.connect(self.fix_permissions)
        btn_layout.addWidget(self.fix_btn)

        self.retry_btn = QPushButton("Retry Connection")
        self.retry_btn.setStyleSheet("""
            QPushButton {
                background-color: #2e2e3e; color: #c0c0ca;
                padding: 8px 15px; border-radius: 6px;
                border: 1px solid #3a3a4a;
            }
            QPushButton:hover { background-color: #3a3a4e; }
        """)
        self.retry_btn.clicked.connect(self.check_connection)
        btn_layout.addWidget(self.retry_btn)

        layout.addLayout(btn_layout)

        # Manual instructions box
        self.manual_label = QLabel()
        self.manual_label.setWordWrap(True)
        self.manual_label.setTextInteractionFlags(
            Qt.TextInteractionFlag.TextSelectableByMouse
        )
        self.manual_label.setStyleSheet(
            "font-family: monospace; font-size: 11px; "
            "background-color: #111118; padding: 10px; "
            "border-radius: 6px; color: #9a9ab0;"
        )
        layout.addWidget(self.manual_label)

        self.setLayout(layout)
        self.update_status()

    # ──────────────────────────────────────────────────────────────────────
    def update_status(self):
        """Analyse connection state and refresh the UI"""
        if hid is None:
            self.desc_label.setText(
                "<b>Error: python-hidapi bindings are missing.</b><br><br>"
                "Please make sure they are installed in your virtual environment."
            )
            self.fix_btn.setEnabled(False)
            self.manual_label.setText("pip install hidapi")
            return

        devices = hid.enumerate(VENDOR_ID, PRODUCT_ID)
        if not devices:
            self.desc_label.setText(
                "<b>Status: Device not found.</b><br><br>"
                "Make sure your Ulanzi StreamDeck is plugged in via USB and switched on.<br>"
                "If it is connected, try a different USB port or cable."
            )
            self.fix_btn.setEnabled(False)
            self.manual_label.setText("lsusb | grep 2207")
            return

        self.desc_label.setText(
            "<b>Status: Permission Denied.</b><br><br>"
            "Your Ulanzi device is connected, but your user account doesn't have "
            "permission to write to it.<br>"
            "You need to install the udev rules to allow non-root USB communication."
        )
        self.fix_btn.setEnabled(True)

        import sys
        if getattr(sys, 'frozen', False) and hasattr(sys, '_MEIPASS'):
            rules_path = Path(sys._MEIPASS) / '99-ulanzi.rules'
        else:
            rules_path = Path(__file__).parent.parent / '99-ulanzi.rules'

        self.manual_label.setText(
            f"sudo cp '{rules_path}' /etc/udev/rules.d/\n"
            "sudo udevadm control --reload-rules\n"
            "sudo udevadm trigger"
        )

    # ──────────────────────────────────────────────────────────────────────
    def fix_permissions(self):
        """Run pkexec to install udev rules, then auto-poll for connection."""
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
            res = subprocess.run(cmd, shell=True, capture_output=True, text=True)
            if res.returncode == 0:
                self._start_polling()
            else:
                QMessageBox.warning(
                    self, "Failed",
                    f"Failed to install rules:\n{res.stderr or res.stdout}"
                )
        except Exception as e:
            QMessageBox.critical(self, "Error", f"Error executing authentication helper: {e}")

    # ──────────────────────────────────────────────────────────────────────
    def _start_polling(self):
        """Begin automatically retrying the connection every 2 s."""
        self._poll_attempts = 0
        self.fix_btn.setEnabled(False)

        self.desc_label.setText(
            "<b>Udev rules installed ✓</b><br><br>"
            "Waiting for device to re-enumerate…<br>"
            "<span style='color:#7a7a90;'>Unplug and re-plug the device if it "
            "doesn't connect automatically within 40 s.</span>"
        )
        self.poll_label.setVisible(True)
        self.poll_bar.setVisible(True)
        self.poll_bar.setValue(0)
        self.manual_label.setVisible(False)

        self._poll_timer.start(self.POLL_INTERVAL_MS)
        self._update_poll_label()

    def _update_poll_label(self):
        remaining = self.POLL_MAX_ATTEMPTS - self._poll_attempts
        secs = remaining * (self.POLL_INTERVAL_MS // 1000)
        self.poll_label.setText(
            f"⟳  Checking connection…  (auto-retry for ~{secs} s)"
        )

    def _poll_connection(self):
        self._poll_attempts += 1
        self.poll_bar.setValue(self._poll_attempts)
        self._update_poll_label()

        # Try to open the device
        try:
            device = UlanziDevice()
            device.close()
            # Success!
            self._poll_timer.stop()
            self.accept()
            return
        except Exception:
            pass

        # Give up after max attempts
        if self._poll_attempts >= self.POLL_MAX_ATTEMPTS:
            self._poll_timer.stop()
            self.poll_label.setText(
                "\u26a0  Auto-retry timed out.  Unplug & re-plug the device, "
                "then press 'Retry Connection'."
            )
            self.poll_label.setStyleSheet("font-size: 12px; color: #f59e0b;")
            self.fix_btn.setEnabled(True)
            self.manual_label.setVisible(True)
            self.update_status()

    # ──────────────────────────────────────────────────────────────────────
    def check_connection(self):
        """Manual retry button — attempt to open device immediately."""
        try:
            device = UlanziDevice()
            device.close()
            self._poll_timer.stop()
            self.accept()
        except Exception as e:
            self.update_status()
            self._show_still_failing(str(e))

    def _show_still_failing(self, detail: str):
        msg = QMessageBox(self)
        msg.setWindowTitle("Still unable to connect")
        msg.setIcon(QMessageBox.Icon.Warning)
        msg.setText(
            "The device could not be opened yet.\n\n"
            "Try unplugging and re-plugging the USB cable, then press Retry."
        )
        msg.setDetailedText(detail)
        msg.exec()
