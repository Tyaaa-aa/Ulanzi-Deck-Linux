Ulanzi Deck 2.1 adds a plugin system and a plugin store.

### New

- **Plugin store.** Browse, install and update plugins from the new Plugins window. Plugins run in a
  sandbox with only the permissions they declare, and every package is signature-checked before it
  is installed. No plugins are installed by default. The first two are OBS Studio and Home
  Assistant.
- **Live key state.** Keys can reflect what they control: a Home Assistant light key turns yellow
  when the light is on, an air conditioner key shows a snowflake while cooling, and OBS keys show
  the active scene and recording or streaming time.
- **Label Shows.** Choose a live value, such as a room temperature or the current OBS scene, as a
  key's label from a menu.
- **Ready-made keys.** Select Home Assistant lights, fans, air conditioners and other devices, and
  Ulanzi Deck creates matching keys on the current page or a new one.
- **Cycle Through Actions.** A single key runs a different action on each press, with its own image
  and label for each step.
- **Redesigned key editor.** A single page (Press, Label, When Held, Look) and a category menu in
  the action list.
- **Play Sound volume.** A per-key volume, a preview button, and a global sound volume in
  Preferences.

### Changed

- OBS Studio support is now provided by the OBS Studio plugin. Existing OBS keys and the OBS password
  are migrated automatically; install the plugin from **Plugins → Discover**.
- Profiles are saved in a new file format that 2.0 cannot read. Your 2.0 files are copied once to
  `~/.config/ulanzi-deck/before-2.1/`.

See the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md) for the
full list of changes.

### Installation

Download the package for your distribution: Arch package, `.deb`, `.rpm`, AppImage (run it with
`--install` to add it to the application menu) or the tarball with `install.sh`. Requires GTK 4.16
and libadwaita 1.6 or later.
