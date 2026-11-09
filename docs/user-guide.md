# User guide

This guide covers setting up the deck, configuring keys, dials and the wide screen, and managing
profiles, plugins and backups. For problems, see [Troubleshooting](troubleshooting.md).

## Contents

- [First start](#first-start)
- [The main window](#the-main-window)
- [Keys](#keys)
- [Actions](#actions)
- [Macros](#macros)
- [Dials and small buttons](#dials-and-small-buttons)
- [The wide screen](#the-wide-screen)
- [Profiles, pages and folders](#profiles-pages-and-folders)
- [Auto-switch](#auto-switch)
- [Plugins](#plugins)
- [OBS Studio](#obs-studio)
- [Templates, sharing and backups](#templates-sharing-and-backups)
- [Preferences](#preferences)
- [Updates](#updates)
- [Tray icon](#tray-icon)
- [Keyboard shortcuts](#keyboard-shortcuts)

## First start

1. Install Ulanzi Deck (see the [README](../README.md#install)) and connect your D200 or D200X.
2. Open Ulanzi Deck from the application menu. The setup assistant has four steps:
   1. Connect: detects the deck and shows its model.
   2. Permission: installs a udev rule so your user account can access the deck and send key
      presses. Select **Grant Access…** and enter your password. The page also shows the equivalent
      terminal command.
   3. Import: imports keys from version 1 (the Python app), or starts with an empty profile or a
      template.
   4. Background service: enable **Start at Login** so the deck works while the window is closed.

Changes are applied to the deck immediately; there is no save step. To run the setup again, open
the main menu (☰) and select **Set Up Deck**.

## The main window

| Area | Contents |
|---|---|
| Sidebar (left) | The Auto-Switch toggle, profiles, the pages of the current profile, and brightness. |
| Deck (centre) | A preview of the deck that follows it live, so a key shows its current state and cycle step. Select a key, dial, small button or the wide screen to edit it. |
| Side panel (right) | The action list when nothing is selected, or the editor for the selection. Close the editor with **×** or Escape. |
| Header bar | Connection status, undo and redo, the **Plugins** button and the main menu. Click the status to see the deck's firmware version, or to use **Restart Service** if something is stuck. |

In a narrow window the sidebar is hidden; the button at the top left shows it.

## Keys

Select a key to open its editor. The editor has four sections:

- **Press**: the action that runs when the key is pressed. Choosing an action also sets a matching
  icon and colours.
- **Label**: the text on the key, and its position (top, middle, bottom or hidden). Labels can
  include live values such as `{cpu}`, `{ram}`, `{time}`, `{media_title}`, `{output_name}` and
  `{timer}`; insert them with the **{ }** button. For plugin actions, **Label Shows** displays a
  value reported by the plugin instead, such as an OBS scene name or a room temperature.
- **When Held**: a second action that runs when the key is held for half a second. The hold time
  is set in Preferences.
- **Look**: image, background colour, icon colour and icon size. **Choose…** opens the image picker
  with line icons, brand logos, installed application icons, emoji, Steam games and image files.
  You can also drop an image file onto a key.

When Held and Look are collapsed until opened, and stay open as you move between keys. **Clear Key**
removes everything from the key.

### Live state

With **Show Live State** enabled (under Look), a key reflects the state of what it controls. A
play key switches to pause, a recording key turns red and shows the elapsed time, and a muted
microphone key turns red. Plugin keys work the same way: a Home Assistant light key turns yellow
when the light is on, and an air conditioner key shows a blue snowflake while cooling.

### Cycle Through Actions

**Cycle Through Actions** (in System) runs the next of several steps on each press and returns to
the first step after the last. Each step can have its own image, colour and label. **Key Shows**
controls whether the key displays the step that last ran (the default) or the step that will run
next.

### Play Sound

**Play Sound** has its own volume, relative to **Sound Volume** in Preferences → General. Use the
**Play** button next to the file to preview it.

### Effects and GIFs

Keys can move. Pick a GIF in **Look → Image**, or give a key a border effect:

Open the **Effects** section under **Look**:

- **Effect** is what the key shows at rest: **Glow**, **Pulse**, **Chase** or **Fire**.
- **When Held** is what it shows when you hold the key for a moment. **Default** is a Pulse, and
  **None** keeps the key plain. A quick tap shows only the deck's own shrink, because the deck takes
  about 0.2 seconds to show a new picture. The Default Pulse needs the key to have an action; an effect
  you choose plays on any key. If the key rests on the same effect, nothing changes when you hold it.
- Choosing an effect adds a colour button and **Speed**. A new resting effect starts on Slow, which is
  easier on the deck. The automatic colour uses the key's own colour, or your desktop's accent colour if
  that's too dark to see. Every effect can be any colour, Fire included.

A page with seven or more moving keys (effects or GIFs) shows a note under the deck, because many
moving keys slow the deck down.

A GIF can be up to 16 MB, 500 frames and 4096 pixels across. The deck can only play small ones, so the
app cuts colours and then frames until it fits, and says so under **Image** if frames went.

With **Show Live State** on, a key that has live states gets one Effect row per state, so an air
conditioner can burn while it's off and chase while it's on. **Effect Otherwise** covers any other
state, and a state set to **Same as Otherwise** follows it. Letting go of a key always lands on the
effect for the state it's in now.

The preview at the top of the panel plays the effect. Hold the mouse down on it to see the pressed
look. Plugins can offer ready-made sets in **Effect Preset**.

The deck can't show a GIF over about 300 KB, so bigger ones are shrunk to fit: fewer colours first, then
fewer frames, and a note in the Look section says so if frames went. A GIF can be up to 16 MB and 500
frames. A key shows its still look for a moment the first time its effect is drawn.

To turn the default Pulse off, use **Preferences → Key Effects**. Keys with their own effect keep it.

### Editing several keys

- Drag an action from the action list onto a key. **Search actions** (Ctrl+L) filters by name,
  category or plugin, and the category menu next to it limits the list to one category or plugin.
  Select a category heading to collapse it.
- With a key selected, Ctrl+L shows the action list in place of the editor. Select an action to
  assign it, or press Escape to return to the editor.
- Ctrl+click selects several keys, so you can change them together.
- Right-click a key to copy, paste or clear it, or to copy only its style. Drag a key onto another
  to swap them.
- Ctrl+Z and Ctrl+Shift+Z undo and redo any change.

A key with a **⚠** badge cannot run as configured, for example because the application is not
installed, the sound file is missing or the OBS scene no longer exists. Select the key to see the
reason at the top of the editor.

## Actions

| Category | Actions |
|---|---|
| Navigation | Next or previous page, go to page, open folder, back, switch profile, toggle auto-switch |
| System | Run command (optionally in a terminal), launch application, open website or file, keyboard shortcut, type text, multi action, cycle through actions, Display Brightness |
| Media | Play/pause, next, previous, stop, skip back, skip forward, for a specific player or whichever is playing |
| Audio | Volume up, down or set, and mute, for the default output, the microphone, a specific device or a single application; switch default output; play sound |
| Device | Brightness up, down or set, put the deck to sleep, next widget on the wide screen |
| Timer | Start/pause, reset, add minutes |

Plugins add their own categories, such as OBS Studio and Home Assistant. See [Plugins](#plugins).

Notes on specific actions:

- **Keyboard Shortcut**: select **Record** and press the shortcut. Any combination works, including
  Super. To record a single modifier, press and release it. Escape cancels. While recording, the
  desktop's own shortcuts are suspended for the window; GNOME may ask for permission the first time.
- **Type Text** types any character on a US keyboard layout directly. Other characters (accented
  letters, emoji) are pasted through the clipboard, which requires `wl-clipboard` on Wayland or
  `xclip` on X11.
- **Multi Action** runs several actions in sequence, with optional delays between them.
- **App volume**: select an installed application, or use the speaker button to choose from the
  applications currently playing audio.
- **Switch Output**: each press makes the next ticked output the default. Open an output to give
  the key an image, colour and label while that output plays (whatever changed it), and the volumes
  a switch to it sets: the output's own and any app's. Volumes are set before the switch, so nothing
  plays loud on the new output. An app that makes no sound at that moment gets its volume when it
  next plays, as long as that output is still the default.
- **Plugin actions**: fields that refer to an item, such as an OBS scene or a Home Assistant
  entity, open a searchable list (with a group filter where the plugin provides one). Some lists
  allow multiple selections; confirm with **Done**. If the list is empty or the plugin is not
  connected, you can enter the name manually.

**Display Brightness** changes your PC monitors, not the deck. Choose one monitor or **All Displays**,
then **Brighter**, **Dimmer** or **Set To**. A key remembers a monitor by its name, so restarts and
replugging don't break it. If that monitor isn't connected, the key says so and works again when
it's back. Two monitors with the same name are told apart only by their order, so they can swap
after replugging.

KDE Plasma 6 needs no setup. Any other desktop needs `ddcutil`, which talks to external monitors
over their video cable:

1. Install `ddcutil` from your distro.
2. Load the kernel module with `sudo modprobe i2c-dev`, and add `i2c-dev` to a file in
   `/etc/modules-load.d/` to keep it after a restart.
3. Check that `ddcutil detect` lists your monitors without `sudo`. If it reports a permissions
   problem, add yourself to the `i2c` group with `sudo usermod -aG i2c $USER` and log in again. On
   some distros the group comes from a separate rules package, such as `ddcutil-i2c-udev-rules` on
   openSUSE.
4. Turn on DDC/CI in the monitor's own menu if `ddcutil detect` calls it an invalid display.

`ddcutil` can't reach every monitor or TV, and it can't change a laptop's built-in screen, so outside
KDE those don't appear in the list. Quick dial turns are added together, so the monitor may catch up
in one jump.

## Macros

A macro is a named list of inputs that plays back in whatever app is in front. Macros live in a
library, so you can put one on any number of keys. To run several actions from one key, use Multi
Action instead.

**Make one:** click the **Macros** button next to Plugins and choose **New Macro**. That window lists
every macro, to edit, duplicate or delete.

**Put one on a key:** drag it from **Macros** in the action list, or drag **Macro** from **System** onto
a key and choose from the list on the key. That list also swaps a key to a different macro, or makes a new one for it.

**Record:** select **Record**, press your keys, then select **Stop**. Delays are recorded too.

Recording works while the macro window is in front. Linux stops apps from watching what you type in
other apps, which keeps your passwords safe. Your macro still plays back in whatever app you're using.

| To record | Do this |
|---|---|
| Keys | Press them |
| Mouse buttons and scrolling | Use the pad under the Record button |
| A shortcut the desktop keeps for itself | **+** → **Key…** |
| A spot on screen | **+** → **Pick a Spot…**, then click where the pointer should go. Escape cancels, and it cancels by itself after 30 seconds |
| Movement in pixels | **+** → **Move Pointer…** |
| Text | **+** → **Text…** |

**Save:** **Save** keeps the window open, and **Done** saves and closes. **Cancel** asks before dropping
unsaved changes, and the title says when there are any.

**Arrange:** steps sit in a numbered grid, coloured by kind. Drag a step to move it. Click a step to select it, right-click it to edit or delete it, and click a delay to change it.
Ctrl+click selects steps, Shift+click selects a range and Ctrl+A selects them all, to delete them or
drag them together. **Clear All** removes every step, after asking. **Add Action** adds any app action as a step. **Use Standard Delays** gives every step the same delay
and shows each press as one chip; turning it off brings back the recorded delays.

**Types:**

| Type | What the key does |
|---|---|
| Play Once | Plays once. You choose what pressing again does: stop, restart, nothing, or play again after |
| While Holding | Repeats while the key is down. You choose whether letting go finishes the pass |
| Toggle | Repeats until the key is pressed again |
| Sequence | Separate steps for press, while held and release |

Point at a type to see a short animation of it. It shows the choice you've made for pressing again or
letting go.

A macro starts as soon as the key goes down. On a key that also has a When Held action, Play Once and
Toggle wait for a tap instead. While Holding and Sequence can't share a key with a When Held action.

**Key feedback:** show nothing, a progress bar, a step count, or a look of your own for each step. Any
macro key can also use `{macro.step}`, `{macro.total}`, `{macro.percent}`, `{macro.name}` and
`{macro.running}` in its label and in Live key looks.

**One key's version:** **Edit for This Key Only** gives a key its own copy, which stops following the
library and isn't in the Macros list. From there, **Save as New Macro**, **Update** and **Use Library Version** all work with Undo.

**Going back:** a library macro remembers the version before its last save. **Restore Previous
Version** in the macro window brings it back. Backups keep these old versions; shared files don't.

**Stopping:** put **Stop All Macros** on a key to stop everything at once. Stopping always releases any
keys and buttons a macro is holding.

## Dials and small buttons

Each dial has four actions: **Turn Left**, **Turn Right**, **Press** and **Hold**, and a name shown
under it on the drawn deck (empty means "Dial 1" to "Dial 3"). Dials are configured per page, and a new
page starts without dials. Volume, OBS input volume, brightness, page navigation and media seeking work
well on dials. When a dial turns or is pressed, its ring on the drawn deck lights up, and a turn also
sweeps an arc in the direction of the turn.

### Dial library

The library is opt-in, like macros. A dial belongs to its page until you add it to the library.

- **Add to Library** (in the inspector or the right-click menu) moves the dial into the profile's dial
  library and links it on this page. A linked dial shows a link badge. Editing it changes every page
  that uses it, and the inspector shows how many pages that is.
- **Edit for This Page Only** turns a linked dial back into this page's own copy.
- **Pin to Every Page** shows the dial at the same position on every page, folders included. Each page
  keeps its own dial underneath, and **Unpin** shows it again. A pinned dial has a pin badge, and a page's
  own dial that a pin covers is marked as covered in the inspector. A dial can be both linked and pinned.
- **Remove from This Page** clears the dial. Delete does the same on a selected dial, except one that is
  pinned.

### Library tray

The **Library** button in the header, next to Macros, opens the tray under the deck. It lists the
library dials with their name and where they are used (pinned, a number of pages, or not placed).

- Select a dial in the tray to edit it. Drag one onto a dial on the deck to place it as a linked dial.
  Dropping it on a pinned position replaces the pin, which stays pinned; Undo reverses this.
- **New Dial**, the last tile, creates an empty library dial.
- More than four rows are split into pages. Use the arrow buttons or the dots under the grid.
- An empty dial's inspector has **Open Library**.
- **?** in the tray replays the guide. **Take the Library Tour** on the **All Set** page of Set Up Deck
  starts it too.

### Moving and copying dials

- Drag a dial onto another one to swap them. A pin travels with its dial. Swapping is one Undo step.
- Right-click a dial for **Copy**, **Paste**, **Add to Library** or **Edit for This Page Only**,
  **Pin to Every Page** or **Unpin**, **Rename** and **Remove from This Page**.
- Ctrl+C, Ctrl+V and Delete work on the selected dial. Pasting in the same profile keeps a link; pasting
  into another profile makes an own copy.

### Small buttons

The two small buttons next to the dials have one action each. They are drawn with their action's icon
and colour, and the action's name is the tooltip. They belong to a page: each page has its own pair.
**Pin to Every Page** and **Unpin** work as for dials, and a side without a pin shows the page's own
button. Small buttons from earlier versions load as pinned, so nothing changes until you unpin one.

On its first start, 2.5 backs up your profiles (see [Templates, sharing and backups](#templates-sharing-and-backups)) and pins each dial that is the same on every
page of a profile. Each page keeps its copy underneath, so unpinning changes nothing you see. This
happens once; placing a dial on every page later leaves it unpinned. Profiles imported from files made
before 2.5 are pinned the same way.

Profiles that use the library or pins need Ulanzi Deck 2.5 or newer.

## The wide screen

Select the wide screen in the deck preview to edit it. The **Display** section offers three modes:

- **Clock** styles and **System Stats**, rendered by the deck firmware.
- **Widgets**, rendered by Ulanzi Deck, with a live preview in the window.

### Widgets

| Widget | Shows |
|---|---|
| Clock | Digital, analogue, minimal or split face; 12 or 24 hours, seconds, date |
| Now Playing | Title, artist, album art and progress, optionally over a blurred cover |
| System | CPU, memory, GPU, temperature and network, with one-minute graphs |
| Timer | A countdown or Pomodoro timer (focus period, then a 5-minute break); tap to start or pause |
| Weather | Temperature, conditions and the day's high and low; **Place → Choose…** searches locations |
| Command Output | The first two lines of a command's output, refreshed every few seconds |
| Pictures | A slideshow from a folder |
| OBS Studio | Current scene, recording and streaming time, frame rate and dropped frames (requires the OBS Studio plugin) |

Widget settings:

- **Add Widget** adds a widget. Use the arrows to reorder them and the star to set the default
  widget, which the screen starts on and returns to.
- **Rotation**: stay on the default widget, rotate on a timer, or advance on tap.
- **Now Playing While Music Plays**: shows Now Playing while music is playing and returns to the
  other widgets a set number of seconds after it stops.
- **Background**: dark, solid colour, gradient or image. **Accent Colour** sets the colour of
  clock hands, bars and graphs.
- **When Pressed**: tap and hold actions for the screen. Without them, a tap plays or pauses
  media, starts the timer or shows the next widget, depending on the widget.

Volume changes from a key or dial briefly show a volume bar on the screen with the device or
application name. To turn this off, disable **Show Volume Changes** in Preferences → Device.

## Profiles, pages and folders

- A **profile** is a complete configuration. Create one with **+** next to Profiles, either empty
  or from a template. Right-click a profile to give it an icon and colour, rename it, export it, save
  it as a template, delete it, or set its auto-switch applications. A profile set to follow an app
  takes that app's icon and colour unless you've chosen your own, and both travel with exports. A
  backup is made before a profile is deleted. Drag profiles
  to reorder them, or double-click to rename.
- A profile can have several **pages**. Switch pages with the **‹ ›** buttons on the deck or with
  the Next Page and Previous Page actions.
- A **folder** is a page opened from a key with **Open Folder** and closed with **Back**.

## Auto-switch

Ulanzi Deck can switch profiles based on the focused application, for example to show editing
controls while a video editor is focused and streaming controls while OBS is.

1. Right-click a profile, select **Auto-Switch** and add one or more applications.
2. Make sure **Auto-Switch** at the top of the sidebar is on.

When you leave that app, the deck returns to the profile you were on. An app with no profile changes
nothing, so a profile you chose yourself stays until you change it.

The dialog shows whether auto-switch is supported on your desktop. KDE Plasma, Hyprland, Sway and
X11 are supported directly; GNOME requires the bundled shell extension (see
[Troubleshooting](troubleshooting.md#auto-switch-doesnt-work)). While auto-switch is on, the tray
icon shows a coloured dot.

## Plugins

Plugins add actions, widgets and integrations with other software. None are installed by default.
Open the plugin manager with the **Plugins** button in the header bar.

- **Discover** lists the plugins in the plugin store. Search or filter by category, then select
  **Install**. Store plugins are signed, and the signature is checked before installation. When a
  newer version is available, **Update** appears, and **Update All** shows how many are waiting
  when there is more than one. Plugins that need a newer version of Ulanzi Deck state the required
  version.
- **Installed** lists installed plugins with their status and an on/off switch; verified plugins
  have a check mark. Select a plugin to change its settings (such as an address, password or
  token), restart it or remove it.
- **Install Plugin…** installs a `.ulanziplugin` file. Signed packages install directly; unsigned
  packages require **Developer Mode**. The dialog lists the permissions the plugin requests before
  you confirm.

Plugins marked **Official** are maintained by the Ulanzi Deck project. Other plugins show their
author's name.

## OBS Studio

OBS Studio support is provided by the OBS Studio plugin.

1. Open **Plugins → Discover** and install **OBS Studio**.
2. In OBS 28 or later, open **Tools → WebSocket Server Settings**, enable the WebSocket server,
   and select **Show Connect Info** to get the password.
3. In **Plugins → Installed → OBS Studio**, enter the password, and the port if it is not 4455. The
   status changes to running once the plugin connects.
4. Add OBS actions to keys and select scenes, sources, inputs and filters from the lists.

With live state enabled, the active scene is shown in green, recording and streaming keys show
their elapsed time, muted inputs are red, hidden sources are dimmed, and all OBS keys are dimmed
while OBS is not running. The plugin reconnects automatically when OBS starts.

Profiles from Ulanzi Deck 2.0 are converted to use the plugin automatically. Install the plugin and
enter the OBS password once.

## Templates, sharing and backups

- **Templates**: under Profiles, select **+ → From Template** to start from Streaming, Media Desk,
  Video Editing or Productivity, or from your own templates. **Save as Template** on any profile
  creates one.
- **Sharing**: right-click a profile and select **Export** to save a `.ulanzideck` file. To import
  one, open it, drag it onto the window, or use **Import** in the main menu. If a profile with the
  same name exists, you can keep both, replace it or skip the import. Passwords and tokens are
  never exported. The export dialog has an **Include Library** switch; when it is off, dials placed from the
  library are written as own copies. Importing a file that carries a library
  shows **Also Import Its Library**; when it is off, linked dials become own copies.
- **Backups**: Ulanzi Deck makes a backup once a day and before every import, deletion or restore,
  and keeps the 20 most recent. **Preferences → Backups** lists them; **Restore…** restores one
  after backing up the current configuration.

## Preferences

Open Preferences from the main menu or with Ctrl+,.

| Page | Settings |
|---|---|
| General | Light or dark style, Start at Login, tray icon, auto-switch, hold time, sound volume, Key Effects |
| Device | Brightness, sleep timeout and dim level, Show Volume Changes, label style (visibility, position, colour) |
| Integrations | Whether keyboard shortcut and text typing actions are allowed; a link to the plugin manager |
| Backups | Create a backup, restore a backup, open the backup folder |

## Updates

Ulanzi Deck asks GitHub once a day whether a new version is out. To turn this off, open
**Preferences → General → Updates** and switch off **Check for Updates**. **Check Now** asks straight
away.

When a new version is out, a banner appears at the top of the window. **See What's New** shows the
release notes, with any important note first.

| Button | What it does |
|---|---|
| Update | Downloads the new version, checks its signature, backs up your profiles, installs it and restarts the app |
| Not Now | Hides the banner until the next start |
| Skip This Version | Hides the banner until a newer version is out |
| View on GitHub | Opens the release page |

**Update** works differently depending on how the app was installed:

| Installed from | What happens |
|---|---|
| AppImage | The AppImage file is replaced. Its file name stays the same. |
| Arch, Debian/Ubuntu, Fedora or openSUSE package | The package manager installs it. You're asked for your password. |
| Tarball, for one user | The new tarball's `install.sh` runs. |
| Tarball with `--system`, or anything else | No **Update** button. Download the new version from GitHub. |

The backup is listed in **Preferences → Backups** as "Before updating to *version*".

After an update through your package manager, or a new AppImage, the open window and the service
keep running the old version. The window then shows "Ulanzi Deck was updated. Restart to finish."
**Restart** restarts the service and the app.

### Files from a newer version

A profile or `.ulanzideck` file from a newer version opens with everything this version understands.
A key it doesn't understand is kept unchanged and shows a **⚠**. Select it to see which version it
needs. **Clear Key** removes it; otherwise it works again once the app is updated.

A profile too new to read at all is listed in the sidebar as "Needs a newer version". Its file isn't
changed.

## Tray icon

The tray icon works without the main window. From its menu you can switch profiles, change the
brightness, toggle auto-switch, open the window or quit the background service.

## Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Ctrl+Z / Ctrl+Shift+Z | Undo / redo |
| Ctrl+C / Ctrl+V | Copy / paste keys |
| Ctrl+Shift+C / Ctrl+Shift+V | Copy / paste style only (colours, image, label) |
| Ctrl+A / Escape | Select all keys / clear selection and close the editor |
| Delete | Clear the selected keys |
| Ctrl+L | Search actions |
| Ctrl+1 to Ctrl+9 | Go to that profile in the sidebar |
| Ctrl+O / Ctrl+E | Import / export `.ulanzideck` files |
| Ctrl+, | Preferences |
| Ctrl+? | Show all shortcuts |

While a text field has focus, Ctrl+A, copy, paste, undo, redo, the style shortcuts, Delete and
Escape apply to the text field rather than to keys. Escape closes an open dialog or menu, or
cancels shortcut recording, before it closes the key editor.
