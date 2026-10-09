"""Doom Eternal's own HUD textures (the pieces its Flash HUD is built from), from the user's install.

The HUD textures live in each level's resources (not gameresources): exported with samuel-cli from
base/game/sp/e1m1_intro/e1m1_intro.resources and written to dist/natives/doom_ui/dh_<name>.png
(name without the "hud_slayer_" prefix). Nothing from the game is committed.

    python tools/convert_hud_textures.py
"""
import shutil
import subprocess
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

ROOT = Path(__file__).resolve().parents[1]
OUT = paths.NATIVES / "doom_ui"
SAMUEL = paths.SAMUEL
RES = paths.DOOM_BASE / "game/sp/e1m1_intro/e1m1_intro.resources"
TMP = paths.WORK / "hudtex"

PIECES = [
    # health / armor rows
    "hud/health_bars/hud_slayer_health_container_leftside_shell",
    "hud/health_bars/hud_slayer_health_container_leftside_fill",
    "hud/health_bars/hud_slayer_health_container_segment_shell",
    "hud/health_bars/hud_slayer_health_container_segment_fill",
    "hud/health_bars/hud_slayer_health_container_right_shell",
    "hud/health_bars/hud_slayer_health_container_right_fill",
    "hud/health_bars/hud_slayer_armor_container_left_shell",
    "hud/health_bars/hud_slayer_armor_container_left_fill",
    "hud/health_bars/hud_slayer_armor_container_center_shell",
    "hud/health_bars/hud_slayer_armor_container_center_fill",
    "hud/health_bars/hud_slayer_armor_container_right_shell",
    "hud/health_bars/hud_slayer_armor_container_right_fill",
    "hud/health_bars/hud_slayer_health_pip_full",
    "hud/health_bars/hud_slayer_health_pip_empty",
    "hud/health_bars/hud_slayer_health_pip_gradient",
    "hud/health_bars/hud_slayer_health_stroke_glow",
    "hud/health_bars/hud_slayer_health_fadednumbers",
    "hud/health_bars/hud_slayer_health_decor_a",
    "hud/health_bars/hud_slayer_health_decor_b",
    # ammo / equipment
    "hud/ammo_equipment/equipment_new/hud_slayer_ammo_container_new",
    "hud/ammo_equipment/equipment_new/hud_slayer_ammo_container_new_nomod",
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_backer_new",
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_backer_flash",
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_fill_1pip_backer",
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_fill_1pip_fill",
    # the Crucible's box: Doom's 3-bar version (a bar per charge)
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_fill_3pip_backer",
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_fill_3pip_fill",
    "hud/ammo_equipment/equipment_new/hud_slayer_equipment_fill_3pip_leftline",
    # Blood Punch rune hex (buffs)
    "hud/buffs/hud_slayer_rune_container_pulserim",
    "hud/buffs/hud_slayer_rune_container_pulsehex",
    "hud/buffs/hud_slayer_rune_container_numbered",
    "hud/buffs/hud_slayer_charge_glow_new",
    # icons the HUD itself uses
    "icons/icon_health_new",
    "icons/icon_health_new_glow",
    "icons/icon_armor",
    "icons/icon_armor_glow",
    "icons/icon_ammo_fuel",
    "icons/hud_slayer_radialfill",
    "icons/hud_slayer_radialfill_glow",
    "icons/hud_slayer_radialfill_halves",
    "icons/hud_slayer_radialfill_halves_backer",
    "icons/hud_slayer_radialfill_halves_glow",
    # "[E] USE" interact prompt frame (call to action)
    "hud/dialogue_containers/hud_slayer_cta_decor",
    "hud/dialogue_containers/hud_slayer_speaker_decor",
]


# Settings window (src/settings_ui.rs): Doom's own settings-menu pieces, from gameresources
# (+ patches) -> doom_ui/st_<name>.png
GR = RES.parents[3]
GR_RES = [GR / f"{n}.resources" for n in ("gameresources", "gameresources_patch1", "gameresources_patch2", "gameresources_patch3")]
SETTINGS_PIECES = [
    "settings/buttons/button_default_list_base",
    "settings/buttons/button_default_list_down_stroke",
    "settings/keybinding_bg_bar",
    "common/frame/frame_backplate",
    "common/frame/frame_header_2500w",
    "common/frame/frame_accent_2500w",
    "common/frame/deco_accent",
    "common/graphics/active_item_checkmark",
    "common/graphics/arrows",
    "common/buttons/root/root_norm_base",
    "common/buttons/root/root_norm_stroke",
    # The Crucible (doomhud crucible reticle + its equipment box): Doom's own pieces
    "reticles/crucible/ret_center_circle_4pins",
    "reticles/crucible/ret_center_circle_4pins_glow",
    "reticles/crucible/ret_meter_border",
    "reticles/crucible/ret_meter_border_glow",
    "reticles/crucible/ret_meter_pips",
    "reticles/crucible/ret_meter_pips_glow",
    "reticles/crucible/ret_meter_pips_glare_add",
    "icons/icon_ammo_crucible",
    "icons/icon_weapon_sword",
]


# The Crucible reticle + its equipment icon live in a level's resources (not gameresources or
# e1m1): Mission 9 Taras Nabad (sp/e3m1_slayer), where the Crucible is found. Base game only -
# never DLC files (not every player owns the DLC - user).
CRUCIBLE_RES = RES.parents[1] / "e3m1_slayer" / "e3m1_slayer.resources"


# Non-swf art the HUD uses (gameresources, base game): (resource name, doom_ui file name)
ART_PIECES = [
    # the Crucible's shard: the ammo row while the Crucible is out (user)
    ("art/ui/icons/ammo/ico_crucible.tga$bc3$streamed", "st_ico_crucible"),
    # the Crucible's equipment box icon (shown tilted 45 deg left - user)
    ("art/ui/weapons/icon_crucible.tga$bc3$streamed", "st_icon_crucible"),
]


def art_pieces():
    tmp = paths.WORK / "crucible_tex"
    for res_name, out in ART_PIECES:
        for res in GR_RES:
            subprocess.run([str(SAMUEL), str(res), "export", str(tmp), res_name], capture_output=True, stdin=subprocess.DEVNULL)
        src = next(tmp.rglob(Path(res_name).name + ".png"), None)
        if src is None:
            print("!! missing", res_name)
            continue
        shutil.copy(src, OUT / f"{out}.png")
        print(out)


def settings_pieces():
    tmp = paths.WORK / "settings_ui"
    tmp.mkdir(parents=True, exist_ok=True)
    for p in SETTINGS_PIECES:
        name = f"textures/swf_images/{p}.png$bc7$streamed$mtlkind=ui"
        for res in GR_RES + [CRUCIBLE_RES]:
            subprocess.run([str(SAMUEL), str(res), "export", str(tmp), name], capture_output=True, stdin=subprocess.DEVNULL)
        src = next(tmp.rglob(f"{Path(p).name}.png$*.png"), None)
        if src is None:
            print("!! missing", p)
            continue
        shutil.copy(src, OUT / f"st_{Path(p).name}.png")
        print(f"st_{Path(p).name}")


def main():
    TMP.mkdir(parents=True, exist_ok=True)
    for p in PIECES:
        name = f"textures/swf_images/{p}.png$bc7$streamed$mtlkind=ui"
        (TMP / "e1m1_intro" / name).parent.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(SAMUEL), str(RES), "export", str(TMP), name], check=True, capture_output=True, stdin=subprocess.DEVNULL)
        src = next((q for q in (TMP / "e1m1_intro" / f"{name}.png", TMP / f"{name}.png") if q.exists()), None)
        short = Path(p).name.replace("hud_slayer_", "")
        if src is None:
            print("!! missing", p)
            continue
        shutil.copy(src, OUT / f"dh_{short}.png")
        print(f"dh_{short}")


# Controller button icons (gamepad.rs): Doom's own Xbox One and PS4 sets from gameresources
# (textures/guis/controller/<set>/png/*) -> doom_ui/pad_xb_<name>.png / pad_ps_<name>.png, 64 px
# (drawn at ~24 px: a 256 px source shimmered without mips).
PAD_SETS = (("xbone", "xb"), ("ps4", "ps"))


def pad_icons():
    from PIL import Image
    tmp = paths.WORK / "controller_icons"
    lst_file = paths.WORK / "gameresources_list.txt"
    if not lst_file.exists():
        lst_file.parent.mkdir(parents=True, exist_ok=True)
        r = subprocess.run([str(SAMUEL), str(paths.DOOM_BASE / "gameresources.resources"), "list"], capture_output=True, text=True,
                           creationflags=paths.NO_WINDOW)
        lst_file.write_text(r.stdout, encoding="utf-8")
    lst = lst_file.read_text(encoding="utf-8", errors="ignore")
    for folder, short in PAD_SETS:
        names = sorted({l.split("	")[0].strip() for l in lst.splitlines() if l.startswith(f"textures/guis/controller/{folder}/png/")})
        for name in names:
            for res in GR_RES:
                r = subprocess.run([str(SAMUEL), str(res), "export", str(tmp), name], capture_output=True, stdin=subprocess.DEVNULL, text=True)
                if "exporting 0" not in r.stdout:
                    break
            # (both sets use the same file names: take this set's own folder - a bare name match
            # gave the Xbox set the PS4 pictures, user)
            src = next((q for q in tmp.rglob(Path(name).name + ".png") if f"/{folder}/" in q.as_posix()), None)
            if src is None:
                print("!! missing", name)
                continue
            base = Path(name).name.split(".png")[0]
            Image.open(src).convert("RGBA").resize((64, 64), Image.LANCZOS).save(OUT / f"pad_{short}_{base}.png")
        print(f"pad_{short}: {len(names)} icons")


if __name__ == "__main__":
    import sys
    if "--pad" in sys.argv:
        pad_icons()
    elif "--settings" in sys.argv:
        settings_pieces()
        art_pieces()
    else:
        main()
        settings_pieces()
        art_pieces()
