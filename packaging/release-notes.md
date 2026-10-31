Ulanzi Deck 2.2.0 adds macros: record what you do, arrange it, and put it on any key.

### New

- **Macros.** Record keys, mouse buttons and scrolling, or add text, pointer moves, waits, other
  macros and any app action. Steps sit in a numbered grid, coloured by kind, and you can drag, edit
  and select them. Use recorded delays, or one standard delay for every step.
- **Four ways to play.** Play Once, While Holding, Toggle and Sequence, each with a short animation
  of what it does. You choose what pressing again or letting go does.
- **Key feedback.** While a macro plays, its key can show a progress bar, a step count or a look for
  each step. Key labels can use `{macro.step}`, `{macro.total}` and more.
- **A Macros window** next to the Plugins button, and a list on each macro key to swap it or make a
  new one. Macros travel with `.ulanzideck` files and backups.
- **A key's own copy.** Edit a macro for one key only, then keep it, add it to the library or go back.
- **Stop All Macros**, for a key that stops everything and lets go of anything held.

Recording works while the macro window is in front. Linux stops apps from watching what you type in
other apps, which keeps your passwords safe. Macros still play back in whatever app you're using.

### Changed

- Tooltips in the action list say what each action does. Plugin actions can now describe themselves.

### Fixed

- Messages appear on top of Preferences instead of behind it.

### Good to know

- A key with a macro shows a warning in 2.1.1 and works again on 2.2, and profiles keep the key.
- **Pick a Spot** and **Move Pointer** may land differently across monitor setups. Try them first in
  **Try It**.

See the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md) for the
full list of changes.

### Installation

Already on 2.1.1? Use **Update** in the app. Otherwise, download the package for your distribution:
Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or
the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later
for the `.deb`, tarball and AppImage.
