# Ulanzi Deck Linux

A Linux application for managing the Ulanzi D200/D200X StreamDeck device. Configure button images, labels, and actions to control OBS Studio, launch applications, execute commands, manage system audio volume, and simulate keyboard shortcuts.

The GUI and its background daemon can be run completely standalone without installing python packages system-wide.

## Features

- 🎨 **Custom Button Images** - Set 196×196 PNG images for each button
- 🏷 **Button Labels** - Add text labels with customizable styling
- 🎬 **OBS Integration** - Control OBS Studio scenes, sources, recording, and streaming
- 🚀 **App Launcher** - Launch applications with a button press
- ⌨️ **Keyboard Shortcuts** - Simulate keyboard input
- 🔊 **Audio & Media Control** - Adjust volume, mute/unmute, and control media players
- 🔄 **Hot-Reload** - Update configuration without restarting
- 🌙 **Background Daemon** - Run as a systemd user service or background process

---

## Getting Started

### 1. Install System Prerequisites

Install the USB HID API library and optional keyboard shortcut tool (`xdotool`):

#### Ubuntu/Debian:

```bash
sudo apt update
sudo apt install python3 python3-pip python3-venv xdotool libhidapi-hidraw0
```

#### Fedora/RHEL:

```bash
sudo dnf install python3 python3-pip xdotool hidapi
```

#### Arch Linux:

```bash
sudo pacman -S python python-pip xdotool hidapi
```

### 2. Install Udev Rules (Required for non-root USB access)

Copy the udev rule file so your user has permission to communicate with the USB device:

```bash
sudo cp 99-ulanzi.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

_Note: Please disconnect and reconnect the Ulanzi device after installing the rule._

---

## Standalone GUI Usage

You can run the GUI directly from the cloned repository or via a compiled portable binary release.

### Option A: Running from Source (No installation required)

1. Clone the repository:

   ```bash
   git clone https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux.git
   cd Ulanzi-Deck-Linux
   ```

2. Create a local virtual environment and install dependencies:

   ```bash
   python3 -m venv venv
   source venv/bin/activate
   pip install -r requirements.txt
   ```

3. Run the GUI script:
   ```bash
   python ulanzi_gui/main.py
   ```

### Option B: Running the Portable Executable Release

If you downloaded a standalone release binary (e.g. `ulanzi-gui`):

1. Make the binary executable:

   ```bash
   chmod +x ulanzi-gui
   ```

2. Run the executable:
   ```bash
   ./ulanzi-gui
   ```

---

## Configuration

Both the GUI and CLI default to using the configuration file located at:
`~/.config/ulanzi/config.yaml`

### Basic Configuration Example

```yaml
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

# Button definitions (13 buttons total, index 0-12)
buttons:
  - image: ./icons/firefox.png
    label: Firefox
    action: app
    params:
      name: firefox

  - image: ./icons/terminal.png
    label: Terminal
    action: command
    params:
      cmd: "gnome-terminal"

  - null # Empty button
```

---

## Subcommand / CLI Modes (Portable Binary / Script)

The unified entry point script (`ulanzi_gui/main.py`) or compiled executable (`ulanzi-gui`) can perform all management actions by appending flags:

### Run the Background Daemon Standalone

To launch the background daemon monitoring the device:

- **From executable:** `./ulanzi-gui --daemon`
- **From source:** `python ulanzi_gui/main.py --daemon`

### Run CLI Commands Standalone

To interact with the device or configuration via CLI:

- **From executable:** `./ulanzi-gui --cli [subcommand]`
- **From source:** `python ulanzi_gui/main.py --cli [subcommand]`

#### Available CLI subcommands:

```bash
# Check device connection status
./ulanzi-gui --cli status

# Set brightness level (0-100)
./ulanzi-gui --cli brightness 80

# Validate configuration file
./ulanzi-gui --cli validate

# Apply configuration to device
./ulanzi-gui --cli configure

# Generate a default configuration file template
./ulanzi-gui --cli generate-config config.yaml
```

---

## Systemd User Service Integration

To configure the daemon to run automatically in the background on startup:

1. Copy the systemd service file:

   ```bash
   mkdir -p ~/.config/systemd/user
   cp systemd/ulanzi-daemon.service ~/.config/systemd/user/
   ```

2. Enable and start the service:

   ```bash
   systemctl --user daemon-reload
   systemctl --user enable ulanzi-daemon
   systemctl --user start ulanzi-daemon
   ```

3. Check service status:
   ```bash
   systemctl --user status ulanzi-daemon
   ```

---

## Building Standalone Binary

To package the application yourself into a single portable binary:

```bash
bash build_appimage.sh
```

The compiled standalone executable will be saved in `dist/ulanzi-gui`.

---

## License

MIT

## Reference

Based on [ulanzi-d200-linux](https://github.com/racerxdl/ulanzi-d200-linux/) by [racerxdl](https://github.com/racerxdl)
