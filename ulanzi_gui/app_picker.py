import re
from pathlib import Path
from PyQt6.QtWidgets import QDialog, QVBoxLayout, QHBoxLayout, QLineEdit, QListWidget, QListWidgetItem, QPushButton, QLabel, QFormLayout
from PyQt6.QtGui import QIcon
from PyQt6.QtCore import Qt, QSize

class AppPicker(QDialog):
    """Dialog to list and pick installed system applications"""
    
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Select Application")
        self.setMinimumSize(450, 500)
        self.selected_app = None
        self.apps = []
        
        self.init_ui()
        self.scan_desktop_files()
        self.populate_list()
        
    def init_ui(self):
        self.setStyleSheet("""
            QDialog { background-color: #0f0f14; }
            QListWidget {
                background-color: #12121a;
                border: 1px solid #1f1f2e;
                border-radius: 8px;
                color: #e2e2e5;
                font-size: 13px;
                padding: 4px;
            }
            QListWidget::item { padding: 8px 10px; border-radius: 5px; }
            QListWidget::item:selected { background-color: #1c2538; color: #3a86f0; }
            QListWidget::item:hover { background-color: #1a1a28; }
        """)
        
        layout = QVBoxLayout()
        layout.setSpacing(10)
        layout.setContentsMargins(16, 16, 16, 16)
        
        # Search bar
        self.search_input = QLineEdit()
        self.search_input.setPlaceholderText("Search applications...")
        self.search_input.textChanged.connect(self.filter_apps)
        layout.addWidget(self.search_input)
        
        # App list
        self.list_widget = QListWidget()
        self.list_widget.setIconSize(QSize(40, 40))
        self.list_widget.setSpacing(2)
        self.list_widget.itemSelectionChanged.connect(self.on_selection_changed)
        self.list_widget.itemDoubleClicked.connect(self.accept_selection)
        layout.addWidget(self.list_widget)
        
        # Custom Exec Command Form
        form_layout = QFormLayout()
        form_layout.setSpacing(8)
        self.cmd_input = QLineEdit()
        self.cmd_input.setPlaceholderText("Execution command...")
        form_layout.addRow("Command:", self.cmd_input)
        
        self.name_input = QLineEdit()
        self.name_input.setPlaceholderText("Label name...")
        form_layout.addRow("Label Name:", self.name_input)
        
        layout.addLayout(form_layout)
        
        # Action Buttons
        btn_layout = QHBoxLayout()
        btn_layout.setSpacing(8)
        self.ok_btn = QPushButton("Select Application")
        self.ok_btn.setEnabled(False)
        self.ok_btn.setStyleSheet("""
            QPushButton { background-color: #10b981; border: none; color: #fff; font-weight: bold; border-radius: 6px; padding: 8px 18px; }
            QPushButton:hover { background-color: #059669; }
            QPushButton:disabled { background-color: #2a2a38; color: #555; }
        """)
        self.ok_btn.clicked.connect(self.accept_selection)
        btn_layout.addWidget(self.ok_btn)
        
        self.cancel_btn = QPushButton("Cancel")
        self.cancel_btn.clicked.connect(self.reject)
        btn_layout.addWidget(self.cancel_btn)
        
        layout.addLayout(btn_layout)
        self.setLayout(layout)

    def scan_desktop_files(self):
        """Scan system and user desktop files to find apps"""
        dirs = [
            Path('/usr/share/applications'),
            Path.home() / '.local/share/applications'
        ]
        
        scanned_execs = set() # Avoid duplicates
        
        for app_dir in dirs:
            if not app_dir.exists():
                continue
                
            for desktop_file in app_dir.glob('*.desktop'):
                try:
                    content = desktop_file.read_text(errors='ignore')
                    
                    # Parse basic metadata
                    name_match = re.search(r'^Name=(.+)$', content, re.MULTILINE)
                    exec_match = re.search(r'^Exec=(.+)$', content, re.MULTILINE)
                    icon_match = re.search(r'^Icon=(.+)$', content, re.MULTILINE)
                    no_display = re.search(r'^NoDisplay=true$', content, re.MULTILINE)
                    
                    if not name_match or not exec_match or no_display:
                        continue
                        
                    name = name_match.group(1).strip()
                    cmd = exec_match.group(1).strip()
                    icon_name = icon_match.group(1).strip() if icon_match else "application-x-executable"
                    
                    # Clean command (remove %f, %u placeholders)
                    cmd_clean = re.sub(r'%[fFuU]', '', cmd).strip()
                    
                    # Deduplicate based on clean Exec command
                    if cmd_clean in scanned_execs:
                        continue
                    scanned_execs.add(cmd_clean)
                    
                    self.apps.append({
                        'name': name,
                        'exec': cmd_clean,
                        'icon': icon_name
                    })
                except Exception:
                    pass
                    
        # Sort alphabetically
        self.apps.sort(key=lambda x: x['name'].lower())

    def populate_list(self, filter_text=""):
        self.list_widget.clear()
        
        for app in self.apps:
            if filter_text and filter_text.lower() not in app['name'].lower() and filter_text.lower() not in app['exec'].lower():
                continue
                
            item = QListWidgetItem()
            item.setText(f"{app['name']}\n{app['exec']}")
            item.setData(Qt.ItemDataRole.UserRole, app)
            
            # Resolve system icon
            icon_name = app['icon']
            if Path(icon_name).is_absolute() and Path(icon_name).exists():
                icon = QIcon(icon_name)
            else:
                icon = QIcon.fromTheme(icon_name)
                
            if icon.isNull():
                icon = QIcon.fromTheme("application-x-executable")
                
            item.setIcon(icon)
            self.list_widget.addItem(item)

    def filter_apps(self, text):
        self.populate_list(text)

    def on_selection_changed(self):
        items = self.list_widget.selectedItems()
        if not items:
            self.ok_btn.setEnabled(False)
            return
            
        app = items[0].data(Qt.ItemDataRole.UserRole)
        self.cmd_input.setText(app['exec'])
        self.name_input.setText(app['name'])
        self.ok_btn.setEnabled(True)

    def accept_selection(self):
        items = self.list_widget.selectedItems()
        if not items:
            return
            
        app = items[0].data(Qt.ItemDataRole.UserRole)
        self.selected_app = {
            'name': self.name_input.text().strip() or app['name'],
            'exec': self.cmd_input.text().strip() or app['exec'],
            'icon': app['icon']
        }
        self.accept()
