"""DOOM RING setup engine: finds the games through Steam, checks them, and builds the mod's Doom content
from the player's own DOOM Eternal install with the bundled converters (setup GUI: setup.pyw).

Package layout (installer/build_package.py makes it):
    Setup DOOM RING.exe     launcher -> runtime\\pythonw.exe setup\\setup.pyw
    runtime\\               Python 3.12 + numpy / scipy / Pillow / tkinter
    setup\\                 this engine, the GUI, tools\\ (converters), data\\
    bin\\                   samuel-cli.exe (GPL-3), ww2ogg.exe + codebooks (BSD-3), licenses
    payload\\               the mod itself (DLL, settings, me3, launcher .bat, README, fonts)
"""
import ctypes
import json
import os
import re
import shutil
import subprocess
import sys
import threading
import time
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ELDEN_RING_APP = "1245620"
DOOM_ETERNAL_APP = "782330"
NO_WINDOW = 0x08000000
# ffmpeg (GPL) when the PC has none: BtbN's Windows build, downloaded by the setup - not shipped by us
FFMPEG_URL = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip"

HERE = Path(__file__).resolve().parent            # <package>/setup
PACKAGE = HERE.parent
TOOLS = HERE / "tools"
BIN = PACKAGE / "bin"
PAYLOAD = PACKAGE / "payload"
# (development: run from the repo - installer/engine.py with tools/ beside it)
if not TOOLS.exists():
    TOOLS = HERE.parent / "tools"
WEAPONS = ["combat_shotgun", "heavy_cannon", "plasma_rifle", "rocket_launcher", "super_shotgun", "ballista",
           "chaingun", "bfg", "fists", "chainsaw", "crucible"]
# what DOOM Eternal must have (base game only - no DLC files are used)
DOOM_FILES = ["base/gameresources.resources", "base/gameresources_patch1.resources",
              "base/gameresources_patch2.resources", "base/game/sp/e1m1_intro/e1m1_intro.resources",
              "base/game/sp/e3m1_slayer/e3m1_slayer.resources", "base/sound/soundbanks/pc/sfx.snd",
              "base/sound/soundbanks/pc/music.snd", "base/sound/soundbanks/pc/mus.pck", "oo2core_8_win64.dll"]


# ------------------------------------------------------------------------------------- Steam

def steam_root():
    try:
        import winreg
        for hive, key, val in ((winreg.HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
                               (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
                               (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam", "InstallPath")):
            try:
                with winreg.OpenKey(hive, key) as k:
                    p = Path(winreg.QueryValueEx(k, val)[0])
                    if p.exists():
                        return p
            except OSError:
                pass
    except ImportError:
        pass
    p = Path("C:/Program Files (x86)/Steam")
    return p if p.exists() else None


def steam_libraries():
    root = steam_root()
    libs = []
    if root:
        libs.append(root)
        vdf = root / "steamapps" / "libraryfolders.vdf"
        if vdf.exists():
            for m in re.finditer(r'"path"\s+"([^"]+)"', vdf.read_text(encoding="utf-8", errors="ignore")):
                p = Path(m.group(1).replace("\\\\", "\\"))
                if p not in libs:
                    libs.append(p)
    return [p for p in libs if p.exists()]


def find_app(app_id):
    """The game's install folder from Steam's manifests, or None."""
    for lib in steam_libraries():
        acf = lib / "steamapps" / f"appmanifest_{app_id}.acf"
        if acf.exists():
            m = re.search(r'"installdir"\s+"([^"]+)"', acf.read_text(encoding="utf-8", errors="ignore"))
            if m:
                p = lib / "steamapps" / "common" / m.group(1)
                if p.exists():
                    # (the real folder name casing - Steam's library file stores the path lower case)
                    return Path(os.path.realpath(p))
    return None


def file_version(path):
    """'2.7.1.0'-style file version of an exe (Windows), or None."""
    try:
        size = ctypes.windll.version.GetFileVersionInfoSizeW(str(path), None)
        if not size:
            return None
        buf = ctypes.create_string_buffer(size)
        ctypes.windll.version.GetFileVersionInfoW(str(path), 0, size, buf)
        ptr, n = ctypes.c_void_p(), ctypes.c_uint()
        ctypes.windll.version.VerQueryValueW(buf, "\\", ctypes.byref(ptr), ctypes.byref(n))
        ms, ls = ctypes.cast(ptr.value + 8, ctypes.POINTER(ctypes.c_uint32 * 2)).contents
        return f"{ms >> 16}.{ms & 0xFFFF}.{ls >> 16}.{ls & 0xFFFF}"
    except Exception:
        return None


def check_elden_ring(path):
    """(ok, message)"""
    if path is None:
        return False, ("ELDEN RING was not found. Install it through Steam (the base game is enough), "
                       "then press Check again - or point to its folder with Browse.")
    exe = Path(path) / "Game" / "eldenring.exe"
    if not exe.exists():
        return False, f"No Game\\eldenring.exe in {path}. Is this the ELDEN RING folder?"
    ver = file_version(exe)
    msg = f"Found: {path}" + (f"  (version {ver})" if ver else "")
    if ver and not ver.startswith("2.7.1"):
        msg += "\nNote: DOOM RING was made for ELDEN RING 1.16 (2.7.1). Other versions may not work."
    return True, msg


def check_doom(path):
    if path is None:
        return False, ("DOOM Eternal was not found. The mod's sounds, music and guns are made from YOUR copy of "
                       "DOOM Eternal - install it through Steam (no DLC needed), then press Check again.")
    missing = [f for f in DOOM_FILES if not (Path(path) / f).exists()]
    if missing:
        return False, f"DOOM Eternal at {path} is missing files ({missing[0]} ...). Let Steam verify the game files."
    return True, f"Found: {path}"


def setup_version():
    """This setup's version.json: {"version": "1.1", "content": 1}."""
    v = json.loads((HERE / "version.json").read_text(encoding="utf-8"))
    return {"version": str(v["version"]), "content": int(v["content"])}


INSTALLED_FILE = "doomring_version.json"


def installed_version(target):
    """What is already in `target`: {"version", "content"} or None (nothing usable - a full install).
    A complete 1.0 install has no version file; it is recognised by its converted DOOM files."""
    t = Path(target)
    f = t / INSTALLED_FILE
    if f.exists():
        try:
            v = json.loads(f.read_text(encoding="utf-8"))
            return {"version": str(v["version"]), "content": int(v["content"])}
        except (ValueError, KeyError, OSError):
            return None
    natives = t / "game" / "natives"
    if (natives / "doomslayer.dll").exists() and all(
            (natives / "doom_vm" / w / "model.bin").exists() for w in WEAPONS) and all(
            (natives / d).is_dir() for d in ("doom_audio", "doom_music", "doom_ui", "doom_fx")):
        return {"version": "1.0", "content": 1}
    return None


def free_gb(path):
    p = Path(path)
    while not p.exists() and p.parent != p:
        p = p.parent
    return shutil.disk_usage(p).free / 1e9


# ------------------------------------------------------------------------------------- install

class Cancelled(Exception):
    pass


class Installer:
    """Runs the steps in a thread; `on_progress(fraction, step text)` and `on_log(line)` are called
    from that thread (the GUI marshals them)."""

    def __init__(self, doom, target, on_progress, on_log, workers=None):
        # (None: DOOM Eternal isn't installed - fine for a mod-files update, see steps())
        self.doom = Path(doom) if doom else None
        self.target = Path(target)
        self.version = setup_version()
        # an update over the same converted content only copies the new mod files (seconds); a new
        # content number (converters / sound lists changed) or no usable install rebuilds everything
        old = installed_version(self.target)
        self.update_from = old["version"] if old else None
        self.quick = bool(old and old["content"] == self.version["content"])
        self.natives = self.target / "game" / "natives"
        self.work = Path(os.environ.get("LOCALAPPDATA", Path.home())) / "DRSetup"
        self.on_progress = on_progress
        self.on_log = on_log
        self.cancelled = False
        self.procs = []
        self.workers = workers or max(1, min(4, (os.cpu_count() or 2) // 2))
        self.log_file = None
        self.ffmpeg = None
        self._base, self._span = 0.0, 0.0

    # -- helpers
    def log(self, line):
        line = line.rstrip()
        if self.log_file:
            self.log_file.write(line + "\n")
            self.log_file.flush()
        self.on_log(line)

    def python(self):
        exe = Path(sys.executable)
        con = exe.with_name("python.exe")
        return str(con if con.exists() else exe)

    def env(self):
        e = dict(os.environ)
        e.update({
            "DOOMRING_DOOM": str(self.doom or ""),
            "DOOMRING_WORK": str(self.work),
            "DOOMRING_OUT": str(self.natives),
            "DOOMRING_VM_OUT": str(self.natives / "doom_vm"),
            "DOOMRING_VM_SOURCE": "doom",
            "DOOMRING_SAMUEL": str(BIN / "samuel-cli.exe") if (BIN / "samuel-cli.exe").exists() else e.get("DOOMRING_SAMUEL", ""),
            "DOOMRING_WW2OGG": str(BIN / "ww2ogg.exe") if (BIN / "ww2ogg.exe").exists() else e.get("DOOMRING_WW2OGG", ""),
            "PYTHONUNBUFFERED": "1",
            "PYTHONIOENCODING": "utf-8",
            # (the bundled Python only: never a player's own Python settings or packages)
            "PYTHONNOUSERSITE": "1",
        })
        for k in ("PYTHONPATH", "PYTHONHOME", "PYTHONSTARTUP"):
            e.pop(k, None)
        if self.ffmpeg:
            e["DOOMRING_FFMPEG"] = str(self.ffmpeg)
        return {k: v for k, v in e.items() if v != ""}

    def run(self, script, *args):
        if self.cancelled:
            raise Cancelled()
        cmd = [self.python(), str(TOOLS / script), *args]
        self.log(f"> {script} {' '.join(args)}")
        p = subprocess.Popen(cmd, cwd=str(TOOLS), env=self.env(), stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                             text=True, encoding="utf-8", errors="replace", creationflags=NO_WINDOW)
        self.procs.append(p)
        for line in p.stdout:
            self.log("  " + line.rstrip())
            if self.cancelled:
                p.kill()
        p.wait()
        self.procs.remove(p)
        if self.cancelled:
            raise Cancelled()
        if p.returncode != 0:
            raise RuntimeError(f"{script} {' '.join(args)} failed (exit {p.returncode}) - see the log")

    def sub_progress(self, frac, text):
        """Progress inside the running step (0..1 of its share of the bar)."""
        self.on_progress(self._base + self._span * min(max(frac, 0.0), 1.0), text)

    def cancel(self):
        self.cancelled = True
        for p in list(self.procs):
            try:
                p.kill()
            except OSError:
                pass

    # -- steps (weights = rough share of the time)
    def steps(self):
        if self.quick:
            return [
                (1, "Updating the mod files", self.copy_payload),
                (1, "Checking the result", self.verify),
            ]
        return [
            (2, "Copying the mod files", self.copy_payload),
            (3, "Checking for ffmpeg (audio tool)", self.get_ffmpeg),
            (8, "Reading DOOM Eternal's sounds", lambda: self.run("extract_sounds.py")),
            (6, "Converting the sound effects", lambda: self.run("convert_audio.py")),
            (14, "Building the combat music", self.music),
            (6, "HUD art, icons and fonts", self.hud),
            (1, "Muzzle flashes", lambda: self.run("convert_fx.py")),
            (60, "Guns, arms and animations", self.weapons),
            (1, "Checking the result", self.verify),
        ]

    def copy_payload(self):
        if not PAYLOAD.exists():
            raise RuntimeError(f"payload folder missing: {PAYLOAD}")
        self.target.mkdir(parents=True, exist_ok=True)
        for src in PAYLOAD.rglob("*"):
            dst = self.target / src.relative_to(PAYLOAD)
            if src.is_dir():
                dst.mkdir(parents=True, exist_ok=True)
            elif not (dst.exists() and dst.name in ("doomslayer.toml", "erfps2.toml")):
                # (a reinstall keeps the player's settings)
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(src, dst)
        self.log(f"mod files -> {self.target}")

    def get_ffmpeg(self):
        """ffmpeg for the audio steps: bundled, on the PC already, or downloaded once (~130 MB)."""
        for c in (BIN / "ffmpeg.exe", self.work / "ffmpeg" / "ffmpeg.exe"):
            if c.exists():
                self.ffmpeg = c
                break
        else:
            found = shutil.which("ffmpeg")
            if found:
                self.ffmpeg = Path(found)
        if self.ffmpeg:
            self.log(f"ffmpeg: {self.ffmpeg}")
            return
        import urllib.request
        import zipfile
        zpath = self.work / "ffmpeg.zip"
        self.log(f"downloading ffmpeg from {FFMPEG_URL}")
        with urllib.request.urlopen(FFMPEG_URL, timeout=60) as r, open(zpath, "wb") as f:
            total = int(r.headers.get("Content-Length") or 0)
            got = 0
            while True:
                if self.cancelled:
                    raise Cancelled()
                chunk = r.read(1 << 20)
                if not chunk:
                    break
                f.write(chunk)
                got += len(chunk)
                if total:
                    self.sub_progress(got / total, f"Downloading ffmpeg (free audio tool): {got * 100 // total}%")
        with zipfile.ZipFile(zpath) as z:
            name = next(n for n in z.namelist() if n.endswith("bin/ffmpeg.exe"))
            (self.work / "ffmpeg").mkdir(exist_ok=True)
            with z.open(name) as src, open(self.work / "ffmpeg" / "ffmpeg.exe", "wb") as dst:
                shutil.copyfileobj(src, dst)
        zpath.unlink()
        self.ffmpeg = self.work / "ffmpeg" / "ffmpeg.exe"
        self.log(f"ffmpeg: {self.ffmpeg}")

    def music(self):
        self.run("doom_music_bank.py")
        self.run("convert_music.py", "--suites")

    def hud(self):
        self.run("convert_hud_textures.py")
        self.run("convert_hud_textures.py", "--settings")
        self.run("convert_hud_textures.py", "--pad")
        self.run("convert_ammo_icons.py")
        self.run("convert_reticles.py")
        self.run("convert_ui.py")
        self.run("convert_font.py")

    def weapons(self):
        self.run("vm/convert_weapon.py", "--prepare")
        done = []
        errors = []

        def one(w):
            try:
                self.run("vm/convert_weapon.py", w)
                done.append(w)
                self.sub_progress(len(done) / len(WEAPONS), f"Guns, arms and animations: {len(done)} of {len(WEAPONS)} done")
            except Exception as e:  # noqa: BLE001
                errors.append(f"{w}: {e}")

        with ThreadPoolExecutor(self.workers) as pool:
            list(pool.map(one, WEAPONS))
        if self.cancelled:
            raise Cancelled()
        if errors:
            raise RuntimeError("; ".join(errors))

    def verify(self):
        need = {"doom_audio": 262, "doom_music": 137, "doom_ui": 297, "doom_fx": 9}
        bad = []
        for d, n in need.items():
            have = sum(1 for p in (self.natives / d).rglob("*") if p.is_file()) if (self.natives / d).exists() else 0
            self.log(f"{d}: {have} files")
            if have < n:
                bad.append(f"{d} has {have} of {n} files")
        for w in WEAPONS:
            if not (self.natives / "doom_vm" / w / "model.bin").exists():
                bad.append(f"doom_vm/{w} missing")
        if bad:
            raise RuntimeError("incomplete: " + ", ".join(bad))

    # -- main
    def install(self):
        self.work.mkdir(parents=True, exist_ok=True)
        log_path = self.target / "setup_log.txt"
        self.target.mkdir(parents=True, exist_ok=True)
        self.log_file = open(log_path, "w", encoding="utf-8")
        self.log(f"DOOM RING {self.version['version']} setup {time.strftime('%Y-%m-%d %H:%M')}: "
                 f"DOOM Eternal {self.doom or '(not installed: update only)'} -> {self.target}")
        if self.update_from:
            self.log(f"update from {self.update_from}" + (" (mod files only)" if self.quick else " (DOOM content rebuilt)"))
        if self.doom is None and not self.quick:
            raise RuntimeError("DOOM Eternal is needed to build DOOM RING's DOOM files - install it through Steam")
        steps = self.steps()
        total = sum(w for w, _, _ in steps)
        done = 0
        t0 = time.time()
        try:
            for weight, text, fn in steps:
                self._base, self._span = done / total, weight / total
                self.on_progress(done / total, text)
                s = time.time()
                fn()
                self.log(f"[{text}: {time.time() - s:.0f} s]")
                done += weight
            (self.target / INSTALLED_FILE).write_text(json.dumps(self.version), encoding="utf-8")
            self.on_progress(1.0, "Done")
            self.log(f"finished in {(time.time() - t0) / 60:.1f} min")
            shutil.rmtree(self.work, ignore_errors=True)
        finally:
            self.log_file.close()
            self.log_file = None
        return log_path


def desktop_shortcut(target):
    """A desktop shortcut to the game's launcher (.lnk through Windows Script Host)."""
    bat = Path(target) / "Play DOOM RING.bat"
    icon = Path(target) / "doomring.ico"

    def q(p):  # PowerShell single-quoted string
        return "'" + str(p).replace("'", "''") + "'"

    # (Windows' own Desktop folder - it is often moved to OneDrive, not USERPROFILE\Desktop)
    ps = ("$d=[Environment]::GetFolderPath('Desktop');$l=Join-Path $d 'DOOM RING.lnk';"
          "$s=(New-Object -ComObject WScript.Shell).CreateShortcut($l);"
          f"$s.TargetPath={q(bat)};$s.WorkingDirectory={q(bat.parent)};"
          + (f"$s.IconLocation={q(icon)};" if icon.exists() else "") + "$s.Save();Write-Output $l")
    r = subprocess.run(["powershell", "-NoProfile", "-Command", ps], creationflags=NO_WINDOW, capture_output=True,
                       text=True)
    lnk = r.stdout.strip().splitlines()[-1] if r.stdout.strip() else ""
    return bool(lnk) and Path(lnk).exists()


def start_thread(fn):
    t = threading.Thread(target=fn, daemon=True)
    t.start()
    return t
