# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added
- Plugins: sandboxed WebAssembly programs that add actions to keys. They can use timers, secrets (system
  keyring), status, WebSocket, TCP/TLS and HTTP connections limited to the hosts and local-network access
  they declare, and show live values on key labels as `{<plugin>.<key>}`. A plugin that crashes is
  restarted once, then left off. The deck starts without waiting for plugins, and the keyring is asked
  only when a plugin first needs its secrets. Manage them with `ulanzictl plugin
  list|install|remove|enable|disable|restart|dev-mode` and the matching D-Bus methods. Includes the
  plugin SDK (`ulanzi-plugin`), a hello example and the [plugin guide](docs/plugins.md).
- Plugin contract 1.1: `list-items` and `[[pickers]]`, so plugins offer lists to pick from (entries with
  label, subtitle, group, icon and live state, bounded by the service). Plugins built for 1.0 keep
  working; a 1.0 manifest's `[[pickers]]` tables are still accepted and ignored. `ulanzictl plugin items <id> <picker> [query] [--group G]` and the D-Bus method
  `PluginListItems` show a list from the command line.
- Plugins as one file: `<id>-<version>.ulanziplugin`, a ZIP with `plugin.toml`, `plugin.wasm` and optional
  `assets/`. `scripts/package-plugin.sh <plugin-dir>` builds one, `ulanzictl plugin install` takes a file
  or a folder, and the D-Bus methods `PluginInspectFile` and `PluginInstallFile` unpack it with strict
  limits and no links. Installing needs Developer Mode, since packages are unsigned.
- Settings → Integrations: a page for each plugin, built from its manifest, with its status, on/off,
  Restart and Remove…. Passwords and tokens go straight into its secret store. **Install Plugin…**
  installs a `.ulanziplugin` file and shows what the plugin may do, with an "Unverified plugin"
  warning; with Developer Mode on, **Install from Folder (developers)…** does the same for a folder.
- Plugin actions in the key editor, listed by plugin, with rows for their parameters. A parameter that
  picks something opens the plugin's list (search, group filter, live state, ticks for several), keeps
  showing a stored value the list lacks, and offers typing by hand only when the plugin can't list.
  Such rows show the names the plugin gives (Kitchen, not `light.kitchen`), asked in the background.
  Keys show a warning when their plugin is missing, turned off, stopped working or can't be loaded.
- Home Assistant plugin (`plugins/ha`): toggle, run, set level and call any action (service), with live
  entity state on key labels (`{ha.light.desk}`, `{ha.sensor.temp.unit}`) and a list to pick entities
  from. Settings are in `[plugins.ha.settings]`; the token moves to the secret store.
- OBS Studio plugin (`plugins/obs`): the built-in OBS actions and live state (`{obs.scene}`,
  `{obs.rec_time}`, ...) as a sandboxed plugin, with lists for scenes, inputs, sources, filters,
  collections and profiles. It runs next to the built-in integration. Settings are in
  `[plugins.obs.settings]`.

### Changed
- Function keys are written f1 to f24; zero-padded names such as f01 are no longer accepted.
- The action list (the side panel with nothing selected, and the key editor's Choose / Change list) is
  easier to use with many plugins. One search field finds actions by name, category or plugin name. A
  row of category buttons (All, the built-in categories, then one per plugin) shows one category at a
  time and is remembered while the app runs. Sections fold away and list their actions A to Z. Plugin
  actions show their plugin's icon and name and can be dragged from the side panel. Searching inside a
  category offers "Show N more in All" when other categories match.
- The top bar's Actions button is gone. A key's editor has a × at the top of the panel, and Escape does
  the same when you are not typing: the key is deselected and the action list comes back. Ctrl+L puts
  the cursor in the action search (not while a dialog or menu is open). With a key selected, the list
  then gives way to the key's editor again when you click an action (it goes to the key), pick another
  key, or press the × ("Back to the key") or Escape.
- The key editor no longer scrolls as one long column. The key preview stays at the top, with a row of
  buttons under it: Look, Label, Press and Hold for a key; Left, Right, Press and Hold for a dial;
  Display and Press for the wide screen; Look, Label and Keys for several keys. Each section fits the
  panel at the usual window sizes and scrolls on its own only when it has to. The section you pick
  stays open as you click from key to key while the app runs. Clear Key and Use This Dial on Every Page
  stay in sight under every section.
- Menu items no longer end in "…" (Rename, Delete, Export, Import, Set Up Deck and the like). The
  guides use the same names.

### Fixed
- The shortcut recorder captures Super (Meta) and any combination: Super alone, Super+X, Ctrl+Super and
  so on. Press and release modifiers alone to record just them; Escape still cancels. While it listens,
  the desktop's own shortcuts are paused on the window so KDE and GNOME don't swallow Super, and they
  come back as soon as recording ends. The app's own shortcuts are off too, so any combination can be
  recorded. Caps Lock, Pause and keypad keys now work in shortcuts. A key the deck can't send (for
  example `<` or `ü` on some layouts) is no longer saved: the row says "This key can’t be sent" and
  keeps listening.
- Text fields keep their own shortcuts: Ctrl+A, copy, paste, undo, redo and Delete edit the text while
  you type, instead of selecting, copying or clearing keys on the deck. An open dialog or menu keeps its
  Escape. The deck shortcuts work as before when no text field has the focus.
- The sandbox script keeps the app's own icons visible: your real data folder is added to
  `XDG_DATA_DIRS`, so they no longer show as broken images.
- OBS Studio plugin: an OBS that accepts the connection but refuses its version or scene list is retried
  after 1 s, 2 s, 4 s and so on up to every 10 s instead of every second, and the refusal is logged
  once. Volume up on an input at 0 % no longer jumps to full volume. Clearer labels for studio mode and
  "Mute or unmute".
- Home Assistant plugin: service calls get 30 s to answer before "didn't answer" is logged, since
  scripts and slow devices took longer than the 5 s other requests get. A key with no entity chosen
  says "Choose an entity for this key first". "Call a service" is now "Call an action (service)", as
  Home Assistant names them, and a new Set level key starts at 100 %.

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

[Unreleased]: https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/releases/tag/v2.0.0
