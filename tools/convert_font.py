"""Doom Eternal HUD fonts -> textures for the HUD.

Reads the user's own extracted fonts (~/doom-extract/fonts: <name>.font.bin + the 64 px distance
field atlas) and writes dist/natives/doom_ui/font_<name>.png (white, alpha = glyph coverage) and
font_<name>.json ({"size", "glyphs": {char: [x, y, w, h, top, left, advance]}}).

font.bin layout (little endian), as far as the HUD needs it:
  10-byte header, then one 10-byte record per glyph: u8 w, u8 h, i8 top, i8 left, u8 advance,
  u8 flags, u16 x, u16 y (a few leading placeholder records), then 8 bytes, then one u32 code
  per real glyph (the last records), one more u32, then the atlas path.
"""
import json
import struct
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

import numpy as np
from PIL import Image

SRC = paths.WORK / "fonts"
OUT = paths.NATIVES / "doom_ui"


def convert(name, edge=0.5, out=None):
    """`edge`: distance-field level of the glyph outline (lower = heavier); `out`: output name."""
    out = out or name
    d = (SRC / f"{name}.font.bin").read_bytes()
    end = d.index(b"fonts/")
    # The u32 right before the path is not a code; the code table runs back from there.
    codes = []
    i = end - 8
    while i >= 0:
        v = struct.unpack_from("<I", d, i)[0]
        if not (0x20 <= v < 0x10000):
            break
        codes.append(v)
        i -= 4
    codes.reverse()
    table = i + 4
    n_rec, rem = divmod(table - 8 - 10, 10)
    assert rem == 0 and n_rec >= len(codes), (name, table, len(codes))
    skip = n_rec - len(codes)  # leading null / placeholder records
    recs = [struct.unpack_from("<BBbbBBHH", d, 10 + 10 * k) for k in range(n_rec)]
    png = next(p for p in SRC.rglob("64_df*.png") if p.parent.name == name)
    im = Image.open(png)
    ch = max(range(len(im.getbands())), key=lambda k: np.ptp(np.asarray(im.split()[k])))
    df = np.asarray(im.split()[ch]).astype(np.float32) / 255.0
    # distance field -> coverage: edge at 0.5, a couple of texels of antialiasing
    a = np.clip((df - edge) / 0.06 + 0.5, 0.0, 1.0)
    rgba = np.zeros(df.shape + (4,), np.uint8)
    rgba[..., :3] = 255
    rgba[..., 3] = (a * 255).astype(np.uint8)
    OUT.mkdir(parents=True, exist_ok=True)
    Image.fromarray(rgba).save(OUT / f"font_{out}.png")
    glyphs = {}
    for k, code in enumerate(codes):
        w, h, top, left, adv, _, x, y = recs[k + skip]
        glyphs[chr(code)] = [x, y, w, h, top, left, adv]
    json.dump({"size": 64, "atlas": list(df.shape[::-1]), "glyphs": glyphs}, open(OUT / f"font_{out}.json", "w"))
    print(f"{out}: {len(glyphs)} glyphs, atlas {df.shape[1]}x{df.shape[0]}")


FONTS = {"eternal_numeral_regular": "eternal numeral regular", "eternal_regular": "eternal regular"}


def export():
    """Font metrics (raw .font entries) and their 64 px distance-field atlases from gameresources."""
    import subprocess
    res = paths.DOOM_BASE / "gameresources.resources"
    SRC.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(paths.SAMUEL), str(res), "raw", str(SRC), *FONTS.values()], capture_output=True,
                   creationflags=paths.NO_WINDOW)
    for name, entry in FONTS.items():
        f = SRC / f"{entry.replace(' ', '_')}.font.bin"
        if f.exists() and not (SRC / f"{name}.font.bin").exists():
            f.rename(SRC / f"{name}.font.bin")
    atlases = [f"fonts/{n}/64_df.tga$alpha$streamed$nomips$mtlkind=font" for n in FONTS]
    subprocess.run([str(paths.SAMUEL), str(res), "export", str(SRC), *atlases], capture_output=True,
                   creationflags=paths.NO_WINDOW)


if __name__ == "__main__":
    if not all((SRC / f"{n}.font.bin").exists() for n in FONTS):
        export()
    convert("eternal_numeral_regular")
    # Doom's letters. eternal_bold's extracted atlas is scrambled, so the heavy weight is baked from
    # eternal_regular's distance field with the outline moved out.
    convert("eternal_regular")
    convert("eternal_regular", edge=0.42, out="eternal_regular_heavy")
