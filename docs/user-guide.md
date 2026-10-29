# Ulanzi Deck user guide

Everything you can do with Ulanzi Deck, from plugging the deck in to sharing your setup.
No technical knowledge needed. If something doesn't work, see [Troubleshooting](troubleshooting.md).

**Contents**
[First start](#first-start) ·
[The window](#the-window) ·
[Keys](#keys) ·
[Actions](#actions) ·
[Dials and small buttons](#dials-and-small-buttons) ·
[The wide screen](#the-wide-screen) ·
[Profiles, pages and folders](#profiles-pages-and-folders) ·
[Switching with the app in front](#switching-with-the-app-in-front) ·
[OBS Studio](#obs-studio) ·
[Templates, sharing and backups](#templates-sharing-and-backups) ·
[Preferences](#preferences) ·
[The tray icon](#the-tray-icon) ·
[Keyboard shortcuts](#keyboard-shortcuts)

---

## First start

1. Install Ulanzi Deck (see the [README](../README.md#install)) and plug in your D200 or D200X.
2. Open **Ulanzi Deck** from your app menu. The setup walks you through four short steps:
   - **Connect**: it finds the deck and shows its model.
   - **Permission**: Linux needs a one-time rule so your user can talk to the deck and press keys
     for you. Press **Grant Access…** and type your password once. (Prefer the terminal? The page
     shows the exact command.)
   - **Bring your setup over**: if you used version 1 (the Python app), import its keys in one click,
     or start empty, or start from a template.
   - **Background service**: turn on **Start at Login** so the deck works without the window open.
3. You're done. Changes reach the deck as you make them. There is no Save button.

> [!TIP]
> You can reopen the setup any time from the menu (☰) → **Set Up Deck**.

## The window

| Area | What it's for |
|---|---|
| **Left: sidebar** | The **Auto-Switch** toggle, your **profiles** and the current profile's **pages**. Brightness at the bottom. |
| **Middle: the deck** | A drawing of your deck. Click a key, dial, small button or the wide screen to edit it. |
| **Right: action list / key editor** | With nothing selected, the **action list** to drag from. Click a key to edit it; **×** at the top of the panel (or **Escape**) closes the editor and brings the list back. |
| **Header** | Connection status, undo/redo, **Plugins** (the puzzle piece) and the main menu (☰). |

Narrow window? The sidebar folds away; the button at the top left brings it back.

## Keys

Click a key to edit it. The editor shows the key's preview at the top and, under it, buttons for
its sections: **Look**, **Label**, **Press** and **Hold**. The section you pick stays open when you
click another key. **Clear Key** at the bottom empties the key.

A key's editor is one page, top to bottom: **Press** (what it does), **Label**, **When Held** and
**Look**. When Held and Look are folded into one row each until you open them (they stay open while you
click from key to key); picking an action already gives the key a matching icon and colours. Every key has:

- **Look**: an image, background colour, icon colour and icon size.
  Press **Choose…** next to Image to pick from:
  - **Icons** (thousands of clean line icons, searchable),
  - **Brands** (logos of apps and services),
  - **Apps** (the icons of apps on your computer),
  - **Emoji**, **Steam** games, or a **File** of your own (or drag a picture onto a key).
- **Label**: text on the key. It can show live values: insert `{cpu}`, `{ram}`, `{time}`,
  `{media_title}`, `{obs_scene}`, `{obs_rec_time}`, `{timer}` and more with the **{ }** button.
  **Label Position** moves it to the top, middle or bottom of that key, or hides it (**Hidden**).
- **Show Live State** (in Look): lets the key change with what it controls. A play key turns into pause, a
  recording key turns red and shows the time, a muted mic turns red.
- **Press** and **When Held**: two actions per key. When Held runs when you keep the key down for half
  a second (adjustable in Preferences).

**Keys that change**
- **Show Live State** (in Look) lets plugin keys follow what they control: a Home Assistant light key
  turns yellow when the light is on, an air conditioner key shows a blue snowflake while cooling.
- With a plugin action, **Label Shows** picks a live value for the label (State, Level, Target
  temperature, Room temperature) instead of text.
- **Cycle Through Actions** (System) runs a different action on each press: add steps, and give each
  its own image, colour and label. It starts over after the last one. **Key Shows** picks what the
  key shows: the step that ran (what the key did, the default) or the next step.

**Working faster**
- Drag an action from the action list onto a key: the key gets a matching icon and colour. Type in
  **Search actions** (Ctrl+L) to find one by name, category or plugin; the menu next to it (All,
  Navigation, System, …, then one per plugin with its logo) shows one category at a time, and clicking
  a section's name folds it away. Searching inside a category offers **Show … more in All** when other
  categories have matches too. The key editor's **Choose** / **Change** list works the same way.
  With a key selected, Ctrl+L shows the list in place of its editor: click an action to give it to
  the key, or press the × (**Back to the key**) or Escape to return to the editor.
- **Ctrl+click** several keys to change them all at once.
- Right-click a key to copy, paste or clear it, or to copy just its style. Drag a key onto another to swap them.
- **Ctrl+Z** / **Ctrl+Shift+Z** undo and redo anything.

A key with a small **⚠** badge can't work as it is: its app isn't installed, its sound file is
gone, its OBS scene doesn't exist… Click it: the top of the editor says what's wrong.

## Actions

| Group | Actions |
|---|---|
| **Navigation** | Next / previous page, go to a page, open a folder, back, switch profile, turn auto-switch on/off |
| **System** | Run a command (optionally in a terminal), launch an app, open a website or file, keyboard shortcut, type text, multi-step action |
| **Media** | Play/pause, next, previous, stop, skip back, skip forward (choose a player or use the one playing) |
| **Audio** | Volume up/down/set, mute, for the default output, the microphone, a specific output device or one app; switch the default output; play a sound |
| **OBS Studio** | Switch scene, toggle two scenes, show/hide a source, record, stream, replay buffer, virtual camera, mute an input, input volume, filters, studio transition, scene collection, OBS profile, screenshot |
| **Device** | Brightness up/down/set, sleep the deck, next widget on the wide screen |
| **Timer** | Start/pause, reset, add minutes |

Notes:
- **Keyboard Shortcut**: press **Record** and type the shortcut. Any combination works, Super
  included; to record a modifier on its own (just Super, say), press and release it. Escape cancels.
  While recording, the desktop's own shortcuts are paused for the window; GNOME may ask once whether
  to allow that.
- **Type Text** types anything a US keyboard can type. For other characters (é, ü, emoji…) it pastes
  through the clipboard, which needs `wl-clipboard` (Wayland) or `xclip` (X11) installed.
- **Plugin actions** (OBS Studio, Home Assistant and others you add) appear in the list under their
  plugin's name. A field that picks something, such as an OBS scene or a Home Assistant light, shows
  its current value; click it to open a list with a search field (and a group filter when the plugin
  has one). Some lists let you tick several items, then press **Done**. If the list is empty or the
  plugin is not connected, you can type the name by hand instead. To add a plugin, open **Preferences
  → Integrations** and press **Install Plugin…**, then choose a `.ulanziplugin` file. Plugins are
  unsigned, so this needs **Developer Mode** (on the same page); the dialog shows what the plugin may
  do before you install.
- **Multi Action** runs several actions in order, with optional pauses between them.
- **App volume**: pick an installed app, or press the speaker button to choose from the apps
  playing sound right now.

## Dials and small buttons

Each **dial** has four slots, one row each in its editor: **Turn Left** and **Turn Right**, **Press**
and **Hold**; open a row for its settings. Dials belong to the page, so each page can use them differently (**Use This
Dial on Every Page**, under the slots, copies one to all pages).
Good dial actions: volume, input volume in OBS, brightness, next/previous page, skip back/forward.

The two **small buttons** beside the dials have one action each.

## The wide screen

Click the wide screen in the drawing. Its editor has a **Display** section and, with Widgets, a
**Press** section (**When Pressed**). **Display** offers:

- **Clock styles** and **System Stats**: drawn by the deck itself.
- **Widgets**: drawn by Ulanzi Deck, with a live preview in the window.

**Widgets**

| Widget | Shows |
|---|---|
| Clock | Digital, analogue, minimal or split face; 12/24 hours, seconds, date |
| Now Playing | Song, artist, album art, progress (optionally a blurred cover behind) |
| System | CPU, memory, GPU, temperature, network, with graphs of the last minute |
| Timer | A countdown or Pomodoro (focus, then a 5-minute break). Tap the screen to start or pause |
| Weather | Temperature, conditions and today's high/low. **Place → Choose…** searches towns worldwide |
| Command Output | The first two lines a command prints, refreshed every few seconds |
| Pictures | A slideshow of your pictures |
| OBS Studio | The scene on air, REC and LIVE times, frame rate and dropped frames |

- **Add Widget** adds one; the arrows reorder them; the star makes one the **default**, the widget
  the screen starts on and comes back to.
- **Rotation**: stay on the default, change every few seconds, or go to the next one when tapped.
- **Now Playing While Music Plays**: music takes over the screen, and the widgets come back a few
  seconds after it stops (you choose how many).
- **Background**: dark, a colour, a gradient or a picture. **Accent Colour** colours hands, bars and graphs.
- **When Pressed**: give the screen its own tap and hold actions. Without one, a tap plays/pauses
  music, starts the timer or shows the next widget.

Turning the volume (on a key or dial) shows a volume bar on the screen for a moment, with the app's
or device's name. Turn it off in Preferences → Device → **Show Volume Changes**.

## Profiles, pages and folders

- A **profile** is a complete setup. Make as many as you like (Streaming, Editing, Gaming…).
  The **+** next to Profiles creates one, empty or from a template. Right-click a profile to rename
  it, export it, save it as a template, set its auto-switch apps or delete it (a backup is taken
  first). Drag profiles to reorder them; double-click to rename.
- **Pages**: a profile can have several pages of keys. The small **‹ ›** buttons on the deck flip
  through them, or use Next/Previous Page actions.
- **Folders** are pages you open from a key (**Open Folder**) and leave with **Back**.

## Switching with the app in front

Ulanzi Deck can change the profile when you switch apps: your editing keys appear when the editor
is in front, your streaming keys when OBS is.

1. Right-click a profile → **Auto-Switch** and add one or more apps.
2. Make sure **Auto-Switch** at the top of the sidebar is on.
3. When none of those apps is in front, the deck goes back to your default profile.

The dialog's top line says whether it works on your desktop. KDE Plasma, Hyprland, Sway and X11
work out of the box; GNOME needs the small extension that comes with Ulanzi Deck
(see [Troubleshooting](troubleshooting.md#auto-switch-doesnt-work)).
While auto-switch is on, the tray icon shows a coloured dot.

## OBS Studio

1. In OBS (version 28 or newer): **Tools → WebSocket Server Settings → Enable WebSocket server**.
2. In Ulanzi Deck: **Preferences → Integrations → Find OBS Settings**. It reads the port and
   password from OBS for you. The top row turns to **Connected · OBS … · N scenes**.
3. Drag OBS actions onto keys and pick scenes, sources, inputs and filters from the list button next
   to each field, or type a name.

If OBS is closed, the keys dim and Ulanzi Deck connects again by itself when OBS starts. Any other
problem is explained in plain words on that same Preferences page. Rename a scene in OBS and your
keys follow the new name.

## Templates, sharing and backups

- **Templates**: Profiles **+** → **From Template** starts from Streaming, Media Desk, Video Editing or
  Productivity, or from one of your own (**Save as Template** on any profile). A layout for the apps
  you use every day makes a great template.
- **Sharing**: right-click a profile → **Export** saves a `.ulanzideck` file. Open one (double-click,
  drag it onto the window, or menu → **Import**) to add it. If you already have a profile with that
  name you choose: keep both, replace or skip. Your OBS password is never included.
- **Backups**: Ulanzi Deck backs everything up once a day and before any import, delete or restore
  (the newest 20 are kept). **Preferences → Backups** lists them; **Restore…** brings one back, and the
  setup you had is backed up first.

## Preferences

Menu (☰) → **Preferences** (**Ctrl+,**):

| Page | Settings |
|---|---|
| **General** | Light/dark style, **Start at Login**, tray icon, switch profiles by app, hold time |
| **Device** | Brightness, sleep after inactivity and how dim, **Show Volume Changes**, label style (hide all, position, colour) |
| **Integrations** | OBS Studio connection; whether keyboard shortcuts and typing are allowed; a link to **Plugins** |
| **Backups** | Back up now, restore, open the backups folder |

## Plugins

The puzzle piece in the header opens **Plugins**. Nothing comes preinstalled: add what you use.

- **Discover** is the plugin store: search, pick a category, and press **Install**. Store plugins are
  signed and checked before they install (**Verified**). **Update** appears when a newer version is out;
  a plugin made for a newer Ulanzi Deck says which version it needs.
- **Installed** lists your plugins with their status, an on/off switch and a check mark for verified
  ones. Click one for its settings (a password or token, an address), Restart and Remove.
  **Install Plugin…** installs a `.ulanziplugin` file: signed ones right away, unsigned ones only with
  **Developer Mode** on.

## The tray icon

The tray icon works without the window: switch profiles, set the brightness, turn auto-switch on or
off, open the window, or quit the background service. With auto-switch on, the icon shows a coloured dot.

## Keyboard shortcuts

| Shortcut | Does |
|---|---|
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo |
| Ctrl+C / Ctrl+V | Copy / paste keys |
| Ctrl+Shift+C / Ctrl+Shift+V | Copy / paste only the style (colours, image, label) |
| Ctrl+A / Escape | Select all keys / select nothing (closes the key editor) |
| Delete | Clear the selected keys |
| Ctrl+L | Search the actions |
| Ctrl+O / Ctrl+E | Import / export `.ulanzideck` files |
| Ctrl+, | Preferences |
| Ctrl+? | All shortcuts |

While you type in a text field, the deck leaves Ctrl+A, copy, paste, undo, redo, the style shortcuts,
Delete and Escape to the text field, so they never change the keys. Escape also closes an open
dialog or menu first, and cancels shortcut recording, before it closes the key editor.
