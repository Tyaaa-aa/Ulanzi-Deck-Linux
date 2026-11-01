# Hardware notes (Ulanzi D200 with dial base, firmware 5.2.3)

Measured on 2026-10-22 with `ulanzictl probe …` on the maintainer's deck. Protocol facts marked
**[homedeck]** come from fkras/homedeck's
[`guides/d200-protocol.md`](https://github.com/fkras/homedeck/blob/main/guides/d200-protocol.md) (MIT),
which recovered them from Ulanzi Studio 3.3.9.0; every one of them was confirmed on this deck.

## Device info (U4)

`request_device_info` (0x0003) is answered with:

```json
{ "SerialNumber": "02D0XXXXXXXXXXXXX", "Dversion": "5.2.3", "error": "0",
  "DeviceType": "D200", "HardwareVersion": "SSD210V100" }
```

The deck with the dial base still reports `DeviceType: "D200"`, so the model cannot be read from the
device. The app treats a deck as having dials once a dial event arrives; onboarding asks once.

## Inputs (U1)

| Control | index | b2 | b3 |
|---|---|---|---|
| Keys 1-13 | 0-12 | 1 | 1 press / 0 release |
| Wide screen | 13 | 1 | 1 / 0 |
| Small buttons left / right | **15 / 16** | 1 | 1 / 0 |
| Dials 1-3 | 17 / 18 / 19 | 2 | 0 release, 1 press, 2 left, 3 right |

The deck sends **no input at all while locked**. It locks itself after ~2 minutes without host traffic
**[homedeck]**; `unlock` (0x0005) brings the keys back.

## Writes

Each 1024-byte packet must be written as **1025 bytes with a leading report ID 0** **[homedeck]**
(`transport/hidraw.rs`, covered by `writes_report_id_then_the_packet`).
Without it the kernel strips the first byte of any continuation chunk that starts with `0x00`, which is
what the old "padding file" workaround was dodging. With the prefix the workaround is gone.

## Wide screen (U2)

* Real shape: **458×196**. The only test pattern drawn undistorted with its border fully visible.
  196×196 and 248×196 are stretched wide, 392×196 slightly tall.
* The top 2-3 rows show a faint ghost of the bottom edge (and vice versa); keep content a few pixels in.
* The small-window command (0x0006) needs **7 fields**: `mode|cpu|mem|HH:MM:SS|gpu|24H|date`
  **[homedeck]**. With the old 5-field payload the deck falls back to CPU/RAM/GPU numbers on top of
  everything, whatever mode is sent.
* Modes: `0` stats, `1` clock (with analogue dial), `2` **uploaded image only**, `200` clock + date +
  weekday, `201` clock + weekday, `202` clock + date, `203` large clock. Clock modes draw **on top of
  the uploaded image**, so a custom background works under the firmware clocks.
* Sent once a second, the small-window update also keeps the deck from locking.

## Uploads (U3, U5)

* The deck caches images **by file name**: re-uploading under the same name does not redraw. Image
  names carry a per-process random seed that is bumped on every connection, so names are never reused
  across connections or daemon restarts; within a connection they are content hashes.
* **Partial update (0x000d) works**: a ZIP with only the changed keys updates just those keys.
* **Keys left out of an upload keep their image** (U5).
* An early round of tests blamed partial updates and fast uploads for blank keys and reboots. Those
  tests ran with a transport bug: packets were written **without the report ID 0 prefix**, so chunks
  starting with `0x00` lost a byte. With the prefix (re-probed 2026-10-22 with `ulanzictl probe crash`),
  92 KB partial updates and 177 KB full pages **back to back** all survived, and the deck never
  rebooted.
* The daemon still spaces uploads 0.5 s apart (folding intermediate states into the latest), waits for
  the device-info reply before the first upload after connecting, and sends only the changed keys for
  changes on the same page.

## GIFs and key speed (U6)

Measured on 2026-10-31 with `ulanzictl probe gif`.

* **The deck plays GIFs by itself.** A looping 196×196 GIF (12 frames, 100 ms each) animated with no
  host traffic, in both a full-page upload and a partial update (0x000d).
* **The file name decides.** The same GIF under a `.png` name left the key blank, so the deck goes by
  the extension, not the content.
* Thirteen ran at once (`ulanzictl probe gif-limits`, 2026-11-01), at 74 KB in all and at 2.7 MB in
  all, with no reboot. The last key looked the choppiest both times.
* **One GIF can't go over about 300 KB.** Files of 265 KB and less played; files of 357 KB and more
  showed the deck's yellow question mark, alone or among others, with no reboot. Frame count, frame
  time and encoder speed made no difference (tested 2026-11-01). Fewer
  colours is the way to keep the frames: 24 frames of Fire fell from 397 KB to 123 KB and played.
* The deck ignores a GIF's loop count: a one-shot fade started over at its end (tested 2026-11-01), so anything that plays once needs the app to swap it out in time.
* Swapping a key's GIF usually took under 100 ms to write. A few writes stalled for 0.4 to 1 s: about
  1 in 10 with one upload per press, about 1 in 3 with a fade and a loop per press.
* Fire at 40 ms a frame played on all 13 keys, at 1.6 MB in all (2026-11-02).
* The deck's own label (`text` in the manifest) shows over a GIF, and the GIF keeps playing.
* One key took 72 partial updates of a 1.3 KB PNG back to back at 30 updates a second, with no gap, no
  blinking and no glitches. Bigger pictures take longer: this is a best case, not a frame rate to
  count on.
* Reacting to a press took about 3.3 ms to write, yet the key sometimes changed up to about a second
  late, mostly on the first press after a pause, and quickly on presses right after. The delay is on
  the deck's side (input report or drawing); which one is not known. The probe sends no idle traffic,
  so the once-a-second small-window update may change this.

## Other commands **[homedeck]**

`0x0004` lock screen, `0x0009` screensaver image (ZIP with one image), `0x001a` deck sleep time in
minutes (0 = off; the daemon sends 0 and dims by itself), `0x00e0` flash "!" on one key (ZIP with
`{"<col>_<row>":{"KeyType":1,"Progress":100,"State":1}}`), and the deck may ask for the label style
(cc 1 / pid 0x0b), which is answered.

## Swapping a key to a file already on the deck

A partial update can name a file sent earlier, with no picture in it, so a swap takes only a 204-byte
ZIP that writes in well under a millisecond. Tested 2026-11-02, the first
swap worked, the next went blank, and later ones did nothing, so stored files don't stay usable. Even
when a swap worked, the key was slow to change, so the delay on a GIF key is the deck loading the GIF,
not the upload.

## Showing a press

Tested on one key on a dark page (2026-11-02), changing one thing at a time:

| Setup | Felt |
|---|---|
| Quiet page, 62 KB GIF on press, no gap between uploads | Very good |
| The same, with a 0.5 s gap | Broken: the pressed look stayed after letting go |
| Key rests on a GIF | Good |
| 3 other keys playing GIFs | Good, first press slow |
| 12 other keys playing GIFs | Not good |
| 12 others, and the key rests on a GIF | Bad |
| 12 others, frames streamed instead of a GIF | Bad |
| Quiet page, a small message every 200 ms | Best: no stalled writes |

So the upload gap hurt most, a quiet deck stalls the next write for 0.4 to 1 s, and many animated keys
slow everything down.

Dark test keys hid the real limit. On a real page, any new picture leaves the key black for about 0.2 s
before it shows: GIF, PNG, JPEG and BMP alike, whatever the size. Frames streamed at 15 or 30 a second,
in any of those formats, left the key black until released. So a press can't change a key's picture
quickly, and the app starts pressed effects only after a short hold.

## Pressing a key

The picture shrinks while a key is held, and the deck does this by itself. Reading the 5.3.1 and 5.3.6
firmware (2026-11-02): pressing a key moves its picture and text views in by 17 pixels on every side, and
letting go puts them back. Nothing in the manifest or the protocol switches this, so the host can't turn
it off or use it. Thirteen guesses at manifest fields changed nothing either.

## Dials and brightness

Turning a dial sets full brightness on some firmware. In 5.3.1, after a turn is handled, the firmware
turns the screen on and sets brightness to a fixed 100, whatever the host chose. In 5.3.6 this is
gone: a turn only wakes a screen that is off, and it restores the saved brightness. Firmware 5.2.3
isn't available to read, but it behaves like 5.3.1 (tested 2026-11-02 with app builds 2.1.1 and current, before and after replugging). Sending the
brightness back after each turn works, but it flickers, so the app doesn't do it.

## Starting up

For about five seconds after being plugged in, the deck shows up as USB device `18d1:d002` (maker
`Zkswe`, product `ulanzi`, serial `0123456789ABCDEF`), then restarts as `2207:0019`, the id the app looks
for, with its real serial number (seen in the kernel log, 2026-11-02).

