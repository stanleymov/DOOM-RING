DOOM RING - DOOM Eternal's guns, HUD, sounds and music inside ELDEN RING

The DOOM content in this folder was built by the setup from YOUR OWN copy of DOOM Eternal; the
download itself contains no DOOM Eternal or ELDEN RING files. Please don't share this folder (it
holds the converted DOOM files) - share the setup instead.

REQUIREMENTS
- Windows 10/11, Steam logged in, ELDEN RING (base game, 1.16 / app 2.7.1) installed through Steam
- The game runs offline (Easy Anti-Cheat off) - that is required for mods. Never take the mod online.
- The mod uses its own save (DOOMRING.sl2) and never touches your normal ELDEN RING save.

START
- Double-click "Play DOOM RING.bat" (or the desktop shortcut). A command window opens while the mod
  loader starts the game - that's normal.

UPDATING
- Run the new version's setup and keep this folder: it finds DOOM RING here and updates it. Your
  settings, keys and progress are kept. (doomring_version.json says which version is installed.)

SETTINGS / KEYS
- F1 opens the settings window in game: difficulty, chainsaw limit, Crucible hit damage, sound +
  music volume, glory kill stagger, HUD on/off and two keys per action (changes save automatically)
- V draws the Crucible (up to 3 charges; a rare ammo drop). Bosses can't be chainsawed, Crucibled,
  glory killed or Blood Punch killed.
- Doom damage grows 1% per character level from level 9; the chainsaw / Crucible limits follow in
  steps of 100 (the recommended values show at the bottom of the F1 window).
- Each character keeps its own Doom ammo, armor, charges and health (by character name); a new
  character starts with the full Doom kit. Saved in game\natives\doomslayer_save.json.
- game\natives\doomslayer.toml  (keys, damage, movement; most changes apply live)
- game\natives\erfps2.toml      (first-person camera, FOV)
- F9 turns the Doom layer off/on; lock-on with nothing targeted toggles first/third person.
- Music: DOOM Eternal's own combat music (8 of its level suites, played by Doom's own rules)
  starts with a fight and fades out 8 s after it ends; the next fight goes on where it stopped.

CONTROLLER (Xbox, or PlayStation through Steam Input)
- RT fire, LT weapon mod, A jump, B dash, RS click melee, X interact, Y Flame Belch,
  LB chainsaw, RB weapon wheel (hold, right stick picks) / last weapon (tap), D-pad up Crucible,
  View (PS: Create) settings. The HUD and the F1 window switch to button icons when the pad is used.
- In the F1 window: left stick = cursor, A = click, B = close. Rebind in the CONTROLLER column.
- Turn OFF Elden Ring's System > Camera > "Camera Auto Rotation" and "Auto Wall Recovery":
  with them on, the controller camera feels uneven with the Doom movement.
- Click into the game window once after it starts (the Windows cursor can block the pad until it hides).
- PlayStation icons show when a Sony pad is connected (game\natives\doomslayer.toml: pad_icons).

CREDITS
- DOOM RING: the mod and its converters (made with AI help - Claude by Anthropic).
- me3 (mod loader, MIT/Apache-2.0), erfps2 (first person, MIT/Apache-2.0, by Dasaav; our fork).
- Setup tools: SAMUEL (GPL-3.0, brongo) as samuel-cli, ww2ogg (BSD-3, hcs64), ffmpeg (GPL v3, run
  portably by the setup - never installed), Python + numpy / scipy / Pillow.
- ChakraPetch font (SIL Open Font License).
- Full credits and every licence: the "license" folder (CREDITS.txt).
- DOOM Eternal is (c) id Software / Bethesda; ELDEN RING is (c) FromSoftware / Bandai Namco. This is an
  unofficial fan mod, not affiliated with or endorsed by them.
