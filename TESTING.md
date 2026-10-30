# Hands-on test

What a person has to check before a release: the deck itself, sounds, other apps. Everything else is
covered by `scripts/test.sh`. Plan about 45 minutes. Tick each box; when something doesn't do what the
**→** line says, note the step number and what happened instead.

It never touches your own setup. It runs on **throwaway profiles** (the showcase set) with your real
deck: your own service pauses while you test and comes back when you finish. OBS gets its own scene
collection, which you can delete afterwards.

## 0. Start (3 min)

- [ ] 0.1 Close the Ulanzi Deck window if it is open (the tray icon can stay).
- [ ] 0.2 In a terminal, in this folder, run:
  ```sh
  scripts/sandbox.sh --showcase
  ```
  → It prints `Sandbox: /tmp/ulanzi-sandbox.…`. The window opens with four profiles: **Streaming**,
  **Media Desk** (ticked), **Video Editing**, **Productivity**. The deck shows the Media Desk keys
  within a few seconds. The header says **Connected**.
- [ ] 0.3 Keep that terminal open. To finish at any time: close the window. Your own service starts
  again by itself.

## 1. Keys and editing (5 min)

- [ ] 1.1 Click **Play / Pause** on the drawn deck. → The right side shows Key 2 with its look and action.
- [ ] 1.2 Change **Background** to any colour. → The key on the real deck changes within a second.
- [ ] 1.3 Press **Ctrl+Z**. → The colour goes back, on screen and on the deck. **Ctrl+Shift+Z** redoes it.
- [ ] 1.4 Drag **Open Website** from the library onto an empty key… (Media Desk has none: first right-click
  **Lock** → **Clear Key**). → The key gets a globe icon. Type `https://example.com` in **Address**.
- [ ] 1.5 Press that key on the deck. → Your browser opens example.com.
- [ ] 1.6 Click **Choose…** next to Image, pick the **Emoji** tab, choose any emoji. → The deck shows it.
- [ ] 1.7 Select two keys with **Ctrl+click**, change **Icon and Text** colour. → Both change at once.
- [ ] 1.8 Menu (☰) → **Preferences** → **Device** → turn on **Hide All Labels**. → Labels vanish from
  the deck and the drawing. Turn it off again.

## 2. Actions (8 min)

Start some music or a video in any player (browser tab is fine).

- [ ] 2.1 Press **Play / Pause** on the deck. → Playback pauses; press again → plays. The key's icon
  swaps between ▶ and ⏸.
- [ ] 2.2 Press **Next** / **Previous**. → Tracks change (if the player has a playlist).
- [ ] 2.3 Turn the **left dial**. → The system volume changes, and the wide screen shows a volume bar
  for a moment, then goes back.
- [ ] 2.4 Press **Mute**. → Sound mutes; the overlay says muted. Press again to unmute.
- [ ] 2.5 Drag **Skip Forward** onto a key and press it. → The track jumps 10 s ahead.
- [ ] 2.6 Drag **Switch Output** onto a key (tick two outputs if you have them) and press it. → The
  default output changes; press again → back.
- [ ] 2.7 Drag **Keyboard Shortcut** onto a key, record **Ctrl+Alt+T** (or any shortcut your desktop
  uses), press the key. → The shortcut happens.
- [ ] 2.8 Drag **Type Text** onto a key, enter `Hello, deck! 123`, click into a text editor, press the key.
  → Exactly that text appears.
- [ ] 2.9 Drag **Launch App**, choose any installed app, press the key. → The app opens; the key shows
  its icon.
- [ ] 2.10 On that key click **Change**, pick **Run Command** and leave the command empty. → A ⚠ badge
  appears on the key, and the top of the editor says why.

## 3. Wide screen (7 min)

- [ ] 3.1 Click the wide screen on the drawing. → **Display: Widgets**, with Now Playing and Clock.
- [ ] 3.2 Tap the wide screen on the deck. → It switches between Now Playing and an analogue clock
  (rotation "Next One When Tapped"). The second hand moves each second.
- [ ] 3.3 **Add Widget → Timer**. Set Rotation to **Every Few Seconds**, 5 seconds. → The deck cycles
  through the three widgets.
- [ ] 3.4 Drag **Timer Start / Pause** onto a key, add **Minutes 1** to the timer widget, press the key.
  → The key's label counts down; at 00:00 a short sound plays.
- [ ] 3.5 **Add Widget → Weather**, type a city. → Within a few seconds the deck shows its temperature
  (or a short message if offline).
- [ ] 3.6 **Add Widget → System**. → CPU, memory and more, with graphs that grow over a minute.
- [ ] 3.7 Set **Background → Picture** and choose a photo. → It shows behind the widgets, dimmed.
- [ ] 3.8 Set **Display** to **Clock** (a firmware style), then turn the volume dial. → The volume bar
  appears over it and the clock comes back about 1.5 s after you stop.

## 4. Pages, dials and buttons (4 min)

- [ ] 4.1 Pages **+**. → A new page is added and the deck shows it, empty; the small **›** button goes back
  and forth between pages.
- [ ] 4.2 On the drawing, click the **middle dial**, set **Turn Right** to **Next Page**. Turn it. → Pages
  change.
- [ ] 4.3 Hold a key that has a **Hold** action (set one: any key → Hold → **Sleep Deck**) for a second.
  → The deck dims; pressing any key wakes it without running that key.

## 5. Profiles and following the app in front (5 min)

- [ ] 5.1 Click **Productivity** in the sidebar. → The deck switches to its keys.
- [ ] 5.2 Right-click **Video Editing** → **Auto-Switch** → add your text editor (or any app), close.
  The dialog's top row says it works on your desktop.
- [ ] 5.3 Bring that app to the front. → The deck switches to Video Editing. Bring something else to the
  front. → It goes back to Media Desk (the default).
- [ ] 5.4 Turn auto-switch off from the **tray icon** menu. → Preferences shows it off too (no restart).

## 6. OBS Studio (8 min, skip if you don't use OBS)

- [ ] 6.1 In OBS: **Scene Collection → New** → name it `Deck Test`. Add scenes **Test A** and **Test B**;
  in Test A add a **Color Source** named `Test Box`.
- [ ] 6.2 **Plugins → Discover → OBS Studio → Install**, then in **Plugins → OBS Studio** type the
  password from OBS's Show Connect Info. → Its status turns to running.
- [ ] 6.3 Switch to **Streaming**. Click its first scene key, use the list button to pick **Test A**; set
  the next one to **Test B**. Press them on the deck. → OBS switches; the key of the scene on air turns
  green.
- [ ] 6.4 Drag **Show / Hide Source** onto a key: scene Test A, source Test Box. Press it. → The box
  hides in OBS; the key dims. Press again → it shows.
- [ ] 6.5 Press the **Record** key. → OBS records; the key turns red and counts `REC 00:01…`. Press
  again → stops (delete the test file afterwards if you like).
- [ ] 6.6 Add the **OBS Studio** widget to the screen. → It shows Test A / Test B and REC while recording.
- [ ] 6.7 Quit OBS. → Keys dim and Plugins → OBS Studio says **OBS is closed**. Start OBS again → it
  reconnects by itself within a few seconds.

## 7. Templates, sharing, backups (4 min)

- [ ] 7.1 Profiles **+** → **From Template**. → Four templates with previews. Pick **Media Desk**. → A
  new profile appears and the deck switches to it.
- [ ] 7.2 Right-click it → **Export**, save to your Desktop. Then menu → **Import** the same file,
  choose **Keep Both**. → A copy "… (2)" appears. **Undo** on the message removes it again.
- [ ] 7.3 Right-click a profile → **Save as Template**. → It's offered next time in From Template.
- [ ] 7.4 Preferences → **Backups** → **Back Up Now**, change a key, then **Restore…** that backup.
  → The key goes back.

- [ ] 7.5 Run `scripts/fake-release.sh`, then start `scripts/sandbox.sh --demo` with the two values
  it prints. Preferences → **Check Now**. → What's New shows 9.9.9 with its important note. **Skip
  This Version** hides it. With `--tamper`, **Update** says the download could not be verified.

## 7a. Macros (10 min)

- [ ] 7a.1 Macros → drag **New Macro** onto a key, select it, **Edit Macro**. **Record**, press Ctrl+A,
  **Stop**. → Four steps with delays. **Use Standard Delays** → one Ctrl+A chip. Turn it off → they're
  back with their delays. **Done**.
- [ ] 7a.2 Click into a text field and press the key. → The text is selected.
- [ ] 7a.3 Make a Toggle that types a letter every 500 ms. Press the key, then press again. → It
  stops and nothing stays held.
- [ ] 7a.4 Set Key Feedback to each option and watch the key while a slow macro plays. → It shows
  progress and returns to normal.
- [ ] 7a.5 Edit for This Key Only, change a step, then try each of the three choices and Undo.
- [ ] 7a.6 **+** → **Pick a Spot…** on each monitor, then play it. → The pointer lands there.
- [ ] 7a.7 Run a macro on https://www.atk.store/pages/mouse-keyboard-test-tool. → Downs, ups and
  timing match.

## 8. Unplug and sleep (2 min)

- [ ] 8.1 Unplug the deck. → The header says it's not connected. Plug it back in. → Within a few seconds
  the deck shows the current page again.
- [ ] 8.2 Drag the brightness slider (bottom left). → The deck follows.

## Finish

- [ ] Close the window. → The terminal says *Starting your own service again*; your own profiles are
  back on the deck.
- [ ] In OBS you can delete the **Deck Test** scene collection.
- Report: the step numbers that didn't match, with what you saw. `ulanzictl diagnostics` prints the
  versions and recent service log for a bug report.
