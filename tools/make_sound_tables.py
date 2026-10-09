"""Development only: which DOOM Eternal sound / music source ids the mod uses, with their names, so the
installer can pull just those out of the player's .snd archives (tools/extract_sounds.py) without
EternalAudioExtractor. Reads this PC's old full extraction (~/doom-extract/sound, music/music) and
the music bank pickle. Writes tools/data/doom_sound_ids.json (ids and names only - no game data).

    python tools/make_sound_tables.py
"""
import json
import os
import pickle
import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import convert_audio  # noqa: E402
import convert_music  # noqa: E402

HOME = Path.home() / "doom-extract"


def sfx_used():
    files = {}
    for f in (HOME / "sound").rglob("*"):
        m = convert_audio.NAME.match(f.name)
        if m:
            files[f"{m.group(1)}#{m.group(2)}"] = f
    used = set()
    for event, patterns in convert_audio.EVENTS.items():
        if isinstance(patterns, dict) and patterns.get("layers"):
            for p in patterns["layers"]:
                used |= {n for n in files if re.match(f"^{p}$", n.split("#")[0])}
            continue
        if isinstance(patterns, dict):
            patterns = patterns["files"]
        regs = [re.compile(f"^{p}$") for p in patterns]
        picks = sorted(n for n in files if any(r.match(n.split("#")[0]) for r in regs))
        used |= set(picks[:12])
    # every archive each one was extracted from (the converter's rglob lets the last one win)
    out = {}
    for f in (HOME / "sound").rglob("*"):
        m = convert_audio.NAME.match(f.name)
        if m and f"{m.group(1)}#{m.group(2)}" in used:
            out.setdefault(m.group(2), {"name": m.group(1), "snd": []})["snd"].append(f.parent.name)
    return out


def music_used():
    objs, tracks, segs, ids, trees = pickle.load(open(HOME / "mus_hirc.pkl", "rb"))
    need = set()

    def walk(n):
        if n["seg"]:
            for t in segs[n["seg"]]["tracks"]:
                for item in tracks[t]:
                    need.add(item[1])
        for k in n["kids"]:
            walk(k)

    for pid in convert_music.SUITES.values():
        walk(trees[pid])
    return {str(sid): ids[sid] for sid in sorted(need) if sid in ids}


def main():
    data = {"sfx": sfx_used(), "music": music_used()}
    out = Path(__file__).resolve().parent / "data" / "doom_sound_ids.json"
    out.parent.mkdir(exist_ok=True)
    out.write_text(json.dumps(data, indent=1, sort_keys=True))
    print(f"{len(data['sfx'])} sfx, {len(data['music'])} music ids -> {out}")


if __name__ == "__main__":
    main()
