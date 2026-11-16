"""Raster helpers for diff.py where ImageMagick is missing (macOS without Homebrew access).

Reading and resizing go through macOS `sips` (PNG → BMP); comparison and the heatmap are plain
Python; PNG output is stdlib zlib. With `magick` on PATH, diff.py uses ImageMagick instead.
"""

from pathlib import Path
import os
import shutil
import struct
import subprocess
import tempfile
import zlib

HAVE_MAGICK = shutil.which("magick") is not None


def read_rgb(path, w, h):
    """RGB bytes of `path`, scaled to exactly w×h."""
    with tempfile.TemporaryDirectory() as tmp:
        bmp = os.path.join(tmp, "x.bmp")
        subprocess.run(["sips", "-s", "format", "bmp", "-z", str(h), str(w), path, "--out", bmp], capture_output=True, check=True)
        data = Path(bmp).read_bytes()
    off, = struct.unpack_from("<I", data, 10)
    bw, bh, _, bpp = struct.unpack_from("<iiHH", data, 18)
    top_down = bh < 0
    bh = abs(bh)
    step = bpp // 8
    stride = (bw * step + 3) & ~3
    out = bytearray(w * h * 3)
    for y in range(min(h, bh)):
        src = off + (y if top_down else bh - 1 - y) * stride
        row = data[src:src + bw * step]
        o = y * w * 3
        for x in range(min(w, bw)):
            b, g, r = row[x * step], row[x * step + 1], row[x * step + 2]
            out[o + 3 * x:o + 3 * x + 3] = bytes((r, g, b))
    return bytes(out)


def write_png(path, w, h, rgb):
    raw = b"".join(b"\x00" + rgb[y * w * 3:(y + 1) * w * 3] for y in range(h))

    def chunk(tag, body):
        return struct.pack(">I", len(body)) + tag + body + struct.pack(">I", zlib.crc32(tag + body) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(raw, 6)) + chunk(b"IEND", b""))


def compare(tp, rp, heat, side, w, h, fuzz):
    """(differing pixel count, normalised RMSE); writes the heatmap PNG and the side-by-side JPEG."""
    a, b = read_rgb(tp, w, h), read_rgb(rp, w, h)
    limit = (fuzz * 255) ** 2 * 3
    diff = 0
    sq = 0
    hm = bytearray(len(a))
    for i in range(0, len(a), 3):
        dr, dg, db = a[i] - b[i], a[i + 1] - b[i + 1], a[i + 2] - b[i + 2]
        d = dr * dr + dg * dg + db * db
        sq += d
        if d > limit:
            diff += 1
            hm[i] = 255
        else:
            # The reference, dimmed, as ImageMagick's lowlight does.
            hm[i], hm[i + 1], hm[i + 2] = a[i] // 4, a[i + 1] // 4, a[i + 2] // 4
    write_png(heat, w, h, bytes(hm))
    rmse = (sq / (len(a) or 1)) ** 0.5 / 255
    strip = bytearray()
    for y in range(h):
        o = y * w * 3
        strip += a[o:o + w * 3] + b[o:o + w * 3] + hm[o:o + w * 3]
    with tempfile.TemporaryDirectory() as tmp:
        png = os.path.join(tmp, "side.png")
        write_png(png, 3 * w, h, bytes(strip))
        subprocess.run(["sips", "-s", "format", "jpeg", "-s", "formatOptions", "70", "--resampleWidth", "2700", png, "--out", side], capture_output=True, check=True)
    return diff, rmse
