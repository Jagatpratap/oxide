#!/usr/bin/env python3
"""Builds the Oxide Hub icons from apps/oxide_hub/assets/icon.svg (macOS only; run by hand
after changing the SVG, and commit the results).

    scripts/make_hub_icons.py

Writes, next to the SVG:
    icon.png      1024 px, transparent corners (window icon, Linux .desktop icon)
    icon.icns     macOS app bundle icon
    icon.ico      Windows program icon (PNG images inside)

macOS's QuickLook renders the SVG on white, so the white around the tile is un-blended back
into transparency (the tile's edge pixels are a mix of the tile colour and white).
"""
import os, struct, subprocess, sys, tempfile, zlib

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ASSETS = os.path.join(ROOT, "apps", "oxide_hub", "assets")
TILE = (0x1C, 0x1C, 0x1E)


def read_png(path):
    d = open(path, "rb").read()
    i, idat = 8, b""
    while i < len(d):
        n, = struct.unpack(">I", d[i : i + 4])
        kind, chunk = d[i + 4 : i + 8], d[i + 8 : i + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", chunk[:10])
            assert depth == 8 and ctype in (2, 6), "expected 8-bit RGB(A)"
        elif kind == b"IDAT":
            idat += chunk
        i += 12 + n
    bpp = 4 if ctype == 6 else 3
    raw, stride = zlib.decompress(idat), w * bpp
    rows, prev = [], bytearray(stride)
    for y in range(h):
        f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1 : (y + 1) * (stride + 1)])
        for x in range(stride):
            a = line[x - bpp] if x >= bpp else 0
            b, c = prev[x], prev[x - bpp] if x >= bpp else 0
            if f == 1:
                line[x] = (line[x] + a) & 255
            elif f == 2:
                line[x] = (line[x] + b) & 255
            elif f == 3:
                line[x] = (line[x] + (a + b) // 2) & 255
            elif f == 4:
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                line[x] = (line[x] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
        rows.append(line)
        prev = line
    px = []
    for line in rows:
        for x in range(w):
            p = line[x * bpp : x * bpp + bpp]
            px.append((p[0], p[1], p[2], p[3] if bpp == 4 else 255))
    return w, h, px


def write_png(path, w, h, px):
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for r, g, b, a in px[y * w : (y + 1) * w]:
            raw += bytes((r, g, b, a))

    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n")
        f.write(chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)))
        f.write(chunk(b"IDAT", zlib.compress(bytes(raw), 9)))
        f.write(chunk(b"IEND", b""))


def unblend_white(px):
    """Grey pixels between white and the tile colour become the tile colour with alpha."""
    out = []
    for r, g, b, _ in px:
        if max(r, g, b) - min(r, g, b) <= 10 and r >= TILE[0]:
            a = (255 - (r + g + b) / 3) / (255 - sum(TILE) / 3)
            a = max(0.0, min(1.0, a))
            out.append((*TILE, round(a * 255)) if a > 0.004 else (0, 0, 0, 0))
        else:
            out.append((r, g, b, 255))
    return out


def main():
    if sys.platform != "darwin":
        sys.exit("needs macOS (qlmanage, sips, iconutil)")
    svg = os.path.join(ASSETS, "icon.svg")
    tmp = tempfile.mkdtemp()
    subprocess.run(["qlmanage", "-t", "-s", "1024", "-o", tmp, svg], check=True, capture_output=True)
    w, h, px = read_png(os.path.join(tmp, "icon.svg.png"))
    png = os.path.join(ASSETS, "icon.png")
    write_png(png, w, h, unblend_white(px))

    def resized(size, name):
        out = os.path.join(tmp, name)
        subprocess.run(["sips", "-z", str(size), str(size), png, "--out", out], check=True, capture_output=True)
        return out

    iconset = os.path.join(tmp, "icon.iconset")
    os.makedirs(iconset)
    for s in (16, 32, 128, 256, 512):
        os.replace(resized(s, f"a{s}.png"), os.path.join(iconset, f"icon_{s}x{s}.png"))
        os.replace(resized(s * 2, f"b{s}.png"), os.path.join(iconset, f"icon_{s}x{s}@2x.png"))
    subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(ASSETS, "icon.icns")], check=True)

    # .ico with PNG entries (Windows Vista and later read these).
    sizes = (16, 24, 32, 48, 64, 128, 256)
    images = [open(resized(s, f"ico{s}.png"), "rb").read() for s in sizes]
    header = struct.pack("<HHH", 0, 1, len(sizes))
    offset = 6 + 16 * len(sizes)
    entries = b""
    for s, data in zip(sizes, images):
        entries += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    with open(os.path.join(ASSETS, "icon.ico"), "wb") as f:
        f.write(header + entries + b"".join(images))
    print("wrote icon.png, icon.icns, icon.ico in", ASSETS)


if __name__ == "__main__":
    main()
