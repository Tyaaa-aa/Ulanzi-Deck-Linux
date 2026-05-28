#!/bin/bash
# Script to compile Ulanzi D200X Manager into a standalone portable Linux binary
set -e

echo "=========================================================="
echo "   Ulanzi D200X Manager - Portable Binary Builder"
echo "=========================================================="
echo

# 1. Activate virtual environment
if [ -f "venv/bin/activate" ]; then
    echo "Activating virtual environment..."
    source venv/bin/activate
elif [ -f "venv/bin/activate.fish" ]; then
    echo "Please run this script using bash/zsh, or ensure venv is active."
fi

# 2. Install PyInstaller if missing
if ! python3 -c "import PyInstaller" &>/dev/null; then
    echo "Installing PyInstaller..."
    pip install pyinstaller
else
    echo "✓ PyInstaller already installed"
fi

# 3. Clean previous builds
echo "Cleaning old build files..."
rm -rf build dist *.spec

# 4. Build Standalone Portable Executable
echo "Building standalone binary with PyInstaller..."
pyinstaller --onefile --windowed \
    --name ulanzi-gui \
    --add-data "99-ulanzi.rules:." \
    ulanzi_gui/main.py

echo
echo "=========================================================="
echo "✓ BUILD COMPLETE SUCCESSFUL!"
echo "=========================================================="
echo
echo "Your portable standalone executable is ready at:"
echo "  $(pwd)/dist/ulanzi-gui"
echo
echo "This file contains the Python interpreter, PyQt6 GUI, and"
echo "all required libraries. It can be moved and run on any"
echo "compatible Linux distribution without needing to install Python."
echo
echo "To run it from the terminal or file manager:"
echo "  ./dist/ulanzi-gui"
echo
