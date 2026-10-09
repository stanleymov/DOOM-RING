# DOOM RING

DOOM Eternal's guns, HUD, sounds and combat music inside ELDEN RING: Doom movement (dash, double
jump), glory kills, chainsaw, Blood Punch, Flame Belch, the Crucible, and 8 of DOOM Eternal's combat
suites played by Doom's own music rules.

> **Unofficial fan mod.** Not affiliated with or endorsed by id Software, Bethesda, FromSoftware or
> Bandai Namco. DOOM Eternal © id Software / Bethesda. ELDEN RING © FromSoftware / Bandai Namco.

## No game files here: bring your own

This repository holds **code only**. It contains no DOOM Eternal or ELDEN RING files and no files
converted from them. The setup builds DOOM RING's DOOM content on your PC from **your own** DOOM
Eternal install, like other "bring your own game files" projects.

You need:
- Windows 10/11 and Steam;
- ELDEN RING (base game, app version 2.7.1);
- DOOM Eternal (base game; no DLC is used);
- about 3 GB of free space while installing.

## Install

1. Download `DOOM RING Setup x.x.zip` from [Releases](../../releases) and unzip it.
2. Run `Setup DOOM RING.exe`. It finds both games through Steam and builds the mod (5-20 minutes).
   Command windows may flash while it runs; that's normal.
3. Start the game with **Play DOOM RING** (desktop shortcut or the .bat in the install folder).

To update, run the new version's setup and keep the same folder. It finds your install and updates
it, keeping your settings, keys and progress.

## Offline only

DOOM RING runs ELDEN RING **offline** with Easy Anti-Cheat off, through the me3 mod loader, as every
ELDEN RING mod of this kind does. It uses its own save file (`DOOMRING.sl2`) and never touches your
normal save. **Never take a modded game online.**

## What's in here

| Folder | What |
|---|---|
| `doomslayer/` | the mod: a Rust DLL loaded by me3 (Doom movement, weapons, HUD, music, settings window) |
| `erfps2/` | our fork of Dasaav's [erfps2](https://github.com/Dasaav-dsv/erfps2) first-person camera (eye-height lock) |
| `tools/` | converters that read DOOM Eternal's files: sounds and music (`.snd`, Wwise), HUD textures, fonts, guns / arms / animations (md6 meshes, skeletons, animations) |
| `installer/` | the setup: wizard (Python / tkinter), engine, package builder, Rust launcher |
| `docs/` | file format notes (`md6_formats.md`), DOOM's music system, the installer |
| `vendor/hudhook/` | hudhook (MIT), the D3D12 overlay library |
| `dist/`, `portable-template/` | the me3 launch profile and default settings |

Building from source: see [BUILDING.md](BUILDING.md).

## Credits

See [CREDITS.txt](installer/licenses/CREDITS.txt). In short:
- [me3](https://github.com/garyttierney/me3) (mod loader);
- [erfps2](https://github.com/Dasaav-dsv/erfps2) by Dasaav;
- [fromsoftware-rs](https://github.com/Dasaav-dsv/fromsoftware-rs);
- [hudhook](https://github.com/veeenu/hudhook) with MinHook;
- [SAMUEL](https://github.com/brongo/SAMUEL) (GPL-3.0; the setup ships it as `samuel-cli`);
- ww2ogg (BSD-3) and ffmpeg (GPL v3), used by the setup;
- the Chakra Petch font (SIL OFL).

**Made with AI.** DOOM RING's code was written with Claude (Anthropic) under the author's direction
and testing.

## Licence

DOOM RING's own code: MIT ([LICENSE](LICENSE)). The erfps2 fork, fromsoftware-rs, hudhook and the
setup tools keep their own licences (in their folders and in `installer/licenses`).
