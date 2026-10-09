# How DOOM Eternal's music works (reverse-engineered 2026-10-08)

Source: `DOOMEternal/base/sound/soundbanks/pc/mus.pck` (Wwise AKPK, one bank, Wwise version 135).
HIRC: 1528 MusicSegment (type 10), 1550 MusicTrack (11), 41 MusicSwitch (12), 198 MusicRanSeq
playlists (13), 61 Events, 62 Actions, 46 States. Parsing scratch: `~/doom-extract/mus_hirc.pkl`.

## Records (v135, little endian)
- **MusicTrack (11)**: id u32, flags u8, numSources u32, sources 14 B (plugin u32, streamType u8,
  sourceID u32, memSize u32, bits u8), numPlaylistItems u32, items 44 B (trackID u32, sourceID u32,
  eventID u32, fPlayAt f64, fBeginTrimOffset f64, fEndTrimOffset f64, fSrcDuration f64; all ms).
  sourceID = the `_id#N` in the extracted `~/doom-extract/music/music/*.ogg` names (882 of 1468 match;
  the rest DLC / patches).
- **MusicSegment (10)**: read from the END: ... f64 gridPeriod, f64 gridOffset, f32 tempo, u8 beats,
  u8 beatValue, u8 meterFlag, u32 numStingers(0), **f64 duration**, u32 numMarkers(2), markers 16 B
  each (id u32, f64 position, u32 nameLen 0). Every segment has exactly 2 markers: ENTRY cue and
  EXIT cue. Children = MusicTrack ids (found by scanning the body). 1449 of 1527 are 120 BPM 4/4.
  e.g. metal_hell heavy_0: 33192 ms long, entry 851, exit 29787 (pre-entry 0.85 s, post-exit 3.4 s
  tail). supergorenest pieces: entry 1111, long ~4.4 s tails.
- **MusicRanSeq (13)**: playlist tree at the end of the body, items 30 B: segmentID u32 (0 = group),
  playlistItemID u32, numChildren u32, eRSType i32 (0 continuous sequence, 1 step sequence,
  2 continuous random, 3 step random, -1 leaf), loop i16 (0 = forever), loopMin i16, loopMax i16,
  weight u32 (50000), avoidRepeat u16, usingWeight u8, shuffle u8. Children follow depth-first.
  The root is the outermost tree that parses to the exact end of the body.
- **Transition rules** (MusicTransNodeParams, after the meter block: f64 grid 1000, f64 offset,
  f32 tempo, u8 beats, u8 beatValue, u8 flag, u32 numStingers, u32 numRules). The heavy playlists
  have one rule, any -> any: source syncType 7 (exit cue), playPostExit 1, no fade; destination
  entry cue, playPreEntry 1, no fade, no transition segment. So cue to cue with lead-in and tail
  is exactly Doom's rule. The metal_hell heavy tracks have one clip each and no clip automation.
- **MusicSwitch (12)**: holds the suite's state playlists (ambient / light / heavy); transition
  rules not decoded yet.

## How a suite plays (metal_hell heavy, playlist 734553204)
continuous sequence:
1. intro: step random 1 of (heavy 11 / 16 / 21)
2. loop forever, continuous sequence:
   a. 1 of (19 / 18)
   b. 1 of (9 / 3 / 8)
   c. 1 of 4 variations: [23 then 1 of (12,1,7,20,13)] | [6 then 1 of (4,17,10,22)] |
      [2 random of (23,12,1,7,20,13)] | [2 random of (6,4,17,10,22)]
   d. 1 of (15 / 14 / 0 / 5)
Light (377613250): step random forever over 8 pieces; ambient (747382736): over 3. Avoid repeat 1.

## Playback rule (why ours sounded like jingles)
Pieces are joined cue to cue: the next piece starts so its ENTRY cue lands on the current piece's
EXIT cue; the current piece's post-exit tail keeps playing over it. We played whole files back to
back at random: every lead-in and tail exposed, no structure.

## Picks (user, 2026-10-08)
Heavy suites kept: supergorenest suite 1, metal_hell, doom_hunter, slayer_city, cultist_base,
hub, samuelsbase, mars_core_phobos. Dropped: supergorenest suite 2, maykr, intro; pieces removed: convert_music.py REMOVE. Tensions wanted: light
and heavy, or heavy only. Intro: maybe skip (`tools/doom_music_render.py ... --no-intro`).

## In the mod (music_mode = "doom", the default)
- Pipeline: `tools/doom_music_bank.py` (mus.pck -> `~/doom-extract/mus_hirc.pkl`), then
  `tools/convert_music.py`. That renders each piece of the 9 heavy suites on its own timeline to
  `dist/natives/doom_music/<suite>/<piece>.ogg` (48 kHz) and writes `suite.json` (tree, entry / exit
  cues in ms, suite gain_db: each suite's median loudness to the median of all, -7.9 LUFS).
  `--levels` redoes only the gains.
- `audio.rs` `init_music_doom`:
  - `Walker` walks the tree (step random with avoid repeat, loop counts, then continues from the
    innermost loop-forever node);
  - `MusicMixer` has its own clock: each piece starts where its ENTRY cue meets the last EXIT cue,
    and tails overlap. A peak limiter sits on the output;
  - when a fight ends, the music fades out over 4 s and the clock pauses; the next fight fades in
    from the same spot. After `music_new_suite` s (20) without a fight, a new suite starts.
- Live settings:
  - `music_intro` (false);
  - `music_suites` (empty = all);
  - `music_skip` ("suite/piece").
- Music test keys:
  - F7 turns the test on or off (the key is off by default: set `music_test = 0x76` in [keys]);
  - F6 next suite;
  - F5 restarts the suite;
  - F4 next piece;
  - F3 skips the playing piece for good (adds it to `music_skip`).
