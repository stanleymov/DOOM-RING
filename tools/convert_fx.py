"""Doom Eternal muzzle-flash textures -> coloured, premultiplied RGBA for the viewmodel renderer.

Input: samuel-cli export of effect/texture/weapon/* from the user's own install (BC5 particle
textures: R = intensity, G = heat; B is a decoder artefact). Output: dist/natives/doom_fx/<ramp>_<name>.png
Nothing from the game is committed.

    python tools/convert_fx.py [export_dir]
"""
import sys
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

import numpy as np
from PIL import Image

SRC = Path(sys.argv[1]) if len(sys.argv) > 1 else paths.WORK / "fx/gameresources/effect/texture/weapon"
OUT = paths.NATIVES / "doom_fx"

TEXTURES = {
    "front": "weap_muzzleflash_front_l$bc5$streamed$mtlkind=particle.png",  # 2x2 flipbook
    "side": "weap_muzzleflash_side_l$bc5$streamed$mtlkind=particle.png",    # 2x2 flipbook, flame along +x
    "star": "fir_muzzleflash_5star_front_02.tga$bc5$streamed$mtlkind=particle.png",
}

# heat 0..1 -> colour (Doom tints these particles with gradients at runtime)
RAMPS = {
    "fire": [(0.0, (0.55, 0.06, 0.0)), (0.45, (1.0, 0.42, 0.05)), (0.8, (1.0, 0.8, 0.35)), (1.0, (1.0, 0.97, 0.85))],
    "blue": [(0.0, (0.05, 0.12, 0.55)), (0.45, (0.25, 0.55, 1.0)), (0.8, (0.65, 0.85, 1.0)), (1.0, (0.95, 0.98, 1.0))],
    "green": [(0.0, (0.05, 0.4, 0.05)), (0.45, (0.3, 0.95, 0.2)), (0.8, (0.75, 1.0, 0.5)), (1.0, (0.97, 1.0, 0.9))],
}


def ramp(stops, t):
    xs = np.array([s[0] for s in stops])
    out = np.zeros(t.shape + (3,), np.float32)
    for c in range(3):
        out[..., c] = np.interp(t, xs, [s[1][c] for s in stops])
    return out


def export():
    """The flash textures from gameresources (effect/texture/weapon/...) into <work>/fx."""
    import subprocess
    names = ["effect/texture/weapon/" + n[:-4] for n in TEXTURES.values()]
    subprocess.run([str(paths.SAMUEL), str(paths.DOOM_BASE / "gameresources.resources"), "export",
                    str(paths.WORK / "fx"), *names], capture_output=True, creationflags=paths.NO_WINDOW)


def _src(name):
    hit = SRC / name
    return hit if hit.exists() else next((paths.WORK / "fx").rglob(name), hit)


def main():
    if not all(_src(n).exists() for n in TEXTURES.values()):
        export()
    OUT.mkdir(parents=True, exist_ok=True)
    for key, name in TEXTURES.items():
        src = _src(name)
        if not src.exists():
            print(f"!! missing {src}")
            continue
        a = np.asarray(Image.open(src).convert("RGBA")).astype(np.float32) / 255.0
        # Some flash textures sit on a non-zero background (the 5-star one): remove it.
        border = np.concatenate([a[0, :, 0], a[-1, :, 0], a[:, 0, 0], a[:, -1, 0]])
        bg = float(np.median(border))
        inten = np.clip((a[..., 0] - bg) / max(1.0 - bg, 1e-3), 0, 1)
        heat = np.clip(a[..., 1] / max(a[..., 1].max(), 1e-3), 0, 1)
        alpha = np.clip(inten, 0, 1) ** 1.2
        for rname, stops in RAMPS.items():
            rgb = ramp(stops, heat) * alpha[..., None]  # premultiplied for additive blending
            img = np.concatenate([rgb, alpha[..., None]], -1)
            Image.fromarray((np.clip(img, 0, 1) * 255).astype(np.uint8), "RGBA").save(OUT / f"{rname}_{key}.png")
        print(f"{key}: {a.shape[1]}x{a.shape[0]}")
    print(f"-> {OUT}")


if __name__ == "__main__":
    main()
