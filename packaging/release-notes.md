Feature update to 2.4.0: a dial library, pinned dials and small buttons, and drag to swap dials.

This is the first of two parts. The library holds dials in 2.5.0; keys come to the same library in the next release.

### Added

- **Dial library** (opt-in): **Add to Library** shares a dial between pages, so editing it changes every page that uses it. **Make a Copy** gives a page its own copy again.
- **Library tray** under the deck (**Library** button in the header): place library dials by dragging them onto the deck, add deck dials by dragging them into the tray, reorder by dragging, right-click for Copy, Paste and Delete. A short guide shows on first open.
- **Pin to Every Page** for dials and small buttons. Each page keeps its own underneath and **Unpin** shows it again.
- Drag a dial onto another to swap them. Dials have names, a right-click menu, and Ctrl+C / Ctrl+V / Delete.
- The drawn deck shows dial turns with an arc in the turn direction and presses with a lit ring.
- Export and import can include the dial library.
- A banner asks for a restart when the app and the service run different versions.

### Changed

- Small buttons belong to each page. Existing small buttons, and dials that were the same on every page, load as pinned, so nothing changes until you unpin them. A backup named **before-pinning-dials** is made first (**Preferences → Backups**).
- Profiles that use the library or pins need 2.5.0 or newer.
- **Use This Dial on Every Page** is replaced by pinning.

Full list in the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md).

### Reporting problems

The library is new, so please report anything that looks wrong: in the app via the main menu → **About Ulanzi Deck** → **Report an Issue**, or directly on [GitHub Issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new/choose). Output of `ulanzictl diagnostics` helps.

### Installation

On 2.1.1 or later: use **Update** in the app. Otherwise download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later for the `.deb`, tarball and AppImage.
