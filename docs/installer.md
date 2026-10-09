# DOOM RING installer ("bring your own DOOM files")

Built overnight 2026-10-08/09. The user's requirements are in memory `doom-ring-installer`. Nothing
is published.

## What players download
`release/DOOM RING Setup/` (built with `python installer/build_package.py [--zip]`), about 205 MB. It
contains no DOOM Eternal or ELDEN RING files.

| Part | What | Licence |
|---|---|---|
| `Setup DOOM RING.exe` | launcher, no console (`installer/launcher`, Rust); icon: TODO (user makes it) | ours |
| `runtime/` | Python 3.12 + numpy, scipy, Pillow, tkinter (copied from this PC's Python) | PSF / BSD / HPND / Tcl |
| `setup/` | `setup.pyw` (wizard), `engine.py`, `tools/` (our converters), `tools/data/doom_sound_ids.json` | ours |
| `bin/samuel-cli.exe` | SAMUEL core + our CLI + raw-geometry patch (`bin/licenses`) | GPL-3.0 |
| `bin/ww2ogg.exe` + codebooks | Wwise Vorbis -> Ogg (music) | BSD-3 (TODO: full COPYING text) |
| `payload/` | doomslayer.dll + settings (bridge off), erfps2, me3, `Play DOOM RING.bat`, README, ChakraPetch (OFL) | ours / MIT-Apache / OFL |

- ffmpeg is used from the PC (`PATH`) or downloaded by the setup from BtbN's GitHub build. It is GPL,
  so we don't ship it.
- EternalAudioExtractor (no licence) and Vega (closed) are no longer needed: our own readers replace
  them (docs/md6_formats.md).

## Wizard (installer/setup.pyw)
1. **Welcome**: what it does, needs (both games via Steam, ~3 GB, 15-45 min), the **command-window
   warning** (user requirement), offline / own-save note.
2. **Your games**: Steam detection (registry + libraryfolders.vdf + appmanifest; ELDEN RING 1245620,
   DOOM Eternal 782330).
   - Checks eldenring.exe (version 2.7.1 note) and the DOOM files it needs.
   - If a game is missing, Next is blocked. Each missing game gets its own explanation: "install it
     through Steam, then press Check again" (or Browse).
3. **Location**: default `<Steam>\steamapps\common\ELDEN RING\DOOM RING` (user, 2026-10-09). The
   fallback is `%USERPROFILE%\Games\DOOM RING`. It does a free-space check and a write test (with an
   "administrator" hint). It refuses the DOOM Eternal folder and ELDEN RING's Game folder.
   - Desktop shortcut: our own check box (✔), ticked from the start. It uses Windows' real Desktop folder,
     which may be redirected to OneDrive.
4. **Building**: progress bar, step text and a scrolling log; the log is also written to `<install>\setup_log.txt`.
5. **Done**: Play / open folder, controller camera tip.

## Engine steps (installer/engine.py)
- Work folder: `%LOCALAPPDATA%\DRSetup` (short, because SAMUEL's export paths hit the 260-character
  limit otherwise). It is deleted after a successful install.
- Steps, with the time each took in the 2026-10-09 test on this PC:

| Step | Tool | Time |
|---|---|---|
| copy payload | | 0 s |
| ffmpeg check / download | | |
| sounds | `extract_sounds.py` (our .snd reader) | 12 s |
| sound effects | `convert_audio.py` | 8 s |
| music | `doom_music_bank.py` + `convert_music.py --suites` | 63 s |
| HUD | `convert_hud_textures` (+ `--settings`, `--pad`), `convert_ammo_icons`, `convert_reticles`, `convert_ui`, `convert_font` | |
| FX | `convert_fx` | |
| weapons | `vm/convert_weapon.py --prepare`, then one process per weapon (up to 4 at once) | |
| verify | | |

- Every converter reads its locations from `tools/paths.py` (`DOOMRING_DOOM / WORK / OUT / SAMUEL /
  FFMPEG`). The defaults are this PC's, so development works as before.
- The weapons use `DOOMRING_VM_SOURCE=doom`.

## Verified (2026-10-09, against dist/natives)
- doom_audio: 262 / 262 files byte-identical.
- doom_ui: 294 / 294 DOOM files identical (plus the ChakraPetch files from the payload).
- doom_fx: 9 / 9 identical.
- doom_music: same 137 files, identical playlist trees and cues.
  - The music sources match EternalAudioExtractor's sample for sample: ww2ogg + our own granule fix
    (`tools/ogg_regranule.py`) replaces revorb.
  - Suite gains within 0.4 dB: dist's were measured before the intro suite and 3 pieces were removed.
- doom_vm:
  - meshes within 1e-5 cm, clips within 1e-6 (after fixing a shared-clip bug);
  - info.json equal apart from float noise;
  - used textures within BC-decoder rounding; normal maps differ only in the blue channel, which the
    shader doesn't read;
  - dist's leftover textures of dropped mod parts aren't produced (nothing loads them);
  - dist's textures are the un-dilated originals, so the installer doesn't dilate.

## Test runs (2026-10-09, this PC)
- The engine with the packaged runtime installed into a fresh folder in **4.4 min**:
  - sounds 12 s, effects 8 s, music 63 s, HUD 33 s, FX 1 s, weapons 145 s (4 at once).
  - The work folder was deleted afterwards.
- ELDEN RING launched from the installed copy (its own me3 + profile, saves backed up first):
  - 8 music suites, 262 sounds, 291 UI textures, both fonts and all 11 gun / arm models loaded;
  - fight music started;
  - screenshots in evidence/installer (Super Shotgun, BFG, Ballista) look like the dev build.
- The wizard pages were rendered and checked with the packaged runtime: game detection, missing
  games (Next blocked with explanations), location / space, done page.

## Updates (1.1, 2026-10-09)
- `installer/version.json`: `version` (shown in the wizard, exe version info, zip name) and `content`.
  Bump `content` only when the converted DOOM files change (converters, sound / clip lists).
- A finished setup writes `<install>/doomring_version.json`. A complete install without it is taken as 1.0,
  content 1 (all doom_* folders, the DLL and all 11 doom_vm models present).
- The location page finds an existing install in the chosen folder and the button becomes **Update**:
  - same content number: only the mod files are copied, then verify (under a second in the test);
  - another content number: the full build runs again;
  - no or damaged version file (and not a complete 1.0): a normal install.
  - doomslayer.toml / erfps2.toml and doomslayer_save.json are always kept; the DLL adds new settings keys
    to an old toml by itself.
- Tested: the packaged engine updated a copy of a 1.0 install (portable) to 1.1. The DLL was replaced,
  the settings marker and save were kept, and the version file was written. The wizard's text and
  button were checked headless for 1.1 / 1.0 / old-content / damaged / empty folders.

## Licences and credits
- One `license` folder (user):
  - in the download: credits, the mod's licences, the setup tools' (SAMUEL GPL-3 + source note, ww2ogg
    BSD-3), and Python / numpy / scipy / Pillow;
  - in the installed mod: credits and the licences of what ships inside it (me3, erfps2,
    fromsoftware-rs, hudhook, MinHook, ChakraPetch OFL).
- `installer/licenses/CREDITS.txt`: mods and libraries used, research references (TGA cheat table,
  Erd-Tools' pattern for the auto pickup, Smithbox / WitchyBND / UXM, Vega, EternalAudioExtractor,
  SAMUEL), setup tools, trademark notice.

## Byte check vs dist (2026-10-09)
- Byte-identical: audio 262 / 262, UI 297 / 297, FX 9 / 9.
- Music: 129 / 129 pieces identical apart from the random Ogg stream serial and the page CRCs that
  depend on it (the audio data is the same). suite.json differs only in the suite gains (-0.39 .. +0.05 dB).
- Guns: model.bin within float rounding; the 242 used textures differ by 0.125 / 255 on average
  (median; worst 3.8 / 255 average, on the Ballista string's normal map).

## ffmpeg
Portable: if the PC has none, the setup downloads it into its temporary work folder, uses it there
and deletes the folder at the end. Nothing is installed, and no PATH or registry change is made. The
mod itself never needs ffmpeg.

## Open items
- ffmpeg download: written but untested. This PC has ffmpeg, and no download was made without the
  user's OK.
- (done 2026-10-09: icon in the exe, window and shortcut)
- A test on a PC without Python / ffmpeg (the other PC).
