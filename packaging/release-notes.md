Feature update to 2.3.1: per-output settings for Switch Output, and a fix for decks that did not respond while Steam was running.

### Added

- **Switch Output** per output: an image, colour and label the key shows while that output plays, and the output and app volumes a switch to it sets. Volumes are set before the switch.
- `{output_name}` label value: the current default sound output.

### Fixed

- The deck connects while Steam is running. Steam opens input devices while looking for game controllers, and 2.3.1 took that as another program using the deck.

Full list in the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md).

### Installation

On 2.1.1 or later: use **Update** in the app. Otherwise download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later for the `.deb`, tarball and AppImage.
