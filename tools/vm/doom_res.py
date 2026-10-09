"""Read raw resources (md6anim / md6skl / ...) from the user's DOOM Eternal install with samuel-cli.

Later archives override earlier ones (patch3 > patch2 > patch1 > gameresources); level archives
(e1m1_intro) hold a few things the shared ones don't (the sticky-bomb reload layer).
"""
import os
import subprocess
import tempfile
from pathlib import Path

import sys  # noqa: E402
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import paths  # noqa: E402,F401  (no console windows for the helper tools)

SAMUEL = Path(os.environ.get("DOOMRING_SAMUEL", Path.home() / "modtools/samuel-src/build/samuel-cli.exe"))
DOOM = Path(os.environ.get("DOOMRING_DOOM", "C:/Program Files (x86)/Steam/steamapps/common/DOOMEternal"))
# lookup order: first archive that has the name wins
ARCHIVES = ["gameresources_patch3", "gameresources_patch2", "gameresources_patch1", "gameresources",
            "game/sp/e1m1_intro/e1m1_intro"]
NO_WINDOW = 0x08000000 if os.name == "nt" else 0


class Resources:
    def __init__(self, doom=DOOM, samuel=SAMUEL):
        self.base = Path(doom) / "base"
        self.samuel = Path(samuel)
        self._index = {}

    def _run(self, *args):
        return subprocess.run([str(self.samuel), *map(str, args)], capture_output=True, text=True,
                              creationflags=NO_WINDOW)

    def index(self, archive):
        """{name: type} of one archive's md6 entries (cached)."""
        if archive not in self._index:
            path = self.base / f"{archive}.resources"
            out = self._run(path, "list", "md6").stdout if path.exists() else ""
            self._index[archive] = {ln.split("\t")[0]: ln.split("\t")[1] for ln in out.splitlines() if "\t" in ln}
        return self._index[archive]

    def find(self, name):
        return next((a for a in ARCHIVES if name in self.index(a)), None)

    def raw(self, names):
        """{name: bytes} - the embedded data of each (missing names are left out)."""
        out = {}
        by_archive = {}
        for n in names:
            a = self.find(n)
            if a:
                by_archive.setdefault(a, []).append(n)
        for a, ns in by_archive.items():
            with tempfile.TemporaryDirectory() as tmp:
                for i in range(0, len(ns), 40):
                    self._run(self.base / f"{a}.resources", "raw", tmp, *ns[i:i + 40])
                for n in ns:
                    fn = n
                    for c in "/ $:":
                        fn = fn.replace(c, "_")
                    hits = list(Path(tmp).glob(fn + ".*.bin"))
                    if hits:
                        out[n] = hits[0].read_bytes()
        return out

    def image_index(self, archive):
        """{art path (.tga): [full image resource names]} of one archive (cached)."""
        key = "img:" + archive
        if key not in self._index:
            path = self.base / f"{archive}.resources"
            out = self._run(path, "list", ".tga").stdout if path.exists() else ""
            idx = {}
            for ln in out.splitlines():
                if "\timage" in ln:
                    n = ln.split("\t")[0]
                    idx.setdefault(n.split("$")[0], []).append(n)
            self._index[key] = idx
        return self._index[key]

    def raw_from(self, archive, names):
        """{name: bytes} from one archive."""
        out = {}
        with tempfile.TemporaryDirectory() as tmp:
            for i in range(0, len(names), 40):
                self._run(self.base / f"{archive}.resources", "raw", tmp, *names[i:i + 40])
            for n in names:
                fn = n
                for c in "/ $:":
                    fn = fn.replace(c, "_")
                hits = list(Path(tmp).glob(fn + ".*.bin"))
                if hits:
                    out[n] = hits[0].read_bytes()
        return out
