# Changelog

All notable changes are listed here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/)
and the project uses [Semantic Versioning](https://semver.org).

## [Unreleased]

### Added
- Plugin host: plugins are sandboxed WebAssembly programs that add actions to keys. They can use
  timers, secrets (system keyring), status, WebSocket, TCP/TLS and HTTP connections limited by the
  hosts and local-network access they declare, and show live values on key labels as `{<plugin>.<key>}`.
  A plugin that crashes is restarted once, then left off. Managed with `ulanzictl plugin
  list|install|remove|enable|disable|restart|dev-mode` and new D-Bus methods; install from a folder
  needs Developer mode. Includes the plugin SDK (`ulanzi-plugin`), a hello example and the
  [plugin guide](docs/plugins.md).

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
