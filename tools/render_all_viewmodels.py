"""Render every Doom weapon sprite: python tools/render_all_viewmodels.py
Models come from the user's own install (samuel-cli export into ~/doom-extract/modelExports)."""
import subprocess
from pathlib import Path

BL = Path.home() / "modtools/blender/blender-5.2.2-windows-x64/blender.exe"
SRC = Path.home() / "doom-extract/modelExports"
OUT = Path(__file__).resolve().parent.parent / "dist/natives/doom_viewmodels"
POSE = "6 -3 -6 0.62 -0.17 -0.19 1".split()

# sprite name (= weapon slot order in weapons.rs) -> (model folder, mesh groups to drop)
WEAPONS = [
    ("combat_shotgun", "shotgun", "poprocket,tripleburst"),
    ("heavy_cannon", "heavy_cannon", "heavy_missiles,heavy_sniper"),
    ("plasma_rifle", "plasmarifle", "plasma_mod"),
    ("rocket_launcher", "rocketlauncher", "rocket_lockon,rocket_mod_2,rocket_remote,rocket_rocket"),
    ("super_shotgun", "double_barrel", "mastery,mod"),
    ("ballista", "gauss_rifle", "destroyer_mod,base_mod", (0, 0, 0, 0, 0, 0.02)),
    ("chaingun", "chaingun", "chaingun_turret,chaingun_round", (0, 0, 0, 0, 0, 0.07)),
    ("bfg", "bfg", "none", (0, 0, 0, 0.03, -0.01, 0.0)),
    ("chainsaw", "chainsaw", "none", (10, -4, 8, -0.05, 0.04, 0.07)),
]

OUT.mkdir(parents=True, exist_ok=True)
for name, folder, skip, *tweak in WEAPONS:
    obj = next((SRC / folder).glob("*.obj"))
    # Optional per-weapon (yaw, pitch, roll, dx, dy, dz) added to the shared pose.
    t = tweak[0] if tweak else (0,) * 6
    pose = [str(float(v) + d) for v, d in zip(POSE[:6], t)] + POSE[6:]
    png = OUT / f"{name}.png"
    r = subprocess.run([str(BL), "-b", "-P", str(Path(__file__).parent / "render_viewmodel.py"), "--",
                        str(obj), str(png), *pose, "--skip", skip], capture_output=True, text=True)
    line = next((l for l in r.stdout.splitlines() if l.startswith("normalise")), "")
    if png.exists():
        # Crop to the gun and record where the crop sits on the 16:9 screen (normalised).
        import json
        from PIL import Image
        im = Image.open(png)
        x0, y0, x1, y1 = im.getbbox()
        W, H = im.size
        im.crop((x0, y0, x1, y1)).save(png)
        meta_path = png.with_suffix(".json")
        meta = json.loads(meta_path.read_text())
        meta["rect"] = [x0 / W, y0 / H, x1 / W, y1 / H]
        meta_path.write_text(json.dumps(meta))
    print(f"{name}: {'ok' if png.exists() else 'FAILED'} {line}")
