"""Where the converters read and write. Defaults = the development PC; the installer sets the
DOOMRING_* environment variables (the player's DOOM Eternal install, a work folder, the output).
"""
import os
import shutil
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
DOOM = Path(os.environ.get("DOOMRING_DOOM", "C:/Program Files (x86)/Steam/steamapps/common/DOOMEternal"))
DOOM_BASE = DOOM / "base"
# work files (extracted sounds / music, SAMUEL exports); never shipped
WORK = Path(os.environ.get("DOOMRING_WORK", Path.home() / "doom-extract"))
# the mod's natives folder (doom_audio, doom_music, doom_ui, doom_fx, doom_vm)
NATIVES = Path(os.environ.get("DOOMRING_OUT", REPO / "dist" / "natives"))
SAMUEL = Path(os.environ.get("DOOMRING_SAMUEL", Path.home() / "modtools/samuel-src/build/samuel-cli.exe"))
EAE = Path(os.environ.get("DOOMRING_EAE", Path.home() / "modtools/eae/EternalAudioExtractor.exe"))
FFMPEG = os.environ.get("DOOMRING_FFMPEG") or shutil.which("ffmpeg") or "ffmpeg"
FFPROBE = os.environ.get("DOOMRING_FFPROBE") or shutil.which("ffprobe") or "ffprobe"
# no console window per tool run (Windows)
NO_WINDOW = 0x08000000 if os.name == "nt" else 0

if os.name == "nt":
    # Every helper (samuel-cli, ffmpeg, ww2ogg) runs without a console window: started from the
    # windowless setup, a console tool would otherwise open its own window each time.
    import subprocess as _sp

    _popen_init = _sp.Popen.__init__

    def _no_window_init(self, *args, **kwargs):
        if not kwargs.get("creationflags"):
            kwargs["creationflags"] = NO_WINDOW
        _popen_init(self, *args, **kwargs)

    if getattr(_sp.Popen.__init__, "__name__", "") != "_no_window_init":
        _sp.Popen.__init__ = _no_window_init
