"""Software preview of a baked viewmodel (model.bin) - checks poses/animations without the game.

    python tools/vm/preview.py super_shotgun shoot_reload out.png [frames...]
"""
import os
import struct
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw

ROOT = Path(os.environ.get("DOOMRING_VM_OUT", Path(__file__).resolve().parents[2] / "dist" / "natives" / "doom_vm"))


def load(name):
    b = (ROOT / name / "model.bin").read_bytes()
    o = 4
    na, ng, nm = struct.unpack_from("<III", b, o); o += 12
    meshes = []
    vt = np.dtype([("p", "<f4", 3), ("n", "<f4", 3), ("uv", "<f4", 2), ("bi", "<u2", 4), ("bw", "<f4", 4)])
    for _ in range(nm):
        part, ln = struct.unpack_from("<BH", b, o); o += 3
        mat = b[o:o + ln].decode(); o += ln
        nv, nf = struct.unpack_from("<II", b, o); o += 8
        v = np.frombuffer(b, vt, nv, o); o += nv * vt.itemsize
        f = np.frombuffer(b, "<u4", nf * 3, o).reshape(-1, 3); o += nf * 12
        meshes.append((part, mat, v, f))
    nc, = struct.unpack_from("<I", b, o); o += 4
    clips = {}
    for _ in range(nc):
        ln, = struct.unpack_from("<H", b, o); o += 2
        name_c = b[o:o + ln].decode(); o += ln
        fps, fr = struct.unpack_from("<fI", b, o); o += 8
        pa = np.frombuffer(b, "<f4", fr * na * 12, o).reshape(fr, na, 3, 4); o += pa.nbytes
        pg = np.frombuffer(b, "<f4", fr * ng * 12, o).reshape(fr, ng, 3, 4); o += pg.nbytes
        clips[name_c] = (fps, pa, pg)
    return meshes, clips


def render(meshes, pa, pg, w=960, h=540, fov=70.0):
    img = Image.new("RGB", (w, h), (90, 110, 90))
    d = ImageDraw.Draw(img)
    tris = []
    f = 1 / np.tan(np.radians(fov) / 2)
    for part, mat, v, faces in meshes:
        pal = pa if part == 0 else pg
        m = pal[v["bi"]]  # n,4,3,4
        p4 = np.concatenate([v["p"], np.ones((len(v), 1), np.float32)], 1)
        sk = np.einsum("nkij,nj->nki", m, p4)
        pos = (sk * v["bw"][:, :, None]).sum(1)
        z = pos[:, 2]
        sx = w / 2 + pos[:, 0] * f / np.maximum(z, 1e-3) * (h / 2)
        sy = h / 2 - pos[:, 1] * f / np.maximum(z, 1e-3) * (h / 2)
        col = (180, 150, 130) if part == 0 else (110, 110, 120)
        for t in faces:
            if (z[t] < 0.02).any():
                continue
            a, b2, c = pos[t]
            nrm = np.cross(b2 - a, c - a)
            ln = np.linalg.norm(nrm)
            if ln == 0:
                continue
            shade = 0.35 + 0.65 * abs(nrm[1] / ln * 0.6 + nrm[2] / ln * -0.8)
            tris.append((z[t].mean(), [(sx[i], sy[i]) for i in t], tuple(int(cc * shade) for cc in col)))
    for _, pts, c in sorted(tris, key=lambda x: -x[0]):
        d.polygon(pts, fill=c)
    return img


if __name__ == "__main__":
    name, clip, out = sys.argv[1], sys.argv[2], sys.argv[3]
    meshes, clips = load(name)
    fps, pa, pg = clips[clip]
    frames = [int(x) for x in sys.argv[4:]] or [0]
    tiles = [render(meshes, pa[min(fr, len(pa) - 1)], pg[min(fr, len(pg) - 1)]) for fr in frames]
    sheet = Image.new("RGB", (480 * len(tiles), 270))
    for i, t in enumerate(tiles):
        sheet.paste(t.resize((480, 270)), (i * 480, 0))
    sheet.save(out)
    print("clips:", list(clips))
