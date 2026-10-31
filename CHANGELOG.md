# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- **Macros.** A library of macros with a recorder and an editor modelled on G Hub: key and mouse down
  and up, text, scrolling, pointer movement, Pick a Spot, other macros and any app action, with
  recorded or standard delays.
- Four macro types: Play Once, While Holding, Toggle and Sequence, with an animation of each.
- Key feedback while a macro plays: a progress bar, a step count, a look for each step, and
  `{macro.*}` label values.
- A key's own copy of a macro, with Save as New Macro, Update and Use Library Version, all undoable.
- Restore Previous Version on a library macro.
- Tooltips in the action list say what each action does. Plugin actions can set `description`.
- **Stop All Macros** action, and `ulanzictl macro list` and `ulanzictl macro stop`.
- Profiles and `.ulanzideck` files carry the macros they use, and backups hold the whole library.

### Changed

- Development builds keep less debug data, so a build no longer fills the disk.

## [2.1.1] - 2026-10-31

### Added

- **Update check.** The app checks GitHub once a day and shows a banner when a new version is out.
  The check can be turned off in **Preferences → General → Updates**.
- **What's New.** Shows the release notes and any important note, and opens once after an update.
- **Self-update.** Downloads the new version, checks its signature, backs up, installs it for the
  way the app was installed (AppImage, Arch, Debian, Fedora, openSUSE or a tarball for one user) and
  restarts.
- `ulanzictl update` and `ulanzictl update check`.
- `ulanzictl probe gif`: checks whether the deck plays GIFs by itself.

### Changed

- The image picker opens at once: tabs are built when first shown, pictures load in the background,
  and icons are kept between openings.
- Release files are signed. `SHA256SUMS.minisig` sits next to `SHA256SUMS`.

### Fixed

- The `.deb`, tarball and AppImage needed the C library of the computer that built them, so they
  would not start on Debian 13, Ubuntu 25.04 or other systems with an older one. Release binaries
  are now built on Debian 13.
- The `.rpm` could not be installed on Fedora or openSUSE, because it asked for the build computer's
  exact library versions.
- A profile from a newer version no longer fails as a whole. Keys this version doesn't understand
  are kept unchanged and show a ⚠ with the version they need.
- A profile too new to read stays in place and is listed as "Needs a newer version". It used to be
  renamed and disappear.
- Settings from a newer version no longer stop the service from starting.
- Importing a `.ulanzideck` file reports which keys need a newer version.

## [2.1.0] - 2026-10-30

This release adds a plugin system and a plugin store, and moves OBS Studio support into a plugin.
Profiles and settings are saved in file format 2, which Ulanzi Deck 2.0 cannot read. Existing 2.0
files are upgraded automatically, and the originals are copied once to
`~/.config/ulanzi-deck/before-2.1/`.

### Added

- Plugins: sandboxed WebAssembly components that add actions to keys. Plugins can use timers,
  secrets (stored in the system keyring), status reporting, and WebSocket, TCP/TLS and HTTP
  connections limited to the hosts and local-network access they declare. Values they publish are
  available in key labels as `{<plugin>.<key>}`. A plugin that crashes is restarted once and then
  disabled. The deck starts without waiting for plugins, and the keyring is only contacted when a
  plugin first needs its secrets. Includes the plugin SDK (`ulanzi-plugin`), a hello example and the
  [plugin guide](docs/plugins.md).
- `ulanzictl plugin list|install|remove|enable|disable|restart|dev-mode` and matching D-Bus methods.
- Plugin contract 1.1: `list-items` and `[[pickers]]` let plugins provide lists to pick from, with
  label, subtitle, group, icon and live state for each entry. Plugins built for 1.0 continue to work.
  `ulanzictl plugin items` and the `PluginListItems` D-Bus method query a list.
- Plugin contract 1.2 (manifest only): plugin and action icons, ready-made keys, live looks, label
  values, `show_if`, `author` and `min_app`.
- Plugin packages: `<id>-<version>.ulanziplugin`, a ZIP archive with `plugin.toml`, `plugin.wasm` and
  an optional `assets/` folder. `scripts/package-plugin.sh` builds one; `ulanzictl plugin install`
  and the `PluginInspectFile` and `PluginInstallFile` D-Bus methods install one, with strict size
  limits and no links. Packages with a valid store signature install without Developer Mode and are
  shown as verified. Unsigned packages require Developer Mode. Invalid signatures are always
  rejected.
- Plugin store: `ulanzictl plugin store` lists the plugins in the signed store index, and
  `ulanzictl plugin get <id>` installs or updates one (D-Bus `PluginStoreIndex`,
  `PluginStoreInstall`, `PluginVerified`, `PluginOfficial`). No plugins are installed by default.
- Plugin manager window, opened from the header bar. **Discover** shows the store with search,
  category filters and Install, Update or Open buttons. **Installed** lists installed plugins. Each
  plugin has a settings page generated from its manifest, with status, an on/off switch, Restart and
  Remove. Passwords and tokens are written directly to the plugin's secret store. **Install
  Plugin…** installs a `.ulanziplugin` file after showing its permissions; with Developer Mode on,
  **Install from Folder (developers)…** installs a plugin folder.
- Store installs report progress: preparing, downloading (with a progress bar and size), verifying
  and installing.
- Plugins show their author. Plugins maintained by the project are marked **Official**; only the
  signed store index can grant this.
- Plugin actions in the key editor, grouped by plugin, with a row for each parameter. Parameters that
  refer to an item open the plugin's list, with search, a group filter, live state and multiple
  selection. Stored values missing from the list are still shown, and manual entry is offered only
  when the plugin cannot provide a list. Rows display the plugin's names for items (for example
  "Kitchen" rather than `light.kitchen`). Keys show a warning when their plugin is missing, disabled,
  failed or incompatible.
- Plugin icons: a full-colour `icon`, a one-colour `symbolic_icon`, a `color` and per-action icons,
  all from the plugin's `assets/` folder. Keys can display a plugin icon
  (`source = { kind = "plugin", … }`), drawn in the key's icon colour. Installation checks that
  every icon named in the manifest is present.
- Ready-made keys (`[[key_sets]]`): pick items from a plugin's list to create matching keys, with live
  state, on the current page or a new one.
- Live looks: keys can change icon, colour and label based on a live value (`[[look.states]]`).
  Plugins declare looks per action (`states`), selectable label values (`labels`, shown in the key
  editor's **Label Shows** menu) and the settings to track (`track`, added automatically as keys use
  the action).
- **Cycle Through Actions**: each press runs the next step. Each step has its own image, colour and
  label. The key shows either the step that last ran (the default) or the next step.
- **Play Sound** has a per-key volume and a **Play** button to preview the sound.
  **Preferences → General → Sound Volume** sets the overall volume.
- Home Assistant plugin, published in
  [Ulanzi-Deck-Plugins](https://github.com/Tyaaa-aa/Ulanzi-Deck-Plugins): toggle entities, run
  scripts, set levels and call any action, with live entity state in labels (`{ha.light.desk}`,
  `{ha.sensor.temp.unit}`) and an entity picker. Ready-made keys for lights, fans, air conditioners,
  covers, media players, scripts, scenes, buttons and switches. Settings are stored in
  `[plugins.ha.settings]`, and the access token is moved to the secret store.
- Home Assistant `climate` action: turn an air conditioner or heater on or off based on its tracked
  state (so integrations whose `climate.toggle` does not turn the device off still work), step the
  target temperature in the device's own increments, or set a temperature or mode.
- OBS Studio plugin, published in Ulanzi-Deck-Plugins: all previous OBS actions and live values
  (`{obs.scene}`, `{obs.rec_time}` and others) in a sandboxed plugin, with lists for scenes, inputs,
  sources, filters, scene collections and profiles, and one ready-made key per scene. Settings are
  stored in `[plugins.obs.settings]`.

### Changed

- The key editor is a single page, in setup order: **Press**, **Label**, **When Held** and
  **Look**. When Held and Look are collapsed until opened, and stay open while the app runs. Show
  Label is replaced by a **Hidden** option in Label Position. A dial's four actions are shown as one
  list of expandable rows. Clear Key and Use This Dial on Every Page stay visible below the editor.
- The action list (the side panel with nothing selected, and the key editor's Choose and Change
  lists) has one search field that matches action names, categories and plugin names. A category
  menu next to it (All, the built-in categories, then one entry per plugin) shows one category at a
  time and is remembered while the app runs. Sections can be collapsed and are sorted
  alphabetically. Searching within a category offers "Show N more in All" when other categories
  match.
- The Actions button in the header bar is removed. The key editor has a close button, and Escape
  closes it when no text field has focus. Ctrl+L focuses the action search. With a key selected, the
  editor returns after you pick an action, select another key, or press the close button or Escape.
- Plugin list searches match every word in any order, so "bedroom aircon" finds "Bedroom Aircon".
- Function keys are written `f1` to `f24`; zero-padded names such as `f01` are no longer accepted.
- Menu items no longer end in an ellipsis (Rename, Delete, Export, Import, Set Up Deck and others).
  The documentation uses the same names.

### Removed

- Built-in OBS Studio support, replaced by the OBS Studio plugin. Install it from **Plugins →
  Discover**. Keys from 2.0 are converted automatically, `{obs_scene}`, `{obs_rec_time}` and
  `{obs_stream_time}` still work, and the OBS password is moved to the plugin's secret store. The
  Find OBS Settings button, which read the port and password from OBS, is not available in this
  release; copy them from **Show Connect Info** in OBS.

### Fixed

- The shortcut recorder captures Super and any key combination, including Super alone, Super+X and
  Ctrl+Super. Pressing and releasing a modifier on its own records it; Escape cancels. While
  recording, desktop and app shortcuts are suspended for the window so KDE and GNOME do not
  intercept Super. Caps Lock, Pause and keypad keys are now supported. Keys the deck cannot send (such
  as `<` or `ü` on some layouts) are rejected with "This key can’t be sent".
- Text fields keep their own shortcuts. Ctrl+A, copy, paste, undo, redo and Delete edit the text
  instead of acting on deck keys, and an open dialog or menu handles Escape first.
- The sandbox script adds the real data directory to `XDG_DATA_DIRS`, so the app's icons are no
  longer shown as broken images.
- OBS Studio plugin: when OBS accepts the connection but rejects the protocol version or scene
  request, the plugin retries with backoff (1 s, doubling to 10 s) instead of every second, and logs
  the error once. Volume up on an input at 0 % no longer jumps to full volume. Clearer labels for
  studio mode and "Mute or unmute".
- Home Assistant plugin: service calls have 30 seconds to respond before a timeout is logged,
  instead of the 5 seconds used for other requests. A key without an entity reports "Choose an
  entity for this key first". "Call a service" is renamed "Call an action (service)", matching Home
  Assistant, and a new Set level key defaults to 100 %.

## [2.0.0] - 2026-10-23

A complete rewrite in Rust with a new GTK 4 / libadwaita app. Version 1 (Python) lives on the
`python-legacy` branch.

### Added
- New app: drag-and-drop actions, a live drawing of the deck, multi-key editing, undo/redo, copy and
  paste, a searchable action library, icons, brand logos, app icons, emoji, Steam games and pictures.
- Background service with tray icon; the deck works with the window closed.
- First-run setup: finds the deck, installs the permission rule, imports version 1's configuration.
- Wide-screen widgets: clocks, system stats with graphs, Now Playing, timer/Pomodoro, weather with
  place search, command output, pictures and OBS status; rotation, default widget, Now Playing
  takeover while music plays, backgrounds and accent colour, live preview.
- Volume overlay on the wide screen for outputs, the microphone and apps (can be turned off).
- OBS Studio: scenes, studio mode, sources, filters, mute and volume, recording, streaming, replay
  buffer, virtual camera, collections, profiles and screenshots; live state on keys; every connection
  problem explained, settings found automatically; renamed scenes followed.
- Profiles that follow the app in front (KDE Plasma, GNOME, Hyprland, Sway, X11), with a sidebar
  toggle and a tray indicator.
- Pages and folders, per-page dials, small buttons, tap and hold on every control.
- Actions: media and seek, volume and mute for any target, switch output, play sound, shortcuts,
  type text, commands, apps, websites, multi-step actions, brightness, sleep, timer.
- Templates, `.ulanzideck` import/export (double-click or drag to import), daily and automatic backups
  with restore.
- Warning badges on keys whose app, file, page or scene is missing.
- `ulanzictl` command line: status, profiles, pages, import/export, backups, validate, monitor,
  diagnostics.
- Packages: Arch, .deb, .rpm, AppImage and a tarball with an installer.

[2.1.0]: https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/compare/v2.0.0...v2.1.0
[2.0.0]: https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/releases/tag/v2.0.0
