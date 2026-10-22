#!/usr/bin/env python3
"""Capture the packets the legacy Python implementation sends, as golden fixtures.

Usage: python -I tools/capture_python_packets.py <legacy-src-dir> <out-dir>

A fake `hid` module records every write, so no device is needed.
"""
import io
import struct
import sys
import types
import zipfile
import zlib
from pathlib import Path

src, out = Path(sys.argv[1]), Path(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)

writes = []


class FakeDevice:
    def open_path(self, path):
        pass

    def set_nonblocking(self, flag):
        pass

    def write(self, data):
        writes.append(bytes(data))
        return len(data)

    def read(self, n):
        return []

    def close(self):
        pass


fake = types.ModuleType("hid")
fake.device = FakeDevice
fake.enumerate = lambda vid, pid: [{"path": b"/dev/hidraw-fake"}]
sys.modules["hid"] = fake
sys.path.insert(0, str(src))

from ulanzi_manager.device import UlanziDevice  # noqa: E402


def solid_png(path, w, h, rgb):
    raw = b"".join(b"\x00" + bytes(rgb) * w for _ in range(h))

    def chunk(kind, data):
        c = kind + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c) & 0xFFFFFFFF)

    png = (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
           + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))
    path.write_bytes(png)


def capture(name, fn):
    writes.clear()
    fn()
    assert len(writes) == 1, (name, len(writes))
    (out / f"{name}.bin").write_bytes(writes[0])


dev = UlanziDevice()
capture("brightness_37", lambda: dev.set_brightness(37))
capture("label_default", lambda: dev.set_label_style({}))
capture("label_hidden", lambda: dev.set_label_style({"ShowTitle": False}))
capture("small_window", lambda: dev.set_small_window_data(
    {"mode": 2, "cpu": 12, "mem": 34, "time": "12:34:56", "gpu": 0}))

red = out / "red.png"
solid_png(red, 196, 196, (255, 0, 0))
writes.clear()
dev.set_buttons({
    0: {"label": "Hi", "image": str(red)},
    5: {"label": "Ünïcode ✓"},
    13: {},
})
first = writes[0]
size = struct.unpack("<I", first[4:8])[0]
data = first[8:1024] + b"".join(writes[1:])
with zipfile.ZipFile(io.BytesIO(data[:size])) as zf:
    (out / "manifest_sample.json").write_bytes(zf.read("manifest.json"))
red.unlink()
print("captured", sorted(p.name for p in out.iterdir()))
