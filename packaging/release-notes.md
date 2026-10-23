Version 2 is a complete rewrite in Rust with a new GTK 4 / libadwaita app.

**New**
- Wide-screen widgets: clocks, system stats with graphs, Now Playing, a Pomodoro timer, weather,
  command output, pictures and OBS status, with rotation, backgrounds and a live preview.
- OBS Studio: scenes, studio mode, sources, filters, mute and volume, recording, streaming, replay
  buffer, virtual camera. Keys show what's on air and REC / LIVE times. Setup finds OBS's settings.
- Profiles that switch with the app in front (KDE Plasma, GNOME, Hyprland, Sway, X11).
- Templates, `.ulanzideck` import and export, automatic backups with restore.
- Volume overlay, Skip Forward / Back, Switch Output, Type Text, warning badges on keys that can't work.
- A background service with a tray icon; the deck works with the window closed.

**Upgrading from version 1:** the first run offers to import your old `~/.config/ulanzi/config.yaml`.

**Install:** pick the file for your distribution: Arch package, .deb, .rpm, AppImage (run it, `--install` adds it to the menu) or the tarball with `install.sh`. Needs GTK 4.16 and libadwaita 1.6.
