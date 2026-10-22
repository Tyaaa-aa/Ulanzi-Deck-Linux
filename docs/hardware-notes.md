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

Each 1024-byte packet must be written as **1025 bytes with a leading report ID 0** **[homedeck]**.
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

* The deck caches images **by file name**: re-uploading under the same name does not redraw. Every
  image gets a content-hash name.
* **Partial update (0x000d) works**: a ZIP with only the changed keys updates just those keys.
* **Keys left out of an upload keep their image** (U5).
* **Back-to-back uploads break the screen.** 10-30 full uploads 30-130 ms apart left all keys black.
  The daemon spaces uploads at least 1 s apart, folds intermediate states into the latest, and uses
  partial updates for live changes.

## Other commands **[homedeck]**

`0x0004` lock screen, `0x0009` screensaver image (ZIP with one image), `0x001a` deck sleep time in
minutes (0 = off; the daemon sends 0 and dims by itself), `0x00e0` flash "!" on one key (ZIP with
`{"<col>_<row>":{"KeyType":1,"Progress":100,"State":1}}`), and the deck may ask for the label style
(cc 1 / pid 0x0b), which is answered.
