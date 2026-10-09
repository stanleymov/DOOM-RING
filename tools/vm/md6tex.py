"""A model's textures as Vega exported them (what the viewmodel renderer was tuned on), from
SAMUEL's model export (modelExports/<model>/images + material2 decls) and the images' own headers:

- single-channel images (BIM format 0x18, BC4) are saved greyscale (SAMUEL writes RGBA);
- colour images get the header's linear-light scale and bias (floats at bytes 36 and 32), as Vega
  applies them (SAMUEL leaves them out: guns up to 3x too bright);
- normal maps get Z rebuilt from X / Y (the renderer only reads X / Y anyway).
Checked against Vega's _images for every weapon (within BC decoder rounding).
"""
import re
from pathlib import Path

import numpy as np
from PIL import Image


def _s2l(c):
    c = c / 255.0
    return np.where(c <= 0.04045, c / 12.92, ((c + 0.055) / 1.055) ** 2.4)


def _l2s(c):
    return 255.0 * np.where(c <= 0.0031308, c * 12.92, 1.055 * np.power(np.maximum(c, 0.0), 1 / 2.4) - 0.055)


def decl_images(model_dir):
    """{png stem: art path (.tga)} from the exported material2 decls."""
    out = {}
    for d in (Path(model_dir) / "material2").glob("*.decl"):
        for path in re.findall(r'filePath = "([^"]+)"', d.read_text(errors="ignore")):
            out.setdefault(Path(path).stem, path)
    return out


def image_header(res, art_path):
    """The BIM header (first 64 bytes) of the streamed image for an art path, or None."""
    import struct  # noqa: F401
    best = None
    for a in ("gameresources_patch3", "gameresources_patch2", "gameresources_patch1", "gameresources",
              "game/sp/e1m1_intro/e1m1_intro"):
        names = res.image_index(a)
        hits = [n for n in names.get(art_path, []) if "/rt/" not in n]
        if hits:
            best = min(hits, key=len)
            raw = res.raw_from(a, [best])
            return raw.get(best)
    return best


def convert(src_png, header):
    """SAMUEL's PNG -> the image Vega would have written."""
    import struct
    img = Image.open(src_png)
    if header is None:
        return img
    fmt = header[41]
    bias, scale = struct.unpack_from("<2f", header, 32)
    if fmt == 0x18:
        return img.convert("RGBA").getchannel("R")
    if fmt == 0x19:
        # BC5 normal map: rebuild Z = sqrt(1 - x^2 - y^2)
        a = np.asarray(img.convert("RGBA")).astype(np.float64)
        x = a[..., 0] / 255.0 * 2.0 - 1.0
        y = a[..., 1] / 255.0 * 2.0 - 1.0
        a[..., 2] = (np.sqrt(np.clip(1.0 - x * x - y * y, 0.0, 1.0)) * 0.5 + 0.5) * 255.0
        return Image.fromarray(np.clip(np.round(a), 0, 255).astype(np.uint8), "RGBA")
    if abs(scale - 1.0) > 1e-6 or abs(bias) > 1e-9:
        a = np.asarray(img.convert("RGBA")).astype(np.float64)
        a[..., :3] = _l2s(_s2l(a[..., :3]) * scale + bias)
        return Image.fromarray(np.clip(np.round(a), 0, 255).astype(np.uint8), "RGBA")
    return img
