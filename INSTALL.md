# Installation Guide

This guide walks you through setting up the Ulanzi D200 Manager from source.

## Prerequisites

- Python 3.8 or higher
- Linux system with USB support
- `xdotool` for keyboard shortcuts (optional but recommended)

## Step 1: Install System Dependencies

### Ubuntu/Debian:
```bash
sudo apt update
sudo apt install python3 python3-pip python3-venv xdotool libhidapi-hidraw0
```

### Fedora/RHEL:
```bash
sudo dnf install python3 python3-pip xdotool hidapi
```

### Arch Linux:
```bash
sudo pacman -S python python-pip xdotool hidapi
```

## Step 2: Clone and Setup

```bash
git clone https://github.com/mariovalney/ulanzi-d200-linux.git
cd ulanzi-d200-linux

# Create and activate virtual environment
python3 -m venv venv
source venv/bin/activate

# Install package in editable mode
pip install -e .
```

## Step 3: Install Udev Rule

Copy the rules file to allow non-root users to communicate with the USB device:

```bash
sudo cp 99-ulanzi.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```

*Make sure to unplug and reconnect the Ulanzi device after this step.*

## Step 4: Create Configuration Directory

```bash
mkdir -p ~/.config/ulanzi
mkdir -p ~/.local/share/ulanzi
```

## Step 5: Generate Configuration

```bash
ulanzi-manager generate-config ~/.config/ulanzi/config.yaml
```

## Step 6: Edit Configuration

Edit `~/.config/ulanzi/config.yaml` with your preferred button definitions, icons, and actions.

## Step 7: Test and Configure

```bash
# Validate your YAML configuration syntax (defaults to ~/.config/ulanzi/config.yaml)
ulanzi-manager validate

# Apply configuration to device
ulanzi-manager configure
```

## Step 8: Run Daemon

### Option A: Manual Start (running in terminal foreground)
```bash
ulanzi-daemon
```

### Option B: Systemd User Service (Recommended background daemon)

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

3. Check status:
   ```bash
   systemctl --user status ulanzi-daemon
   ```

4. View logs:
   ```bash
   journalctl --user -u ulanzi-daemon -f
   ```

---

## Troubleshooting

### Device Not Found
```
RuntimeError: Ulanzi D200 device not found
```
**Solution:**
1. Check USB physical connection: `lsusb | grep 2207`
2. Add your user to the plugdev group:
   ```bash
   sudo usermod -a -G plugdev $USER
   newgrp plugdev
   ```

### Permission Denied / Open Failed
```
PermissionError: [Errno 13] Permission denied
ERROR: open failed
```
**Solution:** Ensure the udev rule is correctly copied and triggered:
```bash
sudo cp 99-ulanzi.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger
```
Then reconnect the USB device.

### OBS Connection Failed
```
Failed to connect to OBS
```
**Solution:**
1. Make sure OBS is running.
2. Verify WebSocket Server is enabled in OBS under **Tools** -> **WebSocket Server Settings**.
3. Check that the port and password settings in `~/.config/ulanzi/config.yaml` match OBS settings.

### Keyboard Shortcuts Not Working
**Solution:** Install `xdotool`:
```bash
sudo apt install xdotool
```

---

## Uninstall

To clean up all files and configurations:

```bash
# Stop and disable systemd service
systemctl --user disable ulanzi-daemon
systemctl --user stop ulanzi-daemon
rm -f ~/.config/systemd/user/ulanzi-daemon.service

# Remove configuration and logs
rm -rf ~/.config/ulanzi
rm -rf ~/.local/share/ulanzi
```
