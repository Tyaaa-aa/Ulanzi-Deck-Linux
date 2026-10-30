# Troubleshooting

Find the symptom below and work through the steps in order. If the problem remains,
[open an issue](../CONTRIBUTING.md#reporting-a-bug) and include the output of `ulanzictl diagnostics`.

## Contents

- [The deck is not detected](#the-deck-is-not-detected)
- [The service is not running](#the-service-is-not-running)
- [Keys do not respond](#keys-do-not-respond)
- [Keyboard Shortcut or Type Text does nothing](#keyboard-shortcut-or-type-text-does-nothing)
- [Auto-switch doesn't work](#auto-switch-doesnt-work)
- [A plugin does not install](#a-plugin-does-not-install)
- [OBS does not connect](#obs-does-not-connect)
- [Now Playing is empty](#now-playing-is-empty)
- [Weather is unavailable](#weather-is-unavailable)
- [No tray icon](#no-tray-icon)
- [Resetting the configuration](#resetting-the-configuration)

## The deck is not detected

1. Disconnect the deck, wait two seconds and reconnect it. Connect it directly rather than through a
   hub if possible.
2. If the window reports that the computer needs permission to use the deck, select **Fix…** (or
   open the main menu, select **Set Up Deck**, then **Grant Access…**), enter your password, and
   reconnect the deck.
3. If the window reports that another program is using the deck, close it. This is usually Ulanzi
   Studio running under Wine. `ulanzictl status` shows which program holds the device.
4. Check that the system sees the device: `lsusb | grep -i 2207` should list `2207:0019`.

## The service is not running

Select **Start** on the warning bar, or run:

```sh
systemctl --user enable --now ulanzi-deckd
```

If the service stops again, `journalctl --user -u ulanzi-deckd -n 50` shows the reason. Exit code 3
means another instance is already running, for example one started from an AppImage; quit it from
its tray icon.

## Keys do not respond

- The deck locks itself after about two minutes without a connection, and the service keeps it
  awake. If the keys stopped responding after the computer resumed from sleep, reconnect the deck.
- `ulanzictl monitor` prints every key press. If nothing is printed, see
  [The deck is not detected](#the-deck-is-not-detected). If presses are printed but nothing
  happens, the action's error is shown at the bottom of the main window and in `ulanzictl monitor`.
- A key with a **⚠** badge cannot run as configured. Select it to see the reason.

## Keyboard Shortcut or Type Text does nothing

- Both actions need access to `/dev/uinput` to create a virtual keyboard. **Preferences →
  Integrations → Shortcuts and Typing** shows whether access is granted; **Set Up Deck → Grant
  Access…** grants it.
- Type Text pastes characters that are not on a US keyboard (accented letters, emoji) through the
  clipboard, which requires `wl-clipboard` on Wayland or `xclip` on X11.
- With a keyboard layout other than US, Type Text may produce different punctuation. Letters and
  digits work on most Latin layouts.

## Auto-switch doesn't work

Right-click a profile and select **Auto-Switch**. The first line of the dialog shows whether your
desktop is supported.

| Desktop | Support |
|---|---|
| KDE Plasma | Supported. If the dialog says it is waiting, switch windows once. |
| GNOME | Requires the bundled shell extension. Enable it, then log out and back in: `gnome-extensions enable ulanzi-deck-focus@tyaaa-aa.github.io` |
| Hyprland, Sway, X11 | Supported. |
| COSMIC and other Wayland compositors | Not supported. These compositors do not expose the focused window to other applications. |

Also check that **Auto-Switch** is on at the top of the sidebar. When it is, the tray icon shows a
coloured dot.

## A plugin does not install

| Message | Cause and fix |
|---|---|
| Can't reach the plugin store | No network connection, or GitHub is unreachable. The Discover page keeps showing the last list it downloaded, but installs need a connection. |
| The signature does not match | The package was modified or was not signed by the plugin store. Download it again from the store. |
| Needs Ulanzi Deck *version* or newer | The plugin requires a newer version of the app. Update Ulanzi Deck. |
| Turn on Developer Mode to install unverified plugins | The file has no valid signature. Enable **Developer Mode** in the plugin manager only for plugins you trust. |

## OBS does not connect

Open **Plugins → Installed → OBS Studio**. The status line describes the problem.

| Status | Fix |
|---|---|
| Not installed | Install it from **Plugins → Discover**. |
| OBS is closed or its WebSocket server is off | Start OBS, then enable the server in **Tools → WebSocket Server Settings**. |
| Password needed or rejected | In OBS, open **Tools → WebSocket Server Settings → Show Connect Info**, and enter the password in the plugin's settings. |
| Wrong port | Use the port shown in **Show Connect Info** (4455 by default). |
| OBS 28 or newer needed | Update OBS. |

## Now Playing is empty

- The player must support MPRIS. Spotify, VLC, Elisa, Strawberry and most browsers do.
  `busctl --user list | grep mpris` lists the players the system can see.
- For media playing in a browser, KDE's Plasma Browser Integration extension provides better titles
  and album art.

## Weather is unavailable

- The widget needs an internet connection. The forecast refreshes every 15 minutes.
- If the location cannot be found, open the widget settings and select it with **Place → Choose…**
  instead of typing it.

## No tray icon

- On GNOME, install the AppIndicator and KStatusNotifierItem Support extension.
- Check that **Preferences → General → Tray Icon** is on.

## Resetting the configuration

The configuration is stored in `~/.config/ulanzi-deck`. To reset it while keeping a copy:

```sh
ulanzictl export ~/ulanzi-before-reset.ulanzideck
systemctl --user stop ulanzi-deckd
mv ~/.config/ulanzi-deck ~/.config/ulanzi-deck.old
systemctl --user start ulanzi-deckd
```

Automatic backups are kept in `~/.local/state/ulanzi-deck/snapshots/`. Restore one from
**Preferences → Backups** or with `ulanzictl restore <name>`.
