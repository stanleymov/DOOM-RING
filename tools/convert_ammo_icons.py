"""Doom's own ammo icons for the HUD, from the user's Doom install.

Exports art/ui/icons/ammo/ico_*.tga from base/gameresources.resources with samuel-cli and writes
dist/natives/doom_ui/ammo_<type>.png (white glyph on alpha, tinted by the HUD).

    python tools/convert_ammo_icons.py
"""
import subprocess
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
OUT = paths.NATIVES / "doom_ui"
SAMUEL = paths.SAMUEL
RES = paths.DOOM_BASE / "gameresources.resources"
TMP = paths.WORK / "ammoicons"

# HUD name -> Doom texture
ICONS = {
    "ammo_shells": "ico_shells",
    "ammo_bullets": "ico_bullets",
    "ammo_cells": "ico_cell",
    "ammo_rockets": "ico_rocket",
    "ammo_bfg": "ico_bfg",
    "ammo_fuel": "ico_fuel",
    "ammo_crucible": "ico_crucible",
}


# Weapon mod icons for the weapon wheel / HUD mod slot: HUD name -> Doom texture
MODS = {
    "mod_sticky": "weapon_shotgun/mod2/icon_mod_stickybomb.png",
    "mod_bolt": "weapon_har/mod1/icon_mod_boltaction.png",
    "mod_heatblast": "weapon_plasma/mod1/icon_mod_heatblast.png",
    "mod_remote": "weapon_rocket/mod1/icon_mod_remotedetonate.png",
    "mod_meathook": "weapon_supershotgun/mod2/icon_mod_meathook.png",
    "mod_arbalest": "weapon_gauss/mod1/icon_mod_arbalest.png",
    "mod_turret": "weapon_chaingun/mod2/icon_mod_mobileturret.png",
}


def glyph(src, dst):
    im = Image.open(src).convert("RGBA")
    r, g, b, a = im.split()
    lum = Image.merge("RGB", (r, g, b)).convert("L")
    out = Image.new("RGBA", im.size, (255, 255, 255, 0))
    out.putalpha(Image.composite(lum, Image.new("L", im.size, 0), a))
    out.save(dst)
    return im.size


def _exported(rel):
    """An exported image (SAMUEL writes it with or without the archive name as a top folder)."""
    name = Path(rel).name
    return next(p for p in TMP.rglob(name) if p.as_posix().endswith(rel))


def mods():
    base = "textures/guis/icons/perks/weapons"
    names = [f"{base}/{t}$bc3$streamed" for t in MODS.values()]
    for t in MODS.values():
        (TMP / "gameresources" / base / t).parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(SAMUEL), str(RES), "export", str(TMP), *names], check=True, capture_output=True)
    for hud, t in MODS.items():
        # (samuel-cli writes these with or without the "gameresources" prefix)
        src = _exported(f"{base}/{t}$bc3$streamed.png")
        size = glyph(src, OUT / f"{hud}.png")
        print(hud, "<-", t, size)


# Health / armor for the bottom-left HUD: HUD name -> Doom texture
VITALS = {
    "hud_health": "art/ui/icons/argent_cells_upgrade/argent_cell_health.tga",
    "hud_armor": "art/ui/icons/argent_cells_upgrade/argent_cell_armor.tga",
}


def vitals():
    names = [f"{t}$bc3$streamed" for t in VITALS.values()]
    for t in VITALS.values():
        (TMP / "gameresources" / t).parent.mkdir(parents=True, exist_ok=True)
        (TMP / t).parent.mkdir(parents=True, exist_ok=True)
    subprocess.run([str(SAMUEL), str(RES), "export", str(TMP), *names], check=True, capture_output=True)
    for hud, t in VITALS.items():
        src = _exported(f"{t}$bc3$streamed.png")
        print(hud, "<-", t, glyph(src, OUT / f"{hud}.png"))


def main():
    vitals()
    mods()
    names = [f"art/ui/icons/ammo/{t}.tga$bc3$streamed" for t in ICONS.values()]
    (TMP / "gameresources" / "art/ui/icons/ammo").mkdir(parents=True, exist_ok=True)
    subprocess.run([str(SAMUEL), str(RES), "export", str(TMP), *names], check=True, capture_output=True)
    for hud, tex in ICONS.items():
        src = _exported(f"art/ui/icons/ammo/{tex}.tga$bc3$streamed.png")
        im = Image.open(src).convert("RGBA")
        # white glyph, alpha = luminance x alpha (the HUD tints it per ammo type)
        r, g, b, a = im.split()
        lum = Image.merge("RGB", (r, g, b)).convert("L")
        alpha = Image.eval(Image.merge("LA", (lum, a)).split()[0], lambda v: v)
        out = Image.new("RGBA", im.size, (255, 255, 255, 0))
        out.putalpha(Image.composite(alpha, Image.new("L", im.size, 0), a))
        out.save(OUT / f"{hud}.png")
        print(hud, "<-", tex, im.size)


if __name__ == "__main__":
    main()
