import subprocess
from PyQt6.QtWidgets import QWidget, QVBoxLayout, QFormLayout, QComboBox, QLineEdit, QRadioButton, QButtonGroup, QLabel, QSpinBox, QCheckBox
from PyQt6.QtCore import pyqtSignal

class VolumePicker(QWidget):
    """Widget to configure Volume and Mute actions for buttons or dials"""
    
    # Emitted when the volume configuration parameters change
    changed = pyqtSignal(dict)
    
    def __init__(self, parent=None):
        super().__init__(parent)
        self.init_ui()
        
    def init_ui(self):
        layout = QVBoxLayout()
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(12)
        
        # 1. Target Type Selection
        target_label = QLabel("Target Output")
        target_label.setStyleSheet("font-size: 12px; font-weight: bold; color: #8a8a9a; letter-spacing: 1px;")
        layout.addWidget(target_label)
        
        self.btn_group = QButtonGroup(self)
        
        self.target_default = QRadioButton("Default Output Device")
        self.target_default.setChecked(True)
        self.btn_group.addButton(self.target_default)
        layout.addWidget(self.target_default)
        
        self.target_device = QRadioButton("Specific Output Device")
        self.btn_group.addButton(self.target_device)
        layout.addWidget(self.target_device)
        
        self.target_app = QRadioButton("Specific Application Stream")
        self.btn_group.addButton(self.target_app)
        layout.addWidget(self.target_app)
        
        # 2. Parameters form
        self.form_layout = QFormLayout()
        self.form_layout.setSpacing(8)
        self.form_layout.setContentsMargins(0, 8, 0, 0)
        
        # Device Selection ComboBox
        self.device_combo = QComboBox()
        self.device_combo.setPlaceholderText("Select device...")
        self.form_layout.addRow("Audio Device:", self.device_combo)
        
        # Application name input
        self.app_input = QLineEdit()
        self.app_input.setPlaceholderText("Application name (e.g. Spotify)")
        self.form_layout.addRow("App Name:", self.app_input)
        
        # Operation selection (Up, Down, Mute)
        self.op_combo = QComboBox()
        self.op_combo.addItems(["Volume Up", "Volume Down", "Toggle Mute"])
        self.form_layout.addRow("Operation:", self.op_combo)

        # Volume Step percentage
        self.step_spin = QSpinBox()
        self.step_spin.setRange(1, 50)
        self.step_spin.setSuffix("%")
        self.step_spin.setValue(5)
        self.form_layout.addRow("Volume Step:", self.step_spin)

        # Limit to 100% volume checkbox
        self.limit_check = QCheckBox("Limit to 100% volume")
        self.limit_check.setChecked(True)
        self.form_layout.addRow("", self.limit_check)
        
        layout.addLayout(self.form_layout)
        self.setLayout(layout)
        
        # Signals
        self.btn_group.buttonClicked.connect(self.update_widget_visibility)
        self.device_combo.currentTextChanged.connect(self.emit_params)
        self.app_input.textChanged.connect(self.emit_params)
        self.op_combo.currentTextChanged.connect(self.emit_params)
        self.step_spin.valueChanged.connect(self.emit_params)
        self.limit_check.toggled.connect(self.emit_params)
        
        self.load_devices()
        self.update_widget_visibility()

    def load_devices(self):
        """Query system audio outputs (sinks) using pactl"""
        self.device_combo.clear()
        try:
            res = subprocess.run(['pactl', 'list', 'sinks', 'short'], capture_output=True, text=True)
            if res.returncode == 0:
                for line in res.stdout.strip().split('\n'):
                    if not line:
                        continue
                    parts = line.split('\t')
                    if len(parts) >= 2:
                        sink_name = parts[1]
                        self.device_combo.addItem(sink_name)
        except Exception:
            self.device_combo.addItem("@DEFAULT_AUDIO_SINK@")

    def update_widget_visibility(self):
        is_device = self.target_device.isChecked()
        is_app = self.target_app.isChecked()
        
        # Show/Hide fields based on selected target
        self.device_combo.setVisible(is_device)
        self.form_layout.labelForField(self.device_combo).setVisible(is_device)
        
        self.app_input.setVisible(is_app)
        self.form_layout.labelForField(self.app_input).setVisible(is_app)
        
        # Hide step and limit for Mute operation
        is_mute = self.op_combo.currentText() == "Toggle Mute"
        self.step_spin.setVisible(not is_mute)
        self.form_layout.labelForField(self.step_spin).setVisible(not is_mute)
        self.limit_check.setVisible(not is_mute and self.op_combo.currentText() == "Volume Up")
        
        self.emit_params()

    def get_params(self) -> dict:
        """Construct configuration parameters for the volume action"""
        # Map Operation UI text to parameters
        op_map = {
            "Volume Up": "up",
            "Volume Down": "down",
            "Toggle Mute": "mute"
        }
        
        op = op_map.get(self.op_combo.currentText(), "up")
        
        params = {
            'operation': op,
            'step': self.step_spin.value(),
            'limit_100': self.limit_check.isChecked()
        }
        
        if self.target_default.isChecked():
            params['target'] = 'default'
        elif self.target_device.isChecked():
            params['target'] = 'device'
            params['name'] = self.device_combo.currentText()
        elif self.target_app.isChecked():
            params['target'] = 'app'
            params['name'] = self.app_input.text().strip()
            
        return params

    def set_params(self, params: dict):
        """Restore widget state from saved configuration parameters"""
        self.btn_group.blockSignals(True)
        self.device_combo.blockSignals(True)
        self.app_input.blockSignals(True)
        self.op_combo.blockSignals(True)
        self.step_spin.blockSignals(True)
        self.limit_check.blockSignals(True)
        
        target = params.get('target', 'default')
        if target == 'default':
            self.target_default.setChecked(True)
        elif target == 'device':
            self.target_device.setChecked(True)
            self.device_combo.setCurrentText(params.get('name', ''))
        elif target == 'app':
            self.target_app.setChecked(True)
            self.app_input.setText(params.get('name', ''))
            
        op_map = {
            "up": "Volume Up",
            "down": "Volume Down",
            "mute": "Toggle Mute"
        }
        self.op_combo.setCurrentText(op_map.get(params.get('operation', 'up'), "Volume Up"))
        self.step_spin.setValue(params.get('step', 5))
        self.limit_check.setChecked(params.get('limit_100', True))
        
        self.btn_group.blockSignals(False)
        self.device_combo.blockSignals(False)
        self.app_input.blockSignals(False)
        self.op_combo.blockSignals(False)
        self.step_spin.blockSignals(False)
        self.limit_check.blockSignals(False)
        
        self.update_widget_visibility()

    def emit_params(self):
        self.changed.emit(self.get_params())
