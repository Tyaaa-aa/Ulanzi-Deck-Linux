# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added
- The app picks plugin items from lists: an `item` setting or parameter opens the plugin's list (search,
  group filter, live state, ticks for several), keeps showing a stored value the list lacks, and offers
  typing by hand only when the plugin can't list. Settings → Integrations → Install Plugin… installs a
  `.ulanziplugin` file (with Developer Mode on); Install from Folder (developers)… stays for folders.
- Plugins as one file: `<id>-<version>.ulanziplugin` (a ZIP with `plugin.toml`, `plugin.wasm` and
  optional `assets/`). `ulanzictl plugin install` takes a file or a folder, `scripts/package-plugin.sh
  <plugin-dir>` builds a package, and the D-Bus methods `PluginInspectFile` and `PluginInstallFile`
  (Developer mode, since packages are unsigned) unpack it with strict limits and no links.
- Plugin contract 1.1: `list-items` and `[[pickers]]`, so plugins offer lists to pick from (entries with
  label, subtitle, group, icon and live state; bounded by the service). Plugins built for 1.0 keep
  working. `ulanzictl plugin items <id> <picker> [query] [--group G]` and the D-Bus method
  `PluginListItems`.
- Settings → Integrations: manage plugins, their settings and secrets; plugin actions in the key editor.
  Each plugin has a page built from its manifest's `[[settings]]` (passwords and tokens go straight into
  its secret store), with its status, on/off, Restart and Remove…; Developer Mode adds Install from
  Folder… with an "Unverified plugin" warning and the plugin's permissions. The action picker lists
  plugin actions by plugin, with rows for their parameters, and keys show a warning when their plugin
  is missing, turned off, stopped working or can't be loaded.
- Home Assistant plugin (`plugins/ha`): toggle, run, set level and call any service, with live entity state on key labels (`{ha.light.desk}`, `{ha.sensor.temp.unit}`). Settings in `[plugins.ha.settings]`; the token moves to the secret store.
- OBS Studio plugin (`plugins/obs`): the built-in OBS actions and live state (`{obs.scene}`, `{obs.rec_time}`, …) as a sandboxed plugin, for trying out next to the built-in integration. Settings in `[plugins.obs.settings]`.
- Plugin host: plugins are sandboxed WebAssembly programs that add actions to keys. They can use
  timers, secrets (system keyring), status, WebSocket, TCP/TLS and HTTP connections limited by the
  hosts and local-network access they declare, and show live values on key labels as `{<plugin>.<key>}`.
  A plugin that crashes is restarted once, then left off. Managed with `ulanzictl plugin
  list|install|remove|enable|disable|restart|dev-mode` and new D-Bus methods; install from a folder
  needs Developer mode. Includes the plugin SDK (`ulanzi-plugin`), a hello example and the
  [plugin guide](docs/plugins.md).
  The deck starts without waiting for plugins: they come up in the background, and the keyring
  is asked only when a plugin first needs its secrets. With a pretend deck (`scripts/sandbox.sh --demo`)
  plugin secrets stay in the sandbox and the keyring is never asked.

### Changed
- The action list (the side panel with nothing selected, and the key editor's Choose / Change list) is
  easier to use with many plugins: one search field finds actions by name, category or plugin name; a
  row of category buttons (All, the built-in categories, then one per plugin) shows one category at a
  time and is remembered while the app runs; sections fold away and list their actions A–Z; plugin
  actions show their plugin's icon and name, and plugin actions can now be dragged from the side panel
  too. Searching inside a category offers "Show N more in All" when other categories match.
- The top bar's Actions button is gone. A key's editor has a × at the top of the panel, and Escape
  does the same when you are not typing: the key is deselected and the action list comes back. A
  text field, an open dialog or menu, and the shortcut recorder keep their own Escape (and the
  deck's copy, paste and Delete shortcuts leave them alone too). While the recorder listens, the
  app's own shortcuts (Ctrl+L, Ctrl+O, Ctrl+E, Ctrl+comma, Ctrl+?) are off, so any combination can be
  recorded. Ctrl+L puts the cursor in the action search (not while a dialog or menu is open).
- The key editor no longer scrolls as one long column. The key preview stays at the top, with a row
  of buttons under it: Look, Label, Press and Hold for a key; Left, Right (turning), Press and Hold
  for a dial; Display and Press for the wide screen; Look, Label and Keys for several keys. Each
  section fits the panel at the usual window sizes and scrolls on its own only when it has to. The
  section you pick stays open as you click from key to key (and on to a dial's Press or Hold) while
  the app runs. Clear Key and Use This Dial on Every Page stay in sight under every section.

### Fixed
- Menu items no longer end in "…" (Rename, Delete, Export, Import, Set Up Deck and the like); the guides use the same names.
- The sandbox script keeps the app's own icons visible: your real data folder is added to `XDG_DATA_DIRS`, so they no longer show as broken images.
- Text fields keep their own shortcuts: Ctrl+A, copy, paste, undo, redo and Delete edit the text while
  you type, instead of selecting, copying or clearing keys on the deck. The deck shortcuts work as before
  when no text field has the focus.
- OBS Studio plugin: an OBS that accepts the connection but refuses its version or scene list is now
  retried 1 s, 2 s, 4 s … up to every 10 s instead of every second, and the refusal is logged once. Volume
  up on an input at 0 % no longer jumps to full volume. Clearer labels for studio mode and "Mute or unmute".
- Home Assistant plugin: service calls get 30 s to answer before "didn't answer" is logged (scripts and slow
  devices took longer than the 5 s other requests get). A key with no entity chosen says "Choose an entity
  for this key first". "Call a service" is now "Call an action (service)", as Home Assistant names them, and
  a new Set level key starts at 100 %.
- The shortcut recorder captures Super (Meta) and any combination: Super alone, Super+X, Ctrl+Super
  and so on. Press and release modifiers alone to record just them; Escape still cancels. While it
  listens, the desktop's own shortcuts are paused on the window so KDE and GNOME don't swallow Super,
  and they come back as soon as recording ends. Caps Lock, Pause and keypad keys now work in shortcuts.

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
