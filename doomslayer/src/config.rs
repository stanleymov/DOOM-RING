//! `doomslayer.toml` next to the DLL. Reloaded when the file changes, so tuning is live.

use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::SystemTime,
};

use serde::Deserialize;
use windows::Win32::{Foundation::HMODULE, System::LibraryLoader::GetModuleFileNameW};

static MODULE: AtomicUsize = AtomicUsize::new(0);

pub fn set_module(module: usize) {
    MODULE.store(module, Ordering::Relaxed);
}

pub fn mod_dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let mut buf = [0u16; 1024];
        let len = unsafe {
            GetModuleFileNameW(Some(HMODULE(MODULE.load(Ordering::Relaxed) as _)), &mut buf)
        } as usize;
        let path = PathBuf::from(String::from_utf16_lossy(&buf[..len]));
        path.parent().map(PathBuf::from).unwrap_or_default()
    })
    .clone()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Keys {
    pub fire: u16,
    pub alt_fire: u16,
    pub dash: u16,
    pub jump: u16,
    pub melee: u16,
    pub chainsaw: u16,
    pub flame_belch: u16,
    pub weapon_wheel: u16,
    pub last_weapon: u16,
    pub slots: [u16; 8],
    /// Turn the whole Doom layer off/on (play plain Elden Ring for a bit).
    pub doom_toggle: u16,
    /// Stuck in a wall / floor: each press lifts the player 2 m.
    pub unstick: u16,
    /// Mark the enemy under the crosshair: it always shows a health bar (press again to unmark).
    pub mark: u16,
    /// Open / close the settings window.
    pub settings: u16,
    /// The Crucible: draw it / put it away (user: V, taken off the chainsaw's second key).
    pub crucible: u16,
    pub crucible_alt: u16,
    /// Second key for each action (0 = none; the settings window sets them). Mouse back = 0x05,
    /// mouse forward = 0x06, middle = 0x04.
    pub fire_alt: u16,
    pub alt_fire_alt: u16,
    pub dash_alt: u16,
    pub jump_alt: u16,
    pub melee_alt: u16,
    pub chainsaw_alt: u16,
    pub flame_belch_alt: u16,
    pub weapon_wheel_alt: u16,
    pub mark_alt: u16,
    pub doom_toggle_alt: u16,
    pub unstick_alt: u16,
    pub settings_alt: u16,
    /// Elden Ring's interact key (only labels the Doom "[E] INTERACT" prompt).
    pub interact: u16,
    /// Music test keys (audio.rs MUSIC_TEST): on / off, next track, back to the start point,
    /// -5 s, +5 s, mark the start point here. The test is off by default (music_test = 0; 0x76 =
    /// F7 turns it on - user, 2026-10-08).
    pub music_test: u16,
    pub music_next: u16,
    pub music_restart: u16,
    pub music_back: u16,
    pub music_fwd: u16,
    pub music_mark: u16,
    /// Gun inspect debugger (viewmodel.rs: the arrows / 1-6 move and turn the gun). Off by default
    /// (0); 0x79 = F10 turns it on.
    pub inspect: u16,
    /// Controller buttons (gamepad.rs codes 0x200-0x211: their own category, never a keyboard
    /// key; 0 = none). One per action; the settings window shows them in pad mode.
    pub fire_pad: u16,
    pub alt_fire_pad: u16,
    pub dash_pad: u16,
    pub jump_pad: u16,
    pub melee_pad: u16,
    pub chainsaw_pad: u16,
    pub flame_belch_pad: u16,
    pub crucible_pad: u16,
    pub weapon_wheel_pad: u16,
    pub mark_pad: u16,
    pub doom_toggle_pad: u16,
    pub unstick_pad: u16,
    pub settings_pad: u16,
    /// The Doom interact button: Elden Ring sees its own interact button (`pad_er_interact`).
    pub interact_pad: u16,
}

impl Default for Keys {
    fn default() -> Self {
        Self {
            fire: 0x01,         // LMB
            alt_fire: 0x02,     // RMB
            dash: 0xA0,         // Left Shift
            jump: 0x20,         // Space
            melee: 0x46,        // F
            chainsaw: 0x43,     // C
            flame_belch: 0x47,  // G
            weapon_wheel: 0x51, // Q (hold)
            last_weapon: 0,     // (quick swap is a tap of the weapon wheel key now - user)
            slots: [0x31, 0x32, 0x33, 0x34, 0x35, 0x36, 0x37, 0x38],
            doom_toggle: 0x78, // F9
            unstick: 0x77,     // F8
            mark: 0x5A,        // Z
            settings: 0x70,    // F1
            crucible: 0x56,    // V
            crucible_alt: 0,
            fire_alt: 0,
            alt_fire_alt: 0,
            dash_alt: 0,
            jump_alt: 0,
            melee_alt: 0x05,       // mouse back (XButton1)
            chainsaw_alt: 0,       // (V is the Crucible now - user)
            flame_belch_alt: 0x06, // mouse forward (XButton2)
            weapon_wheel_alt: 0x04, // middle mouse (hold)
            mark_alt: 0,
            doom_toggle_alt: 0,
            unstick_alt: 0,
            settings_alt: 0,
            interact: 0x45,         // E (Elden Ring's own binding)
            music_test: 0,       // off (F7 = 0x76)
            music_next: 0x75,    // F6
            music_restart: 0x74, // F5
            music_back: 0x71,    // F2
            music_fwd: 0x73,     // F4
            music_mark: 0x72,    // F3
            inspect: 0,          // off (F10 = 0x79)
            // controller (user, 2026-10-08)
            fire_pad: crate::gamepad::RT,
            alt_fire_pad: crate::gamepad::LT,
            dash_pad: crate::gamepad::B,
            jump_pad: crate::gamepad::A,
            melee_pad: crate::gamepad::RS,
            chainsaw_pad: crate::gamepad::LB,
            flame_belch_pad: crate::gamepad::Y,
            crucible_pad: crate::gamepad::DPAD_UP,
            weapon_wheel_pad: crate::gamepad::RB,
            mark_pad: crate::gamepad::DPAD_DOWN,
            doom_toggle_pad: crate::gamepad::DPAD_RIGHT,
            unstick_pad: crate::gamepad::DPAD_LEFT,
            settings_pad: crate::gamepad::BACK,
            interact_pad: crate::gamepad::X,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    pub keys: Keys,
    /// Multiplies every NpcParam HP once at boot (user asked for tankier enemies).
    pub enemy_hp_mult: f32,
    /// Multiplies Doom weapon damage. Tune together with enemy_hp_mult.
    pub weapon_damage_mult: f32,
    /// Doom movement: root-motion speed multiplier while running on the ground.
    pub move_speed_mult: f32,
    /// Doom ground movement: our own instant-acceleration controller instead of ER's run anims.
    pub doom_move: bool,
    /// In the air, the movement keys / left stick are hidden from Elden Ring (its run animation
    /// kept playing footsteps mid-jump - user, 2026-10-08). Live.
    pub air_hide_move: bool,
    /// Enemies drop real ER items (ammo / health tokens) instead of their vanilla loot.
    pub doom_loot: bool,
    /// Ground speed (m/s), acceleration and stopping (m/s^2), jump apex height (m).
    pub ground_speed: f32,
    /// First-person eye height above the model origin (m); 0 = learn it from the standing pose.
    pub eye_height: f32,
    pub ground_accel: f32,
    pub ground_friction: f32,
    pub jump_height: f32,
    pub dash_distance: f32,
    pub dash_time: f32,
    pub dash_recharge: f32,
    pub double_jump_height: f32,
    /// Gravity used for our part of the double jump (m/s^2).
    pub air_gravity: f32,
    /// Horizontal speed given by the double jump toward the WASD direction (m/s).
    pub double_jump_push: f32,
    /// WASD steering speed while airborne (m/s).
    pub air_control: f32,
    /// Enemy is staggered (glory-killable) below this fraction of max HP.
    pub stagger_hp_frac: f32,
    pub glory_range: f32,
    /// Glory kills teleport you to the demon at ground level (true) or lunge along the ground (false).
    pub glory_teleport: bool,
    /// Teleport glory kills: how far in front of the demon you land (m).
    pub glory_tp_dist: f32,
    /// Teleport glory kills: seconds you stay at the demon after the kill (finisher).
    pub glory_hold: f32,
    /// Teleport glory kills: seconds the left jab plays before the finishing hook lands.
    pub glory_jab_time: f32,
    /// Mouse speed while aiming the Ballista (right click), x normal (1 = unchanged).
    pub ballista_aim_look: f32,
    /// Seconds after a weapon switch before the new gun can fire.
    pub switch_fire_delay: f32,
    /// Seconds after a landing the camera rides the body directly (no stair smoothing); 0 = off.
    pub land_cam_time: f32,
    /// Landing part 1: bring the body down the last bit to the ground at fall speed (off: the
    /// game's own landing; it made jumping on stairs worse - user).
    pub land_settle: bool,
    /// Landing parts 1/2 only for real landings: a jump, or a drop of more than this (m).
    /// Stair steps (<= 0.5 m) stay under it.
    pub land_min_drop: f32,
    /// Jumping uphill: extra lift = this x how fast the ground ahead rises under you (live).
    pub uphill_jump_boost: f32,
    /// Doom landing dip: [max depth m, down s, back up s]; depth scales with the landing speed.
    pub land_bounce: [f32; 3],
    /// Plant models that block movement anyway (big trees), by name prefix, e.g. "AEG801_254".
    pub solid_plants: Vec<String>,
    pub glory_heal_frac: f32,
    /// Enemies above this base (unscaled) HP are heavies/bosses: glory kills only bite.
    pub glory_heavy_hp: f32,
    pub glory_heavy_frac: f32,
    pub chainsaw_range: f32,
    /// Chainsaw viewmodel framing (Doom's handsFovScale 0.65; lower = tighter, less hand).
    pub chainsaw_fov: f32,
    /// Crucible viewmodel framing (same scale as the chainsaw's; live).
    pub crucible_fov: f32,
    /// Crucible reach (m, live), measured to the enemy's body (its radius is added).
    pub crucible_range: f32,
    /// Crucible swing animation speed (1 = as baked; the swings looked twice too fast - user). Live.
    pub crucible_swing_speed: f32,
    /// Crucible cost (settings: CRUCIBLE HIT DAMAGE): base HP under this = 1 charge, under 2x = 2,
    /// anything else (no boss bar) = 3. Its own value, not the chainsaw limit (user).
    pub crucible_hp: f32,
    /// Level scaling (user): every character level above level_base adds this much to all Doom damage
    /// (0.01 = +1% a level; WEAPON DAMAGE still multiplies on top). The chainsaw HP limit and
    /// CRUCIBLE HIT DAMAGE follow the same growth in steps of 100.
    pub level_damage: f32,
    /// The damage reference level: x1.0 there, +level_damage per level above, -level_damage per
    /// level below (WEAPON DAMAGE makes up for it). 9 = where the Doomslayer class starts (it
    /// was 24 while the class started at 24 - user). The chainsaw / Crucible defaults
    /// (1200 / 1500) belong to it too.
    pub level_base: u32,
    /// Unstick key (F8): metres the player is lifted per press (was a fixed 2 - user: 4).
    pub unstick_height: f32,
    /// Controller icons: "auto" (a Sony pad connected = PlayStation), "xbox" or "ps".
    pub pad_icons: String,
    /// Elden Ring's own interact button on the pad (pressed for the Doom interact button).
    pub pad_er_interact: u16,
    pub meathook_range: f32,
    /// Doom melee / Blood Punch power (AtkParam physical correction %).
    pub melee_power: f32,
    pub blood_punch_power: f32,
    /// Pickups closer than this fly to you.
    pub pickup_magnet: f32,
    /// Meathook pull speed (m/s).
    pub meathook_speed: f32,
    pub armor_max: i32,
    /// Health max (shield not included); the game's max HP is scaled to it. 0 = the game's own.
    pub max_health: i32,
    /// Fraction of incoming damage absorbed by armor while it lasts.
    pub armor_absorb: f32,
    pub infinite_ammo: bool,
    /// SpEffect applied to glory-killed enemies for the gore burst (6400 = blood loss proc).
    pub gore_speffect: i32,
    /// Doom sound effects volume (0..1+).
    pub volume: f32,
    /// Doom combat music while fighting.
    pub music: bool,
    pub music_volume: f32,
    /// "doom" (DOOM Eternal's heavy combat suites played by its own playlist rules, doom_music) or
    /// "tracks" (full jukebox tracks from their start points, doom_music_tracks).
    pub music_mode: String,
    /// Doom mode: the suites to pick from (folder names in doom_music; empty = all), live.
    pub music_suites: Vec<String>,
    /// Doom mode: pieces never played, "suite/piece" (the music test's F3 adds the playing one), live.
    pub music_skip: Vec<String>,
    /// Doom mode: open a suite with its intro piece (off: straight into the loop - user).
    pub music_intro: bool,
    /// Doom mode: seconds without a fight after which the next fight starts another suite
    /// (sooner, it goes on where the last fight left it; 0 = never).
    pub music_new_suite: f32,
    /// The fight playlist: "name@start seconds" (the start point = the drop), live.
    pub music_tracks: Vec<String>,
    /// Seconds without any sign of a fight before the music fades (it faded mid-fight at 8 - user).
    pub music_hold: f32,
    /// The Doom HUD (vitals, ammo, abilities, health bars, pickups, messages). Off: only the
    /// crosshair, the Heavy Cannon scope and the interact prompt stay (settings window).
    pub show_hud: bool,
    /// Glory kills (settings window). Off: damage doesn't stop at the stagger window - enemies
    /// just die - and no glory kills are offered.
    pub glory_kills: bool,
    pub bridge: bool,
    /// Hide the Elden Ring body/arms/weapons in first person (shadow stays).
    pub hide_body: bool,
    /// Hide Elden Ring's own HP/FP/stamina + equipment HUD (boss bars stay).
    pub hide_er_hud: bool,
    /// Game speed while the weapon wheel is open (Doom slow-mo).
    pub wheel_slowmo: f32,
    /// 3D viewmodel tuning (view space metres / degrees).
    pub vm_offset: [f32; 3],
    pub vm_pitch: f32,
    pub vm_fov: f32,
    pub vm_light: f32,
    pub vm_ambient: f32,
    pub vm_arms: bool,
    /// Strength of the fake sky reflection on metal.
    pub vm_env: f32,
    /// Gun kick multiplier per weapon (CS, HC, PR, RL, SSG, BAL, CG, BFG); 0 = only Doom's clip.
    pub recoil: [f32; 8],
    /// Weapon sway (live): [tilt degrees at full strafe / fast turn, sideways trail in metres,
    /// jump lift in metres, landing dip in metres]. 0 turns that part off.
    pub sway: [f32; 4],
    /// Walking bob (live): [height, sideways, speed] multipliers; 1 = the old quarter-of-Doom bob.
    pub bob: [f32; 3],
    /// Walking bob, the gun's tip (live): [side swing degrees, up/down nod degrees]. The gun
    /// pivots near the hands, so the muzzle swings the most (like Doom's rotational bob).
    pub bob_tip: [f32; 2],
    /// Walking bob variation (live): 0 = every step identical; 0.3 = each step's size and pace
    /// varies by up to ~30%, blended smoothly between steps.
    pub bob_noise: f32,
    /// Which walking bob (live): 0 = ours (bob / bob_tip / bob_noise), 1 = Doom's own weaponBob.
    pub bob_mode: u32,
    /// Walking bob + sway while aiming (Ballista right click, scopes), live: 1 = unchanged,
    /// 0.25 = a quarter of it.
    pub aim_steady: f32,
    /// HUD look (live): 1 = built from Doom's own HUD textures, 0 = the previous drawn HUD.
    pub hud_style: u32,
    /// Doom HUD live tuning: left / right block [dx, dy, scale] (1080p px), pips [size, spacing px],
    /// sizes [health/armor icons, dash/blood punch rings, ability boxes, numbers].
    pub hud_left: [f32; 3],
    pub hud_right: [f32; 3],
    pub hud_pips: [f32; 2],
    /// Doom HUD pips per row (live): [armor, health].
    pub hud_pip_count: [u32; 2],
    /// Doom HUD per-element tuning (live, [hud] table): name = [move x, move y, scale, opacity].
    pub hud: std::collections::HashMap<String, [f32; 4]>,
    /// Per-sound volume in dB on top of the files ([sound_db] table: event = dB), live. Defaults:
    /// the levelled guns (user, 2026-10-08, from measured loudness).
    pub sound_db: std::collections::HashMap<String, f32>,
    /// Fists / punches field-of-view scale, live (lower = framed tighter; hides the shoulders).
    pub melee_fov: f32,
    /// Melee: each arm pushed this many cm out to its side (left arm left, right arm right), live.
    pub melee_arm_push: f32,
    /// Doom bob (live): [movement size, rotation size, step speed, pivot (0 = camera, 1 = the
    /// gun's visible back end)]. 1, 1, 1 = Doom's numbers.
    pub doom_bob: [f32; 4],
    /// Doom bob per part (live multipliers, 1 = Doom): movement [forward/back, side, up/down].
    pub doom_bob_move: [f32; 3],
    /// Doom bob per part (live multipliers, 1 = Doom): rotation [pitch, yaw, roll].
    pub doom_bob_rot: [f32; 3],
    /// Per-gun position on top of Doom's (view-space metres: right, up, forward; negative forward =
    /// closer to the camera). Order CS, HC, PR, RL, SSG, BAL, CG, BFG.
    pub gun_offset: [[f32; 3]; 8],
    /// Viewmodel look-dev: 0 normal, 1 albedo, 2 normals, 3 no normal map, 4 no reflections,
    /// 5 normal + every hidden mesh drawn.
    pub vm_debug: u32,
    /// Weapon anti-aliasing: MSAA sample count for the gun pass (0/1 = off, 2, 4, 8). Live.
    pub vm_msaa: u32,
    /// Weapon anti-aliasing: FXAA-style edge smoothing over the gun's pixels only. Live.
    pub vm_fxaa: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            keys: Keys::default(),
            enemy_hp_mult: 2.0,
            weapon_damage_mult: 0.5,
            move_speed_mult: 1.35,
            doom_move: true,
            air_hide_move: true,
            doom_loot: true,
            ground_speed: 9.0,
            eye_height: 0.0,
            ground_accel: 80.0,
            ground_friction: 70.0,
            jump_height: 1.5,
            dash_distance: 6.0,
            dash_time: 0.16,
            dash_recharge: 1.25,
            double_jump_height: 2.8,
            air_gravity: 24.0,
            double_jump_push: 5.5,
            air_control: 3.5,
            stagger_hp_frac: 0.2,
            glory_range: 10.0,
            glory_teleport: true,
            glory_tp_dist: 2.0,
            glory_hold: 0.25,
            glory_jab_time: 0.25,
            ballista_aim_look: 0.5,
            switch_fire_delay: 0.2,
            land_cam_time: 0.0,
            land_settle: false,
            land_min_drop: 0.8,
            uphill_jump_boost: 1.5,
            land_bounce: [0.06, 0.06, 0.22],
            solid_plants: Vec::new(),
            glory_heal_frac: 0.15,
            glory_heavy_hp: 1200.0,
            glory_heavy_frac: 0.12,
            chainsaw_range: 3.0,
            chainsaw_fov: 0.65,
            crucible_fov: 0.65,
            crucible_range: 6.0,
            crucible_swing_speed: 0.7,
            crucible_hp: 1500.0,
            level_damage: 0.01,
            level_base: 9,
            unstick_height: 4.0,
            pad_icons: "auto".into(),
            pad_er_interact: crate::gamepad::Y,
            meathook_range: 30.0,
            melee_power: 450.0,
            blood_punch_power: 2400.0,
            pickup_magnet: 4.0,
            meathook_speed: 28.0,
            armor_max: 200,
            max_health: 0,
            armor_absorb: 1.0,
            infinite_ammo: false,
            gore_speffect: 6400,
            volume: 0.5,
            music: true,
            music_volume: 0.2,
            music_mode: "doom".into(),
            music_suites: vec![],
            music_skip: vec![],
            music_intro: false,
            music_new_suite: 20.0,
            music_tracks: vec!["bfg_division@0".into(), "flesh_and_metal@0".into(), "rip_and_tear@0".into()],
            music_hold: 8.0,
            show_hud: true,
            glory_kills: true,
            bridge: true,
            hide_body: true,
            hide_er_hud: true,
            wheel_slowmo: 0.25,
            vm_offset: [0.0, 0.0, 0.0],
            vm_pitch: 0.0,
            vm_fov: 55.0,
            vm_light: 1.6,
            vm_ambient: 0.35,
            vm_arms: true,
            vm_env: 0.35,
            recoil: [1.0; 8],
            // Measured from Doom footage: ~1-1.5 deg tilt and a small trail when strafing or
            // turning, a small lift on jumps and a ~4% screen dip on landing.
            sway: [5.0, 0.004, 0.005, 0.011],
            bob: [1.0, 1.0, 1.0],
            bob_tip: [1.5, 0.5],
            bob_noise: 0.3,
            bob_mode: 1,
            aim_steady: 0.25,
            hud_style: 1,
            hud_left: [0.0, 0.0, 1.0],
            hud_right: [0.0, 0.0, 1.0],
            hud_pips: [1.0, 24.0],
            hud_pip_count: [4, 8],
            hud: std::collections::HashMap::new(),
            sound_db: [
                ("shotgun_fire", -2.5),
                ("rocket_fire", -1.5),
                ("arb_fire", 0.0),
                ("ballista_fire", -2.0),
                ("ssg_fire", 6.0),
                ("bfg_fire", 1.5),
                ("bfg_explode", 6.0),
                ("rocket_explode", 4.0),
                ("sticky_fire", -1.0),
                ("sticky_explode", 4.0),
                ("hc_bolt_fire", 6.5),
                ("heavy_cannon_fire", 2.0),
                ("plasma_fire", -1.0),
                ("flame_belch", -1.5),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect(),
            melee_fov: 1.0,
            melee_arm_push: 17.0,
            doom_bob: [1.0, 1.0, 1.0, 1.0],
            doom_bob_move: [1.0; 3],
            doom_bob_rot: [1.0; 3],
            vm_debug: 0,
            vm_msaa: 4,
            vm_fxaa: false,
            // User: rocket launcher and plasma rifle a bit closer, combat shotgun a bit further.
            gun_offset: [
                [0.0, 0.0, 0.07],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, -0.09],
                [0.0, 0.0, -0.04],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
                [0.0, 0.0, 0.0],
            ],
        }
    }
}

struct Loaded {
    cfg: Arc<Config>,
    stamp: Option<SystemTime>,
}

/// The finished HUD's tuning, built in (tools/hud_bake.py). Anything in doomslayer.toml overrides it.
const HUD_BAKED: &str = include_str!("hud_baked.toml");

/// `over` into `base`: tables merge key by key (`over` wins), everything else is replaced.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

static CONFIG: RwLock<Option<Loaded>> = RwLock::new(None);

fn path() -> PathBuf {
    mod_dir().join("doomslayer.toml")
}

fn stamp() -> Option<SystemTime> {
    std::fs::metadata(path()).and_then(|m| m.modified()).ok()
}

/// The baked HUD with doomslayer.toml on top; None when the file doesn't parse.
fn load() -> Option<Config> {
    let mut base: toml::Table = toml::from_str(HUD_BAKED).unwrap_or_else(|e| {
        log::error!("hud_baked.toml: {e}");
        toml::Table::new()
    });
    if let Ok(text) = std::fs::read_to_string(path()) {
        match toml::from_str::<toml::Table>(&text) {
            Ok(user) => merge(&mut base, user),
            Err(e) => {
                log::error!("doomslayer.toml: {e}");
                return None;
            }
        }
    }
    match toml::Value::Table(base).try_into::<Config>() {
        Ok(c) => Some(c),
        Err(e) => {
            log::error!("doomslayer.toml: {e}");
            None
        }
    }
}

/// Write settings into doomslayer.toml in place (the settings window): each `key = value` line
/// keeps its comment and column; a missing key is added to its section ("" = top level, "keys" =
/// [keys]). The file watcher then reloads it like a hand edit.
pub fn save_values(updates: &[(&str, &str, String)]) {
    let p = path();
    let Ok(text) = std::fs::read_to_string(&p) else { return };
    // keep the file's line endings (a save showed the whole file as changed in git)
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    for (section, key, value) in updates {
        let mut cur = String::new();
        let mut first_header: Option<usize> = None;
        let mut sec_end: Option<usize> = None;
        let mut sec_seen = section.is_empty();
        let mut done = false;
        for i in 0..lines.len() {
            let t = lines[i].trim_start().to_string();
            if t.starts_with('[') {
                first_header.get_or_insert(i);
                if cur == *section && sec_end.is_none() {
                    sec_end = Some(i);
                }
                cur = t.trim_start_matches('[').split(']').next().unwrap_or("").trim().to_string();
                if cur == *section {
                    sec_seen = true;
                }
                continue;
            }
            if cur != *section {
                continue;
            }
            let Some(rest) = t.strip_prefix(*key) else { continue };
            let rest = rest.trim_start();
            let Some(after_eq) = rest.strip_prefix('=') else { continue };
            let indent = &lines[i][..lines[i].len() - t.len()];
            let mut new = format!("{indent}{key} = {value}");
            if let Some(h) = after_eq.find('#') {
                let col = lines[i].len() - after_eq.len() + h;
                let pad = col.saturating_sub(new.len()).max(1);
                new = format!("{new}{}{}", " ".repeat(pad), &after_eq[h..]);
            }
            lines[i] = new;
            done = true;
            break;
        }
        if done {
            continue;
        }
        let line = format!("{key} = {value}");
        if section.is_empty() {
            lines.insert(first_header.unwrap_or(lines.len()), line);
        } else if !sec_seen {
            lines.push(String::new());
            lines.push(format!("[{section}]"));
            lines.push(line);
        } else {
            // end of the section (before trailing blank lines)
            let mut at = sec_end.unwrap_or(lines.len());
            while at > 0 && lines[at - 1].trim().is_empty() {
                at -= 1;
            }
            lines.insert(at, line);
        }
    }
    let mut out = lines.join(nl);
    out.push_str(nl);
    if let Err(e) = std::fs::write(&p, out) {
        log::error!("settings: couldn't write {}: {e}", p.display());
    }
}

/// Last loaded config without touching the file system (for background threads and the HUD,
/// which reads it many times a frame: a shared pointer, nothing is copied).
pub fn get_cached() -> Arc<Config> {
    CONFIG.read().unwrap().as_ref().map(|l| l.cfg.clone()).unwrap_or_default()
}

/// Current config; re-reads the file when its mtime changes.
pub fn get() -> Config {
    let now = stamp();
    if let Some(l) = CONFIG.read().unwrap().as_ref() {
        if l.stamp == now {
            return (*l.cfg).clone();
        }
    }
    let cfg = match load() {
        Some(c) => {
            log::info!("config (re)loaded: {c:?}");
            c
        }
        // A typo in the file: keep the last good settings (it used to fall back to all defaults).
        None => match CONFIG.read().unwrap().as_ref() {
            Some(l) => {
                log::error!("doomslayer.toml has an error - keeping the last good settings");
                (*l.cfg).clone()
            }
            None => Config::default(),
        },
    };
    *CONFIG.write().unwrap() = Some(Loaded { cfg: Arc::new(cfg.clone()), stamp: now });
    cfg
}
