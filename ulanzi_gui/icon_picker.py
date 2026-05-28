import os
from pathlib import Path
from PyQt6.QtWidgets import (QDialog, QVBoxLayout, QHBoxLayout, QLineEdit, 
                             QScrollArea, QWidget, QGridLayout, QPushButton, 
                             QLabel, QComboBox, QColorDialog, QMessageBox)
from PyQt6.QtGui import QColor, QPainter, QImage, QPixmap, QIcon
from PyQt6.QtSvg import QSvgRenderer
from PyQt6.QtCore import Qt, QByteArray, QRectF, QSize, QTimer
from ulanzi_gui.lucide_data import LUCIDE_ICONS

# Preset Colors (Name -> Hex)
COLOR_PRESETS = {
    "Black": "#000000",
    "Dark Gray": "#1e1e24",
    "Light Gray": "#7f8c8d",
    "White": "#ffffff",
    "Red": "#e74c3c",
    "Green": "#2ecc71",
    "Blue": "#3498db",
    "Purple": "#9b59b6",
    "Orange": "#e67e22",
    "Yellow": "#f1c40f",
    "Cyan": "#1abc9c"
}

class IconPicker(QDialog):
    """Dialog to customize and render Lucide icons to device-compatible PNGs"""
    
    def __init__(self, parent=None, current_icon_name=None, current_bg_color=None, current_fg_color=None):
        super().__init__(parent)
        self.setWindowTitle("Icon Picker")
        self.setMinimumSize(600, 480)
        self.selected_icon_name = current_icon_name
        self.selected_icon_path = None
        
        # Colors defaults
        self.bg_color = current_bg_color or "#1e1e24"
        self.fg_color = current_fg_color or "#ffffff"
        
        self.icon_buttons = {}
        
        # Custom styling to match Ulanzi D200X Manager's premium theme
        self.setStyleSheet("""
            QDialog {
                background-color: #0f0f14;
                color: #e2e2e5;
                font-family: 'Segoe UI', -apple-system, BlinkMacSystemFont, Roboto, sans-serif;
            }
            QLabel {
                color: #8e8e9f;
                font-size: 13px;
            }
            QLineEdit {
                background-color: #1c1c28;
                border: 1px solid #2e2e3e;
                border-radius: 8px;
                padding: 8px 12px;
                color: #ffffff;
                font-size: 13px;
            }
            QLineEdit:hover {
                border: 1px solid #3e3e54;
            }
            QLineEdit:focus {
                border: 2px solid #3a86f0;
                background-color: #181c28;
            }
            QComboBox {
                background-color: #1c1c28;
                border: 1px solid #2e2e3e;
                border-radius: 8px;
                padding: 8px 12px;
                color: #ffffff;
                font-size: 13px;
            }
            QComboBox:hover {
                border: 1px solid #3e3e54;
            }
            QComboBox::drop-down {
                border: none;
                padding-right: 12px;
            }
            QScrollArea {
                border: 1px solid #22222e;
                background-color: #0c0c10;
                border-radius: 8px;
            }
            QPushButton#use_icon_btn {
                background-color: #2ecc71;
                color: white;
                font-weight: bold;
                padding: 10px;
                border-radius: 8px;
                border: none;
            }
            QPushButton#use_icon_btn:hover {
                background-color: #27ae60;
            }
            QPushButton#use_icon_btn:disabled {
                background-color: #2c3e50;
                color: #7f8c8d;
            }
            QPushButton#cancel_btn {
                background-color: #2e2e3e;
                color: #e2e2e5;
                border: 1px solid #3e3e54;
                border-radius: 8px;
                padding: 8px;
            }
            QPushButton#cancel_btn:hover {
                background-color: #3e3e54;
            }
        """)
        
        self.init_ui()
        self.populate_icons()
        
    def init_ui(self):
        main_layout = QHBoxLayout()
        main_layout.setSpacing(15)
        main_layout.setContentsMargins(15, 15, 15, 15)
        
        # Left Side - Icon Grid & Search
        left_layout = QVBoxLayout()
        
        self.search_input = QLineEdit()
        self.search_input.setPlaceholderText("Search icons...")
        self.search_input.textChanged.connect(self.filter_icons)
        left_layout.addWidget(self.search_input)
        
        self.scroll_area = QScrollArea()
        self.scroll_area.setWidgetResizable(True)
        self.grid_widget = QWidget()
        self.grid_widget.setStyleSheet("background-color: transparent;")
        self.grid_layout = QGridLayout(self.grid_widget)
        self.grid_layout.setSpacing(8)
        self.scroll_area.setWidget(self.grid_widget)
        left_layout.addWidget(self.scroll_area)
        
        main_layout.addLayout(left_layout, stretch=3)
        
        # Right Side - Customizer & Preview
        right_layout = QVBoxLayout()
        right_layout.setSpacing(15)
        right_layout.setAlignment(Qt.AlignmentFlag.AlignTop)
        
        # Preview Area
        preview_label = QLabel("Preview (196x196)")
        preview_label.setStyleSheet("font-weight: bold; color: #ffffff;")
        right_layout.addWidget(preview_label)
        
        self.preview_box = QLabel()
        self.preview_box.setFixedSize(140, 140)
        self.preview_box.setStyleSheet("border: 2px solid #232332; background-color: #08080a; border-radius: 12px;")
        self.preview_box.setAlignment(Qt.AlignmentFlag.AlignCenter)
        right_layout.addWidget(self.preview_box)
        
        # BG Color Picker
        bg_label = QLabel("Background Color:")
        right_layout.addWidget(bg_label)
        
        self.bg_combo = QComboBox()
        self.bg_combo.addItems(list(COLOR_PRESETS.keys()) + ["Custom..."])
        
        # Set initial text
        bg_name = None
        for name, val in COLOR_PRESETS.items():
            if val.lower() == self.bg_color.lower():
                bg_name = name
                break
        if bg_name:
            self.bg_combo.setCurrentText(bg_name)
        else:
            self.bg_combo.setCurrentText("Custom...")
            
        self.bg_combo.currentTextChanged.connect(self.on_bg_changed)
        right_layout.addWidget(self.bg_combo)
        
        # FG Color Picker
        fg_label = QLabel("Foreground (Icon) Color:")
        right_layout.addWidget(fg_label)
        
        self.fg_combo = QComboBox()
        self.fg_combo.addItems(list(COLOR_PRESETS.keys()) + ["Custom..."])
        
        # Set initial text
        fg_name = None
        for name, val in COLOR_PRESETS.items():
            if val.lower() == self.fg_color.lower():
                fg_name = name
                break
        if fg_name:
            self.fg_combo.setCurrentText(fg_name)
        else:
            self.fg_combo.setCurrentText("Custom...")
            
        self.fg_combo.currentTextChanged.connect(self.on_fg_changed)
        right_layout.addWidget(self.fg_combo)
        
        # Action Buttons
        right_layout.addStretch()
        
        self.save_btn = QPushButton("Use Icon")
        self.save_btn.setObjectName("use_icon_btn")
        self.save_btn.setEnabled(self.selected_icon_name is not None)
        self.save_btn.clicked.connect(self.generate_and_save_png)
        right_layout.addWidget(self.save_btn)
        
        self.cancel_btn = QPushButton("Cancel")
        self.cancel_btn.setObjectName("cancel_btn")
        self.cancel_btn.clicked.connect(self.reject)
        right_layout.addWidget(self.cancel_btn)
        
        main_layout.addLayout(right_layout, stretch=1)
        self.setLayout(main_layout)
 
    def populate_icons(self, filter_text=""):
        # Clear existing buttons
        for i in reversed(range(self.grid_layout.count())): 
            self.grid_layout.itemAt(i).widget().setParent(None)
            
        col_count = 5
        row = 0
        col = 0
        
        self.icon_buttons = {}
        active_btn = None
        
        for icon_name, svg_xml in LUCIDE_ICONS.items():
            if filter_text and filter_text.lower() not in icon_name.lower():
                continue
                
            btn = QPushButton()
            btn.setCheckable(True)
            btn.setFixedSize(56, 56)
            btn.setProperty("icon_name", icon_name)
            
            # Render small preview icon
            pixmap = self.render_to_pixmap(svg_xml, "#00000000", "#ffffff", 32)
            btn.setIcon(QIcon(pixmap))
            btn.setIconSize(QSize(28, 28))
            
            btn.setStyleSheet("""
                QPushButton {
                    background-color: #15151f;
                    border: 1px solid #232332;
                    border-radius: 8px;
                }
                QPushButton:hover {
                    background-color: #1d1d2b;
                    border-color: #3e3e54;
                }
                QPushButton:checked {
                    background-color: #181c28;
                    border: 2px solid #3a86f0;
                }
            """)
            
            if icon_name == self.selected_icon_name:
                btn.setChecked(True)
                active_btn = btn
            
            btn.setToolTip(icon_name)
            btn.clicked.connect(self.select_icon)
            
            self.icon_buttons[icon_name] = btn
            self.grid_layout.addWidget(btn, row, col)
            col += 1
            if col >= col_count:
                col = 0
                row += 1
                
        # Update preview box if icon is pre-selected
        if self.selected_icon_name:
            self.update_preview()
            if active_btn:
                # Scroll down to the selected icon button after UI rendering layout completes
                QTimer.singleShot(100, lambda: self.scroll_area.ensureWidgetVisible(active_btn))

    def filter_icons(self, text):
        self.populate_icons(text)

    def select_icon(self):
        btn = self.sender()
        icon_name = btn.property("icon_name")
        
        # Uncheck all other icon buttons
        for name, button in self.icon_buttons.items():
            if name != icon_name:
                button.setChecked(False)
                
        self.selected_icon_name = icon_name
        btn.setChecked(True)
        self.save_btn.setEnabled(True)
        self.update_preview()

    def on_bg_changed(self, text):
        if text == "Custom...":
            color = QColorDialog.getColor(QColor(self.bg_color), self, "Select Background Color")
            if color.isValid():
                self.bg_color = color.name()
            else:
                self.bg_combo.setCurrentText("Dark Gray")
        else:
            self.bg_color = COLOR_PRESETS[text]
        self.update_preview()

    def on_fg_changed(self, text):
        if text == "Custom...":
            color = QColorDialog.getColor(QColor(self.fg_color), self, "Select Foreground Color")
            if color.isValid():
                self.fg_color = color.name()
            else:
                self.fg_combo.setCurrentText("White")
        else:
            self.fg_color = COLOR_PRESETS[text]
        self.update_preview()

    def update_preview(self):
        if not self.selected_icon_name:
            return
            
        svg_xml = LUCIDE_ICONS[self.selected_icon_name]
        pixmap = self.render_to_pixmap(svg_xml, self.bg_color, self.fg_color, 120)
        self.preview_box.setPixmap(pixmap)

    def render_to_pixmap(self, svg_xml: str, bg_color_hex: str, fg_color_hex: str, size: int) -> QPixmap:
        """Helper to render vector SVG directly to QPixmap"""
        # Inject stroke color
        svg_colored = svg_xml.replace('stroke="currentColor"', f'stroke="{fg_color_hex}"')
        
        image = QImage(size, size, QImage.Format.Format_ARGB32)
        
        # Handle transparent background
        if bg_color_hex == "#00000000" or bg_color_hex == "transparent":
            image.fill(Qt.GlobalColor.transparent)
        else:
            image.fill(QColor(bg_color_hex))
            
        painter = QPainter(image)
        renderer = QSvgRenderer(QByteArray(svg_colored.encode('utf-8')))
        
        # Inner padding for visual balance (15% padding)
        pad = size * 0.15
        inner_size = size - (pad * 2)
        renderer.render(painter, QRectF(pad, pad, inner_size, inner_size))
        painter.end()
        
        return QPixmap.fromImage(image)

    def generate_and_save_png(self):
        """Generate high-quality 196x196 PNG and save to ~/.local/share/ulanzi/icons/"""
        if not self.selected_icon_name:
            return
            
        # Target path
        icons_dir = Path.home() / '.local/share/ulanzi/icons'
        try:
            icons_dir.mkdir(parents=True, exist_ok=True)
        except Exception as e:
            QMessageBox.critical(self, "Error", f"Failed to create icon directory:\n{e}")
            return
            
        clean_bg = self.bg_color.replace("#", "")
        clean_fg = self.fg_color.replace("#", "")
        filename = f"{self.selected_icon_name}_bg_{clean_bg}_fg_{clean_fg}.png"
        output_path = icons_dir / filename
        
        # Render high res 196x196
        svg_xml = LUCIDE_ICONS[self.selected_icon_name]
        pixmap = self.render_to_pixmap(svg_xml, self.bg_color, self.fg_color, 196)
        
        try:
            pixmap.save(str(output_path), "PNG")
            self.selected_icon_path = str(output_path)
            self.accept()
        except Exception as e:
            QMessageBox.critical(self, "Save Failed", f"Could not save rendered icon file:\n{e}")
