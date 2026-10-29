Ulanzi Deck 2.1 brings plugins.

**New**
- **Plugins** (the puzzle piece in the header): a plugin store with search and categories. Plugins are
  sandboxed and signed; the app checks every download before it installs. Nothing comes preinstalled:
  add what you use. First up: **OBS Studio** and **Home Assistant**.
- **Keys that follow what they control**: a light key turns yellow when the light is on, an air
  conditioner key shows a blue snowflake while cooling, OBS keys show what's on air and REC / LIVE.
- **Label Shows**: pick a live value for a key's label (room temperature, a scene) by name.
- **Ready-made keys**: pick your lights, fans and air conditioners in Home Assistant and get keys that
  fit each one, on this page or a new one.
- **Cycle Through Actions**: one key, a different action on each press, each with its own image and label.
- A simpler key editor: one page (Press, Label, When Held, Look) and a category menu in the action list.

**Changed**
- OBS Studio now works through the OBS Studio plugin. Your OBS keys and password move over by
  themselves: install the plugin from Plugins → Discover.
- 2.1 saves profiles in a new format that 2.0 can't open. Your 2.0 files are kept once in
  `~/.config/ulanzi-deck/before-2.1/`.

**Install:** pick the file for your distribution: Arch package, .deb, .rpm, AppImage (run it, `--install` adds it to the menu) or the tarball with `install.sh`. Needs GTK 4.16 and libadwaita 1.6.
