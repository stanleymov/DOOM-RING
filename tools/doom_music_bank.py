"""Parse DOOM Eternal's Wwise music bank (mus.pck, the user's own install) into ~/doom-extract/mus_hirc.pkl:
music tracks (clips), segments (length, ENTRY / EXIT cues) and playlist trees. Layouts:
docs/doom_music_system.md. Needs the music extracted to ~/doom-extract/music/music (EternalAudioExtractor -c)
for the file names.

    python tools/doom_music_bank.py
"""
import collections
import os
import pickle
import re
import struct
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import paths  # noqa: E402

PCK = str(paths.DOOM_BASE / "sound/soundbanks/pc/mus.pck")
HOME = str(paths.WORK)
RST = {0: "continuous sequence", 1: "step sequence", 2: "continuous random", 3: "step random", -1: "piece"}


def hirc(d):
    o = d.find(b"HIRC")
    n = struct.unpack_from("<I", d, o + 8)[0]
    q = o + 12
    objs = collections.defaultdict(dict)
    for _ in range(n):
        t, ln = d[q], struct.unpack_from("<I", d, q + 1)[0]
        body = d[q + 5:q + 5 + ln]
        objs[t][struct.unpack_from("<I", body, 0)[0]] = body
        q += 5 + ln
    return dict(objs)


def music_tracks(objs, ids):
    """track id -> [(file stem, source id, playAt, beginTrim, endTrim, srcDuration)] (ms)."""
    tracks = {}
    for tid, b in objs[11].items():
        q = 9 + 14 * struct.unpack_from("<I", b, 5)[0]
        npl = struct.unpack_from("<I", b, q)[0]
        q += 4
        items = []
        for _ in range(npl):
            _, sid, _, pa, bt, et, sd = struct.unpack_from("<IIIdddd", b, q)
            q += 44
            items.append((ids.get(sid), sid, pa, bt, et, sd))
        tracks[tid] = items
    return tracks


def segments(objs, tracks):
    """segment id -> {dur, markers [entry, exit], tempo, sig, tracks} (read from the record's end)."""
    segs = {}
    for sid, b in objs[10].items():
        for n in range(8, -1, -1):
            st = len(b) - 4 - n * 16
            if st < 24 or struct.unpack_from("<I", b, st)[0] != n:
                continue
            dur = struct.unpack_from("<d", b, st - 8)[0]
            ms = [struct.unpack_from("<Id", b, st + 4 + i * 16) for i in range(n)]
            if 0 < dur < 1e7 and all(-1 <= m[1] <= dur + 1 for m in ms) \
                    and all(struct.unpack_from("<I", b, st + 4 + i * 16 + 12)[0] == 0 for i in range(n)):
                segs[sid] = {
                    "dur": dur,
                    "markers": sorted(m[1] for m in ms),
                    "tempo": struct.unpack_from("<f", b, st - 19)[0],
                    "sig": (b[st - 15], b[st - 14]),
                    "tracks": [t for t in tracks if struct.pack("<I", t) in b],
                }
                break
    return segs


def parse_tree(b, segs):
    """The playlist tree: the outermost item tree that parses to the exact end of the record."""
    def item(o):
        if o + 30 > len(b):
            raise ValueError
        sid, _, nch, rst = struct.unpack_from("<IIIi", b, o)
        loop = struct.unpack_from("<h", b, o + 16)[0]
        w = struct.unpack_from("<I", b, o + 22)[0]
        avoid = struct.unpack_from("<H", b, o + 26)[0]
        if nch > 500 or rst not in RST or (sid == 0) != (nch > 0) or (sid != 0 and sid not in segs):
            raise ValueError
        node = {"seg": sid, "type": RST[rst], "loop": loop, "weight": w, "avoid": avoid, "kids": []}
        o += 30
        for _ in range(nch):
            k, o = item(o)
            node["kids"].append(k)
        return node, o

    for root in range(0, len(b) - 30):
        try:
            node, end = item(root)
            if end == len(b) and node["kids"]:
                return node
        except (ValueError, struct.error, RecursionError):
            pass
    return None


def main():
    objs = hirc(open(PCK, "rb").read())
    ids = {}
    for f in os.listdir(os.path.join(HOME, "music", "music")):
        m = re.match(r"^(.*)_id#(\d+)\.ogg$", f)
        if m:
            ids[int(m.group(2))] = m.group(1)
    tracks = music_tracks(objs, ids)
    segs = segments(objs, tracks)
    trees = {pid: t for pid, b in objs[13].items() if (t := parse_tree(b, segs))}
    print(f"{len(tracks)} tracks, {len(segs)} segments, {len(trees)} of {len(objs[13])} playlists")
    pickle.dump((objs, tracks, segs, ids, trees), open(os.path.join(HOME, "mus_hirc.pkl"), "wb"))


if __name__ == "__main__":
    main()
