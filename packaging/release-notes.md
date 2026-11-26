Feature update to 2.5.0: the key library and pinned keys, drag and drop between pages, and a D200 fix.

This is the second part of the library: keys join dials in the same Library tray.

### Added

- **Key library** (opt-in), in a **Keys** tab of the Library tray: **Add to Library** shares a key between pages, so editing it changes every page that uses it. **Make a Copy** gives a page its own copy again. Library keys are edited in the full key editor from the tray, and **New Key** makes a blank one.
- **Pin to Every Page** for keys, folders included. Each page keeps its own key underneath and **Unpin** shows it again. Several selected keys can be added, pinned or unpinned at once.
- Library tray: pick several tiles (Ctrl-click, Shift-click, Ctrl+A) and copy, paste or delete them together, also into another profile. Search once a library has more than one page.
- While a tile is carried from the tray to the deck, the tray drops to a strip under the deck so every key is in reach.
- Holding a dragged key or library tile over a page in the sidebar opens that page. Deck keys move between pages this way; a pinned key moved to another page is no longer pinned.
- Library reordering: drop on a tile to swap the two, or between two tiles to insert there. Holding a tile at the tray's left or right edge turns the page.

### Changed

- Profiles that use the key library or pinned keys need 2.6.0 or newer.
- The clipboard is kept across profiles; pictures used by copied keys come along.
- Holding a tile over the tray's ‹ › buttons no longer turns the page.

### Fixed

- D200: the deck's own clock no longer shows briefly over a custom wide-screen widget when the wide screen is pressed (#5).

Full list in the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md).

### Reporting problems

Please report anything that looks wrong in the key library or the new drag and drop: in the app via the main menu → **About Ulanzi Deck** → **Report an Issue**, or directly on [GitHub Issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/issues/new/choose). Output of `ulanzictl diagnostics` helps.

### Installation

On 2.1.1 or later: use **Update** in the app. Otherwise download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later for the `.deb`, tarball and AppImage.
