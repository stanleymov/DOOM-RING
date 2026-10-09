"""DOOM Eternal .md6mesh reader (header layout from SAMUEL's MD6.cpp; skin weights reverse-engineered
2026-10-09 against Vega's Cast exports). Input: the header and decompressed LOD0 geometry that
samuel-cli writes with SAMUEL_RAW_GEO=1. Output like Vega's Cast meshes (cm, Doom axes).
"""
import struct

import numpy as np


class Header:
    def __init__(self, b):
        o = 0
        n = struct.unpack_from("<I", b, o)[0]
        o += 4
        self.skeleton = b[o:o + n].decode("ascii")
        o += n
        o += 6 * 4 + 1 + 4  # MD6_UNK_FLOATS
        nb = struct.unpack_from("<H", b, o)[0]
        o += 2
        self.bone_numbers = list(b[o:o + nb])
        o += nb
        # MD6_BONE_INFO: 6 floats, u64, 2 floats, 48 pad
        unk7, unk8 = struct.unpack_from("<2f", b, o + 24 + 8)
        o += 24 + 8 + 8 + 48
        nm = struct.unpack_from("<I", b, o)[0]
        o += 4
        self.meshes = []
        short = unk7 == 0 and unk8 == 0
        for _ in range(nm):
            ln = struct.unpack_from("<I", b, o)[0]
            name = b[o + 4:o + 4 + ln].decode("ascii", "replace")
            o += 4 + ln
            ln = struct.unpack_from("<I", b, o)[0]
            mat = b[o + 4:o + 4 + ln].decode("ascii", "replace")
            o += 4 + ln
            unk = struct.unpack_from("<B5I", b, o)  # MD6_MESH_UNKNOWNS
            o += 1 + 4 * 5
            lods = []
            for _ in range(1 if short else 3):
                lods.append(_lod(b, o))
                o += LOD_SIZE
                if short:
                    o += 8
            o += 13  # MD6_MESH_FOOTER
            self.meshes.append({"name": name, "material": mat, "lod": lods[0], "unk": unk})
        # stream layouts: 5 x (header 12 + data 36) then 5 x disk layout 16, at the end
        sz = 12 + 36 + 16
        o = len(b) - sz * 5
        self.streams = []
        for i in range(5):
            h = struct.unpack_from("<HHII", b, o + i * 48)
            d = struct.unpack_from("<9I", b, o + i * 48 + 12)
            self.streams.append({"version": h[0], "size": h[2], "offsets": d})


# MD6_LOD_INFO: u32 pad, u32 verts, u32 faces, GEO_METADATA (13 floats), GEO_FLAGS (2 u16), 2 floats
LOD_SIZE = 12 + 52 + 4 + 8


def _lod(b, o):
    pad, nv, nf = struct.unpack_from("<3I", b, o)
    # GEO_METADATA (SAMUEL Common.h): bounds min[3], max[3], vertex offset[3], vertex scale,
    # uv offset u, v, uv scale
    m = struct.unpack_from("<13f", b, o + 12)
    return {"verts": nv, "faces": nf, "voff": m[6:9], "vscale": m[9], "uvoff": m[10:12], "uvscale": m[12]}




def read_mesh(hdr, geo):
    """md6 header + LOD0 geometry -> list of meshes like Vega's Cast: dict(name, material (full decl
    path), pos (n,3) cm, nrm (n,3), uv (n,2), bi (n,4) skeleton bone, bw (n,4), faces (m,3))."""
    h = Header(hdr)
    d = h.streams[0]["offsets"]
    nv = sum(m["lod"]["verts"] for m in h.meshes)
    nf = sum(m["lod"]["faces"] for m in h.meshes)
    P = np.frombuffer(geo, np.uint16, nv * 4, 0).reshape(nv, 4)
    N = np.frombuffer(geo, np.uint8, nv * 8, d[5]).reshape(nv, 8)
    U = np.frombuffer(geo, np.uint16, nv * 2, d[6]).reshape(nv, 2)
    C = np.frombuffer(geo, np.uint8, nv * 4, d[7]).reshape(nv, 4)
    F = np.frombuffer(geo, np.uint16, nf * 3, d[8]).reshape(nf, 3)
    # palette slot -> skeleton bone (the header lists the palette slot of each skeleton bone)
    slot = {}
    for bone, s in enumerate(h.bone_numbers):
        slot.setdefault(s, bone)
    out = []
    vo = fo = 0
    for m in h.meshes:
        lod = m["lod"]
        n, f = lod["verts"], lod["faces"]
        p = P[vo:vo + n, :3] / 65535.0 * lod["vscale"] + np.array(lod["voff"])
        nb = N[vo:vo + n].astype(np.float64)
        nrm = (nb[:, :3] - 128.0) / 127.0
        nrm /= np.maximum(np.linalg.norm(nrm, axis=1, keepdims=True), 1e-9)
        uv = U[vo:vo + n] / 65535.0 * lod["uvscale"] + np.array(lod["uvoff"])
        w2 = (N[vo:vo + n, 7] & 127) / 254.0
        w3 = (N[vo:vo + n, 3] >> 4) / 45.0
        w4 = (N[vo:vo + n, 3] & 15) / 60.0
        bw = np.stack([1.0 - w2 - w3 - w4, w2, w3, w4], 1)
        # (each mesh's slots start at its palette offset - MD6_MESH_UNKNOWNS' 4th field)
        pal = m["unk"][3]
        bi = np.vectorize(lambda s: slot.get(int(s) + pal, 0))(C[vo:vo + n]).astype(np.uint16)
        out.append({"name": m["name"], "material": m["material"], "pos": (p * 100.0).astype(np.float32),
                    "nrm": nrm.astype(np.float32), "uv": uv.astype(np.float32), "bi": bi,
                    "bw": bw.astype(np.float32), "faces": F[fo:fo + f, ::-1].astype(np.uint32)})
        vo += n
        fo += f
    return h, out
