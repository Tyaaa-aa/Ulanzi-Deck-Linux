Ulanzi Deck 2.3.1 is the polish that came out of trying 2.3.0 the way a brand-new user would.

### Changed

- **Discover feels much faster.** It opens with an animated placeholder, shows the plugins as soon as the
  list arrives, and fades their icons in after, so a slow connection no longer holds up the page.
- **Auto-switch leaves your choice alone.** An app with no profile no longer sends you back to the default
  profile. Leaving an auto-switched app returns you to the profile you were on.
- **The setup assistant asks first.** It offers to import your setup from the old Python app, instead of
  the service importing it by itself.
- **Clearer setup buttons.** Until a step is done, the button that moves on is grey and says **Skip for
  Now**, so it can't be mistaken for the step itself.
- The Macros button at the top uses the same icon as Macros in the action list.

### Fixed

- What's New opens at the top, and its text no longer breaks mid-sentence.

See the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md) for the
full list of changes.

### Installation

Already on 2.1.1 or later? Use **Update** in the app. Otherwise, download the package for your distribution:
Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or
the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later
for the `.deb`, tarball and AppImage.
