"""Move the tuned HUD between the live config and the build.

The HUD's tuning lives in two places:
  - doomslayer/src/hud_baked.toml: built into the DLL (the finished HUD; a typo in the live
    config can't touch it, and nothing is read from disk for it)
  - dist/natives/doomslayer.toml: the live config; anything here overrides the baked values and
    reloads about a second after saving

    python tools/hud_bake.py live   # copy the baked HUD into the live config to tune it (no rebuild)
    python tools/hud_bake.py bake   # move the live HUD values into hud_baked.toml, then rebuild

HUD values = the top-level hud_style / hud_left / hud_right / hud_pips / hud_pip_count lines and
the whole [hud] table (kept last in the live config).
"""
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIVE = ROOT / "dist" / "natives" / "doomslayer.toml"
BAKED = ROOT / "doomslayer" / "src" / "hud_baked.toml"
TOP_KEYS = ("hud_style", "hud_left", "hud_right", "hud_pips", "hud_pip_count")
TOP = re.compile(rf"^({'|'.join(TOP_KEYS)})\s*=")
HEADER = "# Baked HUD tuning, built into the DLL (tools/hud_bake.py). The live doomslayer.toml overrides any of it.\n"
MARK = "# HUD tuning is baked into the build (doomslayer/src/hud_baked.toml); `python tools/hud_bake.py live` brings it back here.\n"


def split(text):
    """(top-level HUD lines, [hud] table lines, everything else)"""
    lines = text.splitlines(keepends=True)
    top, hud, rest = [], [], []
    table = None
    for ln in lines:
        m = re.match(r"^\[([^\]]+)\]", ln)
        if m:
            table = m.group(1).strip()
        if table == "hud":
            hud.append(ln)
        elif table is None and TOP.match(ln):
            top.append(ln)
        else:
            rest.append(ln)
    return top, hud, rest


def key_of(ln):
    m = re.match(r"^\s*([A-Za-z0-9_]+)\s*=", ln)
    return m.group(1) if m else None


def merge(base, new):
    """`base` lines with each key from `new` replaced in place, or appended. Baking only part of
    the HUD (e.g. just the settings window) used to replace the whole baked file and drop the
    rest of the tuning."""
    out = [ln if ln.endswith("\n") else ln + "\n" for ln in base]
    for ln in new:
        k = key_of(ln)
        if k is None:
            continue
        ln = ln if ln.endswith("\n") else ln + "\n"
        idx = next((i for i, b in enumerate(out) if key_of(b) == k), None)
        if idx is None:
            out.append(ln)
        else:
            out[idx] = ln
    return out


def bake():
    top, hud, rest = split(LIVE.read_text(encoding="utf-8"))
    if not top and not hud:
        sys.exit("nothing to bake: no HUD values in the live config")
    btop, bhud, _ = split(BAKED.read_text(encoding="utf-8"))
    btop = merge(btop, top)
    bhud = merge(bhud or ["[hud]\n"], hud[1:])
    BAKED.write_text(HEADER + "".join(btop) + "\n" + "".join(bhud).rstrip() + "\n", encoding="utf-8")
    rest = [ln for ln in rest if ln != MARK]
    # leave a pointer where the top-level HUD lines were
    out = "".join(rest).rstrip() + "\n\n" + MARK
    LIVE.write_text(out, encoding="utf-8")
    print(f"baked {len(top)} top-level + {max(len(hud) - 1, 0)} [hud] lines into {BAKED.name}; rebuild the DLL")


def live():
    btop, bhud, _ = split(BAKED.read_text(encoding="utf-8"))
    top, hud, rest = split(LIVE.read_text(encoding="utf-8"))
    if top or hud:
        sys.exit("the live config already has HUD values; bake or remove them first")
    rest = [ln for ln in rest if ln != MARK]
    # top-level keys must sit before the first [table]
    first = next((i for i, ln in enumerate(rest) if re.match(r"^\[", ln)), len(rest))
    rest[first:first] = btop + ["\n"]
    LIVE.write_text("".join(rest).rstrip() + "\n\n" + "".join(bhud).rstrip() + "\n", encoding="utf-8")
    print(f"HUD values copied into {LIVE.name}: tune live (no rebuild needed); `bake` when done")


if __name__ == "__main__":
    {"bake": bake, "live": live}.get(sys.argv[1] if len(sys.argv) > 1 else "", lambda: sys.exit(__doc__))()
