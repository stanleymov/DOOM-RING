"""Pull the sounds and music the mod uses out of the player's DOOM Eternal .snd archives (our own
reader, tools/doom_snd.py) into the work folder, laid out like EternalAudioExtractor's output so
convert_audio.py / convert_music.py read them unchanged:

  <work>/sound/<archive>/<name>_id#<id>.opus|wem     (sound effects, Ogg Opus as stored)
  <work>/music/music/<name>_id#<id>.ogg              (music: Wwise Vorbis -> Ogg with ww2ogg + our
                                                      granule fix)

Which ids: tools/data/doom_sound_ids.json (tools/make_sound_tables.py).

    python tools/extract_sounds.py
"""
import json
import os
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import doom_snd  # noqa: E402
import ogg_regranule  # noqa: E402
import paths  # noqa: E402

TABLE = Path(__file__).resolve().parent / "data" / "doom_sound_ids.json"
# ww2ogg (BSD-3, hcs64/ww2ogg) and its codebook file
WW2OGG = Path(os.environ.get("DOOMRING_WW2OGG", Path.home() / "modtools/eae/utils/ww2ogg.exe"))
CODEBOOKS = Path(os.environ.get("DOOMRING_CODEBOOKS", WW2OGG.parent / "packed_codebooks_aoTuV_603.bin"))


def main():
    table = json.loads(TABLE.read_text())
    sb = doom_snd.soundbanks(paths.DOOM_BASE)
    recs = {}

    def rec(snd):
        if snd not in recs:
            f = sb / f"{snd}.snd"
            recs[snd] = doom_snd.records(f) if f.exists() else {}
        return recs[snd]

    n = 0
    for sid, info in table["sfx"].items():
        for snd in info["snd"]:
            r = rec(snd).get(int(sid))
            if r is None:
                print(f"!! {info['name']} ({sid}) not in {snd}.snd")
                continue
            out = paths.WORK / "sound" / snd / f"{info['name']}_id#{sid}{doom_snd.extension(r[2])}"
            out.parent.mkdir(parents=True, exist_ok=True)
            out.write_bytes(doom_snd.read(sb / f"{snd}.snd", r))
            n += 1
    print(f"{n} sound files -> {paths.WORK / 'sound'}")

    music = paths.WORK / "music" / "music"
    music.mkdir(parents=True, exist_ok=True)
    raw = paths.WORK / "music" / "wem"
    raw.mkdir(parents=True, exist_ok=True)
    m = 0
    for sid, name in table["music"].items():
        r = rec("music").get(int(sid))
        if r is None:
            print(f"!! music {name} ({sid}) not in music.snd")
            continue
        wem = raw / f"{sid}.wem"
        wem.write_bytes(doom_snd.read(sb / "music.snd", r))
        ogg = music / f"{name}_id#{sid}.ogg"
        subprocess.run([str(WW2OGG), str(wem), "-o", str(ogg), "--pcb", str(CODEBOOKS)], capture_output=True,
                       creationflags=paths.NO_WINDOW)
        if ogg.exists():
            # ww2ogg leaves wrong granule positions (the piece would end 1024 frames short): fix them
            # like revorb does (ogg_regranule.py - our own, revorb's licence is unclear)
            ogg.write_bytes(ogg_regranule.regranule(ogg.read_bytes()))
            m += 1
        else:
            print(f"!! ww2ogg failed on {name}")
    print(f"{m} music files -> {music}")


if __name__ == "__main__":
    main()
