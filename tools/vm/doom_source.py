"""convert_weapon.py's model / clip inputs straight from the user's DOOM Eternal install (no Vega):
the same calls it makes on Vega's Cast exports (Skeleton().Bones(), Meshes(), Vertex*Buffer...),
answered by our md6 readers. Set DOOMRING_VM_SOURCE=doom to use it.

Work files (SAMUEL's model exports, decls) go to DOOMRING_WORK (default ~/doom-extract/vm_src).
"""
import os
import subprocess
from pathlib import Path

import numpy as np

import doom_res
import md6anim
import md6mesh

WORK = Path(os.environ.get("DOOMRING_WORK", Path.home() / "doom-extract" / "vm_src"))
RES = doom_res.Resources()
NO_WINDOW = doom_res.NO_WINDOW


class _Bone:
    def __init__(self, name, parent, t, q):
        self._n, self._p, self._t, self._q = name, parent, t, q

    def Name(self):
        return self._n

    def ParentIndex(self):
        return self._p

    def LocalPosition(self):
        return self._t

    def LocalRotation(self):
        return self._q


class _Skeleton:
    def __init__(self, bones):
        self._b = bones

    def Bones(self):
        return self._b


class _Material:
    def __init__(self, name):
        self._n = name

    def Name(self):
        return self._n


class _Mesh:
    def __init__(self, m):
        self.m = m

    def Material(self):
        return _Material(self.m["material"])

    def VertexPositionBuffer(self):
        return self.m["pos"].ravel()

    def VertexNormalBuffer(self):
        return self.m["nrm"].ravel()

    def VertexUVLayerBuffer(self, _layer):
        return self.m["uv"].ravel()

    def MaximumWeightInfluence(self):
        return 4

    def VertexWeightBoneBuffer(self):
        return self.m["bi"].ravel()

    def VertexWeightValueBuffer(self):
        return self.m["bw"].ravel()

    def FaceBuffer(self):
        return self.m["faces"].ravel()


class Model:
    """An md6mesh + its md6skl, exported once with samuel-cli (SAMUEL_RAW_GEO)."""

    def __init__(self, mesh_res):
        self.res_name = mesh_res
        stem = Path(mesh_res).stem
        self.dir = WORK / "modelExports" / stem
        raw = mesh_res.replace("/", "_")
        hdr, geo = self.dir / f"{raw}.md6hdr", self.dir / f"{raw}.md6geo"
        if not (hdr.exists() and geo.exists()):
            arch = RES.find(mesh_res)
            if arch is None:
                raise FileNotFoundError(mesh_res)
            (WORK / "x").mkdir(parents=True, exist_ok=True)
            subprocess.run([str(RES.samuel), str(RES.base / f"{arch}.resources"), "export", str(WORK / "x"), mesh_res],
                           env=dict(os.environ, SAMUEL_RAW_GEO="1"), capture_output=True, creationflags=NO_WINDOW)
        h, meshes = md6mesh.read_mesh(hdr.read_bytes(), geo.read_bytes())
        names, parents, bind = md6anim.read_skeleton(skeleton_bytes(h.skeleton))
        self._skel = _Skeleton([_Bone(n, p, np.array(t, float), np.array(q, float))
                                for n, p, (t, q, _s) in zip(names, parents, bind)])
        self._meshes = [_Mesh(m) for m in meshes]
        self.part_names = [m["name"] for m in meshes]
        self.images = self.dir / "images"

    def Skeleton(self):
        return self._skel

    def Meshes(self):
        return self._meshes


_skel_cache = {}


def skeleton_bytes(name):
    if name not in _skel_cache:
        _skel_cache[name] = RES.raw([name])[name]
    return _skel_cache[name]


_anim_cache = {}


def anim(res_name):
    """md6anim resource -> md6anim.Anim (curves like a Cast clip), None if Doom has no such clip."""
    if res_name not in _anim_cache:
        raw = RES.raw([res_name]).get(res_name)
        a = None
        if raw is not None:
            n = int.from_bytes(raw[:4], "little")
            skel = raw[4:4 + n].decode("ascii")
            names = md6anim.read_skeleton(skeleton_bytes(skel))[0]
            a = md6anim.Anim(raw, names, additive="/additive/" in res_name)
        _anim_cache[res_name] = a
    return _anim_cache[res_name]


def export_decls(names):
    """Text decls (md6def ...) from gameresources and gameresources_patch1 into WORK/decls/<archive>."""
    for arch in ("gameresources", "gameresources_patch1"):
        out = WORK / "decls" / arch
        out.mkdir(parents=True, exist_ok=True)
        subprocess.run([str(RES.samuel), str(RES.base / f"{arch}.resources"), "export", str(out), *names],
                       capture_output=True, creationflags=NO_WINDOW)
