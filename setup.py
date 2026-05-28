from setuptools import setup, find_packages

setup(
    name="ulanzi-manager",
    version="0.1.0",
    description="Ulanzi D200 StreamDeck device manager for Linux",
    author="Lucas",
    packages=find_packages(),
    install_requires=[
        "pyusb>=1.2.1",
        "hidapi>=0.14.0",
        "pyyaml>=6.0.1",
        "obsws-python>=1.8.0",
        "pillow>=10.1.0",
        "python-daemon>=3.0.1",
        "PyQt6>=6.0.0",
    ],
    entry_points={
        "console_scripts": [
            "ulanzi-manager=ulanzi_manager.cli:main",
            "ulanzi-daemon=ulanzi_manager.daemon:main",
            "ulanzi-gui=ulanzi_gui.main:main",
        ],
    },
    python_requires=">=3.8",
)
