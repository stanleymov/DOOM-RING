"""Edge padding for the baked viewmodel textures (dist/natives/doom_vm/<weapon>/*.png).

Doom's texture atlases have black space between the painted UV islands. When the GPU samples a
smaller mip of a texture, that black bleeds into the islands' edges and shows on the gun as thin
dark outlines along every seam. This fills the unused space around each island with the island's
own edge colour (nearest used pixel), for every map of a material (albedo, normal, spec, gloss,
emissive), using the gun's own UV layout as the mask - painted black details inside an island are
never touched.

Undo: the originals are copied to texture_originals/<weapon>/ (repo root) the first time;
    python tools/vm/dilate_textures.py --restore [weapons...]
puts them back. Run:
    python tools/vm/dilate_textures.py [weapons...]      (default: every weapon folder)
"""

import os
import shutil
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw
from scipy import ndimage

sys.path.insert(0, str(Path(__file__).resolve().parent))
import preview  # noqa: E402

ROOT = Path(os.environ.get("DOOMRING_VM_OUT", Path(__file__).resolve().parents[2] / "dist" / "natives" / "doom_vm"))
# outside dist (not shipped); the installer keeps them in its work folder
ORIG = Path(os.environ.get("DOOMRING_TEX_ORIG", Path(__file__).resolve().parents[2] / "texture_originals"))
SUFFIXES = ("", "_n", "_s", "_g", "_e", "_pm")
GROW = 2  # keep a couple of pixels of the island's own border as "used"


def uv_mask(tris_uv, w, h):
    """Pixels covered by any UV triangle (rasterised at texture resolution)."""
    img = Image.new("L", (w, h), 0)
    d = ImageDraw.Draw(img)
    for t in tris_uv:
        pts = [(float(u) * w, float(v) * h) for u, v in t]
        d.polygon(pts, fill=255, outline=255)
    m = np.asarray(img) > 0
    return ndimage.binary_dilation(m, iterations=GROW)


def dilate(arr, used):
    """Fill every unused pixel with the value of its nearest used pixel."""
    if used.all() or not used.any():
        return arr
    _, (iy, ix) = ndimage.distance_transform_edt(~used, return_indices=True)
    return arr[iy, ix]


def process(weapon, restore=False):
    folder = ROOT / weapon
    if not (folder / "model.bin").exists():
        return
    backup = ORIG / weapon
    if restore:
        if backup.exists():
            for f in backup.glob("*.png"):
                shutil.copy(f, folder / f.name)
            print(f"{weapon}: originals restored")
        return
    meshes, _ = preview.load(weapon)
    by_mat = {}
    for part, mat, v, faces in meshes:
        by_mat.setdefault(mat, []).append(v["uv"][faces] % 1.0)
    backup.mkdir(parents=True, exist_ok=True)
    for mat, uvs in by_mat.items():
        tris = np.concatenate(uvs)
        files = [folder / f"{mat}{s}.png" for s in SUFFIXES if (folder / f"{mat}{s}.png").exists()]
        if not files:
            continue
        masks = {}
        for f in files:
            orig = backup / f.name
            if not orig.exists():
                shutil.copy(f, orig)
            im = Image.open(orig)
            mode = im.mode
            arr = np.asarray(im)
            h, w = arr.shape[:2]
            if (w, h) not in masks:
                masks[(w, h)] = uv_mask(tris, w, h)
            used = masks[(w, h)]
            out = dilate(arr, used)
            Image.fromarray(out, mode).save(f)
        cover = next(iter(masks.values())).mean() * 100
        print(f"{weapon}: {mat} ({len(files)} maps, islands cover {cover:.0f}%)")


if __name__ == "__main__":
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    restore = "--restore" in sys.argv
    weapons = args or sorted(p.name for p in ROOT.iterdir() if p.is_dir() and not p.name.startswith("_"))
    for w in weapons:
        process(w, restore)
