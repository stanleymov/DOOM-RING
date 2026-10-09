# Building DOOM RING

Windows, 64-bit. The DLLs are Rust (MSVC target); the setup is Python 3.12 with numpy, scipy, Pillow
and tkinter.

## 1. Get the source

```
git clone --recursive https://github.com/stanleymov/DOOM-RING.git
```

`erfps2/fromsoftware-rs` is a submodule (Dasaav's fromsoftware-rs, pinned).

## 2. The DLLs

```
cd erfps2      && cargo build --release
cd doomslayer  && cargo build --release
```

- `doomslayer/target/release/doomslayer.dll`
- `erfps2/target/release/erfps2.dll`

The shaders are compiled already (`doomslayer/shaders/*.cso`, from the `.hlsl` next to them).

## 3. The setup package

`installer/build_package.py` builds `release/DOOM RING Setup/` from:
- `portable/DOOM RING/`: start from `portable-template/DOOM RING/`. Add me3's `bin/` and licence files
  into `me3/` ([me3 releases](https://github.com/garyttierney/me3/releases)), and the two DLLs plus
  `dist/natives/*.toml` into `game/natives/`.
- SAMUEL built with our CLI front end and patch (`tools/vm/samuel_cli_main.cpp`,
  `tools/vm/samuel_raw_geo.patch`) as `samuel-cli.exe`.
- ww2ogg with its packed codebooks, and an ffmpeg build.
- The launcher: `cargo build --release` in `installer/launcher` (it embeds the icon with `llvm-rc`).

The tool locations are set at the top of `build_package.py` and in `tools/paths.py` (`DOOMRING_*`
environment variables). Then:

```
python installer/build_package.py --zip
```

`installer/version.json` holds the version. Bump `content` only when the converted DOOM files change:
an update then rebuilds them.

## Converters without the setup

Every converter in `tools/` reads its locations from `tools/paths.py`: `DOOMRING_DOOM` (DOOM Eternal
folder), `DOOMRING_WORK`, `DOOMRING_OUT`, `DOOMRING_SAMUEL`, `DOOMRING_FFMPEG`. `installer/engine.py`
shows the order the setup runs them in.

Never commit converted files. `dist/natives/doom_*` is ignored for that reason.
