# DOOM Eternal md6 / BIM / snd formats (reverse-engineered 2026-10-09)

Worked out overnight so the installer can build the mod without Vega (closed, GUI only) or
EternalAudioExtractor (no licence). Every reader was checked against Vega's Cast exports / the old
extractions:
- 137 clips: worst 7.6e-6 cm;
- 11 skeletons;
- every gun and arm mesh: positions 1.5e-5, weights exact;
- 262 sound files: byte-identical.

Code: `tools/vm/md6anim.py`, `md6mesh.py`, `md6tex.py`, `doom_res.py`, `doom_source.py`, `tools/doom_snd.py`.

All values are little endian. Raw resources come out with `samuel-cli <archive> raw <dir> <name>`. Later
archives override earlier ones: patch3 > patch2 > patch1 > gameresources, and level archives such as
e1m1_intro hold a few extras.

## md6skl (skeleton, resource type `skeleton`, version 12)
- `u32 names_off, u16 ?, u16 n_bones`, then 20 u16 section offsets at 0x0E (relative to byte 4).
- The arrays are sized for **n rounded up to 8** (`npad`):
  - local rotations `npad x (x, y, z, w)` at 0x44;
  - scales `npad x 3f`;
  - translations `npad x 3f` in metres (Vega shows cm = x100).
- Parents: `n x i16` at `table[1] + 4`.
- `table[0] + 4` holds 3x4 inverse bind matrices.
- Names: from `names_off + 4`, each as `u32 len + ascii`.

## md6anim (animation, type `anim`, version 17)
- Header:
  - `u32 len + skeleton path`;
  - 12 floats (root motion start / end);
  - block `B` at name end + 52.
- At `B`:
  - +0x0C u16 frameset table offset (from `B`);
  - +0x0F flags;
  - +0x10 u16 frames;
  - +0x12 u16 fps;
  - +0x14 u16 frameset count;
  - +0x16 7 u16 offsets (from `B`): [?, ?, const rot, const scale, const trans, ?, ?].
- **Bone lists**: find `u16 1, u16 skeleton hash, u16 0x14 ...`, then 8 u16 offsets from that start.
  - There are 7 lists: const rot, const scale, const trans, (unused), anim rot, anim scale, anim trans.
  - Each list is run-length coded: `u8 total` then `(u8 run length, u8 first bone)` pairs.
- **Quaternions**: 3 x u16, smallest-three.
  - Component = `((v & 0x7FFF) - 16384) / 16384 / sqrt2`.
  - The dropped (largest, positive) component index = `3 - ((b>>15)<<1 | (a>>15))`.
  - The stored three follow it cyclically.
- **Framesets** (long clips are split; 62 frames each here):
  - Each starts with a `"_FRAMESET_"` table of 19 u16 at `fs`.
  - `t[0..11]`:
    - rot first, scale first, trans first;
    - rot keys, scale keys, trans keys;
    - rot masks, scale masks, trans masks.
    - Scale sits between rot and trans. In each pair, the end of one section is the start of the next.
  - `t[16]` size (next set at `fs + t[16]`), `t[17]` start frame, `t[18]` frame count.
  - Per animated bone, the first value is at the set's start frame. Its keys follow bone after bone, at
    the frames its bitmask marks: `ceil(count/8)` bytes, MSB first, one bit per frame of the set.
  - Values are quaternions (6 bytes) or 3 floats (12 bytes).
- Additive clips (the meathook layers) are flagged by path only (`/additive/`).
- Vega's frame count = last key + 1.

## md6mesh (type `baseModel`, version 31)
- The header layout is SAMUEL's MD6.h (skeleton path, bone palette `BoneNumbers`, per mesh: name,
  material decl, MD6_MESH_UNKNOWNS, LOD infos, footer; the stream layouts at the end).
- Geometry: LOD0 from streamdb, Oodle-compressed. Dumped by samuel-cli with `SAMUEL_RAW_GEO=1`
  (`tools/vm/samuel_raw_geo.patch`). Streams (offsets from the stream layout):
  - positions `u16 x,y,z,pad` → `/65535 * vscale + voff`, in metres (no axis swap);
  - normal + tangent 8 bytes:
    - normal `(b - 128) / 127`, normalised;
    - **byte 3 = weights 3 / 4**: high nibble / 45, low nibble / 60;
    - **byte 7 low 7 bits = weight 2 / 254** (bit 7 = tangent sign);
    - weight 1 = the rest;
  - uv `u16 u,v` → `/65535 * uvscale + uvoff`;
  - "colour" = 4 u8 bone slots;
  - faces `u16 x3`, reversed winding for Cast.
- Bone of a slot: the skeleton bone `b` with `BoneNumbers[b] == slot + mesh palette offset`. The
  offset is the 4th u32 of MD6_MESH_UNKNOWNS.

## BIM images (`image`, version 21)
- Header float at byte 32 is a **bias**, at 36 a **scale**. Vega applies them in linear light
  (`srgb(lin(c) * scale + bias)`); SAMUEL doesn't, so its guns come out up to 3x too bright.
- Byte 41 is the format: 0x18 = BC4 (Vega saves greyscale), 0x19 = BC5 (normal; the renderer only
  reads X / Y), 0x21 = BC7 / BC1 sRGB.

## .snd (base/sound/soundbanks/pc)
- Header: `u32 version, u32 info size, u32 header size`, then a `header size` body.
- Then `(info - header) / 32` records:
  - 8 bytes;
  - `u32 source id, u32 size, u32 absolute offset, u32 decoded size`;
  - `u16 format`: 2 = an Ogg Opus stream as is, else a Wwise .wem (music: Wwise Vorbis 0xFFFF → ww2ogg);
  - 6 bytes.
- Names: event names in soundmetadata.bin. The installer doesn't parse them: it uses the ids the mod
  needs (`tools/data/doom_sound_ids.json`, made by `tools/make_sound_tables.py`).
- Without revorb (unclear licence), ww2ogg output decodes the same but drops the last 1024 frames
  of each music piece (~21 ms of tail).
