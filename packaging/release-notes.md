Ulanzi Deck 2.1.1 adds update checks and keeps what it doesn't understand from newer versions. Install
this one by hand; from here on the app can update itself.

### New

- **Updates.** A banner tells you when a new version is out, What's New shows what changed, and
  **Update** downloads it, checks its signature, backs up and restarts the app. Turn the daily check
  off in **Preferences → General → Updates**.
- **Files from newer versions.** A key this version doesn't understand is kept unchanged and shows a
  **⚠** with the version it needs. A profile too new to read stays in place instead of disappearing.

### Fixed

- The `.deb`, tarball and AppImage now start on Debian 13, Ubuntu 25.04 and newer. They need glibc
  2.39 or later.
- The `.rpm` now installs on Fedora and openSUSE.

### Changed

- The image picker opens at once.
- Release files are signed.

See the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md) for the
full list of changes.

### Installation

Download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with
`--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16
and libadwaita 1.6 or later.
