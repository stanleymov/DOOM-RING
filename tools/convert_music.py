"""DOOM Eternal combat music (from the user's own install) for doomslayer.dll, played by Doom's own rules.

Each kept suite's heavy (full combat) playlist from the Wwise bank (tools/doom_music_bank.py ->
~/doom-extract/mus_hirc.pkl): every piece rendered on its own timeline (its clips placed, 48 kHz) to
doom_music/<suite>/<piece>.ogg, and suite.json = the playlist tree, each piece's ENTRY / EXIT cues and
the suite's level match. The mod walks the tree and joins pieces cue to cue (audio.rs, docs/doom_music_system.md).

    python tools/convert_music.py            (suites + jukebox tracks)
    python tools/convert_music.py --tracks   (jukebox tracks only)
    python tools/convert_music.py --levels   (the suites' level match only)
    python tools/convert_music.py --remove   (take REMOVE out of the exported suites only)
"""

import json
import re
import shutil
import subprocess
import sys
import wave
from pathlib import Path

import numpy as np

sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths  # noqa: E402

SRC = paths.WORK / "music" / "music"
OUT = paths.NATIVES / "doom_music"
BACKUP = paths.WORK / "doom_music_backup"
BACKUP_REMOVED = paths.WORK / "doom_music_removed"
RATE = 48000

# suite -> its heavy playlist (picks: user, 2026-10-08; supergorenest suite 2, maykr and intro dropped)
SUITES = {
    "supergorenest_1": 326855636,
    "metal_hell": 734553204,
    "doom_hunter": 1008781720,
    "slayer_city": 62861113,
    "cultist_base": 138416226,
    "hub": 56154643,
    "samuelsbase": 880693864,
    "mars_core_phobos": 536570859,
}

# pieces removed for good, "suite/piece" (user, in-game music test F3, 2026-10-08)
REMOVE = ["doom_hunter/heavy_10", "mars_core_phobos/heavy_5_0", "samuelsbase/heavy_9x"]


# Full-length fight tracks (the Fortress of Doom jukebox, base game): played whole from their drop
# (user: the suites above played as short jingles). Drop points live in doomslayer.toml
# (music_tracks); level-matched by tools/music_gains.py. Out: doom_music_tracks/<name>.ogg
TRACKS = {
    "bfg_division": "hub_music_jukebox_jukebox_doom16_BFG_Division",
    "flesh_and_metal": "hub_music_jukebox_jukebox_doom16_Flesh_Metal",
    "rip_and_tear": "hub_music_jukebox_jukebox_doom16_Rip_Tear",
    "into_sandys_city": "hub_music_jukebox_jukebox_doom2_Into_SandysCity",
    "doom3_theme": "hub_music_jukebox_jukebox_doom3_theme",
    "doom64_intro": "hub_music_jukebox_jukebox_doom64_intro",
    "at_dooms_gate": "hub_music_jukebox_juke_box_doom_at_dooms_gate",
    "descent_into_cerberon": "hub_music_jukebox_jukebox_quake2_descent_into_cerberon",
    "quad_machine": "hub_music_jukebox_jukebox_quake2_quad_machine",
    "quake2_rage": "hub_music_jukebox_jukebox_quake2_rage",
    "quake3_theme": "hub_music_jukebox_jukebox_quake3_theme",
    "goroth": "hub_music_jukebox_jukebox_quake_champions_Goroth",
    "keen_shadows": "hub_music_jukebox_jukebox_keen_shadows",
    "keen_vegetables": "hub_music_jukebox_jukebox_keen_vegetables",
    "wolf_get_them": "hub_music_jukebox_jukebox_wolf_get_them",
    "wolf_wondering": "hub_music_jukebox_jukebox_wolf_wondering",
}
OUT_TRACKS = OUT.parent / "doom_music_tracks"


def tracks():
    OUT_TRACKS.mkdir(parents=True, exist_ok=True)
    for name, stem in TRACKS.items():
        src = next(SRC.glob(f"{stem}_id#*.ogg"), None)
        if src is None:
            print(f"!! {name}: {stem} not found")
            continue
        shutil.copy(src, OUT_TRACKS / f"{name}.ogg")
        print(f"track {name}")


def suites():
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    import doom_music_render as r

    r.RATE = RATE
    if OUT.exists():
        if BACKUP.exists():
            shutil.rmtree(BACKUP)
        shutil.move(OUT, BACKUP)
    for suite, pid in SUITES.items():
        d = OUT / suite
        d.mkdir(parents=True)
        names = {}

        def node(n):
            if n["seg"]:
                if n["seg"] not in names:
                    s = r.segs[n["seg"]]
                    stem = r.tracks[s["tracks"][0]][0][0]
                    name = re.sub(r"^.*_main_", "", stem)
                    while name in names.values():
                        name += "x"
                    names[n["seg"]] = name
                return {"piece": names[n["seg"]]}
            return {"type": n["type"], "loop": n["loop"], "avoid": n["avoid"], "weight": n["weight"],
                    "kids": [node(k) for k in n["kids"]]}

        tree = node(r.trees[pid])
        pieces = {}
        for seg, name in names.items():
            a = np.clip(r.pcm(seg), -1.0, 1.0)
            wav = d / f"{name}.wav"
            with wave.open(str(wav), "wb") as w:
                w.setnchannels(2)
                w.setsampwidth(2)
                w.setframerate(RATE)
                w.writeframes((a * 32767).astype(np.int16).tobytes())
            subprocess.run([paths.FFMPEG, "-nostdin", "-v", "error", "-y", "-i", str(wav), "-c:a", "libvorbis", "-q:a", "6",
                            str(d / f"{name}.ogg")], check=True)
            wav.unlink()
            s = r.segs[seg]
            pieces[name] = {"entry": round(s["markers"][0], 3), "exit": round(s["markers"][1], 3),
                            "length": round(s["dur"], 3)}
        (d / "suite.json").write_text(json.dumps({"playlist": pid, "tree": tree, "pieces": pieces}, indent=1))
        print(f"{suite}: {len(pieces)} pieces")
    drop_removed()
    level_match()


def drop_removed():
    """REMOVE out of the exported suites: the piece's file and its place in the tree (a group left
    empty goes too). The files go to BACKUP_REMOVED."""
    def prune(n, gone):
        if "piece" in n:
            return None if n["piece"] in gone else n
        n["kids"] = [k for k in (prune(k, gone) for k in n["kids"]) if k]
        return n if n["kids"] else None

    for suite in SUITES:
        gone = {e.split("/", 1)[1] for e in REMOVE if e.split("/", 1)[0] == suite}
        js = OUT / suite / "suite.json"
        if not gone or not js.exists():
            continue
        data = json.loads(js.read_text())
        data["tree"] = prune(data["tree"], gone)
        for name in gone:
            data["pieces"].pop(name, None)
            f = OUT / suite / f"{name}.ogg"
            if f.exists():
                (BACKUP_REMOVED / suite).mkdir(parents=True, exist_ok=True)
                shutil.move(f, BACKUP_REMOVED / suite / f.name)
        js.write_text(json.dumps(data, indent=1))
        print(f"{suite}: removed {sorted(gone)}")


def level_match():
    """Each suite as a whole to the median loudness of all pieces (its own dynamics kept)."""
    sys.path.insert(0, str(Path(__file__).resolve().parent))
    from music_gains import lufs

    levels = {}
    for suite in SUITES:
        for f in sorted((OUT / suite).glob("*.ogg")):
            lv = lufs(f)
            if np.isfinite(lv):  # (a silent piece has no loudness)
                levels[(suite, f.stem)] = lv
    target = float(np.median(list(levels.values())))
    for suite in SUITES:
        js = OUT / suite / "suite.json"
        data = json.loads(js.read_text())
        med = float(np.median([lv for (s, _), lv in levels.items() if s == suite]))
        data["gain_db"] = round(max(-6.0, min(6.0, target - med)), 2)
        js.write_text(json.dumps(data, indent=1))
        print(f"{suite}: {med:.1f} LUFS, {data['gain_db']:+.1f} dB")
    print(f"target {target:.1f} LUFS -> {OUT}")


if __name__ == "__main__":
    import sys
    if "--tracks" in sys.argv:
        tracks()
    elif "--suites" in sys.argv:
        # (the installer: Doom's suites only, not the jukebox tracks of the old track mode)
        suites()
    elif "--levels" in sys.argv:
        level_match()
    elif "--remove" in sys.argv:
        drop_removed()
    else:
        suites()
        tracks()
