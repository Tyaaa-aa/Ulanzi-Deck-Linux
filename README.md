<div align="center">

<img src="crates/ulanzi-deck/data/icons/io.github.tyaaa_aa.UlanziDeck.svg" width="112" alt="">

# Ulanzi Deck

**Set up the Ulanzi D200 and D200X stream decks on Linux.**
Drag actions onto keys, put live widgets on the wide screen, control OBS Studio, and let your
profiles follow the app you're using.

[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
![Rust](https://img.shields.io/badge/Rust-2024-orange?logo=rust)
![GTK 4 + libadwaita](https://img.shields.io/badge/GTK_4-libadwaita-4a86cf?logo=gnome)
![Linux](https://img.shields.io/badge/Linux-Wayland_%26_X11-333?logo=linux&logoColor=white)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/main-dark.png">
  <img src="docs/screenshots/main-light.png" alt="Ulanzi Deck: the deck with its keys, dials and wide screen, and the action library" width="900">
</picture>

</div>

## What it does

- **Every control.** 13 keys, 3 dials (turn left, turn right, press, hold), 2 small buttons and the wide
  screen, each with a tap and a hold action. Changes reach the deck as you make them.
- **Actions.** Media and seeking, volume and mute for any output, app or microphone, switching audio
  outputs, keyboard shortcuts, typing text, commands, apps, websites, sounds, multi-step actions, pages,
  folders and profiles.
- **OBS Studio.** Scenes, studio mode, sources, filters, mute and volume, recording, streaming, replay
  buffer and virtual camera. Keys show what OBS is doing: the scene on air, REC and LIVE with their time.
- **The wide screen.** Clocks, system stats with graphs, Now Playing, a Pomodoro timer, weather, command
  output, a picture slideshow and OBS status, one after another or on a tap. Volume changes show an
  overlay.
- **Profiles that follow you.** A profile can switch in when an app comes to the front (KDE Plasma,
  GNOME, Hyprland, Sway and X11).
- **Looks.** Icons (Lucide), brand logos (Simple Icons), installed apps, emoji, Steam games or your own
  pictures, with colours, labels and live state.
- **Share and keep.** Templates to start from, `.ulanzideck` files to share profiles, and automatic
  backups you can restore.
- **Runs in the background.** A small service drives the deck with the window closed and sits in the
  tray. The window is only for setting things up.

<table>
  <tr>
    <td><img src="docs/screenshots/widgets-dark.png" alt="Wide-screen widgets with a live preview"></td>
    <td><img src="docs/screenshots/key-light.png" alt="Editing a key"></td>
  </tr>
  <tr>
    <td align="center">Widgets for the wide screen, previewed live</td>
    <td align="center">Tap and hold actions, look and live state</td>
  </tr>
  <tr>
    <td><img src="docs/screenshots/obs-light.png" alt="Connecting OBS Studio"></td>
    <td><img src="docs/screenshots/templates-dark.png" alt="Starting from a template"></td>
  </tr>
  <tr>
    <td align="center">OBS: every connection problem explained, one-click setup</td>
    <td align="center">Start from a template, or save your own</td>
  </tr>
</table>

<p align="center"><img src="docs/screenshots/widgets-sheet.png" width="760" alt="Clock, analogue clock, Now Playing, system stats, timer, weather, command output and OBS widgets"></p>

## Install

Download from the [latest release](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/releases/latest).
You need GTK 4.16 and libadwaita 1.6 or newer (Fedora 41+, Ubuntu 25.04+, Debian 13, Arch, openSUSE
Tumbleweed).

| Distribution | Package | Install |
|---|---|---|
| Arch, CachyOS, Manjaro | `ulanzi-deck-*.pkg.tar.zst` | `sudo pacman -U ulanzi-deck-*.pkg.tar.zst` (or build with the `PKGBUILD`) |
| Debian, Ubuntu, Mint | `ulanzi-deck_*.deb` | `sudo apt install ./ulanzi-deck_*.deb` |
| Fedora, openSUSE | `ulanzi-deck-*.rpm` | `sudo dnf install ./ulanzi-deck-*.rpm` |
| Any distribution | `ulanzi-deck-*.tar.gz` | unpack, then `./install.sh` (into `~/.local`) or `./install.sh --system` |

Then open **Ulanzi Deck** from your app menu. The first run walks you through it: it finds the deck,
asks once for permission to use it (a udev rule), offers to bring over a configuration from the old
Python version, and turns on the background service.

After installing a package, start the service for your user once (the setup screen offers it too):

```sh
systemctl --user enable --now ulanzi-deckd
```

### OBS Studio

In OBS 28 or newer, open **Tools → WebSocket Server Settings** and tick **Enable WebSocket server**.
Then in Ulanzi Deck open **Preferences → Integrations** and press **Find OBS Settings**: it reads the port
and password from OBS for you. If something is off, the same page says what, in plain words.

### Following the app in front

| Desktop | How |
|---|---|
| KDE Plasma 6 | built in (a small KWin script) |
| GNOME | the bundled Shell extension: `gnome-extensions enable ulanzi-deck-focus@tyaaa-aa.github.io` |
| Hyprland, Sway | built in |
| X11 desktops | built in |

## Command line

`ulanzictl` talks to the service: `status`, `profile`, `page`, `brightness`, `import`, `export`,
`snapshots`, `restore`, `monitor` (shows presses as they happen) and `diagnostics` (for bug reports).

## Templates

Any profile can become a template: right-click it and choose **Save as Template…**. Templates are plain
`.ulanzideck` files in `~/.local/share/ulanzi-deck/templates/`, so a layout for your editor, design
tool or DAW is easy to keep and to share.

## Building from source

```sh
cargo build --release
scripts/dev-install.sh          # installs into ~/.local for testing
scripts/test.sh                 # format, lints and the whole test suite
scripts/sandbox.sh --demo       # try the app with a pretend deck and throwaway profiles
scripts/release.sh              # every package format into dist/
```

The workspace has five crates: `ulanzi-proto` (the deck's USB protocol), `ulanzi-core` (profiles,
rendering, import and export), `ulanzi-deckd` (the service), `ulanzictl` (the CLI) and `ulanzi-deck`
(the GTK app). [TESTING.md](TESTING.md) is the hands-on test before a release.

## Credits

- The USB protocol builds on the research in [racerxdl/ulanzi-d200-linux](https://github.com/racerxdl/ulanzi-d200-linux)
  and [mariovalney/ulanzi-d200-linux](https://github.com/mariovalney/ulanzi-d200-linux).
- Icons: [Lucide](https://lucide.dev) (ISC). Brand logos: [Simple Icons](https://simpleicons.org) (CC0).
  Brand names and logos are trademarks of their owners.
- Weather: [Open-Meteo](https://open-meteo.com).
- Version 1 (Python) lives on the [`python-legacy`](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/tree/python-legacy) branch.

Ulanzi Deck is not affiliated with Ulanzi. [MIT License](LICENSE).
