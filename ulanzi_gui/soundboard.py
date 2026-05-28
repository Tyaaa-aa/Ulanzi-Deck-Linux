import shutil
import os
from pathlib import Path
from PyQt6.QtWidgets import QWidget, QVBoxLayout, QHBoxLayout, QLineEdit, QPushButton, QLabel, QFileDialog, QMessageBox
from PyQt6.QtCore import pyqtSignal

class SoundboardPicker(QWidget):
    """Widget to copy and configure audio files for soundboard buttons"""
    
    # Emitted when the play command changes
    changed = pyqtSignal(dict)
    
    def __init__(self, parent=None):
        super().__init__(parent)
        self.sound_file_path = ""
        self.init_ui()
        
    def init_ui(self):
        layout = QVBoxLayout()
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(10)
        
        # Heading
        title = QLabel("Soundboard Setup:")
        title.setStyleSheet("font-weight: bold;")
        layout.addWidget(title)
        
        # Audio File Selection Row
        file_layout = QHBoxLayout()
        self.file_input = QLineEdit()
        self.file_input.setReadOnly(True)
        self.file_input.setPlaceholderText("No audio file selected...")
        file_layout.addWidget(self.file_input)
        
        self.browse_btn = QPushButton("Browse...")
        self.browse_btn.clicked.connect(self.browse_sound)
        file_layout.addWidget(self.browse_btn)
        
        layout.addLayout(file_layout)
        
        # Playback command info (read-only for user clarity)
        self.cmd_label = QLabel("Command: -")
        self.cmd_label.setStyleSheet("font-family: monospace; font-size: 11px; color: #888;")
        layout.addWidget(self.cmd_label)
        
        self.setLayout(layout)

    def browse_sound(self):
        file_filter = "Audio Files (*.wav *.mp3 *.ogg *.flac *.m4a)"
        file_path, _ = QFileDialog.getOpenFileName(self, "Select Audio File", "", file_filter)
        if not file_path:
            return
            
        src_path = Path(file_path)
        dest_dir = Path.home() / '.local/share/ulanzi/sounds'
        
        try:
            dest_dir.mkdir(parents=True, exist_ok=True)
            dest_path = dest_dir / src_path.name
            
            # Copy file to local share directory
            shutil.copy2(src_path, dest_path)
            
            self.sound_file_path = str(dest_path)
            self.file_input.setText(src_path.name)
            
            self.emit_params()
        except Exception as e:
            QMessageBox.critical(self, "Copy Failed", f"Failed to import audio file to soundboard:\n{e}")

    def get_audio_player(self) -> str:
        """Find the best available command-line audio player on the system"""
        for player in ['pw-play', 'paplay', 'play', 'aplay']:
            if shutil.which(player):
                return player
        return 'aplay'

    def get_params(self) -> dict:
        """Construct the configuration for the command action type"""
        if not self.sound_file_path:
            return {}
            
        player = self.get_audio_player()
        # Escaping quotes in filename
        escaped_path = self.sound_file_path.replace("'", "'\\''")
        
        return {
            'action': 'command',
            'params': {
                'cmd': f"{player} '{escaped_path}'"
            }
        }

    def set_params(self, action_type: str, action_params: dict):
        """Restore widget state from a command action"""
        cmd = action_params.get('cmd', '')
        if not cmd:
            self.clear()
            return
            
        # Parse path from command: e.g. "pw-play '/path/to/sound.wav'"
        parts = cmd.split("'", 2)
        if len(parts) >= 2:
            sound_path = parts[1]
            if Path(sound_path).exists():
                self.sound_file_path = sound_path
                self.file_input.setText(Path(sound_path).name)
                self.cmd_label.setText(f"Command: {cmd}")
                return
                
        self.clear()

    def clear(self):
        self.sound_file_path = ""
        self.file_input.clear()
        self.cmd_label.setText("Command: -")

    def emit_params(self):
        params = self.get_params()
        if params:
            self.cmd_label.setText(f"Command: {params['params']['cmd']}")
            self.changed.emit(params)
