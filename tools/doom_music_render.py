"""Render a DOOM Eternal combat suite offline the way its Wwise music bank plays it (docs/doom_music_system.md):
walk the playlist tree (continuous sequence / step random, loop counts, avoid repeat), and join the
segments cue to cue - the next segment's ENTRY cue lands on the current one's EXIT cue, the current
one's post-exit tail rings on over it.

Needs ~/doom-extract/mus_hirc.pkl (the parsed bank) and the extracted music in ~/doom-extract/music/music.

    python tools/doom_music_render.py <playlist id> <seconds> <out.wav> [--no-intro] [--seed N]

--no-intro starts at the looping body (a suite's heavy playlist opens with a one-off intro piece).
"""
import os
import pickle
import random
import subprocess
import sys
import wave

import numpy as np

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402

HOME = str(paths.WORK)
RATE = 44100

objs, tracks, segs, ids, trees = pickle.load(open(os.path.join(HOME, "mus_hirc.pkl"), "rb"))
files = {}
for f in os.listdir(os.path.join(HOME, "music", "music")):
    if "_id#" in f and f.endswith(".ogg"):
        files[int(f.split("_id#")[1][:-4])] = os.path.join(HOME, "music", "music", f)


def walk(node, last):
    """Yield segment ids forever (or for the node's loops), like Wwise's playlist."""
    loops = node["loop"]
    n = 0
    while loops == 0 or n < loops:
        n += 1
        if node["seg"]:
            yield node["seg"]
            continue
        kids = node["kids"]
        if node["type"] in ("step random", "step sequence"):
            # one child per pass (step), never the one played last time (avoid repeat)
            if node["type"] == "step sequence":
                k = kids[(n - 1) % len(kids)]
            else:
                choices = [k for k in kids if id(k) not in last.get(id(node), [])] or kids
                k = random.choice(choices)
                last[id(node)] = [id(k)]
            yield from walk(k, last)
        else:
            order = kids if node["type"] == "continuous sequence" else random.sample(kids, len(kids))
            for k in order:
                yield from walk(k, last)


def skip_intro(root):
    """A play-once sequence of [intro, looping body] -> the body."""
    kids = root["kids"]
    if not root["seg"] and root["type"] == "continuous sequence" and root["loop"] == 1 and len(kids) == 2             and kids[1]["loop"] == 0:
        return kids[1]
    return root


_cache = {}


def pcm(seg_id):
    """The segment as audio on its own timeline (ms 0 = segment start): its tracks' clips mixed."""
    if seg_id in _cache:
        return _cache[seg_id]
    s = segs[seg_id]
    out = np.zeros((int(s["dur"] / 1000 * RATE) + 1, 2), np.float32)
    for t in s["tracks"]:
        for name, sid, play_at, btrim, etrim, sdur in tracks[t][:1]:
            if sid not in files:
                continue
            raw = subprocess.run([paths.FFMPEG, "-nostdin", "-v", "error", "-i", files[sid], "-f", "f32le", "-ac", "2",
                                  "-ar", str(RATE), "-"], capture_output=True).stdout
            a = np.frombuffer(raw, dtype=np.float32).reshape(-1, 2)
            # clip window on the segment timeline: [playAt + beginTrim, playAt + srcDur + endTrim]
            f0 = int(btrim / 1000 * RATE)
            f1 = min(len(a), int((sdur + etrim) / 1000 * RATE))
            at = int((play_at + btrim) / 1000 * RATE)
            clip = a[f0:f1]
            if at < 0:
                clip, at = clip[-at:], 0
            n = min(len(clip), len(out) - at)
            if n > 0:
                out[at:at + n] += clip[:n]
    _cache[seg_id] = out
    return out


def main():
    pid, seconds, out = int(sys.argv[1]), float(sys.argv[2]), sys.argv[3]
    if "--seed" in sys.argv:
        random.seed(int(sys.argv[sys.argv.index("--seed") + 1]))
    root = trees[pid]
    if "--no-intro" in sys.argv:
        root = skip_intro(root)
    total = int(seconds * RATE)
    mix = np.zeros((total + RATE * 60, 2), np.float32)
    cue = None  # output time where the next segment's entry cue lands
    for seg in walk(root, {}):
        s = segs[seg]
        entry, exit_ = s["markers"][0] / 1000, s["markers"][1] / 1000
        start_t = 0.0 if cue is None else cue - entry
        a = pcm(seg)
        start = max(0, int(start_t * RATE))
        n = min(len(a), len(mix) - start)
        mix[start:start + n] += a[:n]
        name = tracks[s["tracks"][0]][0][0] if s["tracks"] and tracks[s["tracks"][0]] else "?"
        print(f"{start_t:7.2f}s  {name}")
        cue = start_t + exit_
        if cue > seconds:
            break
    mix = mix[:total]
    pk = np.abs(mix).max()
    if pk > 0.98:
        mix *= 0.98 / pk
    w = wave.open(out, "wb")
    w.setnchannels(2)
    w.setsampwidth(2)
    w.setframerate(RATE)
    w.writeframes((mix * 32767).astype(np.int16).tobytes())
    w.close()


if __name__ == "__main__":
    main()
