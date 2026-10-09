"""DOOM Eternal .md6anim / .md6skl reader (reverse-engineered 2026-10-09, checked against Vega's Cast
exports) - lets the weapon converter run without Vega. Layout notes: docs/md6_formats.md.

Units like Vega's Cast export: translations in cm (the files store metres), quaternions x, y, z, w.
"""
import math
import struct

INV_SQRT2_Q = 1.0 / (16384.0 * math.sqrt(2.0))


def _u16(b, o):
    return struct.unpack_from("<H", b, o)[0]


def read_skeleton(b):
    """md6skl bytes -> (names, parents, [(t_cm (3), q xyzw (4), scale (3))])."""
    names_off, _, n = struct.unpack_from("<IHH", b, 0)
    table = struct.unpack_from("<20H", b, 0x0E)
    # rotations, scales, translations: arrays sized for the bone count rounded up to 8
    npad = (n + 7) // 8 * 8
    rot0 = 0x44
    scl0 = rot0 + npad * 16
    tr0 = scl0 + npad * 12
    par0 = table[1] + 4
    q = [struct.unpack_from("<4f", b, rot0 + i * 16) for i in range(n)]
    s = [struct.unpack_from("<3f", b, scl0 + i * 12) for i in range(n)]
    t = [tuple(v * 100.0 for v in struct.unpack_from("<3f", b, tr0 + i * 12)) for i in range(n)]
    parents = list(struct.unpack_from(f"<{n}h", b, par0))
    names = []
    o = names_off + 4
    for _ in range(n):
        ln = struct.unpack_from("<I", b, o)[0]
        names.append(b[o + 4:o + 4 + ln].decode("ascii"))
        o += 4 + ln
    return names, parents, [(t[i], q[i], s[i]) for i in range(n)]


def quat(a, b, c):
    """Smallest-three quaternion: 15 bits per stored component (+-1/sqrt2), the dropped (largest,
    positive) one picked by the top bits of the first two; stored ones follow it cyclically."""
    drop = 3 - (((b >> 15) << 1) | (a >> 15))
    comps = [((x & 0x7FFF) - 16384) * INV_SQRT2_Q for x in (a, b, c)]
    q = [0.0] * 4
    for k in range(3):
        q[(drop + 1 + k) % 4] = comps[k]
    q[drop] = math.sqrt(max(0.0, 1.0 - sum(x * x for x in comps)))
    return tuple(q)


def _bone_list(b, o):
    """Run-length list: total, then (run length, first bone) byte pairs."""
    total = b[o]
    out = []
    o += 1
    while len(out) < total:
        ln, first = b[o], b[o + 1]
        out.extend(range(first, first + ln))
        o += 2
    return out


class Anim:
    """Curves like convert_weapon.Clip: {bone: {"rq"|"tx"|"ty"|"tz"|"s": (frames, values, mode)}}.
    `frames` counts like Vega's export (last key + 1); `header_frames` is the clip's own length."""

    def __init__(self, b, skel_names, additive=False):
        n = struct.unpack_from("<I", b, 0)[0]
        self.skeleton = b[4:4 + n].decode("ascii")
        base = 4 + n + 52
        self.flags = b[base + 0x0F]
        self.header_frames = _u16(b, base + 0x10)
        self.fps = float(_u16(b, base + 0x12))
        nsets = _u16(b, base + 0x14)
        offs = struct.unpack_from("<7H", b, base + 0x16)
        mode = "additive" if additive else "absolute"
        # the bone lists: u16 1, u16 skeleton hash, 8 u16 offsets (from the list start), RLE lists
        p = base
        while not (_u16(b, p) == 1 and _u16(b, p + 4) == 0x14):
            p += 1
        lo = struct.unpack_from("<8H", b, p + 4)
        lists = [_bone_list(b, p + lo[k]) if lo[k + 1] > lo[k] + 1 else [] for k in range(7)]
        const_rot, const_scl, const_tr, extra, anim_rot, anim_scl, anim_tr = lists
        if extra:
            raise ValueError(f"unsupported channel list {extra}")
        curves = {}
        nm = lambda i: skel_names[i]
        # constants (one value for the whole clip)
        o = base + offs[2]
        for i in const_rot:
            curves.setdefault(nm(i), {})["rq"] = ([0], [quat(*struct.unpack_from("<3H", b, o))])
            o += 6
        o = base + offs[3]
        for i in const_scl:
            curves.setdefault(nm(i), {})["s"] = ([0], [struct.unpack_from("<3f", b, o)])
            o += 12
        o = base + offs[4]
        for i in const_tr:
            curves.setdefault(nm(i), {})["t"] = ([0], [struct.unpack_from("<3f", b, o)])
            o += 12
        # animated: per frameset its first-frame values, then each bone's keys (bone after bone)
        # at the frames its bitmask (MSB first, one bit per frame of the set) marks
        anim = {("rq", i): ([], []) for i in anim_rot}
        anim.update({("s", i): ([], []) for i in anim_scl})
        anim.update({("t", i): ([], []) for i in anim_tr})
        fs = base + _u16(b, base + 0x0C)
        for _ in range(nsets):
            t = struct.unpack_from("<19H", b, fs)
            start, count = t[17], t[18]
            mb = (count + 7) // 8
            # (rot, scale, trans): first values, keys, masks - scale sits between rot and trans
            sec = {"rq": (t[0], t[4], t[8], anim_rot, 6), "s": (t[1], t[5], t[9], anim_scl, 12),
                   "t": (t[2], t[6], t[10], anim_tr, 12)}
            for kind, (first, keys, masks, bones, size) in sec.items():
                o = fs + keys
                for k, i in enumerate(bones):
                    fr, vals = anim[(kind, i)]
                    fr.append(start)
                    vals.append(self._value(kind, b, fs + first + k * size))
                    m = b[fs + masks + k * mb:fs + masks + (k + 1) * mb]
                    for f in range(count):
                        if m[f >> 3] & (0x80 >> (f & 7)):
                            fr.append(start + f)
                            vals.append(self._value(kind, b, o))
                            o += size
            fs += t[16]
        for (kind, i), v in anim.items():
            curves.setdefault(nm(i), {})[kind] = v
        # to convert_weapon's layout: translations split per axis in cm
        self.curves = {}
        last = 0
        for bone, props in curves.items():
            out = self.curves.setdefault(bone, {})
            for kind, (fr, vals) in props.items():
                last = max(last, max(fr))
                if kind == "t":
                    for a, ax in enumerate(("tx", "ty", "tz")):
                        out[ax] = (fr, [v[a] * 100.0 for v in vals], mode)
                else:
                    out[kind] = (fr, vals, mode)
        self.frames = last + 1

    @staticmethod
    def _value(kind, b, o):
        return quat(*struct.unpack_from("<3H", b, o)) if kind == "rq" else struct.unpack_from("<3f", b, o)
