Ulanzi Deck 2.3.0 makes your keys move. GIFs play on the deck, keys can glow, pulse, chase or burn in any
colour, and the app finally shows what the deck is actually doing.

### New

- **GIF keys.** Drop a GIF on a key and the deck plays it by itself, so it keeps going with the window
  closed. Big GIFs are shrunk to fit, and the app tells you if any frames had to go.
- **Key effects.** Glow, Pulse, Chase and Fire, at three speeds, on any key, including one that's
  already playing a GIF. Every effect takes any colour, so yes, your Fire can be blue.
- **An effect for every state.** A key with live states gets an effect for each one. A light can sit
  plain while it's off and glow while it's on, and the deck follows along.
- **An effect while held.** Hold a key for a moment and it pulses, or plays whatever you pick under
  **When Held**. Let go and it lands on the effect for the state it's in now.
- **The app shows what the deck shows.** Keys in the window now follow their live state and cycle step.
  Turn a fan on from anywhere and its key changes in the app too.
- **Profile icons and colours.** Right-click a profile to give it an icon and a colour. A profile that
  follows an app takes that app's icon and colour by itself. Ctrl+1 to Ctrl+9 jump between profiles.
- **Saved colours.** Keep the colours you use in the colour picker, on every colour button in the app.
- **Display Brightness.** Control your PC monitors from a key or a dial, one at a time or all together,
  through KDE Plasma 6 or `ddcutil`.
- **Update All** in the plugin store, for when several plugins have updates waiting.
- **For plugin authors:** contract 1.3 lets a plugin set default effects and offer effect presets.

### Changed

- **Snappier keys.** The app keeps the deck from dozing off, so the first press after a quiet spell no
  longer stalls, and an action never waits for a picture to finish uploading.
- A new page now shows on the deck straight away.
- The status menu at the top left shows the deck's firmware version and has **Restart Service**.
- In the macro editor, a click selects a step and a right click opens it.
- `.ulanzideck` files and backups can reach 200 MB, up from 50 MB.

### Good to know

- **Effects wait for a hold.** A quick tap shows only the deck's own shrink. Any new picture leaves a key
  black for about 0.2 seconds, in any format, so a press can't change a key's picture any faster.
- **Moving keys cost the deck time.** The deck draws them itself. Three are no trouble, twelve are
  noticeable. New effects start on Slow, and the app shows a note on a page with seven or more.
- **Effects need 2.3.** A key with an effect shows a "needs a newer version" badge in 2.2, and the key is
  kept intact until you update.
- **Dials can jump to full brightness.** Older deck firmware does this when you turn a dial, and the app
  can't change it. See [known firmware issues](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/docs/troubleshooting.md#known-firmware-issues).
- Display Brightness covers laptop screens only on KDE Plasma.

See the [changelog](https://github.com/Tyaaa-aa/Ulanzi-Deck-Linux/blob/main/CHANGELOG.md) for the
full list of changes.

### Installation

Already on 2.1.1 or later? Use **Update** in the app. Otherwise, download the package for your distribution:
Arch package, `.deb`, `.rpm`, AppImage (run it with `--install` to add it to the application menu) or
the tarball with `install.sh`. Requires GTK 4.16 and libadwaita 1.6 or later, and glibc 2.39 or later
for the `.deb`, tarball and AppImage.
