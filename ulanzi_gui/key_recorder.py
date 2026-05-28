from PyQt6.QtWidgets import QWidget, QHBoxLayout, QCheckBox, QComboBox, QLabel
from PyQt6.QtCore import pyqtSignal

KEYS_LIST = [
    # Alphabet
    "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m",
    "n", "o", "p", "q", "r", "s", "t", "u", "v", "w", "x", "y", "z",
    # Numbers
    "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
    # Function keys
    "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    # Navigation / Editing
    "space", "Return", "Escape", "Tab", "BackSpace", "Delete", "Insert",
    "Home", "End", "Page_Up", "Page_Down", "Left", "Up", "Right", "Down",
    # Media keys
    "XF86AudioPlay", "XF86AudioNext", "XF86AudioPrev", "XF86AudioStop",
    "XF86AudioMute", "XF86AudioRaiseVolume", "XF86AudioLowerVolume",
    "Print"
]

class KeyShortcutBuilder(QWidget):
    # Emitted when the shortcut is changed (e.g. "ctrl+alt+t")
    shortcutChanged = pyqtSignal(str)
    
    def __init__(self, initial_shortcut="", parent=None):
        super().__init__(parent)
        self.block_updates = True
        
        layout = QHBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(6)
        
        # Modifiers Checkboxes
        self.ctrl_check = QCheckBox("Ctrl")
        self.alt_check = QCheckBox("Alt")
        self.shift_check = QCheckBox("Shift")
        self.super_check = QCheckBox("Super")
        
        layout.addWidget(self.ctrl_check)
        layout.addWidget(self.alt_check)
        layout.addWidget(self.shift_check)
        layout.addWidget(self.super_check)
        
        layout.addWidget(QLabel("+"))
        
        # Key Selector
        self.key_combo = QComboBox()
        self.key_combo.setEditable(True)
        self.key_combo.setPlaceholderText("Key (e.g. s, Return)")
        self.key_combo.addItems(KEYS_LIST)
        layout.addWidget(self.key_combo, stretch=1)
        
        # Signal Connections
        self.ctrl_check.toggled.connect(self.emit_shortcut)
        self.alt_check.toggled.connect(self.emit_shortcut)
        self.shift_check.toggled.connect(self.emit_shortcut)
        self.super_check.toggled.connect(self.emit_shortcut)
        self.key_combo.currentTextChanged.connect(self.emit_shortcut)
        
        self.set_shortcut(initial_shortcut)
        self.block_updates = False

    def set_shortcut(self, shortcut: str):
        self.block_updates = True
        
        # Reset states
        self.ctrl_check.setChecked(False)
        self.alt_check.setChecked(False)
        self.shift_check.setChecked(False)
        self.super_check.setChecked(False)
        self.key_combo.setCurrentText("")
        
        if not shortcut:
            self.block_updates = False
            return
            
        parts = shortcut.split('+')
        for part in parts:
            part_clean = part.strip().lower()
            if part_clean == "ctrl":
                self.ctrl_check.setChecked(True)
            elif part_clean == "alt":
                self.alt_check.setChecked(True)
            elif part_clean == "shift":
                self.shift_check.setChecked(True)
            elif part_clean in ("super", "meta", "win"):
                self.super_check.setChecked(True)
            else:
                matched = False
                for k in KEYS_LIST:
                    if k.lower() == part.strip().lower():
                        self.key_combo.setCurrentText(k)
                        matched = True
                        break
                if not matched:
                    self.key_combo.setCurrentText(part.strip())
                    
        self.block_updates = False

    def clear_shortcut(self):
        self.set_shortcut("")
        self.emit_shortcut()

    def emit_shortcut(self):
        if self.block_updates:
            return
            
        mods = []
        if self.ctrl_check.isChecked():
            mods.append("ctrl")
        if self.alt_check.isChecked():
            mods.append("alt")
        if self.shift_check.isChecked():
            mods.append("shift")
        if self.super_check.isChecked():
            mods.append("super")
            
        key = self.key_combo.currentText().strip()
        if key:
            mods.append(key)
            
        shortcut_str = "+".join(mods)
        self.shortcutChanged.emit(shortcut_str)
