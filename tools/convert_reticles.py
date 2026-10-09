"""Copy DOOM Eternal's own weapon reticle textures from the user's install into dist/natives/doom_ui.

The reticle images live in the per-level resources (not gameresources): exported once with
samuel-cli from base/game/sp/e1m1_intro/e1m1_intro.resources into ~/doom-extract/reticles,
then copied here with clean names (ret_*.png). White pieces on transparency - the HUD tints and
places them (the layout itself is in Doom's compiled hud_reticle.swf, so it is matched to Doom
footage in doomhud.rs). Nothing from the game is committed.

    python tools/convert_reticles.py
"""

import shutil
import subprocess
from pathlib import Path

import paths  # noqa: E402  (tools/paths.py: DOOMRING_* locations)

DOOM = paths.DOOM_BASE
SAMUEL = paths.SAMUEL
SRC = paths.WORK / "reticles"
OUT = paths.NATIVES / "doom_ui"


def export():
    res = DOOM / "game/sp/e1m1_intro/e1m1_intro.resources"
    listing = subprocess.run([str(SAMUEL), str(res), "list", "reticle"], capture_output=True, text=True).stdout
    names = [l.split("\t")[0] for l in listing.splitlines()
             if l.startswith(("textures/swf_images/reticles/", "textures/guis/reticle/"))]
    subprocess.run([str(SAMUEL), str(res), "export", str(SRC), *names], check=True, capture_output=True)


if not any(SRC.rglob("*.png")):
    export()
OUT.mkdir(parents=True, exist_ok=True)
n = 0
for f in SRC.rglob("*.png"):
    name = f.name.split("$")[0].replace(".png", "")
    shutil.copy(f, OUT / f"{name}.png")
    n += 1
print(f"{n} reticle textures -> {OUT}")
