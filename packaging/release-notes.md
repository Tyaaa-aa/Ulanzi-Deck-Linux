Hotfix update to 2.6.0: D200 fixes and two Library tray fixes. No new features.

### Changed

- A profile set to D200 (no dial base) draws only the 13 keys and the wide screen, and the hints under the deck and above the action list no longer mention dials or the ‹ › page buttons.

### Fixed

- D200: pressing the wide screen no longer shows the deck's own clock for a moment. The mode is sent ahead of any upload in progress, and again shortly after the press (#5).
- Turning the Library tray to a shorter page while carrying a tile no longer shrinks the tray out from under the pointer.
- The Library tray's Keys and Dials tabs and its search field were dark on the dark tray in the light theme.

Thanks to @slawomirpopowicz for the D200 fixes (#6, #7).

Full list in the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md).

### Reporting problems

In the app via the main menu → **About Ulanzi Deck** → **Report an Issue**, or directly on [GitHub Issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new/choose). Output of `ulanzictl diagnostics` helps.

### Installation

On 2.1.1 or later: use **Update** in the app. Otherwise download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later for the `.deb`, tarball and AppImage.
