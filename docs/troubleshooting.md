# Troubleshooting

Find the symptom, try the steps in order. Still stuck? [Open an issue](../CONTRIBUTING.md#reporting-a-bug)
and include the output of `ulanzictl diagnostics`.

**Contents**
[The deck isn't found](#the-deck-isnt-found) ·
[The window says the service isn't running](#the-window-says-the-service-isnt-running) ·
[Keys don't react](#keys-dont-react) ·
[Shortcuts or Type Text do nothing](#shortcuts-or-type-text-do-nothing) ·
[Auto-switch doesn't work](#auto-switch-doesnt-work) ·
[OBS won't connect](#obs-wont-connect) ·
[Now Playing shows nothing](#now-playing-shows-nothing) ·
[Weather says unavailable](#weather-says-unavailable) ·
[No tray icon](#no-tray-icon) ·
[Starting over](#starting-over)

---

## The deck isn't found

1. Unplug the deck, wait two seconds, plug it in again (directly, not through a hub, if you can).
2. *This computer needs permission to use the deck*: press **Fix…** on that bar (or menu ☰ →
   **Set Up Deck** → **Grant Access…**), type your password, then unplug and replug the deck.
3. *Another program is using the deck*: close Ulanzi Studio (in Wine) or any other program using it.
   `ulanzictl status` names it.
4. Check that Linux sees it: `lsusb | grep -i 2207` should list a device `2207:0019`.

## The window says the service isn't running

On the bar *The Ulanzi Deck service is not running*, press **Start**, or run:

```sh
systemctl --user enable --now ulanzi-deckd
```

If it stops again, `journalctl --user -u ulanzi-deckd -n 50` shows why. Exit code 3 means another
copy is already running (for example one started from an AppImage): quit that one from its tray icon.

## Keys don't react

- The deck locks itself after about two minutes without contact; the service keeps it awake. If keys
  stopped after the computer slept, unplug and replug the deck.
- `ulanzictl monitor` prints every press. Nothing printed: see [The deck isn't found](#the-deck-isnt-found).
  Presses printed but nothing happens: the key's action reports its error in the window (a message at
  the bottom) and in `ulanzictl monitor`.
- A key with a **⚠** badge can't work as set up; click it to see why.

## Shortcuts or Type Text do nothing

- These need permission to create a virtual keyboard (`/dev/uinput`). **Preferences → Integrations →
  Shortcuts and Typing** says whether it's allowed; **Set Up Deck → Grant Access…** fixes it.
- Type Text with letters a US keyboard lacks (é, ß, emoji…) pastes through the clipboard and needs
  `wl-clipboard` (Wayland) or `xclip` (X11).
- If your keyboard layout isn't US, Type Text may produce different symbols for punctuation. Letters
  and digits work on most Latin layouts.

## Auto-switch doesn't work

Right-click a profile → **Auto-Switch**: the top line says what your desktop supports.

- **KDE Plasma**: built in. If it says *waiting*, switch windows once.
- **GNOME**: enable the extension that comes with Ulanzi Deck, then log out and in:
  ```sh
  gnome-extensions enable ulanzi-deck-focus@tyaaa-aa.github.io
  ```
- **Hyprland, Sway, X11**: built in.
- **COSMIC and other Wayland desktops**: not possible yet (they don't tell other apps which window is
  in front).
- Make sure **Auto-Switch** is on (top of the sidebar; the tray icon shows a coloured dot).

## OBS won't connect

Open **Preferences → Integrations**: the first row says exactly what's wrong.

| It says | Do this |
|---|---|
| OBS is closed | Start OBS; Ulanzi Deck connects within a few seconds |
| OBS's WebSocket server is off | In OBS: Tools → WebSocket Server Settings → Enable WebSocket server, or press **Turn On WebSocket Server** while OBS is closed |
| Wrong port | Press **Find OBS Settings** |
| Password needed / Wrong password | Press **Find OBS Settings** (it reads the password from OBS) |
| OBS 28 or newer needed | Update OBS |
| OBS Studio not found | Install OBS (Flatpak or native both work) |

## Now Playing shows nothing

- Your player must support MPRIS (Spotify, VLC, most browsers, Elisa, Strawberry… do).
  `busctl --user list | grep mpris` lists the players Linux sees.
- Browser videos: KDE's *Plasma Browser Integration* extension gives the best titles and album art.

## Weather says unavailable

- It needs an internet connection; the forecast refreshes every 15 minutes.
- *Can't find "…"*: open the widget and choose the place with **Place → Choose…** instead of typing it.

## No tray icon

- GNOME needs the *AppIndicator and KStatusNotifierItem Support* extension.
- **Preferences → General → Tray Icon** must be on.

## Starting over

Your setup lives in `~/.config/ulanzi-deck`. To start fresh but keep a copy:

```sh
ulanzictl export ~/ulanzi-before-reset.ulanzideck
systemctl --user stop ulanzi-deckd
mv ~/.config/ulanzi-deck ~/.config/ulanzi-deck.old
systemctl --user start ulanzi-deckd
```

Backups made by the app stay in `~/.local/state/ulanzi-deck/snapshots/` and can be restored from
**Preferences → Backups** or with `ulanzictl restore <name>`.
