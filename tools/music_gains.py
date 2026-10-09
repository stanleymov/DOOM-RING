"""Level-match the combat music: every segment to the median loudness of them all.

Doom's suites differ by ~7 dB between segments (cultist_base / supergorenest suite 2 quiet,
metal_hell loud - user: "some of it is quiet"). Measures each segment's integrated loudness (ITU-R
BS.1770 K-weighting, gated) and writes doom_music/<suite>/gains.json = {"0.ogg": dB, ...}; the mod
applies the gain while playing (audio.rs). The music files themselves are not touched.

    python tools/music_gains.py
"""
import json
import math
import subprocess
from pathlib import Path

import numpy as np
from scipy.signal import lfilter

import sys  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parent))
import paths  # noqa: E402

ROOT = paths.NATIVES / "doom_music"
MAX_DB = 6.0


def kweight(x, fs):
    f0, g, q = 1681.974450955533, 3.999843853973347, 0.7071752369554196
    k = math.tan(math.pi * f0 / fs)
    vh, vb = 10 ** (g / 20), (10 ** (g / 20)) ** 0.4996667741545416
    a0 = 1 + k / q + k * k
    y = lfilter([(vh + vb * k / q + k * k) / a0, 2 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0],
                [1, 2 * (k * k - 1) / a0, (1 - k / q + k * k) / a0], x, axis=0)
    f0, q = 38.13547087602444, 0.5003270373238773
    k = math.tan(math.pi * f0 / fs)
    a0 = 1 + k / q + k * k
    return lfilter([1, -2, 1], [1, 2 * (k * k - 1) / a0, (1 - k / q + k * k) / a0], y, axis=0)


def lufs(path):
    raw = subprocess.run([paths.FFMPEG, "-v", "error", "-i", str(path), "-f", "f32le", "-ac", "2", "-ar", "48000", "-"],
                         capture_output=True).stdout
    d = np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).astype(np.float64)
    y = kweight(d, 48000)
    blk, hop = int(48000 * 0.4), int(48000 * 0.1)
    ms = np.array([(y[i:i + blk] ** 2).mean(axis=0).sum() for i in range(0, len(y) - blk, hop)])
    lv = -0.691 + 10 * np.log10(ms + 1e-12)
    g = ms[lv > -70]
    rel = -0.691 + 10 * np.log10(g.mean()) - 10
    return -0.691 + 10 * np.log10(g[(-0.691 + 10 * np.log10(g)) > rel].mean())


def main():
    levels = {}
    for suite in sorted(p for p in ROOT.iterdir() if p.is_dir()):
        for f in sorted(suite.glob("*.ogg")):
            levels[(suite, f.name)] = lufs(f)
    target = float(np.median(list(levels.values())))
    for suite in sorted({s for s, _ in levels}):
        gains = {n: round(max(-MAX_DB, min(MAX_DB, target - l)), 2) for (s, n), l in levels.items() if s == suite}
        (suite / "gains.json").write_text(json.dumps(gains, indent=1))
        print(f"{suite.name}: {min(gains.values()):+.1f} .. {max(gains.values()):+.1f} dB")
    print(f"target {target:.1f} LUFS")
    # the full tracks (doom_music_tracks/*.ogg) to the same target
    tdir = ROOT.parent / "doom_music_tracks"
    if tdir.exists():
        gains = {f.name: round(max(-MAX_DB, min(MAX_DB, target - lufs(f))), 2) for f in sorted(tdir.glob("*.ogg"))}
        (tdir / "gains.json").write_text(json.dumps(gains, indent=1))
        print("tracks:", gains)


if __name__ == "__main__":
    main()
