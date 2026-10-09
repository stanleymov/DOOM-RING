"""Assemble the DOOM RING download: release/DOOM RING Setup/ (and a zip of it).

Only our own work and freely licensed tools go in - never DOOM Eternal or ELDEN RING files:
  Setup DOOM RING.exe   launcher (installer/launcher, Rust)
  runtime/              Python (PSF) + numpy / scipy / Pillow (BSD-style) + tkinter (Tcl/Tk licence)
  setup/                setup.pyw, engine.py, tools/ (our converters), tools/data (sound ids)
  bin/                  samuel-cli.exe (GPL-3, source + our patch in bin/licenses), ww2ogg (BSD-3)
  payload/              the mod: doomslayer.dll + settings, erfps2 (MIT/Apache), me3 (MIT/Apache),
                        Play DOOM RING.bat, README, ChakraPetch fonts (OFL)
bin/ffmpeg.exe: FFmpeg (GPL v3, gyan.dev build), portable - run from bin/ by the setup only.

    python installer/build_package.py [--zip]
"""
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "release" / "DOOM RING Setup"
PORTABLE = REPO / "portable" / "DOOM RING"
PY = Path(sys.base_prefix)
PACKAGES = ["numpy", "numpy.libs", "scipy", "scipy.libs", "PIL"]
STDLIB_SKIP = {"site-packages", "test", "idlelib", "ensurepip", "turtledemo", "lib2to3", "pydoc_data", "__pycache__"}
TOOLS = ["paths.py", "doom_snd.py", "ogg_regranule.py", "extract_sounds.py", "convert_audio.py", "convert_music.py", "music_gains.py",
         "doom_music_bank.py", "doom_music_render.py", "convert_ui.py", "convert_font.py", "convert_fx.py",
         "convert_hud_textures.py", "convert_ammo_icons.py", "convert_reticles.py", "data/doom_sound_ids.json",
         "vm/convert_weapon.py", "vm/md6anim.py", "vm/md6mesh.py", "vm/md6tex.py", "vm/doom_res.py", "vm/doom_source.py"]
SAMUEL = Path.home() / "modtools/samuel-src"
WW2OGG = Path.home() / "modtools/eae/utils"
# the local gyan.dev build (winget) - shipped unmodified with its licence and source note
FFMPEG = next((Path.home() / "AppData/Local/Microsoft/WinGet/Packages").glob("Gyan.FFmpeg*/ffmpeg-*_build"))


def copytree(src, dst, skip=()):
    for p in src.rglob("*"):
        rel = p.relative_to(src)
        if any(part in skip for part in rel.parts):
            continue
        q = dst / rel
        if p.is_dir():
            q.mkdir(parents=True, exist_ok=True)
        else:
            q.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(p, q)


def runtime():
    rt = OUT / "runtime"
    rt.mkdir(parents=True, exist_ok=True)
    for f in ["python.exe", "pythonw.exe", "python3.dll", "python312.dll", "vcruntime140.dll", "vcruntime140_1.dll",
              "LICENSE.txt"]:
        shutil.copy2(PY / f, rt / f)
    copytree(PY / "DLLs", rt / "DLLs")
    copytree(PY / "tcl", rt / "tcl", skip={"demos"})
    copytree(PY / "Lib", rt / "Lib", skip=STDLIB_SKIP)
    site = PY / "Lib" / "site-packages"
    for pkg in PACKAGES:
        copytree(site / pkg, rt / "Lib" / "site-packages" / pkg, skip={"__pycache__", "tests"})
    for dist in site.glob("*.dist-info"):
        if dist.name.split("-")[0].lower() in ("numpy", "scipy", "pillow"):
            copytree(dist, rt / "Lib" / "site-packages" / dist.name)


def setup_files():
    s = OUT / "setup"
    s.mkdir(parents=True, exist_ok=True)
    for f in ("setup.pyw", "engine.py", "version.json"):
        shutil.copy2(REPO / "installer" / f, s / f)
    for t in TOOLS:
        (s / "tools" / t).parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(REPO / "tools" / t, s / "tools" / t)


def binaries():
    b = OUT / "bin"
    b.mkdir(parents=True, exist_ok=True)
    shutil.copy2(SAMUEL / "build" / "samuel-cli.exe", b / "samuel-cli.exe")
    for f in ("ww2ogg.exe", "packed_codebooks_aoTuV_603.bin"):
        shutil.copy2(WW2OGG / f, b / f)
    # portable ffmpeg (user, 2026-10-09): used from bin/ by the setup, never installed on the PC
    shutil.copy2(FFMPEG / "bin" / "ffmpeg.exe", b / "ffmpeg.exe")


def mod_licences(d):
    """Credits + the licences of everything inside the installed mod (user: one "license" folder)."""
    d.mkdir(parents=True, exist_ok=True)
    lic = REPO / "installer" / "licenses"
    shutil.copy2(lic / "CREDITS.txt", d / "CREDITS.txt")
    for f in ("LICENSE-MIT", "LICENSE-APACHE"):
        shutil.copy2(REPO / "erfps2" / f, d / f"erfps2-{f}.txt")
        shutil.copy2(PORTABLE / "me3" / f, d / f"me3-{f}.txt")
    shutil.copy2(REPO / "erfps2" / "fromsoftware-rs" / "LICENSE-MIT", d / "fromsoftware-rs-LICENSE-MIT.txt")
    shutil.copy2(REPO / "erfps2" / "fromsoftware-rs" / "LICENSE-ASL2", d / "fromsoftware-rs-LICENSE-APACHE.txt")
    shutil.copy2(REPO / "vendor" / "hudhook" / "LICENSE", d / "hudhook-LICENSE.txt")
    shutil.copy2(REPO / "vendor" / "hudhook" / "vendor" / "minhook" / "LICENSE.txt", d / "MinHook-LICENSE.txt")
    shutil.copy2(PORTABLE / "game" / "natives" / "doom_ui" / "OFL.txt", d / "ChakraPetch-OFL.txt")


def setup_licences(d):
    """The download's own licence folder: the mod's, plus the setup tools and the Python runtime."""
    mod_licences(d)
    lic = REPO / "installer" / "licenses"
    shutil.copy2(lic / "ww2ogg-COPYING.txt", d / "ww2ogg-COPYING.txt")
    shutil.copy2(SAMUEL / "LICENSE", d / "SAMUEL-GPL-3.0.txt")
    shutil.copy2(REPO / "tools" / "vm" / "samuel_raw_geo.patch", d / "samuel_raw_geo.patch")
    shutil.copy2(REPO / "tools" / "vm" / "samuel_cli_main.cpp", d / "samuel_cli_main.cpp")
    (d / "SAMUEL-source.txt").write_text(
        "bin/samuel-cli.exe is SAMUEL's core (https://github.com/brongo/SAMUEL, GPL-3.0) with a small command line\n"
        "front end (samuel_cli_main.cpp) and one patch (samuel_raw_geo.patch: SAMUEL_RAW_GEO dumps model\n"
        "geometry). Built with clang-cl from SAMUEL's sources + DirectXTex + jsonxx. The complete corresponding\n"
        "source is SAMUEL's repository at commit 01bed65 plus the two files here.\n", encoding="utf-8")
    shutil.copy2(PY / "LICENSE.txt", d / "Python-LICENSE.txt")
    site = PY / "Lib" / "site-packages"
    for dist, name in (("numpy-*.dist-info", "numpy"), ("scipy-*.dist-info", "scipy"), ("pillow-*.dist-info", "Pillow")):
        for di in site.glob(dist):
            for f in [*di.glob("LICENSE*"), *(di / "licenses").rglob("*")]:
                if f.is_file():
                    shutil.copy2(f, d / f"{name}-{f.name}{'' if f.suffix else '.txt'}")
    shutil.copy2(FFMPEG / "LICENSE", d / "ffmpeg-GPL-3.0.txt")
    shutil.copy2(FFMPEG / "README.txt", d / "ffmpeg-build-README.txt")
    (d / "ffmpeg-source.txt").write_text(
        "bin/ffmpeg.exe is FFmpeg 9.0.2, the unmodified static Windows build from www.gyan.dev (GPL v3; build\n"
        "configuration in ffmpeg-build-README.txt). Corresponding source: https://github.com/FFmpeg/FFmpeg/commit/946fcce07b\n"
        "and https://ffmpeg.org/download.html (release 9.0.2); the external libraries' sources are linked from\n"
        "https://www.gyan.dev/ffmpeg/builds/. It is portable: the setup runs it from bin/ while installing - it is\n"
        "not installed on your PC and the mod never uses it.\n", encoding="utf-8")


def payload():
    p = OUT / "payload"
    copytree(PORTABLE, p, skip={"doom_audio", "doom_fx", "doom_music", "doom_vm", "doom_music_tracks"})
    ui = p / "game" / "natives" / "doom_ui"
    for f in list(ui.glob("*")):
        if not (f.name.startswith("ChakraPetch") or f.name == "OFL.txt"):
            f.unlink()
    # the newest build and the release settings (no debug bridge for players)
    shutil.copy2(REPO / "doomslayer" / "target" / "release" / "doomslayer.dll", p / "game" / "natives" / "doomslayer.dll")
    toml = p / "game" / "natives" / "doomslayer.toml"
    t = toml.read_text(encoding="utf-8").replace("\r\n", "\n")
    import re
    t = re.sub(r"(?m)^bridge = true", "bridge = false", t)
    toml.write_text(t, encoding="utf-8")
    mod_licences(p / "license")
    for f in (p / "me3" / "LICENSE-MIT", p / "me3" / "LICENSE-APACHE", ui / "OFL.txt"):
        f.unlink(missing_ok=True)
    for f in ("README.txt",):
        src = REPO / "installer" / "payload_README.txt"
        if src.exists():
            shutil.copy2(src, p / f)


def launcher():
    exe = REPO / "installer" / "launcher" / "target" / "release" / "doomring_setup.exe"
    if exe.exists():
        shutil.copy2(exe, OUT / "Setup DOOM RING.exe")
    else:
        print("!! launcher not built (cargo build --release in installer/launcher)")


def main():
    # (empty it rather than delete it: a shell sitting in the folder would block removing it)
    OUT.mkdir(parents=True, exist_ok=True)
    for child in OUT.iterdir():
        shutil.rmtree(child) if child.is_dir() else child.unlink()
    runtime()
    setup_files()
    binaries()
    payload()
    setup_licences(OUT / "license")
    launcher()
    # the icon (user, 2026-10-09): the wizard window uses <package>/doomring.ico, the desktop shortcut
    # <install>/doomring.ico (the setup exe has it built in)
    icon = REPO / "installer" / "icon" / "doomring.ico"
    shutil.copy2(icon, OUT / "doomring.ico")
    shutil.copy2(icon, OUT / "payload" / "doomring.ico")
    (OUT / "READ ME FIRST.txt").write_text(
        "DOOM RING - setup\n\nDouble-click \"Setup DOOM RING.exe\". You need ELDEN RING and DOOM Eternal installed through\n"
        "Steam. The setup builds the mod from your own DOOM Eternal files - no DOOM files are included here.\n"
        "Command windows may flash while it runs; that is normal.\n\n"
        "UPDATING: run the new setup and keep the same folder - it finds your DOOM RING and updates it\n"
        "(your settings, keys and progress are kept).\n", encoding="utf-8")
    size = sum(f.stat().st_size for f in OUT.rglob("*") if f.is_file()) / 1e6
    print(f"{OUT}: {size:.0f} MB")
    if "--zip" in sys.argv:
        import json
        version = json.loads((REPO / "installer" / "version.json").read_text(encoding="utf-8"))["version"]
        z = OUT.parent / f"DOOM RING Setup {version}.zip"
        with zipfile.ZipFile(z, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
            for f in OUT.rglob("*"):
                if f.is_file():
                    zf.write(f, f.relative_to(OUT.parent))
        print(f"{z}: {z.stat().st_size / 1e6:.0f} MB")


if __name__ == "__main__":
    main()
