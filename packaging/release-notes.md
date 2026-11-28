Small update to 2.6.1: own images get a size setting. No new major features.

### Changed

- Your own images (PNG, JPG, WebP, SVG and GIF) have their own **Image Size** in Look. Below 100% the picture shrinks into the middle of the key and keeps its shape; at 100% (the default) it fills the key as before, so existing keys look the same.
- The size row in Look shows its value as a percent (**70%**) without a description line.

Thanks to @slawomirpopowicz for the image size change (#9).

Full list in the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md).

### Reporting problems

In the app via the main menu → **About Ulanzi Deck** → **Report an Issue**, or directly on [GitHub Issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new/choose). Output of `ulanzictl diagnostics` helps.

### Installation

On 2.1.1 or later: use **Update** in the app. Otherwise download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later for the `.deb`, tarball and AppImage.
