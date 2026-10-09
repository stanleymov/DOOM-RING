"""DOOM Eternal HUD textures (weapon wheel, ability / rune icons) from the user's install into
dist/natives/doom_ui with clean names: exported with samuel-cli into <work>/ui (gameresources,
textures/swf_images/...), then copied; the Blood Punch icons from <work>/ui_fist.

    python tools/convert_ui.py
"""
import re
import shutil
import subprocess
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

SRC = paths.WORK / "ui" / "textures" / "swf_images"
OUT = paths.NATIVES / "doom_ui"
RES = paths.DOOM_BASE / "gameresources.resources"
# textures/swf_images/... the HUD draws (weapon wheel, its weapon art, ability / rune icons)
PATTERN = re.compile(r"^textures/swf_images/(hud/weapon_wheel|icons/icon_(ability|dash|rune_bloodpunch|sentinel|argent|grenade|slayer_symbol|death))")
WANT = {
    "bfg_selected", "chaingun_selected", "chainsaw_selected", "dbshotgun_selected", "gauss_selected", "har_selected",
    "icon_ability_chainsaw", "icon_ability_change_weapon", "icon_ability_fire_weapon", "icon_ability_flame_belch",
    "icon_ability_use_weapon_mod", "icon_argent_cell", "icon_dash", "icon_death", "icon_grenade_frag",
    "icon_rune_bloodpunch", "icon_sentinel_armor", "icon_slayer_symbol", "plasma_selected", "rocket_selected",
    "shotgun_selected", "unmakyr_selected", "weaponwheel_backer", "weaponwheel_backer_center", "weaponwheel_selector",
    "weaponwheel_wedge_8pc", "weaponwheel_wedge_8pc_decor", "weaponwheel_wedge_8pc_glow_fullammo",
    "weaponwheel_wedge_8pc_glow_noammo", "weaponwheel_wedge_8pc_glow_selected", "weaponwheel_wedge_8pc_glow_selectedpulse",
    "weaponwheel_wedge_8pc_lightbottom_inactive", "weaponwheel_wedge_8pc_lightbottom_off",
    "weaponwheel_wedge_8pc_lightbottom_on", "weaponwheel_wedge_8pc_lighttop_off", "weaponwheel_wedge_8pc_lighttop_on",
    "weaponwheel_wedge_8pc_stroke",
}
FIST = ["art/ui/dossier/icons/rune/icon_rune_base_punch.tga$bc3$streamed",
        "art/ui/dossier/icons/rune/icon_rune_max_punch.tga$bc3$streamed"]


def export():
    listing = subprocess.run([str(paths.SAMUEL), str(RES), "list", "swf_images"], capture_output=True, text=True,
                             creationflags=paths.NO_WINDOW).stdout
    names = sorted({ln.split("\t")[0] for ln in listing.splitlines() if PATTERN.match(ln.split("\t")[0])})
    (paths.WORK / "ui").mkdir(parents=True, exist_ok=True)
    for i in range(0, len(names), 40):
        subprocess.run([str(paths.SAMUEL), str(RES), "export", str(paths.WORK / "ui"), *names[i:i + 40]],
                       capture_output=True, creationflags=paths.NO_WINDOW)
    (paths.WORK / "ui_fist").mkdir(parents=True, exist_ok=True)
    subprocess.run([str(paths.SAMUEL), str(RES), "export", str(paths.WORK / "ui_fist"), *FIST],
                   capture_output=True, creationflags=paths.NO_WINDOW)


def main():
    if not any((paths.WORK / "ui").rglob("*.png")):
        export()
    OUT.mkdir(parents=True, exist_ok=True)
    n = 0
    for f in (paths.WORK / "ui").rglob("*.png"):
        name = f.name.split("$")[0].replace(".tga", "").replace(".png", "")
        if name not in WANT:
            continue
        shutil.copy(f, OUT / f"{name}.png")
        n += 1
    print(f"{n} UI textures -> {OUT}")
    # Blood Punch HUD icons: empty = base punch rune, charged = max punch rune. Made white
    # (brightness -> alpha) so the HUD tints them with its own colours.
    import numpy as np
    from PIL import Image
    for src_name, out_name in [("icon_rune_base_punch", "icon_punch"), ("icon_rune_max_punch", "icon_punch_charged")]:
        src = next((paths.WORK / "ui_fist").rglob(f"{src_name}*.png"), None)
        if not src:
            print("!! missing", src_name)
            continue
        a = np.asarray(Image.open(src).convert("RGBA")).astype(np.float32) / 255.0
        lum = a[..., :3] @ np.array([0.299, 0.587, 0.114], np.float32)
        out = np.zeros_like(a)
        out[..., :3] = 1.0
        # keep the dark inner details readable: alpha follows brightness, scaled by the source alpha
        out[..., 3] = a[..., 3] * np.clip(0.15 + lum * 1.1, 0.0, 1.0)
        Image.fromarray((out * 255).astype(np.uint8)).save(OUT / f"{out_name}.png")
        print(f"{out_name}.png")


if __name__ == "__main__":
    main()
