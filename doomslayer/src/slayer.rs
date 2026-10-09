//! The Doom Slayer: per-frame state machine.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::Instant,
};

use glam::{Quat, Vec3};

use crate::{
    audio, bridge, bullet, damage, pickups::{self, Kind, Pickups}, viewmodel,
    config::{self, Config},
    game::{self, Enemy},
    input::{self, Input},
    params::{self, BULLET_SLOTS},
    raycast,
    weapons::{Ammo, SUPER_SHOTGUN, WEAPONS},
};

/// Procedural gun kick per weapon slot: (impulse per shot, push back toward the camera m, muzzle
/// up deg, sideways deg) at a kick of 1, plus a fast per-shot snap straight back (m) for
/// automatics. User tuning: Combat Shotgun comes back at you hard, Heavy Cannon recoils
/// straight back (no bounce up), Plasma Rifle only a light, smooth push. Others keep only
/// Doom's own clip kick. Scaled per weapon by doomslayer.toml `recoil`.
const RECOIL: [(f32, f32, f32, f32, f32); 8] = [
    (40.0, 0.17, 4.0, 0.8, 0.0),    // combat shotgun
    (0.0, 0.065, 0.2, 0.3, 0.0),    // heavy cannon (envelope, see push; closer gun = less)
    (6.0, 0.018, 0.6, 0.2, 0.004),  // plasma rifle
    (0.0, 0.0, 0.0, 0.0, 0.0),      // rocket launcher
    (0.0, 0.0, 0.0, 0.0, 0.0),      // super shotgun
    (0.0, 0.26, 2.0, 0.4, 0.0),     // ballista (envelope, see push: back toward the camera)
    (0.0, 0.0, 0.0, 0.0, 0.0),      // chaingun
    (0.0, 0.0, 0.0, 0.0, 0.0),      // BFG
];
/// Doom's handsFovScale per weapon slot (weapon decls: base/shotgun 0.6, player/heavy_cannon
/// 0.633, plasma + rocket launcher inherit base/default = 1, double_barrel 0.86, ballista 0.65,
/// chaingun 0.654, bfg_base 0.65): each gun's weapon-view FOV is vm_fov times this.
/// Doom's weaponBob per gun (weapon decls, each merged over base/default.decl): movement
/// amplitudes (m) [forward/back, side, up/down] and their speeds; rotation amplitudes (deg)
/// [pitch, yaw, roll], their speeds and phase angles (deg). Phase = distance walked / stride.
struct DoomBob {
    ta: [f32; 3],
    tv: [f32; 3],
    ra: [f32; 3],
    rv: [f32; 3],
    rp: [f32; 3],
}
const DOOM_BOB_STRIDE: f32 = 0.5;
const DOOM_BOB_DEFAULT: DoomBob = DoomBob {
    ta: [0.0, 0.0, 0.004],
    tv: [0.0, 0.45, 0.9],
    ra: [0.2, 0.0, 2.0],
    rv: [0.9, 0.45, 0.45],
    rp: [180.0, -90.0, 0.0],
};
const DOOM_BOB: [DoomBob; 8] = [
    // Combat Shotgun (base/shotgun)
    DoomBob { ta: [0.0, 0.001, 0.0035], ra: [0.25, 0.4, 2.0], ..DOOM_BOB_DEFAULT },
    // Heavy Cannon (base/assault_rifle)
    DoomBob { ta: [0.0, 0.0, 0.003], tv: [0.0, 0.0, 0.9], ra: [0.5, 0.4, 0.0], rv: [0.9, 0.45, 0.0], rp: [90.0, -90.0, 0.0] },
    // Plasma Rifle (base/plasma_rifle)
    DoomBob { ta: [0.002, 0.0, 0.004], ra: [0.5, 0.6, 0.8], rv: [0.25, 0.5, 0.45], rp: [180.0, -90.0, 180.0], ..DOOM_BOB_DEFAULT },
    // Rocket Launcher (base/rocket_launcher = default)
    DOOM_BOB_DEFAULT,
    // Super Shotgun (base/double_barrel)
    DoomBob { ta: [0.002, 0.0, 0.001], tv: [0.8, 0.4, 0.8], ra: [0.6, 0.4, 0.0], rv: [1.0, 0.5, 0.0], rp: [90.0, -90.0, 0.0] },
    // Ballista (base/gauss_rifle)
    DoomBob { ta: [0.002, 0.004, 0.004], tv: [0.45, 0.45, 0.9], ra: [0.25, 0.7, 0.0], rv: [0.3, 0.45, 0.0], ..DOOM_BOB_DEFAULT },
    // Chaingun (player/chaingun)
    DoomBob { ra: [0.4, 0.0, 2.0], ..DOOM_BOB_DEFAULT },
    // BFG 9000 (player/bfg_base)
    DoomBob { ra: [0.2, 0.0, 1.65], ..DOOM_BOB_DEFAULT },
];

const HANDS_FOV: [f32; 8] = [0.6, 0.633, 1.0, 1.0, 0.86, 0.65, 0.654, 0.65];
const TURRET_MUZZLES: [&str; 4] = ["fx_muzzle_123_1", "fx_muzzle_123_3", "fx_muzzle_123_2", "fx_muzzle_123_4"];
const BFG: usize = 7;
/// Meathook recharge after a hook (user: 3 s).
const MEATHOOK_COOLDOWN: f32 = 3.0;
/// Flaming Hook burn (Doom: double_barrel_meat_hook_flame, 1500 ms).
const HOOK_BURN_TIME: f32 = 1.5;
/// Weapon wheel key: held longer than this opens the wheel, a shorter tap swaps weapons.
const WHEEL_HOLD: f32 = 0.2;
/// Ballista: cells per shot for both the normal shot and the Arbalest (user).
const ARBALEST_AMMO: i32 = 25;
/// Precision Bolt recharge (user: 1.25 s); the scope ring fills over it.
pub const BOLT_RECOVERY: f32 = 1.25;
/// BFG wind-up before the shot (Doom's, measured from a recording: ~0.45 s).
const BFG_CHARGE: f32 = 0.45;

const DASH_CHARGES: f32 = 2.0;
const CHAINSAW_PIPS: f32 = 1.0;
const CHAINSAW_REGEN: f32 = 1.0 / 60.0; // the one charge refills in 1 min
pub const BELCH_COOLDOWN: f32 = 15.0;
/// The Crucible: charges it holds, its reach (user).
pub const CRUCIBLE_MAX: u32 = 3;
/// in-control seconds after a boss dies in which a new boss bar counts as its next stage (Rennala:
/// her 2nd stage's cutscene starts ~4 s after the 1st dies; cutscenes stop the clock)
pub const BOSS_STAGE: f32 = 30.0;
/// Recommended chainsaw HP limit / CRUCIBLE HIT DAMAGE for the character's level (the defaults
/// grown by the level multiplier, rounded down to the 100), packed (saw << 32 | crucible) for
/// the settings window.
pub static REC_LIMITS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Damage multiplier for a character level (user: grows with the character): x1.0 at
/// level_base (the level everything was tuned at), +/- level_damage per level from there.
pub fn level_mult(cfg: &Config, level: u32) -> f32 {
    (1.0 + cfg.level_damage.max(0.0) * (level as f32 - cfg.level_base as f32)).max(0.1)
}

/// The defaults (1200 / 1500) grown by the level multiplier, rounded DOWN to the 100 (user:
/// 1404 -> 1400; the step comes once the value has passed it).
pub fn rec_limits(cfg: &Config, level: u32) -> (u32, u32) {
    let d = Config::default();
    let m = level_mult(cfg, level);
    let up = |v: f32| (((v * m) / 100.0).floor() * 100.0).clamp(100.0, 10000.0) as u32;
    (up(d.glory_heavy_hp), up(d.crucible_hp))
}
const BURN_TIME: f32 = 8.0;
const GLORY_LUNGE_TIME: f32 = 0.1;
/// Havok collision filter used by erfps2 for camera/world casts.
/// Bridge test walk: (degrees right of the camera's forward, until). Real input is ignored by the
/// game when injected, so movement tests drive the controller's wish direction directly.
pub static AUTOWALK: std::sync::Mutex<Option<(f32, Instant)>> = std::sync::Mutex::new(None);
/// Bridge `mtrace on|off`: per-frame movement / camera height log (stair jitter analysis).
pub static TRACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Bridge test walk toward a world point (x, z) until reached or the deadline.
pub static WALKTO: std::sync::Mutex<Option<(f32, f32, Instant)>> = std::sync::Mutex::new(None);

const WORLD_FILTER: u32 = 0x2000058;
/// How fast air momentum turns toward the steering direction (m/s per s): a 90 degree turn at
/// running speed takes ~0.3 s.
const AIR_STEER: f32 = 45.0;
/// Harmless ambient animals (chr ids, vawser/ER-Documentation "Info - Chr IDs"): hawk, deer,
/// owl, goat, gull, dragonfly, turtle, rabbitgaroo. No automatic health bar for these.
pub const AMBIENT_WILDLIFE: [u32; 13] = [6000, 6001, 6010, 6040, 6060, 6070, 6071, 6072, 6080, 6081, 6082, 6090, 6100];

/// First-person eye position relative to the player model (measured standing).
const DEFAULT_EYE: Vec3 = Vec3::new(0.0, 1.603, 0.062);
/// Tallest ledge the ground movement walks straight up (stairs, sills, kerbs).
const STEP_HEIGHT: f32 = 0.5;

/// Meathook pull toward a grappled demon (alt-fire on the Super Shotgun).
pub struct Hook {
    target: usize,
    t: f32,
    dur: f32,
    from: Vec3,
    /// Pull velocity last frame (m/s): kept as momentum if the demon dies mid-pull.
    vel: Vec3,
}

#[derive(Default)]
pub struct Glory {
    target: usize,
    t: f32,
    from: Vec3,
    to: Vec3,
    chainsaw: bool,
    /// teleport mode: already moved to `to`
    placed: bool,
    /// Out of ammo at a boss: the chainsaw only refills the ammo - no damage, no lunge (user).
    boss_ammo: bool,
}

pub struct Slayer {
    pub input: Input,
    pub cfg: Config,
    cfg_check: f32,
    last: Instant,
    post_last: Instant,
    pub weapon: usize,
    pub last_weapon: usize,
    pub owned: [bool; 8],
    pub ammo: [i32; 5],
    /// Character play time when the Doom state was last saved (new-character detection).
    saved_character: String,
    /// Level damage multiplier (cached each frame; see level_mult).
    level_mult: f32,
    /// Last character level seen (a change is logged with its damage multiplier).
    last_level: u32,
    /// The recommended limits last applied to the settings (saved): a change moves the chainsaw
    /// limit and CRUCIBLE HIT DAMAGE by the same amount, keeping the user's own offset.
    rec_applied: Option<(u32, u32)>,
    new_char_checked: bool,
    pub fire_cd: f32,
    fire_buffer: f32,
    /// The current gun just ran dry from its own shots (auto-swap); `dry_prev`: it had ammo last frame.
    ran_dry: bool,
    dry_prev: (usize, bool),
    /// After a swap caused by firing an empty gun: ignore fire until the button is let go.
    fire_lock: bool,
    /// Teleport glory kills: stay at this spot for `glory_hold` s from this time (live).
    glory_hold: Option<(Vec3, f32)>,
    /// Riding a moving map object (lift) until this time: the game moves us, ours stays out.
    lift_until: f32,
    /// After a landing the camera rides the body directly until this time (land_cam_time).
    cam_land_until: f32,
    /// Doom landing dip: (start time, depth m)
    land_dip: Option<(f32, f32)>,
    /// Landing: the body's vertical speed measured in the air (m/s, last frame's) and, right after
    /// touchdown, the fall speed the last bit down to the ground is capped at.
    air_vy_meas: f32,
    land_vy: Option<f32>,
    last_body_y: f32,
    /// Real landings only: highest point of this time in the air, whether it began with a jump,
    /// and when the last real landing (a jump, or a drop over land_min_drop) touched down.
    /// Stairs (short hops off each step) don't count - landing parts 1/2 ruined them (user).
    air_peak_y: f32,
    air_jumped: bool,
    real_land_at: f32,
    /// queued follow-up breakers for loose props: (when, at, dir)
    later_break: Vec<(f32, Vec3, Vec3)>,
    /// queued second looks along a line (when, from, dir, length): a resting loose prop isn't an
    /// asset to our casts until a hit wakes it - looked at again a moment later
    later_look: Vec<(f32, Vec3, Vec3, f32)>,
    /// Jitter detector: recent unexplained body pushes (time, push) and camera wobble flips.
    /// the floor sampled under us last frame (object, spot, height) - for lift detection
    lift_floor: Option<((i32, u32), Vec3, f32)>,
    pub armor: i32,
    prev_hp: i32,
    pub dash_charges: f32,
    /// Dashes usable in this time in the air: what you had at take-off (charges that refill in
    /// the air wait for the ground - you could dash forever, user).
    air_dashes: f32,
    /// Both dashes used: no dash until both are back AND you've touched the ground (user).
    dash_lock: bool,
    /// The ground was touched since the last air dash / lock: dashes that finish recharging
    /// count even in the air (jump, dash twice, land while they recharge, jump again: they're
    /// usable once back - user). Recharged without a landing they still wait for the ground.
    dash_landed: bool,
    /// On the ground for the dash rules (false right after a jump).
    dash_ground: bool,
    /// Usable dashes last frame (the ready sound plays when both become usable).
    dash_ready_last: u32,
    dash_t: f32,
    dash_dir: Vec3,
    double_jumped: bool,
    /// Our own vertical velocity while the double jump owns the vertical axis (m/s, up +).
    air_vy: Option<f32>,
    /// Horizontal velocity added by the double jump, decays in the air.
    air_push: Vec3,
    /// Doom movement: our horizontal ground velocity, and momentum carried into the air.
    ground_vel: Vec3,
    air_carry: Vec3,
    was_on_ground: bool,
    /// Whether each Doom loot token (ammo, health) already has an inventory entry.
    token_seen: (bool, bool),
    token_init: bool,
    /// 0..1: how far the weapon is lowered because a menu (pause, options, inventory) is open.
    pub menu_t: f32,
    faded_for: f32,
    /// Seconds left of Elden Ring movement because we're on a moving platform.
    platform_t: f32,
    /// First-person camera height filter (smoothed y, vertical speed, last body y).
    cam_filter: Option<(f32, f32, f32)>,
    /// Health part of the game's HP pool (the rest is the shield), the game's own max HP, the
    /// max HP we last wrote (base + shield) and last frame's pool.
    pub health: i32,
    pub base_max: i32,
    pool_prev: i32,
    /// Current max-HP rate of the shield effect, and seconds until it's re-applied.
    shield_rate: f32,
    shield_apply_t: f32,
    /// Rate the game's max HP was last built with, while a new rate is pending; last max seen.
    rate_pending: Option<f32>,
    max_seen: i32,
    /// Learned standing eye position, model-local (average, samples).
    eye_learn: Option<(Vec3, f32)>,
    /// Seconds off the ground (camera filter bypass).
    air_t: f32,
    /// Fall-damage guard: seconds airborne, HP before landing, post-landing window, flag set.
    fall_air_t: f32,
    fall_hp: i32,
    fall_land_t: f32,
    fall_guard: bool,
    /// The ignore-all-damage flag is on for a glory / chainsaw kill.
    glory_invuln: bool,
    invuln_until: f32,
    invuln_hp: i32,
    /// Climbing a ladder or in a scripted interaction (the game's movement only).
    on_ladder: bool,
    /// Scripted interaction animation playing (fog wall, door, lever, grace).
    in_event: Option<i32>,
    /// Held in an enemy grab (throw target): movement paused.
    grabbed: bool,
    /// Where our last position write left the player (to see if the game moved us).
    /// Last spot we stood on (fall-through rescue).
    last_safe: Option<(Vec3, f32)>,
    /// Doom layer switched off (F9): plain Elden Ring.
    pub doom_off: bool,
    /// Playback speed of the current clip, and a clip queued to start later (SSG reload).
    vm_speed: f32,
    /// Clips queued to start later: (at, clip, speed, start time, looping).
    vm_queue: Vec<(f32, &'static str, f32, f32, bool)>,
    /// (clip time, seconds left) to hold the current clip at.
    vm_hold: Option<(f32, f32)>,
    /// Procedural gun kick on top of Doom's clips (spring: value, velocity, sideways sign).
    recoil: f32,
    recoil_v: f32,
    recoil_yaw: f32,
    /// Fast per-shot snap (1 on the shot, gone in ~0.06 s).
    snap: f32,
    /// Heavy Cannon kick envelope: fast attack toward push_goal, slow exponential settle.
    push: f32,
    push_goal: f32,
    /// Barrel spin per group (rotary cluster, turret barrels): angle and speed (rad, rad/s).
    spin: [f32; 2],
    spin_v: [f32; 2],
    /// BFG wind-up start (game time): the shot leaves when it ends.
    bfg_charge: Option<f32>,
    /// Meathook fired at nothing: chain visual ends at this time.
    hook_miss_until: f32,
    /// 0..1 arbalest core glow (smoothed).
    arb_glow: f32,
    /// 0..1 Arbalest aim zoom (smoothed).
    arb_zoom: f32,
    /// Seconds a right click on the Super Shotgun waits for the reload to end.
    hook_buffer: f32,
    /// No-target hook twitch start (layered on top of whatever the Super Shotgun plays).
    hook_twitch: Option<f32>,
    /// Seconds the weapon wheel key has been held (tap = swap to the last weapon, hold = wheel).
    wheel_held: f32,
    /// Every weapon is out of ammo: the chainsaw is up and a left click uses it (Doom).
    pub saw_mode: bool,
    saw_click: bool,
    /// The Crucible (user spec, MODLOG): in the hands, its charges (0..3), its own clip (it draws
    /// from its own model, like the fists), the next swing (left / right alternate), when it's put
    /// away, and a short window where a Crucible kill's ER item drops give nothing.
    pub crucible_out: bool,
    pub crucible_charges: u32,
    /// Bosses (boss bar) seen alive, by handle -> HP; a defeated one earns a Crucible charge, given
    /// with the next drop picked up (user) - or after 30 s if nothing is picked up.
    boss_seen: HashMap<eldenring::cs::FieldInsHandle, i32>,
    /// Boss bars for the HUD (name, hp, max): read here on the game's thread - the HUD used to scan
    /// every character from the render thread each frame, racing the game (the 1.1 crash).
    pub boss_bars: Vec<(String, i32, i32)>,
    /// The game's HP pool and max this frame, for the HUD (no game reads on the render thread).
    pub hud_pool: (i32, i32),
    boss_crucible: u32,
    boss_crucible_at: f32,
    // a boss bar that shows up within BOSS_STAGE s (in control) of a boss dying is that fight's next
    // stage: its death gives no charge (Rennala: one fight, one charge - user)
    boss_stage_window: f32,
    boss_stages: std::collections::HashSet<eldenring::cs::FieldInsHandle>,
    cr_clip: &'static str,
    cr_t: f32,
    /// The draw's ignite sound has played (it waits for the fangs and blade - user).
    cr_blade: bool,
    /// A punch / Blood Punch took the Crucible's place: it's drawn again after (user).
    cr_melee: bool,
    cr_loop: bool,
    cr_swing: usize,
    cr_away_at: Option<f32>,
    cr_no_loot_until: f32,
    /// Ammo is back after an out-of-ammo chainsaw kill: when the chainsaw starts lowering
    /// (after its kill animation); the last gun comes back up once it is down.
    saw_drop: Option<f32>,
    /// Reticle hook icon: the point it points at (hookable demon / hooked demon), refreshed
    /// every 0.1 s; when the pull ends it goes home and stays there 2 s.
    pub hook_icon: Option<Vec3>,
    /// Seconds the meathook has been ready (< 0 while it recharges or is out) - HUD fade.
    pub hook_ready_for: f32,
    hook_scan_t: f32,
    hook_end_at: f32,
    /// Seconds left in which a fresh jump ignores "touching ground" (it stays true for a few frames
    /// after take-off and used to cancel the jump instantly).
    jump_grace: f32,
    /// Vertical speed from walking up/down slopes (m/s), smoothed.
    ground_vy: f32,
    gravity_off_by_us: bool,
    pub chainsaw_fuel: f32,
    pub belch_cd: f32,
    burning: HashMap<usize, (f32, i32)>,
    glory: Option<Glory>,
    /// Breakable assets recently shot (handle bits -> time), see break_geom.
    broken_at: HashMap<u64, f32>,
    hook: Option<Hook>,
    hook_cd: f32,
    pub wheel_open: bool,
    /// Slot currently highlighted on the wheel.
    pub wheel_pick: Option<usize>,
    /// Seconds the wheel has been open (for its open animation).
    pub wheel_t: f32,
    slowed: bool,
    pub messages: VecDeque<(String, f32)>,
    pub kills: u32,
    pub glory_kills: u32,
    /// Doom Eternal: each glory kill charges the Blood Punch (max 1 without upgrades).
    pub blood_punch: u32,
    pub shots: u32,
    pub staggered: Vec<(Vec3, bool)>,
    /// Enemy health bars (ER's own are hidden with its HUD).
    pub bars: Vec<EnemyBar>,
    /// Line-of-sight cache for the bars: ChrIns ptr -> (visible, checked at).
    bar_los: HashMap<usize, (bool, f32)>,
    /// Enemies marked with the mark key (always show a bar).
    marked: Vec<eldenring::cs::FieldInsHandle>,
    pub time: f32,
    /// Viewmodel animation clocks (in `time` seconds).
    pub last_shot_at: f32,
    /// Last Precision Bolt (its own recharge; normal Heavy Cannon shots don't wait for it).
    pub bolt_at: f32,
    pub switched_at: f32,
    pub chainsaw_at: f32,
    /// Horizontal speed (m/s) for weapon bob.
    pub speed: f32,
    /// Weapon sway: sideways spring (tilt + trail) and vertical spring (jump / land), each
    /// (position, velocity); last camera direction; smoothed sideways speed; seconds airborne.
    sway: [f32; 2],
    sway_y: [f32; 2],
    sway_fwd: Option<Vec3>,
    sway_lat: f32,
    sway_air: f32,
    /// Walking bob phase (rad of the sideways cycle; the dip runs at twice this).
    bob_phase: f32,
    /// Smoothed bob strength, per-step random variation (side, dip, pace) and its targets,
    /// the step it was rolled for, and the generator state.
    bob_amp: f32,
    bob_var: [f32; 3],
    bob_var_to: [f32; 3],
    bob_step: i32,
    bob_rng: u32,
    /// Doom bob phase (rad): distance walked / stride.
    doom_bob_phase: f32,
    /// Walking bob gate: 0 in the air and while the landing bounce plays, 1 on the ground (eased),
    /// and the seconds left before it may come back after a landing.
    bob_gate: f32,
    bob_hold: f32,
    last_pos: Option<Vec3>,
    /// Seconds the player has existed continuously (reset on loads).
    pub in_world_for: f32,
    /// Doom pickups lying in the world.
    pub pickups: Pickups,
    /// Last seen HP / position per hostile, to detect deaths for drops.
    enemy_track: HashMap<usize, (i32, Vec3, i32)>,
    /// Doom melee: fists clip playing until `melee_until`.
    melee_until: f32,
    melee_clip: &'static str,
    melee_t: f32,
    melee_cd: f32,
    punch_alt: bool,
    /// Viewmodel only once the screen is clear and the Slayer is in control (then pull-out).
    pub ready: bool,
    clear_for: f32,
    /// 3D viewmodel animation state.
    pub(crate) vm_clip: String,
    pub(crate) vm_t: f32,
    vm_loop: bool,
    pub vm_visible: bool,
    /// Last time a fight happened (we hurt a hostile or got hurt) - drives the music.
    pub last_combat: f32,
    save_cd: f32,
    saved: String,
    /// Every character's Doom state by character name (user: loading another character used to
    /// hand it the last one's state, or a fresh kit). The live fields hold `saved_character`'s.
    char_states: serde_json::Map<String, serde_json::Value>,
    /// Fight music: each nearby hostile's HP last frame (by character pointer).
    hostile_hp: std::collections::HashMap<usize, i32>,
    /// Fight music: each hostile's position at the last 0.25 s sample and how far it has been
    /// coming at you lately (decaying sum of its own movement toward you, m).
    hostile_approach: std::collections::HashMap<usize, (Vec3, f32)>,
    approach_sample_at: f32,
    /// whether the music thought a fight was on last frame (to log why it starts)
    music_on: bool,
    /// Test-only: keep HP topped up (bridge `god`).
    pub god: bool,
    /// Bridge `watch`: log every player animation change for this many seconds.
    pub watch_anims: f32,
    last_anim: i32,
    /// Sounds scheduled for later (seconds left, event), e.g. the SSG reload after a shot.
    later: Vec<(f32, &'static str)>,
    blasts: Vec<damage::Pending>,
    pub hitters: crate::hitter::Hitters,
    // weapon mods
    pub sticky_cd: f32,
    pub sticky_mag: u32,
    /// 0..1 progress of the bomb recharging now; when the last bomb came back (flash).
    pub sticky_charge: f32,
    pub sticky_flash_at: f32,
    pub sticky_flash_all: bool,
    pub sticky_reload: f32,
    sticky_idle: f32,
    surge_until: f32,
    pub zoom: f32,
    zoom_on: bool,
    /// Right click held through a weapon switch (e.g. the sticky bombs ran out and the Heavy Cannon
    /// came up): no scope until it is released and pressed again (user).
    zoom_latch: bool,
    scope_down_at: f32,
    pub scope_hidden: bool,
    pub heat: f32,
    arb: Arb,
    pub turret: bool,
    stuck: Vec<Stuck>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Arb {
    Idle,
    Into(f32),
    Charging(f32),
    Charged,
}

/// A sticky bomb or arbalest bolt: flies `travel` s, sticks, explodes after `fuse` s.
struct Stuck {
    target: Option<eldenring::cs::FieldInsHandle>,
    offset: Vec3,
    pos: Vec3,
    t: f32,
    travel: f32,
    fuse: f32,
    radius: f32,
    damage: f32,
    ramp: &'static str,
    sound: &'static str,
    react: usize,
    beeped: bool,
    done: bool,
}

impl Stuck {
    #[allow(clippy::too_many_arguments)]
    fn new(target: Option<eldenring::cs::FieldInsHandle>, at: Vec3, travel: f32, fuse: f32, radius: f32, damage: f32, ramp: &'static str, sound: &'static str, react: usize) -> Self {
        let offset = target
            .and_then(|h| game::enemies(300.0).into_iter().find(|e| e.chr.field_ins_handle == h).map(|e| at - e.pos()))
            .unwrap_or(Vec3::ZERO);
        Self { target, offset, pos: at, t: 0.0, travel, fuse, radius, damage, ramp, sound, react, beeped: false, done: false }
    }
}

/// Precision Bolt scope: erfps2 (first-person camera) owns the FOV; it exports a zoom setter.
/// Sticky Bombs: Five Spot magazine and Quick Rack reload (Doom base ~2.5 s, -20%).
pub const STICKY_MAG: u32 = 5;
pub const STICKY_RELOAD: f32 = 2.5 * 0.8;
/// One bomb of a partly used magazine comes back every this many seconds.
pub const STICKY_RECHARGE: f32 = 2.5;

fn set_zoom(factor: f32) {
    static LAST: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);
    if (f32::from_bits(LAST.load(std::sync::atomic::Ordering::Relaxed)) - factor).abs() < 1e-4 {
        return;
    }
    LAST.store(factor.to_bits(), std::sync::atomic::Ordering::Relaxed);
    static F: std::sync::OnceLock<Option<usize>> = std::sync::OnceLock::new();
    let f = F.get_or_init(|| unsafe {
        let m = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!("erfps2.dll")).ok()?;
        windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::s!("erfps2_set_zoom")).map(|p| p as usize)
    });
    if let Some(f) = *f {
        let call: extern "C" fn(f32) = unsafe { std::mem::transmute(f) };
        call(factor);
    }
}

/// The game's own "player can't die" debug flag (HP floors at 1).
fn set_no_dead(on: bool) {
    if let Ok(f) = unsafe { <eldenring::cs::WorldChrManDbgFlags as fromsoftware_shared::FromStatic>::instance_mut() } {
        f.player_no_dead = on;
    }
}

/// Where the meathook grabs a character: chest height for its body size (trolls and giants used
/// to be hooked by the ankle / pelvis at a fixed 1.2 m).
fn hook_grip(chr: &eldenring::cs::ChrIns) -> f32 {
    let ph = &chr.modules.physics;
    let h = ph.chr_hit_height.max(ph.hit_height);
    if h.is_finite() && h > 0.5 && h < 30.0 { (h * 0.72).max(1.0) } else { 1.2 }
}

/// erfps2 export by name (cached).
fn erfps2_fn(name: &'static [u8]) -> Option<usize> {
    use std::collections::HashMap;
    static CACHE: std::sync::Mutex<Option<HashMap<&'static [u8], Option<usize>>>> = std::sync::Mutex::new(None);
    let mut g = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *g.get_or_insert_with(HashMap::new).entry(name).or_insert_with(|| unsafe {
        let m = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!("erfps2.dll")).ok()?;
        windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::PCSTR(name.as_ptr())).map(|p| p as usize)
    })
}

/// Fixed first-person eye height (model-local metres) instead of the animated head; NaN = off.
fn camera_eye(height: f32) {
    if let Some(f) = erfps2_fn(b"erfps2_set_eye_height ") {
        let call: extern "C" fn(f32) = unsafe { std::mem::transmute(f) };
        call(height);
    }
}

/// Head bone position relative to the model (erfps2, this frame).
fn camera_head() -> Option<Vec3> {
    let f = erfps2_fn(b"erfps2_head_local_xyz ")?;
    let call: unsafe extern "C" fn(*mut f32) = unsafe { std::mem::transmute(f) };
    let mut v = [f32::NAN; 3];
    unsafe { call(v.as_mut_ptr()) };
    let v = Vec3::from_array(v);
    (v.is_finite() && v.y > 0.5 && v.y < 3.0).then_some(v)
}

/// Lock the eye's forward/sideways position too (model-local); NaN = follow the head.
fn camera_eye_xz(x: f32, z: f32) {
    if let Some(f) = erfps2_fn(b"erfps2_set_eye_xz ") {
        let call: extern "C" fn(f32, f32) = unsafe { std::mem::transmute(f) };
        call(x, z);
    }
}

/// Camera height offset for erfps2 (metres added to the first-person camera's y).
fn camera_step(dy: f32) {
    if let Some(f) = erfps2_fn(b"erfps2_set_step ") {
        let call: extern "C" fn(f32) = unsafe { std::mem::transmute(f) };
        call(dy);
    }
}

impl Slayer {
    fn new() -> Self {
        let mut s = Self::fresh();
        s.load();
        s
    }

    fn fresh() -> Self {
        let mut ammo = [0; 5];
        for a in Ammo::ALL {
            ammo[a.index()] = a.max() / 2;
        }
        ammo[Ammo::Bfg.index()] = 1;
        Self {
            input: Input::default(),
            cfg: config::get(),
            cfg_check: 0.0,
            last: Instant::now(),
            post_last: Instant::now(),
            weapon: SUPER_SHOTGUN,
            last_weapon: 0,
            owned: [true; 8],
            ammo,
            saved_character: String::new(),
            level_mult: 1.0,
            last_level: 0,
            rec_applied: None,
            new_char_checked: false,
            fire_cd: 0.0,
            fire_buffer: 0.0,
            ran_dry: false,
            dry_prev: (usize::MAX, false),
            fire_lock: false,
            glory_hold: None,
            lift_until: -1.0,
            cam_land_until: -1.0,
            land_dip: None,
            air_vy_meas: 0.0,
            land_vy: None,
            last_body_y: f32::NAN,
            air_peak_y: f32::NEG_INFINITY,
            air_jumped: false,
            real_land_at: -1.0,
            later_break: Vec::new(),
            later_look: Vec::new(),
            lift_floor: None,
            armor: 50,
            prev_hp: -1,
            dash_charges: DASH_CHARGES,
            air_dashes: DASH_CHARGES,
            dash_lock: false,
            dash_landed: true,
            dash_ground: true,
            dash_ready_last: DASH_CHARGES as u32,
            dash_t: 0.0,
            dash_dir: Vec3::ZERO,
            double_jumped: false,
            air_vy: None,
            air_push: Vec3::ZERO,
            ground_vel: Vec3::ZERO,
            air_carry: Vec3::ZERO,
            was_on_ground: true,
            token_seen: (false, false),
            token_init: false,
            menu_t: 0.0,
            faded_for: 0.0,
            platform_t: 0.0,
            cam_filter: None,
            eye_learn: None,
            health: 0,
            base_max: 0,
            pool_prev: 0,
            shield_rate: 1.0,
            shield_apply_t: 0.0,
            rate_pending: None,
            max_seen: 0,
            air_t: 0.0,
            fall_air_t: 0.0,
            fall_hp: 0,
            fall_land_t: 0.0,
            fall_guard: false,
            on_ladder: false,
            glory_invuln: false,
            invuln_until: 0.0,
            invuln_hp: 0,
            in_event: None,
            grabbed: false,
            last_safe: None,
            doom_off: false,
            vm_speed: 1.0,
            vm_queue: Vec::new(),
            vm_hold: None,
            recoil: 0.0,
            recoil_v: 0.0,
            recoil_yaw: 0.0,
            snap: 0.0,
            push: 0.0,
            push_goal: 0.0,
            spin: [0.0; 2],
            spin_v: [0.0; 2],
            bfg_charge: None,
            hook_miss_until: 0.0,
            arb_glow: 0.0,
            arb_zoom: 0.0,
            hook_buffer: 0.0,
            hook_twitch: None,
            wheel_held: 0.0,
            saw_mode: false,
            saw_click: false,
            crucible_out: false,
            crucible_charges: 0,
            boss_seen: HashMap::new(),
            boss_bars: Vec::new(),
            hud_pool: (0, 0),
            boss_crucible: 0,
            boss_crucible_at: 0.0,
            boss_stage_window: 0.0,
            boss_stages: std::collections::HashSet::new(),
            cr_clip: "idle",
            cr_t: 0.0,
            cr_blade: false,
            cr_melee: false,
            cr_loop: true,
            cr_swing: 0,
            cr_away_at: None,
            cr_no_loot_until: -1.0,
            saw_drop: None,
            hook_icon: None,
            hook_ready_for: 0.0,
            hook_scan_t: 0.0,
            hook_end_at: -10.0,
            jump_grace: 0.0,
            ground_vy: 0.0,
            gravity_off_by_us: false,
            chainsaw_fuel: 1.0,
            belch_cd: 0.0,
            burning: HashMap::new(),
            glory: None,
            broken_at: HashMap::new(),
            hook: None,
            hook_cd: 0.0,
            wheel_open: false,
            wheel_pick: None,
            wheel_t: 0.0,
            slowed: false,
            messages: VecDeque::new(),
            kills: 0,
            glory_kills: 0,
            blood_punch: 0,
            shots: 0,
            staggered: vec![],
            time: 0.0,
            last_shot_at: -10.0,
            bolt_at: -10.0,
            switched_at: -10.0,
            chainsaw_at: -10.0,
            speed: 0.0,
            sway: [0.0; 2],
            sway_y: [0.0; 2],
            sway_fwd: None,
            sway_lat: 0.0,
            sway_air: 0.0,
            bob_phase: 0.0,
            bob_amp: 0.0,
            bob_var: [0.0; 3],
            bob_var_to: [0.0; 3],
            bob_step: 0,
            bob_rng: 0x9E37_79B9,
            doom_bob_phase: 0.0,
            bob_gate: 1.0,
            bob_hold: 0.0,
            last_pos: None,
            in_world_for: 0.0,
            pickups: Pickups::new(),
            bars: Vec::new(),
            bar_los: HashMap::new(),
            marked: Vec::new(),
            enemy_track: HashMap::new(),
            melee_until: -10.0,
            melee_clip: "punch_r",
            melee_t: 0.0,
            melee_cd: 0.0,
            punch_alt: false,
            ready: false,
            clear_for: 0.0,
            vm_clip: "bringup".into(),
            vm_t: 0.0,
            vm_loop: false,
            vm_visible: false,
            last_combat: -100.0,
            save_cd: 2.0,
            saved: String::new(),
            char_states: serde_json::Map::new(),
            hostile_hp: std::collections::HashMap::new(),
            hostile_approach: std::collections::HashMap::new(),
            approach_sample_at: 0.0,
            music_on: false,
            god: false,
            watch_anims: 0.0,
            last_anim: -1,
            later: Vec::new(),
            blasts: Vec::new(),
            hitters: Default::default(),
            sticky_cd: 0.0,
            sticky_mag: STICKY_MAG,
            sticky_charge: 0.0,
            sticky_flash_at: -10.0,
            sticky_flash_all: false,
            sticky_reload: 0.0,
            sticky_idle: 0.0,
            surge_until: -10.0,
            zoom: 0.0,
            zoom_on: false,
            zoom_latch: false,
            scope_down_at: f32::MAX,
            scope_hidden: false,
            heat: 0.0,
            arb: Arb::Idle,
            turret: false,
            stuck: Vec::new(),
        }
    }

    fn save_path() -> std::path::PathBuf {
        config::mod_dir().join("doomslayer_save.json")
    }

    /// Doom resources live outside the ER save; persisted next to the DLL, one entry per
    /// character name: {"characters": {name: state}, "last": name, "rec": [..]}. (The old file
    /// held one state plus "character": it becomes that character's entry.)
    fn load(&mut self) {
        let Ok(text) = std::fs::read_to_string(Self::save_path()) else { return };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return };
        if let Some(chars) = v["characters"].as_object() {
            self.char_states = chars.clone();
            self.saved_character = v["last"].as_str().unwrap_or("").to_string();
            if let Some(st) = self.char_states.get(&self.saved_character).cloned() {
                self.apply_state(&st);
            }
        } else {
            self.apply_state(&v);
            self.saved_character = v["character"].as_str().unwrap_or("").to_string();
            if !self.saved_character.is_empty() {
                self.char_states.insert(self.saved_character.clone(), self.state_json());
            }
        }
        if let (Some(a), Some(b)) = (v["rec"][0].as_u64(), v["rec"][1].as_u64()) {
            self.rec_applied = Some((a as u32, b as u32));
        }
        log::info!("loaded slayer state ({} characters, last {:?}): {text}", self.char_states.len(), self.saved_character);
    }

    /// The current character's Doom state.
    fn state_json(&self) -> serde_json::Value {
        serde_json::json!({
            "ammo": self.ammo, "armor": self.armor, "fuel": self.chainsaw_fuel,
            "blood_punch": self.blood_punch, "weapon": self.weapon, "crucible": self.crucible_charges,
            "health": self.health,
        })
    }

    /// Health as the character had it (the game's own saved HP has the shield mixed in: loading
    /// read the shield as health, or clipped it). Not for a character that never had it saved.
    fn restore_health(&mut self, st: &serde_json::Value) {
        if let Some(h) = st["health"].as_i64().filter(|h| *h > 0) {
            self.health = (h as i32).min(self.base_max.max(1));
            self.prev_hp = self.health;
            log::info!("health restored: {} / {}", self.health, self.base_max);
        }
    }

    fn apply_state(&mut self, v: &serde_json::Value) {
        if let Some(a) = v["ammo"].as_array() {
            for (i, x) in a.iter().enumerate().take(5) {
                self.ammo[i] = x.as_i64().unwrap_or(0) as i32;
            }
        }
        self.armor = v["armor"].as_i64().unwrap_or(50) as i32;
        self.chainsaw_fuel = (v["fuel"].as_f64().unwrap_or(1.0) as f32).min(CHAINSAW_PIPS);
        self.blood_punch = v["blood_punch"].as_u64().unwrap_or(0) as u32;
        self.crucible_charges = (v["crucible"].as_u64().unwrap_or(0) as u32).min(CRUCIBLE_MAX);
        self.weapon = (v["weapon"].as_u64().unwrap_or(4) as usize).min(WEAPONS.len() - 1);
        self.crucible_out = false;
    }

    /// All Doom damage = WEAPON DAMAGE x the level multiplier.
    fn dmg_mult(&self) -> f32 {
        self.cfg.weapon_damage_mult * self.level_mult
    }

    /// Level scaling: the damage multiplier, and the chainsaw limit / CRUCIBLE HIT DAMAGE moved
    /// with the recommended values (by 100 at a time, the user's offset kept; down too, e.g. a
    /// new, lower character). Not while the settings window is open (it holds its own copy).
    fn update_level(&mut self, level: u32) {
        if level == 0 {
            return;
        }
        self.level_mult = level_mult(&self.cfg, level);
        let rec = rec_limits(&self.cfg, level);
        if level != self.last_level {
            log::info!("level {level}: damage x{:.2} (level x{:.2}, WEAPON DAMAGE x{:.2}), recommended limits {rec:?}", self.dmg_mult(), self.level_mult, self.cfg.weapon_damage_mult);
            self.last_level = level;
        }
        REC_LIMITS.store((rec.0 as u64) << 32 | rec.1 as u64, std::sync::atomic::Ordering::Relaxed);
        if crate::remap::SETTINGS_OPEN.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        match self.rec_applied {
            // first run: today's values stand
            None => self.rec_applied = Some(rec),
            Some(old) if old != rec => {
                let saw = (self.cfg.glory_heavy_hp + rec.0 as f32 - old.0 as f32).clamp(100.0, 10000.0);
                let cr = (self.cfg.crucible_hp + rec.1 as f32 - old.1 as f32).clamp(100.0, 10000.0);
                log::info!("level {level}: recommended {rec:?} (was {old:?}) -> chainsaw limit {saw}, crucible {cr}");
                self.cfg.glory_heavy_hp = saw;
                self.cfg.crucible_hp = cr;
                config::save_values(&[("", "glory_heavy_hp", format!("{saw:.1}")), ("", "crucible_hp", format!("{cr:.1}"))]);
                self.rec_applied = Some(rec);
            }
            _ => {}
        }
    }

    fn save_if_changed(&mut self, dt: f32) {
        self.save_cd -= dt;
        if self.save_cd > 0.0 {
            return;
        }
        self.save_cd = 2.0;
        // nothing is saved between a load and knowing whose state this is
        if !self.new_char_checked {
            return;
        }
        // (nothing is filed under a name until the loaded character is known)
        if !self.saved_character.is_empty() {
            self.char_states.insert(self.saved_character.clone(), self.state_json());
        }
        self.save_now();
    }

    /// Write the save file (every character's entry as filed) if it changed.
    fn save_now(&mut self) {
        let text = serde_json::json!({
            "characters": self.char_states,
            "last": self.saved_character,
            "rec": self.rec_applied.map(|(a, b)| [a, b]),
        })
        .to_string();
        if text != self.saved {
            let _ = std::fs::write(Self::save_path(), &text);
            self.saved = text;
        }
    }

    /// A character loaded (checked on every load into the world; the one played before was filed
    /// when its world was left): this one gets its own saved state back - or, never seen before,
    /// the starting kit
    /// (every gun full, full armor, no Crucible charges or Blood Punch).
    fn check_new_character(&mut self) {
        let Some(name) = character_name() else { return };
        self.new_char_checked = true;
        if name == self.saved_character {
            log::info!("character {name:?}: same as before");
            if let Some(st) = self.char_states.get(&name).cloned() {
                self.restore_health(&st);
            }
            return;
        }
        // (the character played before was filed when its world was left)
        match self.char_states.get(&name).cloned() {
            Some(st) => {
                self.apply_state(&st);
                self.restore_health(&st);
                log::info!("character {name:?} (was {:?}): its saved state restored {st}", self.saved_character);
            }
            None => {
                log::info!("character {name:?} (was {:?}): new - starting kit", self.saved_character);
                for a in Ammo::ALL {
                    self.ammo[a.index()] = a.max();
                }
                self.armor = self.cfg.armor_max;
                self.chainsaw_fuel = CHAINSAW_PIPS;
                self.blood_punch = 0;
                self.crucible_charges = 0;
                self.crucible_out = false;
                self.msg("FULL AMMO");
            }
        }
        self.saved_character = name;
        self.save_cd = 0.0;
    }

    /// Grace rest / respawn: Doom-style resupply.
    fn resupply(&mut self) {
        for a in Ammo::ALL {
            let i = a.index();
            let floor = if a == Ammo::Bfg { 1 } else { a.max() / 2 };
            self.ammo[i] = self.ammo[i].max(floor);
        }
        self.armor = self.armor.max(50);
        self.chainsaw_fuel = self.chainsaw_fuel.max(1.0);
        self.msg("RESUPPLIED");
        audio::play_vol("pickup_ammo", 0.6);
    }

    pub fn msg(&mut self, s: impl Into<String>) {
        let s = s.into();
        log::info!("msg: {s}");
        self.messages.push_back((s, 3.0));
        while self.messages.len() > 5 {
            self.messages.pop_front();
        }
    }

    fn watched_keys(&self) -> Vec<u16> {
        let k = &self.cfg.keys;
        let mut v = vec![
            k.fire,
            k.alt_fire,
            k.dash,
            k.jump,
            k.melee,
            k.chainsaw,
            k.flame_belch,
            k.melee_alt,
            k.chainsaw_alt,
            k.flame_belch_alt,
            k.weapon_wheel,
            k.weapon_wheel_alt,
            k.last_weapon,
            k.doom_toggle,
            k.unstick,
            k.mark,
            k.fire_alt,
            k.alt_fire_alt,
            k.dash_alt,
            k.jump_alt,
            k.mark_alt,
            k.doom_toggle_alt,
            k.unstick_alt,
            k.crucible,
            k.crucible_alt,
            // controller buttons (their own codes, gamepad.rs)
            k.fire_pad,
            k.alt_fire_pad,
            k.dash_pad,
            k.jump_pad,
            k.melee_pad,
            k.chainsaw_pad,
            k.flame_belch_pad,
            k.crucible_pad,
            k.weapon_wheel_pad,
            k.mark_pad,
            k.doom_toggle_pad,
            k.unstick_pad,
            k.music_test,
            k.music_next,
            k.music_restart,
            k.music_back,
            k.music_fwd,
            k.music_mark,
        ];
        v.retain(|&vk| vk != 0);
        v.extend_from_slice(&k.slots);
        v.extend_from_slice(&input::MOVE_KEYS);
        v
    }

    pub fn select(&mut self, slot: usize) {
        // a weapon switch puts the Crucible away (the gun comes up)
        if self.crucible_out && slot < WEAPONS.len() && self.owned[slot] {
            self.crucible_out = false;
            self.cr_away_at = None;
            if slot == self.weapon {
                self.vm_play("bringup", false);
                self.switched_at = self.time;
                return;
            }
        }
        if slot < WEAPONS.len() && self.owned[slot] && slot != self.weapon {
            self.zoom_latch = true;
            self.reset_mods();
            self.last_weapon = self.weapon;
            self.weapon = slot;
            self.switched_at = self.time;
            self.vm_play("bringup", false);
            // Doom-fast swap: the new gun is ready almost at once (the old gun's cooldown used to
            // carry over - up to 1.35 s after a Super Shotgun blast); switch_fire_delay (live)
            // keeps a held trigger from firing straight through the swap.
            self.fire_cd = self.cfg.switch_fire_delay.max(0.0);
            self.msg(WEAPONS[slot].name);
            audio::play_vol("weapon_switch", 0.5);
        }
    }

    /// Bosses and mini bosses: the game's boss bar, the heavy HP class, or a giant body (trolls).
    /// Bosses (boss bar) and anything over the chainsaw HP limit (settings window). The old "3 m
    /// or taller" rule is gone: it kept bears out at any limit (user).
    /// A boss: its bar is up, or Elden Ring lists it as a boss (GameAreaParam) even before the
    /// bar shows (a field dragon walked up to died to the Crucible - user).
    fn is_boss(chr: &eldenring::cs::ChrIns) -> bool {
        let bosses = game::active_boss_handles();
        let direct = |c: &eldenring::cs::ChrIns| bosses.contains(&c.field_ins_handle) || params::is_area_boss(c.event_entity_id);
        if direct(chr) {
            return true;
        }
        // A boss's mount is part of the boss (user: Night's Cavalry's horse could be glory killed
        // and showed "Crucible needs 2"): the ride pairs both ends - the ridden character's
        // counter party is its rider, a boss rider's counter party is its mount.
        let Some(w) = game::world() else { return false };
        let ride = &chr.modules.ride;
        if ride.is_ride_character {
            let rider = &ride.ride_node.pair_anim_node.counter_party;
            if !rider.is_empty() && *rider != chr.field_ins_handle && w.chr_ins_by_handle(rider).is_some_and(|r| direct(r)) {
                return true;
            }
        }
        bosses.iter().any(|h| {
            w.chr_ins_by_handle(h).is_some_and(|b| b.modules.ride.ride_node.pair_anim_node.counter_party == chr.field_ins_handle)
        })
    }

    fn chainsaw_immune(&self, e: &Enemy) -> bool {
        self.chainsaw_block(e).is_some()
    }

    fn chainsaw_block(&self, e: &Enemy) -> Option<String> {
        let base_hp = e.max_hp() as f32 / crate::params::applied_hp_mult();
        if Self::is_boss(&e.chr) {
            return Some("boss".into());
        }
        (base_hp >= self.cfg.glory_heavy_hp).then(|| format!("base HP {base_hp:.0} >= limit {:.0}", self.cfg.glory_heavy_hp))
    }

    pub fn is_staggered(&self, e: &Enemy) -> bool {
        // Bosses (a boss bar on screen) never offer glory kills (Doom saves those for the finale).
        // Big field enemies without a bar (bears, trolls) do - the old 4000 base-HP cap
        // kept them out (user).
        // (glory kills off in the settings: nothing is ever offered)
        self.cfg.glory_kills && !Self::is_boss(&e.chr) && (e.hp() as f32) <= e.max_hp() as f32 * self.cfg.stagger_hp_frac
    }

    // ---------------------------------------------------------------- weapons

    pub fn fire(&mut self) -> bool {
        self.fire_dir(None)
    }

    /// Fire the current weapon; `aim` overrides the camera forward (bridge tests).
    pub fn fire_dir(&mut self, aim: Option<Vec3>) -> bool {
        let w = &WEAPONS[self.weapon];
        let a = w.ammo.index();
        // Precision Bolt uses 6 rounds (Doom).
        let per_shot = if self.weapon == 1 && self.zoom > 0.5 { 6 } else { w.ammo_per_shot };
        if !self.cfg.infinite_ammo && self.ammo[a] < per_shot {
            if self.fire_cd <= 0.0 {
                self.msg(format!("NO {}", w.ammo.name()));
                audio::play("dry_fire");
                self.fire_cd = 0.3;
            }
            return false;
        }
        let Some((cam_pos, cam_fwd)) = game::camera() else {
            return false;
        };
        let fwd = aim.unwrap_or(cam_fwd);
        let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        let origin = cam_pos + fwd * 0.7;
        let right = fwd.cross(Vec3::Y).normalize_or(Vec3::X);
        let up = right.cross(fwd).normalize_or(Vec3::Y);
        let bullet_id = BULLET_SLOTS[self.weapon];
        let (mut direct, blast_r, blast) = damage::TABLE[self.weapon];
        if self.weapon == 2 && self.time < self.surge_until {
            direct *= 1.5; // Power Surge
        }
        // Heavy Cannon scoped: Precision Bolt (one big, accurate round).
        let bolt = self.weapon == 1 && self.zoom > 0.5;
        if bolt {
            direct = 450.0;
        }
        let w_spread = if bolt { 0.0 } else { w.spread };
        let mult = self.dmg_mult();
        let mut landed = 0;
        let mut ok = 0;
        let mut ends = Vec::with_capacity(w.pellets as usize);
        let mut reacted: Vec<(eldenring::cs::FieldInsHandle, Vec3, Vec3)> = Vec::new();
        for i in 0..w.pellets {
            // Even ring pattern like Doom's shotguns, plus a little jitter.
            let (yaw, pitch) = if w.pellets > 1 {
                let ang = i as f32 / w.pellets as f32 * std::f32::consts::TAU;
                let r = w_spread * if i % 2 == 0 { 1.0 } else { 0.55 };
                (ang.cos() * r, ang.sin() * r * 0.6)
            } else {
                let j = (self.shots as f32 * 12.9898).sin() * w_spread;
                (j, (self.shots as f32 * 78.233).sin() * w_spread * 0.5)
            };
            let dir = Quat::from_axis_angle(up, (-yaw).to_radians())
                * Quat::from_axis_angle(right, pitch.to_radians())
                * fwd;
            // The real damage: Doom-style hitscan from the eye, independent of ER stats.
            let hit = damage::trace(cam_pos, dir, w.range, if w.pellets > 1 { 0.3 } else { 0.15 });
            ends.push((hit.pos, hit.chr.is_some()));
            ok += 1;
            // Crates, barrels, furniture: a breakable asset gets an object-attack dart.
            // the direct hit first; if that didn't break anything (a floor asset, a blocker in front
            // of fence sides), the breakables-only look along the shot's line
            let broke_direct = hit.geom.is_some_and(|g| self.break_geom(g, hit.pos, dir));
            if !broke_direct {
                // Breakables the shot stopped short of (fence sides sit behind an invisible blocker
                // the trace meets first; the dash, which only looks for breakables, broke them -
                // user): the same breakables-only look along the shot's own line, to a little past
                // where it stopped.
                let reach = (hit.dist + 0.5).min(w.range);
                self.break_along(cam_pos, dir, reach, 0.1);
            }
            if let Some(h) = hit.chr {
                // BFG and the Precision Bolt: what they can kill dies - no stagger catch (user)
                damage::NO_CATCH.store(self.weapon == BFG || bolt, std::sync::atomic::Ordering::Relaxed);
                let res = damage::apply_handle(h, direct * mult);
                damage::NO_CATCH.store(false, std::sync::atomic::Ordering::Relaxed);
                if let Some(killed) = res {
                    landed += 1;
                    // Precision Bolt mastery (Headshot Blast): a bolt kill sets off a blast.
                    if bolt && killed {
                        self.stuck.push(Stuck::new(None, hit.pos, 0.0, 0.05, 3.5, 300.0 * mult, "fire", "rocket_explode", 3));
                    }
                    // No reaction dart into a corpse: the kill plays its own death (saves a slot).
                    if !killed && !reacted.iter().any(|(r, _, _)| *r == h) {
                        reacted.push((h, hit.pos, dir));
                    }
                }
            }
            if blast_r > 0.0 && i == 0 {
                let travel = (hit.dist / crate::fx::LOOKS[self.weapon].speed.max(1.0)).min(3.0);
                self.blasts.push(damage::Pending {
                    at: hit.pos - dir * 0.3,
                    t: travel,
                    radius: blast_r,
                    amount: blast * mult,
                    er_blast: Some(self.weapon as i32),
                    origin: hit.pos - dir * hit.dist,
                    total: travel.max(1e-3),
                });
            }
        }
        // Enemy reaction: one real ER hit per demon per shot, spawned right at its body (an
        // invisible, effect-free dart that dies on contact). It carries the weapon's stagger level,
        // poise damage and the blood splash, and counts as the player attacking (aggro). Nothing
        // visible is spawned through ER: its bullet/effect pools are fixed-size and ran out.
        if blast_r <= 0.0 {
            for (h, at, dir) in &reacted {
                self.hitters.hit_on(Some(*h), self.weapon, *at, *dir);
            }
        }
        let _ = (bullet_id, owner);
        // Visuals (our renderer): muzzle flash at Doom's muzzle tag, the shot from the barrel.
        viewmodel::muzzle_flash(self.weapon);
        let barrel = muzzle_world().unwrap_or(origin);
        crate::fx::shot(self.weapon, barrel, ends);
        if landed > 0 {
            self.last_combat = self.time;
            log::debug!("{} landed {landed}/{} for {}", w.name, w.pellets, direct * mult);
        }
        if !self.cfg.infinite_ammo {
            self.ammo[a] -= per_shot;
        }
        self.shots += 1;
        self.last_shot_at = self.time;
        self.kick();
        // Automatics loop Doom's shoot-state animation while the trigger is held (and recover on
        // release, see update_viewmodel); single shots play their full shot cycle.
        let loops = viewmodel::clip_len(viewmodel::FOLDERS[self.weapon], "recover").is_some();
        if self.turret {
            // Chaingun Mobile Turret: the three-barrel firing loop.
            if !(self.vm_clip == "turret_fire" && self.vm_loop) {
                self.vm_play("turret_fire", true);
            }
        } else if bolt {
            // (gun hidden while scoped)
        } else if w.automatic && loops {
            // The pull-out plays to the end even when firing starts during it (the chaingun's
            // barrels spin and it shoots, but the bring-up isn't cut short); the firing loop
            // takes over on the next shot after it.
            let bringing_up = self.vm_clip == "bringup"
                && viewmodel::clip_len(viewmodel::FOLDERS[self.weapon], "bringup").is_some_and(|len| self.vm_t < len);
            if !bringing_up && !(self.vm_clip == "fire" && self.vm_loop) {
                self.vm_play("fire", true);
            }
        } else if w.automatic {
            // Per-shot clip (Heavy Cannon: bolt cycles every round).
            self.vm_play("fire", false);
        } else if self.weapon == SUPER_SHOTGUN {
            // Doom's own shoot_reload (both shells go in together), sped up, holding for a beat
            // after the kick before it breaks open.
            // Doom's kick + settle (shoot), then shoot_reload picked up at its break-open frame:
            // both shells out and in together. No frozen pose.
            // Doom's shoot_reload (the clip Doom plays for a shot: hasShootToReloadAnims) straight
            // through at one speed, fitted to the shot cycle - no hold (user).
            self.vm_play("fire", false);
            let len = viewmodel::clip_len(viewmodel::FOLDERS[SUPER_SHOTGUN], "fire").unwrap_or(1.53);
            self.vm_speed = (len / (w.interval - 0.05)).max(1.0);
        } else if self.weapon == 0 {
            // Combat Shotgun: kick, a beat with the gun settling back to idle, then the pump
            // (shoot_delay resumed after its kick, sped up to finish before the next shot).
            self.vm_play("fire", false);
            self.vm_queue = vec![
                (self.time + 0.09, "idle", 1.0, 0.0, true),
                (self.time + 0.25, "fire", 1.4, 0.08, false),
            ];
        } else if self.weapon == 5 {
            // Ballista: Doom's shot + reload cycle (1.4 s), sped up to end before the next shot.
            // The kick (Doom's 5-frame shoot part) fast, then the reload cycle fitted into the rest.
            self.vm_play("fire", false);
            if let Some(len) = viewmodel::clip_len(viewmodel::FOLDERS[5], "fire") {
                let kick = 5.0 / 30.0;
                let fast = 2.5;
                self.vm_speed = fast;
                let rest = (w.interval - kick / fast - 0.05).max(0.2);
                self.vm_queue = vec![(self.time + kick / fast, "fire", ((len - kick) / rest).max(1.0), kick, false)];
            }
        } else {
            self.vm_play("fire", false);
        }
        // Upgraded: Precision Bolt Quick Recovery, turret Ultra-Fast spin.
        if bolt {
            self.bolt_at = self.time;
        }
        self.fire_cd = if bolt { w.interval } else if self.turret { w.interval * 0.5 } else { w.interval };
        if bolt {
            audio::play("hc_bolt_fire");
        } else if self.turret {
            // The regular chaingun's sound per shot (user; turret-only mixes were all wrong).
            audio::play(w.fire_sound);
        } else {
            audio::play(w.fire_sound);
        }
        if let Some((delay, ev)) = w.reload_sound {
            self.later.push((delay, ev));
        }
        // Plasma Rifle heat builds while firing (Heat Blast charge levels).
        if self.weapon == 2 {
            let before = self.heat;
            self.heat = (self.heat + 0.022 * 1.25).min(1.0); // Super Heated Rounds: +25% per shot
            for (lv, ev) in [(0.34, "heat_level_1"), (0.67, "heat_level_2"), (1.0, "heat_level_3")] {
                if before < lv && self.heat >= lv {
                    audio::play_vol(ev, 0.8);
                    if lv >= 1.0 {
                        self.later.push((0.25, "heat_ready"));
                    }
                }
            }
        }
        ok > 0
    }

    // ---------------------------------------------------------------- weapon mods

    /// Idle clip for the current weapon state (scoped, turret, charged arbalest).
    fn idle_clip(&self) -> &'static str {
        if self.turret {
            "turret_idle"
        } else if self.weapon == 1 && self.zoom > 0.5 {
            "zoom_idle"
        } else if self.weapon == 5 && self.arb == Arb::Charged {
            "arb_idle"
        } else if self.weapon == 0 && self.sticky_mag == 0 {
            "sticky_empty"
        } else {
            "idle"
        }
    }

    /// 0..1 BFG wind-up progress (HUD).
    pub fn bfg_charge_frac(&self) -> f32 {
        self.bfg_charge.map(|t0| ((self.time - t0) / BFG_CHARGE).clamp(0.0, 1.0)).unwrap_or(0.0)
    }

    /// 0..1 Arbalest draw/charge progress (HUD).
    pub fn arb_charge(&self) -> f32 {
        // One continuous fill over the draw (into) and the charge, so the ring grows from empty.
        let into = viewmodel::clip_len(viewmodel::FOLDERS[5], "arb_into").unwrap_or(0.35);
        let charge = viewmodel::clip_len(viewmodel::FOLDERS[5], "arb_charge").unwrap_or(0.75);
        let total = (into + charge).max(0.1);
        match self.arb {
            Arb::Idle => 0.0,
            Arb::Into(t0) => ((self.time - t0) / total).clamp(0.0, 1.0),
            Arb::Charging(t0) => ((into + self.time - t0) / total).clamp(0.0, 1.0),
            Arb::Charged => 1.0,
        }
    }

    /// Leave every mod state (weapon switch, death).
    fn reset_mods(&mut self) {
        if self.turret {
            audio::play_vol("turret_close", 0.7);
        }
        self.turret = false;
        self.zoom_on = false;
        self.arb = Arb::Idle;
        // Plasma Heat Blast charge doesn't carry over a weapon switch (user).
        self.heat = 0.0;
        if self.bfg_charge.take().is_some() {
            viewmodel::bfg_charge(None);
        }
    }

    /// Gun kick for the shot just fired (procedural, on top of Doom's own fire clip).
    fn kick(&mut self) {
        let m = self.cfg.recoil.get(self.weapon).copied().unwrap_or(1.0);
        let imp = RECOIL[self.weapon].0 * m;
        if (self.weapon == 1 || self.weapon == 5) && m > 0.0 {
            self.push_goal = m;
            self.recoil_yaw = (self.shots as f32 * 91.7).sin();
        }
        if imp > 0.0 && !self.turret {
            self.recoil_v += imp;
            self.recoil_yaw = (self.shots as f32 * 91.7).sin();
            self.snap = m;
        }
    }

    fn update_mods(&mut self, dt: f32, alt_down: bool, alt_pressed: bool, alt_released: bool, enemies: &[Enemy]) {
        let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        let cam = game::camera();
        let w = &WEAPONS[self.weapon];
        let mult = self.dmg_mult();
        self.sticky_cd = (self.sticky_cd - dt).max(0.0);

        // Combat Shotgun - Sticky Bombs: a bomb that sticks to what it hits and blows up.
        // Sticky Bombs, fully upgraded like Doom: Five Spot (5-bomb magazine, one per click),
        // Quick Rack (reload 20% faster), Bigger Boom (+45% blast).
        if self.weapon == 0 && (alt_pressed || alt_down) && self.sticky_mag > 0 && self.sticky_cd <= 0.0 && self.fire_cd <= 0.0 && self.ammo[w.ammo.index()] >= 1 {
            if let Some((cp, fwd)) = cam {
                self.sticky_mag -= 1;
                self.sticky_cd = 0.6; // one bomb every 0.6 s (user, timed in Doom)
                self.sticky_idle = 0.0;
                self.sticky_charge = 0.0;
                if !self.cfg.infinite_ammo {
                    self.ammo[w.ammo.index()] -= 1;
                }
                // The last bomb leaves the loader back (empty) until the magazine reloads.
                self.vm_play(if self.sticky_mag == 0 { "sticky_last" } else { "sticky" }, false);
                audio::play("sticky_fire");
                viewmodel::muzzle_flash(0);
                let hit = damage::trace(cp, fwd, 80.0, 0.12);
                log::info!("sticky bomb -> {:.1} m, on demon: {}, {} left", hit.dist, hit.chr.is_some(), self.sticky_mag);
                let barrel = muzzle_world().unwrap_or(cp + fwd);
                crate::fx::shot(crate::fx::STICKY, barrel, vec![(hit.pos, hit.chr.is_some())]);
                let travel = hit.dist / crate::fx::LOOKS[crate::fx::STICKY].speed;
                self.stuck.push(Stuck::new(hit.chr, hit.pos, travel, 1.0, 3.0 * 1.45, 260.0 * mult, "fire", "sticky_explode", 3));
                if self.sticky_mag == 0 {
                    self.sticky_reload = STICKY_RELOAD;
                    self.sticky_charge = 0.0;
                    self.later.push((0.6, "sticky_reload"));
                }
            }
        }
        self.sticky_idle += dt;
        if self.sticky_reload > 0.0 {
            // Empty magazine: Doom's reload layer starts at 29.3% of the reload and is stretched
            // over the rest of it (decl: dischargeAdditiveAnimFiredSlaveToOverheatDelay*).
            let rest = STICKY_RELOAD * (1.0 - 0.293);
            if self.weapon == 0 && self.sticky_reload > rest && self.sticky_reload - dt <= rest
                && (self.vm_clip == "sticky_empty" || self.vm_clip == "idle" || self.vm_clip == "sticky_last")
            {
                let len = viewmodel::clip_len(viewmodel::FOLDERS[0], "sticky_reload").unwrap_or(1.8);
                self.vm_play("sticky_reload", false);
                self.vm_speed = len / rest;
            }
            self.sticky_reload -= dt;
            if self.sticky_reload <= 0.0 {
                self.sticky_mag = STICKY_MAG;
                self.sticky_flash_at = self.time;
                self.sticky_flash_all = true;
                if self.weapon == 0 {
                    audio::play_vol("sticky_ready", 0.8);
                }
            }
        } else if self.sticky_mag < STICKY_MAG && self.sticky_idle >= 1.0 {
            // A partly used magazine recharges one bomb at a time (Doom: the arrow fills, then
            // flashes green with the passive reload sound).
            self.sticky_charge += dt / STICKY_RECHARGE;
            if self.sticky_charge >= 1.0 {
                self.sticky_charge = 0.0;
                self.sticky_mag += 1;
                self.sticky_flash_at = self.time;
                self.sticky_flash_all = false;
                if self.weapon == 0 {
                    audio::play_vol("sticky_recharge", 0.8);
                }
            }
        } else {
            self.sticky_charge = 0.0;
        }

        // Heavy Cannon - Precision Bolt: hold to scope in (Doom's zoom), fire a bolt.
        if !alt_down {
            self.zoom_latch = false;
        }
        let want_zoom = self.weapon == 1 && alt_down && !self.zoom_latch && self.glory.is_none();
        if want_zoom != self.zoom_on {
            self.zoom_on = want_zoom;
            if self.weapon == 1 {
                audio::play_vol(if want_zoom { "hc_zoom_in" } else { "hc_zoom_out" }, 0.8);
                // Scoping lowers the gun out of frame (then it stays hidden while scoped);
                // unscoping brings it back up.
                self.vm_queue.clear();
                self.vm_play(if want_zoom { "bringdown" } else { "bringup" }, false);
                // Played fast: the gun is out of frame in ~0.08 s while the scope zooms in.
                let raw = viewmodel::clip_len(viewmodel::FOLDERS[1], "bringdown").unwrap_or(0.4);
                self.vm_speed = (raw / 0.08).max(1.6);
                self.scope_down_at = if want_zoom { self.time + raw / self.vm_speed } else { f32::MAX };
            }
        }
        // The scope zooms in while the gun drops out of frame (both start on the press).
        let zt = if self.zoom_on { 1.0 } else { 0.0 };
        self.zoom += (zt - self.zoom) * (dt * 20.0).min(1.0);
        // Arbalest: the view zooms in a little while it is drawn (Doom), and back out on release.
        let arb_t = if self.weapon == 5 && self.arb != Arb::Idle { 1.0 } else { 0.0 };
        self.arb_zoom += (arb_t - self.arb_zoom) * (dt * 14.0).min(1.0);
        set_zoom((1.0 - 0.67 * self.zoom) * (1.0 - 0.35 * self.arb_zoom));
        // Aim slows with the zoom (about the FOV ratio: ~0.35x fully scoped). Ballista aim (right
        // click) slows too: ballista_aim_look (live) at full draw - it didn't at all (user).
        let look = (1.0 - 0.65 * self.zoom) * (1.0 - (1.0 - self.cfg.ballista_aim_look.clamp(0.1, 1.0)) * self.arb_zoom);
        crate::remap::LOOK_SCALE.store((look * 1000.0) as u32, std::sync::atomic::Ordering::Relaxed);

        // Plasma Rifle - Heat Blast: release the built-up heat as a shockwave in front.
        if self.weapon == 2 && alt_pressed && self.heat >= 0.34 {
            if let Some((cp, fwd)) = cam {
                let power = self.heat;
                self.vm_play("heat_blast", false);
                audio::play("heat_blast");
                let center = cp + fwd * 3.5;
                let n = damage::explode(center, 3.0 + 3.0 * power, 900.0 * power * mult);
                self.break_area(center, 3.0 + 3.0 * power);
                if n > 0 {
                    self.last_combat = self.time;
                }
                for e in enemies.iter().filter(|e| is_hostile(e) && e.pos().distance(center) < 3.0 + 3.0 * power) {
                    self.hitters.hit_on(Some(e.chr.field_ins_handle), 3, e.pos() + Vec3::Y, fwd);
                }
                let barrel = muzzle_world().unwrap_or(cp + fwd);
                crate::fx::shot(crate::fx::HEAT_BLAST, barrel, vec![(cp + fwd * (3.0 + 3.0 * power), true)]);
                // Quick Fire: shorter firing delay after a blast. Power Surge (mastery): a full
                // blast boosts plasma damage for a while.
                self.fire_cd = 0.5 * 0.75;
                if power >= 0.99 {
                    self.surge_until = self.time + 6.0;
                    self.msg("POWER SURGE");
                }
                self.heat = 0.0;
            }
        }
        if self.weapon != 2 {
            self.heat = (self.heat - dt * 0.05).max(0.0);
        }
        viewmodel::set_heat(if self.weapon == 2 { self.heat } else { -1.0 });
        // Arbalest: its core glows hotter as it is drawn and charged (Doom), and fades after the shot.
        let want_glow = if self.weapon == 5 { self.arb_charge() } else { 0.0 };
        self.arb_glow += (want_glow - self.arb_glow) * (dt * if want_glow > self.arb_glow { 8.0 } else { 3.0 }).min(1.0);
        viewmodel::set_glow(self.arb_glow);

        // Rocket Launcher - Remote Detonate: blow the newest rocket up mid-flight.
        if self.weapon == 3 && alt_pressed {
            if let Some(b) = self.blasts.iter_mut().rev().find(|b| b.er_blast == Some(3) && b.t > 0.0) {
                let k = (1.0 - b.t / b.total).clamp(0.0, 1.0);
                let at = b.origin.lerp(b.at, k);
                b.at = at;
                b.t = 0.0;
                crate::fx::cut_rocket(at);
                audio::play("rocket_detonate");
                // Concussive blast: staggers everything around without damage.
                for e in enemies.iter().filter(|e| is_hostile(e) && e.pos().distance(at) < 5.0) {
                    self.hitters.hit_on(Some(e.chr.field_ins_handle), 3, e.pos() + Vec3::Y, (e.pos() - at).normalize_or_zero());
                }
                // Proximity Flare + Explosive Array (mastery): detonated near demons, extra
                // explosives go off around the blast.
                if enemies.iter().any(|e| is_hostile(e) && e.pos().distance(at) < 4.0) {
                    for k in 0..3 {
                        let a = k as f32 * std::f32::consts::TAU / 3.0;
                        let p = at + Vec3::new(a.cos(), 0.0, a.sin()) * 2.0;
                        self.stuck.push(Stuck::new(None, p, 0.0, 0.25 + 0.12 * k as f32, 2.5, 180.0 * mult, "fire", "sticky_explode", 3));
                    }
                }
            }
        }

        // Ballista - Arbalest: hold to draw and charge, release to fire a bolt that sticks and
        // explodes. Releasing early lowers it.
        if self.weapon == 5 {
            match self.arb {
                Arb::Idle => {
                    if alt_pressed && self.ammo[w.ammo.index()] >= ARBALEST_AMMO {
                        self.arb = Arb::Into(self.time);
                        self.vm_play("arb_into", false);
                        audio::play("arb_into");
                    }
                }
                Arb::Into(t0) => {
                    let len = viewmodel::clip_len(viewmodel::FOLDERS[5], "arb_into").unwrap_or(0.35);
                    if !alt_down {
                        self.arb = Arb::Idle;
                        self.vm_play("arb_out", false);
                        audio::play("arb_out");
                    } else if self.time - t0 >= len {
                        self.arb = Arb::Charging(self.time);
                        self.vm_play("arb_charge", false);
                    }
                }
                Arb::Charging(t0) => {
                    let len = viewmodel::clip_len(viewmodel::FOLDERS[5], "arb_charge").unwrap_or(0.75);
                    if !alt_down {
                        self.arb = Arb::Idle;
                        self.vm_play("arb_out", false);
                        audio::play("arb_out");
                    } else if self.time - t0 >= len {
                        self.arb = Arb::Charged;
                        self.vm_play("arb_idle", true);
                        audio::play("arb_charged");
                    }
                }
                Arb::Charged => {
                    if alt_released || !alt_down {
                        self.arb = Arb::Idle;
                        if let Some((cp, fwd)) = cam {
                            if !self.cfg.infinite_ammo {
                                self.ammo[w.ammo.index()] -= ARBALEST_AMMO;
                            }
                            self.vm_play("arb_fire", false);
                            self.vm_speed = 1.6;
                            self.kick();
                            let len = viewmodel::clip_len(viewmodel::FOLDERS[5], "arb_fire").unwrap_or(0.5) / 1.6;
                            // Doom lowers the drawn arbalest with ballista_to_idle after a shot.
                            let after = if viewmodel::clip_len(viewmodel::FOLDERS[5], "arb_to_idle").is_some() { "arb_to_idle" } else { "arb_out" };
                            self.vm_queue = vec![(self.time + len, after, 1.5, 0.0, false)];
                            audio::play("arb_fire");
                            viewmodel::muzzle_flash(5);
                            let hit = damage::trace(cp, fwd, 200.0, 0.15);
                            let barrel = muzzle_world().unwrap_or(cp + fwd);
                            crate::fx::shot(crate::fx::ARBALEST, barrel, vec![(hit.pos, hit.chr.is_some())]);
                            self.fire_cd = 1.0;
                            if let Some(h) = hit.chr {
                                damage::apply_handle(h, 300.0 * mult);
                                self.hitters.hit_on(Some(h), 5, hit.pos, fwd);
                                self.last_combat = self.time;
                                // Instant Salvo (mastery): a direct hit recharges at once.
                                self.fire_cd = 0.15;
                            }
                            // Stronger Explosion: +60%.
                            self.stuck.push(Stuck::new(hit.chr, hit.pos, 0.0, 0.6, 3.5 * 1.6, 700.0 * mult, "fire", "arb_explode", 5));
                        }
                    }
                }
            }
        }

        // Chaingun - Mobile Turret: hold to unfold the three barrels (faster fire, slow walk).
        let want_turret = self.weapon == 6 && alt_down && self.glory.is_none();
        if want_turret != self.turret {
            self.turret = want_turret;
            if want_turret {
                self.vm_play("turret_into", false);
                self.vm_speed = 1.5; // Rapid Deploy: transform 50% faster
                let len = viewmodel::clip_len(viewmodel::FOLDERS[6], "turret_into").unwrap_or(0.6) / 1.5;
                self.vm_queue = vec![(self.time + len, "turret_idle", 1.0, 0.0, true)];
                audio::play("turret_open");
                self.fire_cd = self.fire_cd.max(len * 0.6);
            } else {
                self.vm_play("turret_out", false);
                self.vm_speed = 1.5;
                audio::play("turret_close");
                // Normal fire only once the barrels are folded back (firing during the fold
                // restarted the cluster spin with the barrels still out).
                let out = viewmodel::clip_len(viewmodel::FOLDERS[6], "turret_out").unwrap_or(0.8) / 1.5;
                self.fire_cd = self.fire_cd.max(out);
            }
        }

        // Stuck bombs / bolts: follow their demon, beep, explode.
        let mut boom: Vec<(Vec3, f32, f32, &'static str, usize)> = Vec::new();
        for st in &mut self.stuck {
            st.t += dt;
            if let Some(h) = st.target {
                match enemies.iter().find(|e| e.chr.field_ins_handle == h) {
                    Some(e) => st.pos = e.pos() + st.offset,
                    None => st.target = None,
                }
            }
            if st.t >= st.travel && !st.beeped {
                st.beeped = true;
                if st.sound == "sticky_explode" {
                    audio::play_vol("sticky_timer", 0.8);
                }
            }
            if st.t >= st.travel + st.fuse && !st.done {
                st.done = true;
                boom.push((st.pos, st.radius, st.damage, st.sound, st.react));
            }
        }
        self.stuck.retain(|s| !s.done);
        for (at, r, dmg, snd, react) in boom {
            let n = damage::explode(at, r, dmg);
            self.break_area(at, r);
            log::info!("mod explosion {snd} at {at:.1} r {r} dmg {dmg:.0}: {n} hit");
            if n > 0 {
                self.last_combat = self.time;
            }
            self.hitters.hit(react, at, Vec3::NEG_Y);
            crate::fx::explosion(at);
            audio::play(snd);
        }
        if let Ok(mut g) = crate::fx::STUCK.lock() {
            *g = self
                .stuck
                .iter()
                .filter(|s| s.t >= s.travel)
                .map(|s| (s.pos, s.ramp, ((s.t - s.travel) * 8.0) as i32 % 2 == 0))
                .collect();
        }
        let _ = owner;
    }

    // ---------------------------------------------------------------- frame

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.1);
        self.last = now;
        self.time += dt;
        // No character scans while the game loads or fades (half-built world; 1.2 crash fix).
        game::SCAN_OK.store(!game::now_loading() && !game::screen_faded(), std::sync::atomic::Ordering::Relaxed);

        self.cfg_check -= dt;
        if self.cfg_check <= 0.0 {
            self.cfg = config::get();
            self.cfg_check = 1.0;
        }

        if !params::try_apply(&self.cfg) {
            return;
        }

        let keys = self.watched_keys();
        self.input.update(&keys);
        if crate::remap::SETTINGS_OPEN.load(std::sync::atomic::Ordering::Relaxed) {
            self.input.suppress();
        }
        // the game doesn't see keys bound to Doom actions (follows the bindings)
        {
            let k = &self.cfg.keys;
            let mut bound: Vec<u16> = keys.iter().copied().filter(|v| !input::MOVE_KEYS.contains(v)).collect();
            bound.extend([k.settings, k.settings_alt, k.settings_pad]);
            crate::remap::set_bound_keys(&bound, k.jump);
            // the pad's bound buttons are hidden from the game too; Doom interact = its interact
            crate::gamepad::set_bound(&bound, k.interact_pad, self.cfg.pad_er_interact);
        }
        bridge::tick(self, dt);
        // (debug: loose prop watcher, every 0.2 s - off; set PROP_WATCH to look again)
        const PROP_WATCH: bool = false;
        if PROP_WATCH {
            static NEXT: std::sync::Mutex<f32> = std::sync::Mutex::new(0.0);
            if let (Ok(mut n), Some(p)) = (NEXT.lock(), game::player()) {
                if self.time >= *n {
                    *n = self.time + 0.2;
                    game::prop_watch(game::chr_pos(&p.chr_ins));
                }
            }
        }

        // the pad's Doom buttons are only hidden from the game while the Doom weapon is out: every
        // screen that puts it away (menus, tutorials, graces - the grace menu is no menu screen,
        // just the sitting animation - doors, loading, cutscenes, death) gets the whole pad (user)
        crate::gamepad::GAMEPLAY.store(
            self.ready && game::player().is_some() && !game::menu_open() && !game::now_loading() && !game::in_cutscene(),
            std::sync::atomic::Ordering::Relaxed,
        );
        let Some(player) = game::player() else {
            // Left the world (title screen, or a death reload): this character's state is filed
            // NOW, before the next world's HP and leftover pickups can be read as its own (the
            // first second after a load used to land in the previous character's entry - user).
            if self.new_char_checked && !self.saved_character.is_empty() {
                self.char_states.insert(self.saved_character.clone(), self.state_json());
                self.save_cd = 0.0;
                self.save_now();
            }
            self.pickups.list.clear();
            self.fall_land_t = 0.0;
            self.fall_hp = 0;
            self.prev_hp = -1;
            self.in_world_for = 0.0;
            // Back at the title screen: the next character loaded gets checked again, and its
            // first HP reading is a fresh start - not a "hit" from the last character's HP (a
            // switch from a bigger character ate the armor - user).
            self.new_char_checked = false;
            self.pool_prev = 0;
            // the shield rate belonged to that character: the next one starts without it (its
            // max health was divided by the last one's rate and clipped - user)
            if self.shield_rate != 1.0 || self.rate_pending.is_some() {
                params::set_shield_rate(1.0);
            }
            self.shield_rate = 1.0;
            self.rate_pending = None;
            self.max_seen = 0;
            self.shield_apply_t = 0.0;
            return;
        };
        if player.chr_ins.modules.data.max_hp > 0 {
            self.in_world_for += dt;
        }
        if self.in_world_for > 1.0 && !self.new_char_checked {
            self.check_new_character();
        }

        {
            let p = game::chr_pos(&player.chr_ins);
            if let Some(lp) = self.last_pos {
                let d = Vec3::new(p.x - lp.x, 0.0, p.z - lp.z).length();
                if d > 40.0 {
                    // Warp / fast travel (always behind a fade / loading screen): models get
                    // rebuilt, wait again before touching them. A jump with the screen clear is the
                    // game re-basing our position on a map-tile change - that used to hide the gun
                    // and HUD for a moment in the middle of a fight (user).
                    let faded = game::screen_faded();
                    log::info!("position jump {d:.0} m (screen faded {faded}){}", if faded { ": warp" } else { ": map tile change, ignored" });
                    if faded {
                        self.in_world_for = 0.0;
                    }
                    // our stored positions are in the old frame now: forget them (the floor /
                    // out-of-the-world rescue "restored" a glory teleport across a tile border to
                    // the stale spot - out of the map, user)
                    self.last_safe = None;
                    // (and the fall height: a warp mid-air is no landing from a height)
                    self.air_peak_y = f32::NEG_INFINITY;
                }
                let inst = if dt > 0.0 { d / dt } else { 0.0 };
                // Teleports / respawns would spike it; clamp and smooth.
                self.speed += (inst.min(15.0) - self.speed) * (dt * 8.0).min(1.0);
                // Sideways speed along the camera's right (weapon sway).
                if let (Some((_, fwd)), true) = (game::camera(), dt > 0.0 && d < 40.0) {
                    let flat = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
                    let right = Vec3::Y.cross(flat).normalize_or(Vec3::X);
                    let lat = (Vec3::new(p.x - lp.x, 0.0, p.z - lp.z) / dt).dot(right).clamp(-15.0, 15.0);
                    // A dash has no gun animation in Doom (user): its sideways burst doesn't tilt.
                    let lat = if self.dash_t > 0.0 { 0.0 } else { lat };
                    self.sway_lat += (lat - self.sway_lat) * (dt * 10.0).min(1.0);
                }
            }
            self.last_pos = Some(p);
        }

        // Z: mark the enemy under the crosshair (it keeps a health bar).
        if self.input.pressed3(self.cfg.keys.mark, self.cfg.keys.mark_alt, self.cfg.keys.mark_pad) && !self.wheel_open {
            self.toggle_mark();
        }

        // F8: stuck in a wall / the floor - lift unstick_height (live, 4 m) per press.
        if self.input.pressed3(self.cfg.keys.unstick, self.cfg.keys.unstick_alt, self.cfg.keys.unstick_pad) {
            if let Some(p) = game::player() {
                let ph = &mut p.chr_ins.modules.physics;
                ph.position.1 += self.cfg.unstick_height.clamp(0.5, 50.0);
                ph.chr_proxy_pos_update_requested = true;
                self.air_vy = None;
                self.air_carry = Vec3::ZERO;
                self.air_push = Vec3::ZERO;
                self.ground_vel = Vec3::ZERO;
                self.dash_t = 0.0;
                self.last_safe = None;
                log::info!("unstick: lifted {:.1} m to y {:.2}", self.cfg.unstick_height, ph.position.1);
            }
        }

        // Music test (F7 on / off; with it on, Doom mode: F6 next suite, F5 suite from the top, F4 next
        // piece, F3 skip the playing piece for good; track mode: F6 next, F5 back to the start
        // point, F2 / F4 -5 / +5 s, F3 mark the start point where you are)
        {
            use std::sync::atomic::Ordering::Relaxed;
            let k = self.cfg.keys.clone();
            if self.input.pressed(k.music_test) && k.music_test != 0 {
                let on = !audio::MUSIC_TEST.load(Relaxed);
                audio::MUSIC_TEST.store(on, Relaxed);
                self.msg(if on { "MUSIC TEST ON" } else { "MUSIC TEST OFF" });
            }
            if audio::MUSIC_TEST.load(Relaxed) {
                for (key, cmd) in [(k.music_next, 1u8), (k.music_restart, 2), (k.music_back, 3), (k.music_fwd, 4), (k.music_mark, 5)] {
                    if key != 0 && self.input.pressed(key) {
                        audio::MUSIC_CMD.store(cmd, Relaxed);
                        if cmd == 5 {
                            self.msg(if self.cfg.music_mode == "tracks" { "MUSIC START POINT SET" } else { "PIECE SKIPPED FOR GOOD" });
                        }
                    }
                }
            }
        }

        // F9: Doom layer off/on. Off = plain Elden Ring (its HUD, body, movement, attacks and
        // keys); erfps2's camera toggles on its own (lock-on with nothing targeted).
        if self.input.pressed3(self.cfg.keys.doom_toggle, self.cfg.keys.doom_toggle_alt, self.cfg.keys.doom_toggle_pad) {
            self.doom_off = !self.doom_off;
            log::info!("doom layer {}", if self.doom_off { "OFF" } else { "ON" });
            if !self.doom_off {
                self.ready = false; // draw the gun again
                self.clear_for = 0.0;
            }
        }
        if self.doom_off {
            if self.glory_invuln {
                player.chr_ins.debug_flags.set_disabled_hit(false);
                set_no_dead(false);
                self.glory_invuln = false;
            }
            crate::remap::ENABLED.store(false, std::sync::atomic::Ordering::Relaxed);
            crate::remap::ER_JUMP.store(true, std::sync::atomic::Ordering::Relaxed);
            player.chr_ins.modules.physics.motion_multiplier = 1.0;
            game::hide_er_hud(false);
            game::hide_player_body(false);
            self.ready = false;
            self.vm_visible = false;
            viewmodel::set_pose(None);
            return;
        }
        crate::remap::ENABLED.store(true, std::sync::atomic::Ordering::Relaxed);

        // No Doom input while a menu, a loading screen or a cutscene is up (clicking through the
        // settings fired the gun). Keys held through it count again only after a release.
        let in_menu = game::menu_open() && !self.wheel_open;
        let faded = game::screen_faded();
        if !self.ready || in_menu || self.menu_t > 0.0 || faded || game::in_cutscene() {
            self.input.suppress();
        }

        if self.watch_anims > 0.0 {
            self.watch_anims -= dt;
            let a = current_anim(&player.chr_ins);
            if a != self.last_anim {
                log::info!("anim {} -> {a} (t={:.2})", self.last_anim, self.time);
                self.last_anim = a;
            }
        }

        for (_, t) in self.messages.iter_mut() {
            *t -= dt;
        }
        while self.messages.front().is_some_and(|(_, t)| *t <= 0.0) {
            self.messages.pop_front();
        }

        // ---- fall damage refund OFF (user, 1.2): it gave back 99% of ANY hp lost in the 0.6 s
        // after touching ground, so enemy hits right after a jump / meathook / dash landing were
        // cancelled. Full fall damage now (ER's own, see disable_fall_damage below).
        /*
        // ---- fall damage cut by 99% (user): remember HP while airborne and give back 99% of
        // what the landing took. No no-death flag any more: it kept the player alive falling
        // through the void (low-res collision far below always counted as "ground below"), so
        // cliffs, pits and the game's own lethal-height falls kill again.
        if self.cfg.doom_move {
            let ph = &player.chr_ins.modules.physics;
            let pos = game::hpos(&ph.position);
            let hp = player.chr_ins.modules.data.hp;
            if !ph.is_touching_ground && hp > 0 {
                self.fall_air_t += dt;
                self.fall_hp = hp;
                self.fall_land_t = 0.0;
                let _ = pos;
                if self.fall_guard {
                    set_no_dead(false);
                    self.fall_guard = false;
                }
            } else {
                if self.fall_air_t > 0.0 {
                    self.fall_land_t = 0.6;
                }
                self.fall_air_t = 0.0;
                if self.fall_land_t > 0.0 {
                    self.fall_land_t -= dt;
                    let data = &mut player.chr_ins.modules.data;
                    if data.hp > 0 && data.hp < self.fall_hp {
                        let back = ((self.fall_hp - data.hp) as f32 * 0.99).round() as i32;
                        log::info!("fall damage cut 99%: {} -> {}", data.hp, data.hp + back);
                        data.hp = (data.hp + back).min(data.max_hp);
                        self.fall_hp = data.hp;
                    }
                    self.prev_hp = self.prev_hp.max(data.hp);
                } else if self.fall_guard {
                    set_no_dead(false);
                    self.fall_guard = false;
                }
            }
        }
        */

        // ---- armor lives inside the game's own HP. An SpEffect (params::SHIELD_SPEFFECT) raises
        // the game's max HP by exactly the shield, so its damage math is exact: a hit takes the
        // shield first, then health, and only kills when it's bigger than both together.
        // Instant kills still kill. The HUD shows health and shield separately.
        // the game's max HP from its stats alone (no effects): what our shield rate multiplies
        let stat_max = unsafe { player.player_game_data.as_ref() }.base_max_hp as i32;
        let data = &mut player.chr_ins.modules.data;
        // The game's max HP without our shield (its own stats, talismans, buffs). A new shield
        // rate shows up in the game's max HP a frame later: until the max changes, divide by the
        // rate it was built with (dividing by the new one clipped health on the frame shields
        // were added).
        if self.rate_pending.is_some() && data.max_hp != self.max_seen {
            self.rate_pending = None;
        }
        let rate = self.rate_pending.unwrap_or(self.shield_rate).max(1.0);
        self.max_seen = data.max_hp;
        self.base_max = ((data.max_hp as f32) / rate).round() as i32;
        // fixed health max (max_health): the rate scales the game's stat max to it, plus the shield
        let fixed = self.cfg.max_health > 0 && stat_max > 0;
        if fixed {
            self.base_max = self.cfg.max_health;
        }
        let pool = data.hp;
        if pool <= 0 {
            // Dead: the shield goes with you.
            self.armor = 0;
            self.health = 0;
        } else {
            if self.pool_prev <= 0 {
                // First frame alive (load / respawn): the game's HP is all health.
                self.health = pool.min(self.base_max);
            } else {
                let delta = pool - self.pool_prev;
                if delta < 0 {
                    let hit = -delta;
                    let absorbed = hit.min(self.armor);
                    let had = self.armor;
                    self.armor -= absorbed;
                    self.health -= hit - absorbed;
                    if had > 0 && self.armor <= 0 {
                        audio::play("armor_break");
                    }
                    self.last_combat = self.time;
                } else if delta > 0 {
                    // Healing (flasks, graces, glory kills) only fills health, never the shield.
                    self.health += delta;
                }
            }
            if self.god {
                self.health = self.base_max;
            }
            self.armor = self.armor.clamp(0, self.cfg.armor_max);
            self.health = self.health.clamp(1, self.base_max.max(1));
            // HP can only go up to the max the game has this frame (a new shield rate applies
            // next frame); the pool catches up then.
            data.hp = (self.health + self.armor).min(data.max_hp);
        }
        self.pool_prev = data.hp;
        self.hud_pool = (data.hp, data.max_hp);
        // Shield effect: rate for the current shield, (re)applied every few seconds (respawns,
        // loading screens drop it).
        let want = if fixed {
            (self.base_max as f32 + self.armor as f32 + 0.5) / stat_max as f32
        } else if self.base_max > 0 {
            (self.base_max + self.armor) as f32 / self.base_max as f32
        } else {
            1.0
        };
        if (want - self.shield_rate).abs() > 1e-5 {
            params::set_shield_rate(want);
            if self.rate_pending.is_none() {
                self.rate_pending = Some(self.shield_rate);
            }
            self.shield_rate = want;
        }
        self.shield_apply_t -= dt;
        if self.shield_apply_t <= 0.0 && data.hp > 0 {
            self.shield_apply_t = 3.0;
            use eldenring::cs::ChrInsExt;
            player.chr_ins.apply_speffect(params::SHIELD_SPEFFECT, true);
        }
        // HP restored to full from below half (grace rest, respawn): resupply like a Doom checkpoint.
        let full_heal = self.health >= self.base_max && self.prev_hp >= 0 && self.prev_hp * 2 < self.base_max;
        self.prev_hp = self.health;
        let max_hp = self.base_max;
        if full_heal {
            self.resupply();
        }
        self.update_level(unsafe { player.player_game_data.as_ref() }.level);
        self.save_if_changed(dt);

        // ---- timers
        self.fire_cd -= dt;
        if self.belch_cd > 0.0 && self.belch_cd - dt <= 0.0 {
            audio::play_vol("belch_ready", 0.6);
        }
        self.belch_cd = (self.belch_cd - dt).max(0.0);
        self.dash_charges = (self.dash_charges + dt / self.cfg.dash_recharge).min(DASH_CHARGES);
        let now_t = self.time;
        let mut blasted = false;
        let mut blast_hits: Vec<(usize, Vec3)> = Vec::new();
        let mut blast_breaks: Vec<(Vec3, f32)> = Vec::new();
        self.blasts.retain_mut(|b| {
            b.t -= dt;
            if b.t <= 0.0 {
                damage::NO_CATCH.store(b.er_blast == Some(BFG as i32), std::sync::atomic::Ordering::Relaxed);
                if damage::explode(b.at, b.radius, b.amount) > 0 {
                    blasted = true;
                }
                damage::NO_CATCH.store(false, std::sync::atomic::Ordering::Relaxed);
                blast_breaks.push((b.at, b.radius));
                // ER blast reaction (knockback, aggro) through the weapon's hitter.
                if let Some(slot) = b.er_blast {
                    blast_hits.push((slot as usize, b.at));
                }
            }
            b.t > 0.0
        });
        if blasted {
            self.last_combat = now_t;
        }
        for (at, r) in blast_breaks {
            self.break_area(at, r);
        }
        for (slot, at) in blast_hits {
            self.hitters.hit(slot, at, Vec3::NEG_Y);
        }
        let hit_owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        self.hitters.update(dt, hit_owner);
        self.later.retain_mut(|(t, ev)| {
            *t -= dt;
            if *t <= 0.0 {
                audio::play(ev);
            }
            *t > 0.0
        });
        if !self.later_look.is_empty() {
            let now = self.time;
            let due: Vec<(f32, Vec3, Vec3, f32)> = self.later_look.iter().copied().filter(|(t, ..)| *t <= now).collect();
            self.later_look.retain(|(t, ..)| *t > now);
            for (_, from, dir, len) in due {
                if self.break_along(from, dir, len, 0.15) {
                    log::info!("second look broke something along a shot");
                }
            }
        }
        if !self.later_break.is_empty() {
            let now = self.time;
            let due: Vec<(f32, Vec3, Vec3)> = self.later_break.iter().copied().filter(|(t, _, _)| *t <= now).collect();
            self.later_break.retain(|(t, _, _)| *t > now);
            let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
            for (_, at, dir) in due {
                let _ = bullet::spawn(owner, params::BREAKER_BULLET, at - dir * 0.2, dir);
            }
        }
        if self.chainsaw_fuel < 1.0 {
            self.chainsaw_fuel = (self.chainsaw_fuel + dt * CHAINSAW_REGEN).min(1.0);
            // the regenerating fuel pip is full: the chainsaw is ready again (Doom frag-grenade recharged blip,
            // tools/convert_audio.py chainsaw_ready - Doom has no separate "chainsaw ready" event)
            // (not in the first seconds in the world: a save at 0.99 fuel dinged right after
            // loading - user)
            if self.chainsaw_fuel >= 1.0 && self.in_world_for > 5.0 {
                audio::play_vol("chainsaw_ready", 0.7);
            }
        }

        let k = self.cfg.keys.clone();

        // ---- weapon selection
        // Weapon wheel (hold): mouse direction highlights a slot, release selects it.
        let was_open = self.wheel_open;
        // Doom: tap the wheel key to swap to the last weapon, hold it to open the wheel.
        // No weapon switching while the gun is put away (cutscenes, NPC talk, events, loading),
        // an Elden Ring menu or the settings window is up: the switch sound played (user).
        let armed = self.ready
            && self.in_event.is_none()
            && !game::menu_open()
            && !crate::remap::SETTINGS_OPEN.load(std::sync::atomic::Ordering::Relaxed);
        let wheel_key = armed && self.input.down3(k.weapon_wheel, k.weapon_wheel_alt, k.weapon_wheel_pad);
        if wheel_key {
            self.wheel_held += dt;
        } else {
            if !was_open && self.wheel_held > 0.0 && self.wheel_held < WHEEL_HOLD {
                // the Crucible isn't part of the quick swap: a tap brings back the gun you had
                if self.crucible_out {
                    self.crucible_to_gun();
                } else {
                    self.select(self.last_weapon);
                }
            }
            self.wheel_held = 0.0;
        }
        self.wheel_open = wheel_key && self.wheel_held >= WHEEL_HOLD;
        crate::remap::WHEEL_OPEN.store(self.wheel_open, std::sync::atomic::Ordering::Relaxed);
        // Doom slows time while the wheel is open.
        self.set_slowmo(self.wheel_open);
        if !self.wheel_open {
            self.wheel_t = 0.0;
        }
        if self.wheel_open && !was_open {
            crate::remap::WHEEL_DX.store(0, std::sync::atomic::Ordering::Relaxed);
            crate::remap::WHEEL_DY.store(0, std::sync::atomic::Ordering::Relaxed);
            self.wheel_pick = None;
        }
        if self.wheel_open {
            // the right stick steers it too (pushed past half way; the last pick stays when it
            // springs back)
            let (rx, ry) = self.input.rstick;
            if rx * rx + ry * ry > 0.25 {
                crate::remap::WHEEL_DX.store((rx * 300.0) as i32, std::sync::atomic::Ordering::Relaxed);
                crate::remap::WHEEL_DY.store((-ry * 300.0) as i32, std::sync::atomic::Ordering::Relaxed);
            }
            let x = crate::remap::WHEEL_DX.load(std::sync::atomic::Ordering::Relaxed) as f32;
            let y = crate::remap::WHEEL_DY.load(std::sync::atomic::Ordering::Relaxed) as f32;
            if x * x + y * y > 60.0 * 60.0 {
                // Slot 0 at the top, clockwise (matches the HUD wheel layout).
                let ang = x.atan2(-y).rem_euclid(std::f32::consts::TAU);
                let n = WEAPONS.len() as f32;
                let pos = ((ang / std::f32::consts::TAU * n).round() as usize) % WEAPONS.len();
                self.wheel_pick = Some(crate::doomhud::WHEEL_ORDER[pos]);
            }
            self.wheel_t += dt;
        } else if was_open {
            log::info!(
                "wheel closed: dx {} dy {} pick {:?} (raw mouse packets seen {})",
                crate::remap::WHEEL_DX.load(std::sync::atomic::Ordering::Relaxed),
                crate::remap::WHEEL_DY.load(std::sync::atomic::Ordering::Relaxed),
                self.wheel_pick,
                crate::remap::RAW_SEEN.load(std::sync::atomic::Ordering::Relaxed)
            );
            if let Some(slot) = self.wheel_pick.take() {
                self.select(slot);
            }
        }
        // (inspect mode uses the number keys to turn the gun)
        let inspecting = crate::viewmodel::INSPECT_ON.load(std::sync::atomic::Ordering::Relaxed);
        for (i, &vk) in k.slots.iter().enumerate().filter(|_| armed) {
            if self.input.pressed(vk) && !inspecting {
                self.select(i);
            }
        }
        if armed && k.last_weapon != 0 && self.input.pressed(k.last_weapon) {
            if self.crucible_out {
                self.crucible_to_gun();
            } else {
                self.select(self.last_weapon);
            }
        }
        // Mouse wheel: next / previous weapon in Doom's wheel order.
        let notches = crate::remap::WHEEL_NOTCHES.swap(0, std::sync::atomic::Ordering::Relaxed);
        // (settings window open: the wheel doesn't change weapons - user)
        let notches = if armed { notches } else { 0 };
        // Crucible out: the scroll wheel brings back the gun you had (no step to the next one)
        if notches != 0 && self.crucible_out {
            self.crucible_to_gun();
        } else if notches != 0 && !self.wheel_open {
            let order = crate::doomhud::WHEEL_ORDER;
            let n = order.len() as i32;
            let mut pos = order.iter().position(|&w| w == self.weapon).unwrap_or(0) as i32;
            // The scroll wheel skips weapons without ammo, like Doom (the number keys and the
            // weapon wheel can still pick an empty one).
            for _ in 0..notches.abs() {
                for _ in 0..n {
                    pos = (pos + if notches > 0 { -1 } else { 1 }).rem_euclid(n);
                    let w = &WEAPONS[order[pos as usize]];
                    if self.ammo[w.ammo.index()] >= w.ammo_per_shot || self.cfg.infinite_ammo {
                        break;
                    }
                }
            }
            self.select(order[pos as usize]);
            log::info!("wheel {notches:+} -> {} (sources {:#b})", WEAPONS[self.weapon].name, crate::remap::WHEEL_SOURCES.load(std::sync::atomic::Ordering::Relaxed));
        }

        // ---- out of ammo on every weapon: the chainsaw comes up (Doom); a left click uses it
        let all_empty = !self.cfg.infinite_ammo
            && (0..WEAPONS.len()).all(|i| !self.owned[i] || self.ammo[WEAPONS[i].ammo.index()] < WEAPONS[i].ammo_per_shot);
        if all_empty != self.saw_mode {
            self.saw_mode = all_empty;
            if all_empty {
                self.saw_drop = None;
                self.vm_play("bringup", false);
                self.msg("OUT OF AMMO - CHAINSAW");
            } else {
                // Ammo again (usually from the chainsaw kill): the saw finishes its kill, goes
                // down, then a gun comes back up (user): the selected one if it has ammo now,
                // otherwise the gun that got ammo (next in the wheel with some).
                self.saw_drop = Some(self.time.max(self.chainsaw_at + 0.6));
                let cw = &WEAPONS[self.weapon];
                if self.ammo[cw.ammo.index()] < cw.ammo_per_shot {
                    let order = crate::doomhud::WHEEL_ORDER;
                    let pos = order.iter().position(|&w| w == self.weapon).unwrap_or(0);
                    let next = (1..order.len()).map(|k| order[(pos + k) % order.len()]).find(|&i| {
                        self.owned[i] && self.ammo[WEAPONS[i].ammo.index()] >= WEAPONS[i].ammo_per_shot
                    });
                    if let Some(i) = next {
                        self.reset_mods();
                        self.last_weapon = self.weapon;
                        self.weapon = i;
                        self.msg(WEAPONS[i].name);
                    }
                }
            }
        }
        if self.saw_mode && self.input.pressed3(k.fire, k.fire_alt, k.fire_pad) && !self.wheel_open {
            self.saw_click = true;
        }

        // Out of ammo for this gun: when you try to fire it, switch to the next one in the wheel
        // that has some. Selecting an empty gun is allowed (user); it stays up until you shoot.
        if !self.saw_mode && !self.cfg.infinite_ammo && self.bfg_charge.is_none() && self.glory.is_none() && !self.wheel_open {
            let cw = &WEAPONS[self.weapon];
            let empty = self.ammo[cw.ammo.index()] < cw.ammo_per_shot;
            // ran dry by shooting it (it had ammo last frame, same gun): swap on its own, like Doom
            if self.dry_prev.0 == self.weapon && self.dry_prev.1 && empty {
                self.ran_dry = true;
            }
            if self.dry_prev.0 != self.weapon {
                self.ran_dry = false;
            }
            self.dry_prev = (self.weapon, !empty);
            if empty && (self.ran_dry || (self.input.down3(k.fire, k.fire_alt, k.fire_pad) && self.time - self.switched_at > 0.3)) && self.fire_cd <= 0.0 {
                let order = crate::doomhud::WHEEL_ORDER;
                let pos = order.iter().position(|&w| w == self.weapon).unwrap_or(0);
                let next = (1..order.len()).map(|k| order[(pos + k) % order.len()]).find(|&i| {
                    self.owned[i] && self.ammo[WEAPONS[i].ammo.index()] >= WEAPONS[i].ammo_per_shot
                });
                if let Some(i) = next {
                    self.select(i);
                    self.ran_dry = false;
                    // the swap only swaps: the new gun doesn't fire until the button is let go
                    self.fire_lock = self.input.down3(k.fire, k.fire_alt, k.fire_pad);
                    self.fire_buffer = 0.0;
                }
            }
        }

        // ---- firing
        let w = &WEAPONS[self.weapon];
        // Semi-autos remember a click made during the cooldown for a moment (it used to be lost,
        // which felt like the gun ignoring you).
        if self.input.pressed3(k.fire, k.fire_alt, k.fire_pad) {
            self.fire_buffer = 0.25;
        }
        self.fire_buffer = (self.fire_buffer - dt).max(0.0);
        if self.fire_lock {
            if self.input.down3(k.fire, k.fire_alt, k.fire_pad) {
                self.fire_buffer = 0.0;
            } else {
                self.fire_lock = false;
            }
        }
        // No shooting during melee: punches, Blood Punch, glory kills (and their finisher / hold),
        // chainsaw kills.
        let meleeing = self.time < self.melee_until
            || self.crucible_out
            || self.glory.is_some()
            || self.glory_hold.is_some_and(|(_, t0)| self.time < t0 + self.cfg.glory_hold)
            || self.time - self.chainsaw_at < 0.6
            // doors, gates, levers, fog walls (the game's event animations) and enemy grabs: no
            // shots or mods (user)
            || self.in_event.is_some()
            || self.grabbed;
        if meleeing {
            self.fire_buffer = 0.0;
        }
        // Combat Shotgun: shells or sticky bombs, never both (user) - no shells while the empty
        // magazine reloads (the whole reload) or right after a bomb (its 0.6 s cycle).
        let sticky_reloading = self.weapon == 0 && (self.sticky_reload > 0.0 || self.sticky_cd > 0.0);
        if sticky_reloading {
            self.fire_buffer = 0.0;
        }
        // (no shot while the chainsaw is still finishing / going down after ammo turned up: the
        // gun comes up first - a BFG fired with no draw after a boss refill - user)
        if self.saw_drop.is_some() {
            self.fire_buffer = 0.0;
        }
        let trigger = !self.fire_lock
            && self.saw_drop.is_none()
            && !meleeing
            && !sticky_reloading
            && if w.automatic {
                self.input.down3(k.fire, k.fire_alt, k.fire_pad)
            } else {
                self.fire_buffer > 0.0 || (self.input.down3(k.fire, k.fire_alt, k.fire_pad) && self.fire_cd <= -0.05)
            };
        if trigger && self.fire_cd <= 0.0 && !self.wheel_open {
            self.fire_buffer = 0.0;
        }
        let bolt_wait = self.weapon == 1 && self.zoom > 0.5 && self.time - self.bolt_at < BOLT_RECOVERY;
        if trigger && self.fire_cd <= 0.0 && !self.wheel_open && !bolt_wait && !self.saw_mode && !(self.weapon == 5 && self.arb != Arb::Idle) {
            let bfg = &WEAPONS[BFG];
            if self.weapon == BFG && (self.cfg.infinite_ammo || self.ammo[bfg.ammo.index()] >= bfg.ammo_per_shot) {
                // Doom's BFG winds up before the shot: fins open, the core spins up, then it fires.
                self.bfg_charge = Some(self.time);
                self.vm_play("charge", false);
                self.vm_speed = 1.6;
                self.fire_cd = BFG_CHARGE + 0.1;
                audio::play("bfg_charge");
                viewmodel::bfg_charge(Some(BFG_CHARGE));
            } else {
                self.fire();
            }
        }
        if let Some(t0) = self.bfg_charge {
            if self.weapon != BFG || self.glory.is_some() {
                self.bfg_charge = None;
                viewmodel::bfg_charge(None);
            } else if self.time - t0 >= BFG_CHARGE {
                self.bfg_charge = None;
                viewmodel::bfg_charge(None);
                self.fire_cd = 0.0;
                self.fire();
            }
        }

        // ---- weapon mods (alt-fire) and the Super Shotgun Meathook
        let alt_down = self.input.down3(k.alt_fire, k.alt_fire_alt, k.alt_fire_pad) && !self.wheel_open && !self.saw_mode && !meleeing;
        let alt_pressed = self.input.pressed3(k.alt_fire, k.alt_fire_alt, k.alt_fire_pad) && !self.wheel_open && !self.saw_mode && !meleeing;
        let alt_released = self.input.released3(k.alt_fire, k.alt_fire_alt, k.alt_fire_pad);
        let mod_enemies = game::enemies(150.0);
        self.update_mods(dt, alt_down, alt_pressed, alt_released, &mod_enemies);
        self.hook_cd = (self.hook_cd - dt).max(0.0);
        // Busy while the shot / reload plays - not its last ~11% (the settle after the shells are
        // in). A right click during it is kept for 0.4 s and fires the hook the moment it ends.
        // Reticle hook icon target (Super Shotgun only).
        self.hook_scan_t -= dt;
        // Home while the hook is out or recharging; a hookable demon otherwise (the HUD keeps only
        // one inside the brackets).
        self.hook_ready_for = if self.hook.is_some() || self.hook_cd > 0.0 { -1.0 } else { self.hook_ready_for.max(0.0) + dt };
        if self.weapon != SUPER_SHOTGUN || self.hook.is_some() || self.hook_cd > 0.0 {
            self.hook_icon = None;
        } else if self.hook_scan_t <= 0.0 {
            self.hook_scan_t = 0.1;
            self.hook_icon = self.meathook_target().map(|e| e.pos() + Vec3::Y * hook_grip(e.chr));
        }
        let ssg_len = viewmodel::clip_len(viewmodel::FOLDERS[SUPER_SHOTGUN], "fire").unwrap_or(1.53);
        let ssg_busy = (self.vm_clip == "fire" && self.vm_t < ssg_len * 0.89) || self.vm_clip == "fire_single" || !self.vm_queue.is_empty();
        if self.weapon == SUPER_SHOTGUN && self.input.pressed3(k.alt_fire, k.alt_fire_alt, k.alt_fire_pad) {
            self.hook_buffer = 0.4;
        }
        self.hook_buffer = (self.hook_buffer - dt).max(0.0);
        if self.weapon == SUPER_SHOTGUN && self.hook_buffer > 0.0 && self.hook.is_none() && self.hook_cd <= 0.0 && !ssg_busy {
            self.hook_buffer = 0.0;
            self.try_meathook();
        }

        // ---- movement intents (applied in post_physics)
        let on_ground = {
            let ph = &player.chr_ins.modules.physics;
            // standing_on_solid_ground lags behind take-off; touching_ground is immediate.
            ph.is_touching_ground
        };
        if on_ground && self.air_vy.is_none() && self.jump_grace <= 0.0 {
            self.double_jumped = false;
        }
        self.dash_ground = on_ground && self.jump_grace <= 0.0;
        if self.dash_ground {
            self.dash_landed = true;
        }
        if self.dash_lock && self.dash_charges >= DASH_CHARGES && self.dash_landed {
            self.dash_lock = false;
        }
        // On the ground, or in the air after landing since the last dash: the recharged dashes
        // are yours. (An air dash freezes them until the next landing.)
        if self.dash_ground || (self.dash_landed && !self.dash_lock) {
            self.air_dashes = self.dash_charges.floor();
        }
        // The ready sound when both dashes are back AND usable - not mid-air while they're still
        // locked, but on landing (user); not for each half either.
        let ready = self.dash_ready();
        if ready >= DASH_CHARGES as u32 && self.dash_ready_last < DASH_CHARGES as u32 {
            audio::play_vol("dash_recharge", 0.5);
        }
        self.dash_ready_last = ready;
        // On a ladder the game climbs: no Doom jumps, dashes or air control (they slid the
        // invisible body off the ladder while the climb animation carried on).
        self.on_ladder = game::on_ladder(&player.chr_ins);
        let event = game::in_event_anim(&player.chr_ins);
        if event != self.in_event {
            log::info!("event anim: {:?} -> {:?}", self.in_event, event);
            self.in_event = event;
        }
        // Fog walls, doors, levers, graces: the game's animation moves the body (as on a ladder).
        self.on_ladder |= event.is_some();
        // Grabbed by an enemy (its throw holds you: InThrowTarget, or DeathTarget when it kills):
        // the pair animation moves the body - you could walk out of the grab (user).
        let grabbed = {
            use eldenring::cs::ThrowNodeState;
            matches!(player.chr_ins.modules.throw.throw_node.throw_state, ThrowNodeState::InThrowTarget | ThrowNodeState::DeathTarget)
        };
        if grabbed != self.grabbed {
            log::info!("grabbed: {grabbed}");
            self.grabbed = grabbed;
        }
        self.on_ladder |= grabbed;
        // Full fall damage (user, 1.2: ER's own again); no heavy-landing stagger (it bobbed the view).
        if self.cfg.doom_move {
            player.chr_ins.modules.material.disable_fall_damage = false;
            player.chr_ins.modules.fall.disable_fall_motion = true;
            player.chr_ins.modules.fall.fall_timer = 0.0;
        }
        if self.on_ladder {
            self.air_vy = None;
            self.dash_t = 0.0;
            self.air_carry = Vec3::ZERO;
            self.air_push = Vec3::ZERO;
            self.ground_vel = Vec3::ZERO;
        }
        // Doom movement replaces ER's root-motion run (slow acceleration, slide on stop) and its
        // wind-up jump with our own controller (post_physics).
        let doom_move = self.cfg.doom_move && self.platform_t <= 0.0;
        crate::remap::ER_JUMP.store(!doom_move, std::sync::atomic::Ordering::Relaxed);
        // Ladders and scripted interactions keep the game's root motion (it walks you through
        // the fog wall / up the ladder).
        // Riding a lift standing still: the game's own motion back on - at 0 it also dropped the
        // lift's carry (jitter going down, the floor passing through us going up; user).
        let lifting = self.time < self.lift_until && self.wish_dir() == Vec3::ZERO;
        player.chr_ins.modules.physics.motion_multiplier = match (on_ground && !self.on_ladder, doom_move) {
            _ if lifting => 1.0,
            (true, true) => 0.0,
            (true, false) => self.cfg.move_speed_mult,
            _ => 1.0,
        };
        if doom_move && on_ground && !self.on_ladder && self.air_vy.is_none() && self.input.pressed3(k.jump, k.jump_alt, k.jump_pad) && self.glory.is_none() {
            let g = self.cfg.air_gravity;
            // Keep the climbing speed (Doom keeps your velocity): running up a 30 deg slope the
            // ground rises ~5 m/s, as fast as the jump itself - without it you never left the slope.
            // Uphill: the ground rises to meet you and cut the jump short ("bump bump bump" hopping
            // up a hill - user). The rise of the ground just ahead at your speed, unsmoothed
            // (ground_vy lags), times uphill_jump_boost (live) on top of the normal jump.
            let flat_vel = Vec3::new(self.ground_vel.x, 0.0, self.ground_vel.z);
            let speed = flat_vel.length();
            let climb_now = if speed > 0.5 {
                let p0 = game::player().map(|p| game::chr_pos(&p.chr_ins)).unwrap_or(Vec3::ZERO);
                let ahead = p0 + flat_vel / speed * 1.0;
                ground_height(ahead, 0.8).map_or(0.0, |gy| ((gy - p0.y) / 1.0).max(0.0) * speed)
            } else {
                0.0
            };
            let lift = climb_now.max(self.ground_vy.max(0.0)) * self.cfg.uphill_jump_boost.max(0.0);
            self.air_vy = Some((2.0 * g * self.cfg.jump_height.max(0.2)).sqrt() + lift);
            self.air_jumped = true;
            self.air_carry = self.ground_vel;
            self.jump_grace = 0.3;
            audio::play_vol("jump", 0.7);
            self.sway_kick(1.0);
        }

        if self.input.pressed3(k.dash, k.dash_alt, k.dash_pad) && !self.on_ladder && self.dash_charges >= 1.0 && self.dash_t <= 0.0
            && !self.dash_lock && (on_ground && self.jump_grace <= 0.0 || self.air_dashes >= 1.0)
        {
            // (right after a jump the game still says "on the ground": that's the air already)
            if !(on_ground && self.jump_grace <= 0.0) {
                self.air_dashes -= 1.0;
                self.dash_landed = false;
            }
            self.start_dash();
            // Locked until both are back and you're on the ground: after both dashes, or after
            // your second dash in the air (one air dash, the half reloads, dash again - user).
            if self.dash_charges < 1.0 || (!(on_ground && self.jump_grace <= 0.0) && self.air_dashes < 1.0) {
                self.dash_lock = true;
                self.dash_landed = false;
                // the second air dash empties the whole bar: it all reloads, not just one half (user)
                self.dash_charges = 0.0;
            }
        }
        // Double jump: any time we're airborne (rising counts, even while brushing an uphill slope),
        // a moment after take-off.
        let airborne = !on_ground || self.air_vy.is_some();
        crate::remap::set_air_hide(self.cfg.air_hide_move && doom_move && airborne && !self.on_ladder);
        if self.input.pressed3(k.jump, k.jump_alt, k.jump_pad) && airborne && !self.on_ladder && !self.double_jumped && self.jump_grace < 0.2 {
            // Doom: instant upward launch to a fixed apex height, and the stick picks a new
            // direction mid-air.
            self.double_jumped = true;
            audio::play("double_jump");
            self.sway_kick(1.5);
            let g = self.cfg.air_gravity;
            self.air_vy = Some((2.0 * g * self.cfg.double_jump_height).sqrt());
            // The new direction takes over at once (no leftover sideways drift).
            let wish = self.wish_dir();
            if wish != Vec3::ZERO {
                let speed = Vec3::new(self.air_carry.x, 0.0, self.air_carry.z).length().max(self.cfg.double_jump_push);
                self.air_carry = wish * speed;
            }
            self.air_push = Vec3::ZERO;
        }

        // ---- enemies: stagger state, burning -> armor
        let enemies = game::enemies(60.0);
        self.staggered = enemies
            .iter()
            .filter(|e| is_hostile(e) && self.is_staggered(e))
            .map(|e| (e.pos(), e.dist <= self.cfg.glory_range))
            .collect();
        self.update_bars(&enemies);
        // Fight detection for the music: the same hostile within 30 m losing HP (the old sum of
        // all their HP also dropped when one just walked out of range or unloaded - music with
        // no fight, user).
        let mut seen = std::collections::HashMap::new();
        let mut reason: Option<String> = None;
        let me_pos = game::player().map(|p| game::chr_pos(&p.chr_ins));
        // An enemy coming at you starts the fight music before anyone gets hurt (user). ER keeps
        // no usable "targeting you" flag on the ChrIns (lock-on fields are unused for NPCs), so:
        // its own movement toward you, sampled every 0.25 s, summed with a 1 s-ish decay; over
        // ~1.2 m (about 1.5 m/s for a second) within 25 m = it's after you.
        let sample = self.time - self.approach_sample_at >= 0.25;
        if sample {
            self.approach_sample_at = self.time;
        }
        let mut approach = std::collections::HashMap::new();
        // (ambient wildlife - eagles, goats, bats... - never starts the music, like the health
        // bars: eagles taking off set it off - user. You getting hurt still does.)
        for e in enemies.iter().filter(|e| e.dist < 30.0 && is_hostile(e) && !AMBIENT_WILDLIFE.contains(&e.chr.character_id)) {
            let key = &*e.chr as *const _ as usize;
            let hp = e.hp();
            // Attacking (ER NPC attack animations are 3000-3999): rooted enemies (flowers,
            // turrets, casters) never come at you, but they do swing / shoot (user).
            let anim = current_anim(e.chr);
            // In a fight stance: ER NPC battle steps 2000-2099 (zombies backing off / strafing while
            // staring you down: 2001, 2002, 2030-2033), the 5000 "spotted you", attacks 3000-3999.
            // Variant models add millions (5002031 = 2031). Not 2300 (an idle seen before noticing).
            let base = anim.rem_euclid(1_000_000);
            if hp > 0 && e.dist < 25.0 && ((2000..2100).contains(&base) || (3000..4000).contains(&base) || base == 5000) {
                self.last_combat = self.time;
                reason.get_or_insert_with(|| format!("npc {} in a fight stance (anim {anim}) at {:.0} m", e.chr.npc_param_id, e.dist));
            }
            if let Some(me) = me_pos {
                let now = e.pos();
                // (a turn-to-face trigger was tried and dropped: eagles taking off set it off)
                let (prev, score) = self.hostile_approach.get(&key).copied().unwrap_or((now, 0.0));
                let entry = if sample {
                    let to_me = Vec3::new(me.x - now.x, 0.0, me.z - now.z).normalize_or_zero();
                    let step = Vec3::new(now.x - prev.x, 0.0, now.z - prev.z);
                    // (a teleport / tile re-base jump isn't walking)
                    let toward = if step.length() < 3.0 { step.dot(to_me) } else { 0.0 };
                    let score = score * 0.75 + toward;
                    if hp > 0 && e.dist < 25.0 && score > 1.2 {
                        self.last_combat = self.time;
                        reason.get_or_insert_with(|| format!("npc {} coming at you at {:.0} m", e.chr.npc_param_id, e.dist));
                    }
                    (now, score)
                } else {
                    (prev, score)
                };
                approach.insert(key, entry);
            }
            if let Some(&before) = self.hostile_hp.get(&key) {
                if hp < before {
                    self.last_combat = self.time;
                    reason.get_or_insert_with(|| format!("npc {} lost {} HP at {:.0} m", e.chr.npc_param_id, before - hp, e.dist));
                }
            }
            seen.insert(key, hp);
        }
        self.hostile_hp = seen;
        self.hostile_approach = approach;
        // fight music fades out after music_hold s without combat (live; 8 faded mid-fight - user)
        let fight = self.time - self.last_combat < self.cfg.music_hold.max(1.0);
        if fight && !self.music_on {
            log::info!("music: fight started ({})", reason.as_deref().unwrap_or("you hit something or were hit"));
        }
        self.music_on = fight;
        audio::set_combat(fight);

        let mut alive = std::collections::HashSet::new();
        let mut shard_bursts = Vec::new();
        for e in &enemies {
            let key = &*e.chr as *const _ as usize;
            alive.insert(key);
            if let Some((t, prev)) = self.burning.get_mut(&key) {
                *t -= dt;
                if e.hp() < *prev {
                    // Doom: damaging a burning demon sheds armor shards.
                    shard_bursts.push((e.pos() + Vec3::Y * 1.2, 1 + ((*prev - e.hp()) / 80) as usize));
                }
                *prev = e.hp();
            }
        }
        self.burning.retain(|k, (t, _)| *t > 0.0 && alive.contains(k));
        for (at, n) in shard_bursts {
            self.pickups.burst(at, Kind::Armor, n.min(4), 5, false);
        }
        self.track_deaths(&enemies);
        self.track_bosses(dt);
        self.boss_bars = game::boss_bars();
        self.collect_tokens();

        // ---- melee / glory kill
        let melee_pressed = self.input.pressed3(k.melee, k.melee_alt, k.melee_pad);
        let saw_pressed = self.input.pressed3(k.chainsaw, k.chainsaw_alt, k.chainsaw_pad);
        let belch_pressed = self.input.pressed3(k.flame_belch, k.flame_belch_alt, k.flame_belch_pad);
        if self.glory.is_none() && melee_pressed {
            if let Some(target) = self.pick_target(&enemies, self.cfg.glory_range, true) {
                self.start_glory(target, false);
            } else if self.blood_punch > 0 && self.in_punch_reach(&enemies, 5.5, 0.35) {
                // (only with a demon to hit - a swing at nothing is a normal punch and keeps the
                // charge, user)
                self.blood_punch -= 1;
                self.blood_punch_blast(&enemies);
            } else if self.melee_cd <= 0.0 {
                self.punch(&enemies);
            }
        }

        // ---- chainsaw
        let alive = game::player().is_some_and(|p| p.chr_ins.modules.data.hp > 0);
        let saw_click = std::mem::take(&mut self.saw_click);
        // Out of ammo in a boss fight (boss bar up): the chainsaw on the boss refills every gun
        // and doesn't hurt it (user). Big bodies: in reach of its edge, not its centre.
        let boss_saw = if self.glory.is_none() && (saw_pressed || saw_click) && alive && self.saw_mode {
            let bosses = game::active_boss_handles();
            game::camera().and_then(|(cam, fwd)| {
                enemies
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| bosses.contains(&e.chr.field_ins_handle))
                    .filter(|(_, e)| e.dist <= self.cfg.chainsaw_range + e.chr.modules.physics.hit_radius.max(0.5) + 1.0)
                    .filter(|(_, e)| (e.pos() - cam).normalize_or_zero().dot(fwd) > 0.35)
                    .min_by(|a, b| a.1.dist.total_cmp(&b.1.dist))
                    .map(|(i, _)| i)
            })
        } else {
            None
        };
        if let Some(t) = boss_saw.filter(|_| self.chainsaw_fuel + 1e-3 < 1.0) {
            let _ = t;
            self.msg("CHAINSAW NEEDS 1 FUEL");
            audio::play("chainsaw_no_fuel");
        } else if let Some(t) = boss_saw {
            // one fuel pip, like any demon (user); the first pip regenerates on its own
            self.chainsaw_fuel -= 1.0;
            self.start_glory(t, true);
            if let Some(g) = self.glory.as_mut() {
                // stay put: lunging into a giant boss isn't needed for an ammo refill
                g.boss_ammo = true;
                g.to = g.from;
            }
            log::info!("chainsaw on boss npc {}: ammo refill, no damage", enemies[t].chr.npc_param_id);
        } else if self.glory.is_none() && (saw_pressed || saw_click) && alive {
            match self.pick_target(&enemies, self.cfg.chainsaw_range, false) {
                // Bosses and mini bosses (boss bar, heavy HP class, or giant bodies like the trolls)
                // can't be chainsawed.
                Some(t) if self.chainsaw_immune(&enemies[t]) => {
                    log::info!("chainsaw: npc {} blocked ({})", enemies[t].chr.npc_param_id, self.chainsaw_block(&enemies[t]).unwrap_or_default());
                    self.msg("INVALID TARGET");
                    audio::play("chainsaw_no_fuel");
                }
                Some(t) => {
                    let cost = chainsaw_cost(enemies[t].max_hp(), crate::params::applied_hp_mult());
                    if self.chainsaw_fuel + 1e-3 >= cost {
                        self.chainsaw_fuel -= cost;
                        self.start_glory(t, true);
                    } else {
                        self.msg(format!("CHAINSAW NEEDS {cost:.0} FUEL"));
                        audio::play("chainsaw_no_fuel");
                    }
                }
                // (no rev without a target - user; the "chainsaw_no_target" sound stays converted)
                None => self.msg("NO TARGET"),
            }
        }

        // ---- the Crucible
        self.update_crucible(&enemies, armed);

        // ---- flame belch
        if belch_pressed && self.belch_cd <= 0.0 {
            self.belch_cd = BELCH_COOLDOWN;
            let mut lit = 0;
            if let Some((cam, fwd)) = game::camera() {
                for e in &enemies {
                    let to = e.pos() - cam;
                    // range 12 m (user; was 9)
                    if e.dist < 12.0 && to.normalize_or_zero().dot(fwd) > 0.6 && is_hostile(e) {
                        let key = &*e.chr as *const _ as usize;
                        self.burning.insert(key, (BURN_TIME, e.hp()));
                        lit += 1;
                    }
                }
                // Visual: a short fan of explosive rounds.
                let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
                for yaw in [-14.0f32, 0.0, 14.0] {
                    let d = Quat::from_axis_angle(Vec3::Y, yaw.to_radians()) * fwd;
                    let _ = bullet::spawn_capped(bullet::CAP_FX, owner, params::FX_BELCH_BULLET, cam + fwd * 0.8 - Vec3::Y * 0.3, d);
                }
            }
            self.msg(format!("FLAME BELCH ({lit} burning)"));
            audio::play("flame_belch");
        }

        // ---- glory kill progression
        self.update_glory(dt, max_hp, &enemies);
        // Invulnerable during glory and chainsaw kills (Doom): a hit during the lunge killed you
        // and the glory health then "revived" you for a few seconds. The character's own
        // ignore-all-damage flag, only while the kill plays.
        // The lunge is only 0.3 s; attacks already swinging land during the finishing blow, so
        // the protection lasts until 1 s after the kill.
        if self.glory.is_some() {
            self.invuln_until = self.time + 1.0;
        }
        let invuln = self.time < self.invuln_until;
        if let Some(p) = game::player() {
            let hp = p.chr_ins.modules.data.hp;
            if invuln != self.glory_invuln {
                p.chr_ins.debug_flags.set_disabled_hit(invuln);
                // Backup: the game's own can't-die flag (HP floors at 1), in case a hit gets
                // through the hit flag (the last recording: HP hit 0 during the kill).
                set_no_dead(invuln);
                self.glory_invuln = invuln;
                log::info!("glory invulnerability {} (hp {hp}, flag {})", if invuln { "ON" } else { "off" }, p.chr_ins.debug_flags.disabled_hit());
            } else if invuln && hp < self.invuln_hp {
                log::warn!("hit while invulnerable: hp {} -> {hp}", self.invuln_hp);
            }
            self.invuln_hp = hp;
        }
        self.melee_cd = (self.melee_cd - dt).max(0.0);

        // ---- pickups
        let me = game::chr_pos(&player.chr_ins);
        let got = self.pickups.update(dt, me, game::camera(), self.cfg.pickup_magnet);
        self.collect(got);

        self.update_viewmodel(dt);
        game::hide_er_hud(self.cfg.hide_er_hud && self.in_world_for > 1.0);
    }

    fn pick_target(&self, enemies: &[Enemy], range: f32, need_stagger: bool) -> Option<usize> {
        let (cam, fwd) = game::camera()?;
        enemies
            .iter()
            .enumerate()
            .filter(|(_, e)| e.dist <= range && is_hostile(e))
            .filter(|(_, e)| !need_stagger || self.is_staggered(e))
            .filter(|(_, e)| (e.pos() - cam).normalize_or_zero().dot(fwd) > 0.35)
            .min_by(|a, b| a.1.dist.total_cmp(&b.1.dist))
            .map(|(i, _)| i)
    }

    pub fn start_glory(&mut self, idx: usize, chainsaw: bool) {
        let Some(p) = game::player() else { return };
        // Dead (0 HP, the game's death already started): no glory kill / chainsaw.
        if p.chr_ins.modules.data.hp <= 0 {
            return;
        }
        let enemies = game::enemies(60.0);
        let Some(e) = enemies.get(idx) else { return };
        let from = game::chr_pos(&p.chr_ins);
        let to_enemy = e.pos() - from;
        let stop = from + to_enemy - to_enemy.normalize_or_zero() * 1.2;
        // Teleport mode: glory_tp_dist in front of the demon (on our side of it), on the ground there - a
        // ledge or wall between us no longer leaves you hanging while the demon dies.
        let to = if self.cfg.glory_teleport {
            // A spot you can stand on: ground under it, room for the body, and nothing between it
            // and the demon. Your side first, then around the demon (every 45 deg) at the set
            // distance and at 1.2 m; the demon's own feet as the last resort. (Teleporting blindly
            // 2 m toward you landed inside a building / over a drop and out of the map - user.)
            let flat = Vec3::new(to_enemy.x, 0.0, to_enemy.z).normalize_or(Vec3::Z);
            let ep = e.pos();
            let d = self.cfg.glory_tp_dist.max(0.5);
            let chest = ep + Vec3::Y * 1.0;
            let not_chr = |h: &raycast::hknpHit| !raycast::is_chr_hit(h);
            let ok = |dir: Vec3, r: f32| -> Option<Vec3> {
                let spot = Vec3::new(ep.x - dir.x * r, ep.y, ep.z - dir.z * r);
                let gy = ground_height(spot, 3.0)?;
                let feet = Vec3::new(spot.x, gy, spot.z);
                // a clear line from the demon's chest to ours (no wall / rock between)
                let to_me = feet + Vec3::Y * 1.0 - chest;
                if raycast::cast_sphere(chest, to_me, 0.2, WORLD_FILTER, not_chr).is_some() {
                    return None;
                }
                // room for the body: nothing overhead within 1.8 m
                if raycast::cast_sphere(feet + Vec3::Y * 0.4, Vec3::Y * 1.4, 0.25, WORLD_FILTER, not_chr).is_some() {
                    return None;
                }
                Some(feet)
            };
            let around = |r: f32| {
                (0..8).map(move |k| {
                    let a = k as f32 * std::f32::consts::FRAC_PI_4;
                    let (sn, cs) = a.sin_cos();
                    (Vec3::new(flat.x * cs - flat.z * sn, 0.0, flat.x * sn + flat.z * cs), r)
                })
            };
            let pick = around(d).chain(around(1.2)).find_map(|(dir, r)| ok(dir, r));
            if pick.is_none() {
                log::info!("glory teleport: no clear spot around npc {} - landing at its feet", e.chr.npc_param_id);
            }
            pick.unwrap_or(ep)
        } else {
            Vec3::new(stop.x, from.y.max(stop.y), stop.z)
        };
        let locked = game::lock_on(e.chr.field_ins_handle);
        log::info!("glory target npc {} dist {:.1} lock-on {locked}", e.chr.npc_param_id, e.dist);
        self.glory = Some(Glory {
            target: &*e.chr as *const _ as usize,
            t: 0.0,
            from,
            to,
            chainsaw,
            placed: false,
            boss_ammo: false,
        });
        // (no "CHAINSAW!" text - user; glory kills keep theirs)
        if !chainsaw {
            self.msg("GLORY KILL!");
        }
        if chainsaw {
            self.chainsaw_at = self.time;
            self.vm_play("fire", false);
        } else {
            // Execution: the fists come up with a jab during the lunge (the finisher plays on the kill).
            self.melee_clip = "punch_l2";
            self.melee_t = 0.0;
            self.melee_until = self.time + self.glory_wait() + 0.08;
            self.switched_at = self.melee_until;
        }
        audio::play(if chainsaw { "chainsaw_rev" } else { "glory_whoosh" });
    }

    /// Seconds from the start of a glory kill to the finishing hook: the lunge, or with the
    /// teleport the left jab played in full (it was cut to 0.1 s and barely seen - user).
    fn glory_wait(&self) -> f32 {
        // (the lunge itself takes GLORY_LUNGE_TIME; the jab carries on at the demon after it)
        self.cfg.glory_jab_time.max(GLORY_LUNGE_TIME)
    }

    fn update_glory(&mut self, dt: f32, player_max_hp: i32, enemies: &[Enemy]) {
        let jab = self.glory_wait();
        let Some(g) = self.glory.as_mut() else { return };
        g.t += dt;
        // (chainsaw kills keep the short lunge timing: no jab to play)
        if g.t < if g.chainsaw { GLORY_LUNGE_TIME } else { jab } {
            return;
        }
        let target = g.target;
        let chainsaw = g.chainsaw;
        let boss_ammo = g.boss_ammo;
        let spot = g.to;
        self.glory = None;
        game::release_lock_on();
        if !chainsaw {
            // The finishing blow: a heavy right hook as the demon goes down.
            self.melee_clip = "punch_r2";
            self.melee_t = 0.0;
            self.melee_until = self.time + 0.5;
            self.switched_at = self.melee_until;
            audio::play_vol("melee", 0.9);
        }
        // teleport mode: stay at the demon until the finisher (punch / saw) has played out
        if self.cfg.glory_teleport {
            self.glory_hold = Some((spot, self.time));
        }
        if boss_ammo {
            // Boss + no ammo: every gun refilled, the boss untouched (no damage, no gore).
            let at = enemies
                .iter()
                .find(|e| &*e.chr as *const _ as usize == target)
                .map_or_else(|| game::player().map_or(Vec3::ZERO, |p| game::chr_pos(&p.chr_ins)), |e| e.pos())
                + Vec3::Y * 1.2;
            for a in Ammo::ALL {
                if a == Ammo::Bfg {
                    let i = a.index();
                    self.ammo[i] = (self.ammo[i] + 1).min(a.max());
                } else {
                    self.ammo[a.index()] = self.ammo[a.index()].max(a.max());
                }
                self.pickups.burst(at, Kind::Ammo(a), 3, 0, true);
            }
            self.msg("AMMO UP");
            audio::play("chainsaw_kill");
            self.later.push((0.35, "pickup_chainsaw_ammo"));
            return;
        }
        let Some(e) = enemies.iter().find(|e| &*e.chr as *const _ as usize == target) else {
            return;
        };
        // Kill: zero HP; the game runs its normal death (souls, drops, event flags).
        let chr = unsafe { &mut *(target as *mut eldenring::cs::ChrIns) };
        // Gore: invisible darts into the chest whose impact is a blood splash.
        {
            let chest = game::chr_pos(chr) + Vec3::Y * 1.2;
            let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
            for i in 0..if chainsaw { 5 } else { 3 } {
                let a = i as f32 * 2.1;
                let from = chest + Vec3::new(a.cos(), 0.2, a.sin()) * 1.2;
                let _ = bullet::spawn_capped(bullet::CAP_FX, owner, params::FX_GORE_BULLET, from, chest - from);
            }
        }
        // Everything dies to one glory kill, mini bosses (trolls) included (they needed two).
        // Only bosses the game shows a boss bar for take a chunk instead, so glory kills can't
        // cheese a boss.
        let base_hp = chr.modules.data.max_hp as f32 / crate::params::applied_hp_mult();
        if Self::is_boss(chr) {
            let bite = (chr.modules.data.max_hp as f32 * self.cfg.glory_heavy_frac) as i32;
            chr.modules.data.hp = (chr.modules.data.hp - bite).max(0);
            log::info!("glory on boss (base hp {base_hp:.0}): -{bite}");
            if chr.modules.data.hp == 0 {
                self.kills += 1;
            }
        } else {
            chr.modules.data.hp = 0;
            self.kills += 1;
        }
        let _ = e;
        // Mark as handled so the death tracker doesn't drop the normal loot as well.
        self.enemy_track.remove(&target);
        let at = game::chr_pos(chr) + Vec3::Y * 1.2;
        if chainsaw {
            // Every weapon refilled to full right away, plus one BFG round; the shower is the show.
            for a in Ammo::ALL {
                if a == Ammo::Bfg {
                    let i = a.index();
                    self.ammo[i] = (self.ammo[i] + 1).min(a.max());
                } else {
                    self.ammo[a.index()] = self.ammo[a.index()].max(a.max());
                }
                self.pickups.burst(at, Kind::Ammo(a), 3, 0, true);
            }
            self.msg("AMMO UP");
            audio::play("chainsaw_kill");
            self.later.push((0.35, "pickup_chainsaw_ammo"));
        } else {
            self.glory_kills += 1;
            // Doom (with the suit upgrade): up to 2 Blood Punch charges, one per glory kill (user).
            self.blood_punch = (self.blood_punch + 1).min(2);
            // Doom: glory kills burst into health that flies to you.
            let heal = (player_max_hp as f32 * self.cfg.glory_heal_frac) as i32;
            self.pickups.burst(at, Kind::Health, 5, (heal / 5).max(1), true);
            self.msg("HEALTH UP");
            audio::play("glory_snap");
            audio::play_vol("glory_gore", 0.8);
            self.later.push((0.3, "pickup_health"));
        }
    }

    /// Break a map asset a shot hit, if its params say it breaks (AssetGeometryParam hp > 0):
    /// fire ER's own object attack into it. Each ER bullet holds a pool slot for a while, so only
    /// confirmed breakables get one, once per object per 0.3 s, and never into the reserve.
    /// Fire a breaker at an asset; true if one was sent (it was breakable and not on cooldown).
    fn break_geom(&mut self, g: eldenring::cs::FieldInsHandle, at: Vec3, dir: Vec3) -> bool {
        let key = (g.block_id.0 as u64) << 32 | g.selector.0 as u64;
        let Some(info) = game::geom_by_handle(&g) else { return false };
        // loose props move under the impact: shorter cooldown so a follow-up hit isn't skipped
        let cooldown = if info.loose { 0.1 } else { 0.3 };
        if self.broken_at.get(&key).is_some_and(|t| self.time - t < cooldown) {
            return true;
        }
        // Retried (after the cooldown) if a dart misses: only the asset's own state says it broke.
        // (loose physics props read state 4 while the physics moves them: any state goes)
        // (loose props are never plants, whatever their name: the white glowing skulls are AEG800)
        if info.hp <= 0 || !(damage::is_prop(&info.name) || info.loose) || !(info.intact() || info.loose) || crate::hitter::pool_used() >= bullet::CAP_BREAK {
            return false;
        }
        self.broken_at.insert(key, self.time);
        if self.broken_at.len() > 256 {
            let now = self.time;
            self.broken_at.retain(|_, t| now - *t < 5.0);
        }
        let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        // loose props: spawned right at the hit (from 0.6 m out a moving rock / skull was missed)
        let back = if info.loose { 0.2 } else { 0.6 };
        let r = bullet::spawn(owner, params::BREAKER_BULLET, at - dir * back, dir);
        // fences ignore the dart: an explosion-shaped breaker at the hit point as well
        if info.behavior == 0 {
            let _ = bullet::spawn(owner, params::BREAKER_BLAST_BULLET, at - dir * 0.2, dir);
        }
        // Loose physics props take two hits (the first wakes the physics - in Elden Ring itself
        // too): a second breaker right after (heavy cannon shots needed two hits - user).
        // Two at once (the first wakes the physics and the object flies off before a later one
        // arrives), plus the follow-up as a backup.
        log::info!("break {} (hp {} def {} behavior {} loose {}): {r:?}", info.name, info.hp, info.defense, info.behavior, info.loose);
        true
    }

    /// Smash the first breakable prop along a line, looking past everything that isn't one (the
    /// invisible blockers in front of fence sides stopped shots and punches; the dash found them).
    /// Shared by punches, the dash, the glory lunge and shots that stopped short. True if one broke.
    fn break_along(&mut self, from: Vec3, dir: Vec3, len: f32, radius: f32) -> bool {
        let breakable = |h: &raycast::hknpHit| {
            h.field_ins_handle().is_some_and(|f| {
                f.selector.field_ins_type() == Some(eldenring::cs::FieldInsType::ReplayEnemy)
                    && game::geom_by_hit(h).is_some_and(|g| g.hp > 0 && (g.intact() || g.loose) && (damage::is_prop(&g.name) || g.loose))
            })
        };
        let dir = dir.normalize_or_zero();
        match raycast::cast_sphere(from, dir * len, radius, WORLD_FILTER, breakable).and_then(|h| Some((h.field_ins_handle()?, Vec3::from(h.pos)))) {
            Some((f, at)) => {
                self.break_geom(f, at, dir);
                true
            }
            None => false,
        }
    }

    /// Explosions smash the props around them: up to 4 per blast (ER bullet slots are limited).
    fn break_area(&mut self, at: Vec3, radius: f32) {
        let Some(p) = game::player() else { return };
        let me = game::chr_pos(&p.chr_ins);
        let owner = Some(p.chr_ins.field_ins_handle);
        let reach = me.distance(at) + radius;
        let mut n = 0;
        for (name, _, d) in game::breakables_near(reach) {
            let pos = me + d;
            if pos.distance(at) > radius || !damage::is_prop(&name) {
                continue;
            }
            if n >= 4 || crate::hitter::pool_used() >= bullet::CAP_BREAK {
                break;
            }
            let _ = bullet::spawn(owner, params::BREAKER_BULLET, pos + Vec3::Y * 1.2, Vec3::NEG_Y);
            n += 1;
        }
        if n > 0 {
            log::info!("blast at {at:.1} broke {n} props");
        }
    }

    /// The demon the meathook would grab right now (aim cone, range, line of sight).
    fn meathook_target(&self) -> Option<Enemy> {
        let (cam, fwd) = game::camera()?;
        game::enemies(self.cfg.meathook_range)
            .into_iter()
            .filter(is_hostile)
            // Aim points up the body: critters near the ground, people at the chest, trolls and
            // other giants several metres up (the old 0.7 m point failed the cone on them).
            .map(|e| {
                let best = [0.4f32, 1.0, 2.0, 3.5, 5.0]
                    .iter()
                    .map(|h| (e.pos() + Vec3::Y * *h - cam).normalize_or_zero().dot(fwd))
                    .fold(-1.0f32, f32::max);
                (best, e)
            })
            .filter(|(d, _)| *d > 0.97)
            // Must be able to see it: the hook flew blind into walls/terrain before.
            .filter(|(_, e)| {
                [1.0f32, 0.4, 2.0, 3.5, 5.0].iter().any(|h| {
                    let aim = e.pos() + Vec3::Y * *h;
                    damage::trace(cam, aim - cam, (aim - cam).length() + 1.0, 0.1).chr == Some(e.chr.field_ins_handle)
                })
            })
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, e)| e)
    }

    fn try_meathook(&mut self) {
        let Some((_cam, fwd)) = game::camera() else { return };
        let Some(p) = game::player() else { return };
        let from = game::chr_pos(&p.chr_ins);
        let Some(e) = self.meathook_target() else {
            // No (valid) target: Doom doesn't fire - the hook just twitches out and back (its
            // "discharge" additive, 0.3 s). No sound, no chain.
            self.hook_cd = 0.4;
            // Layered on top of the idle (smooth in and out of it, user), not a clip of its own.
            self.hook_twitch = Some(self.time);
            return;
        };
        self.vm_play("hook_shoot", false);
        audio::play("meathook");
        let dist = e.dist;
        log::info!("meathook -> npc {} at {dist:.1} m", e.chr.npc_param_id);
        // No lock-on (user): the pull flies you at the demon, the aim stays yours.
        // The hook bites: a real ER hit with no flinch (aggro only) so the demon notices you and
        // turns on you, instead of standing there while you fly in (user).
        // (Doom's hook does 0 damage; ours is a tiny ER hit. Skipped on a staggered demon so a
        // sliver of HP can't kill it - you fly in and glory kill it, user.)
        if !self.is_staggered(&e) {
            self.hitters.hit_on(Some(e.chr.field_ins_handle), crate::hitter::NOTICE, e.pos() + Vec3::Y * hook_grip(e.chr), fwd);
        }
        // Flaming Hook (mastered, Doom's double_barrel_meat_hook_flame): the demon burns for
        // 1.5 s - damaging it meanwhile sheds armor shards, like the Flame Belch.
        let key = &*e.chr as *const _ as usize;
        self.burning.insert(key, (HOOK_BURN_TIME, e.hp()));
        let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        let chest = e.pos() + Vec3::Y * hook_grip(e.chr);
        let _ = bullet::spawn_capped(bullet::CAP_FX, owner, params::FX_BELCH_BULLET, chest - fwd * 0.6, fwd);
        self.hook = Some(Hook {
            target: &*e.chr as *const _ as usize,
            t: 0.0,
            dur: (dist / self.cfg.meathook_speed).clamp(0.15, 0.8),
            from,
            vel: Vec3::ZERO,
        });
        self.hook_cd = MEATHOOK_COOLDOWN;
        if let Ok(mut g) = crate::fx::HOOK.lock() {
            *g = Some((e.pos() + Vec3::Y * hook_grip(e.chr), std::time::Instant::now()));
        }
        self.later.push((0.08, "meathook_hit"));
    }

    fn set_slowmo(&mut self, on: bool) {
        if on == self.slowed {
            return;
        }
        if let Ok(f) = unsafe { <eldenring::cs::CSFlipper as fromsoftware_shared::FromStatic>::instance_mut() } {
            f.game_speed = if on { self.cfg.wheel_slowmo } else { 1.0 };
            self.slowed = on;
        }
    }

    #[track_caller]
    fn vm_play(&mut self, clip: &str, looping: bool) {
        // (which code drew the weapon: a draw that played twice after a respawn - user)
        if clip == "bringup" {
            log::info!("draw: {} at {}", WEAPONS[self.weapon].name, std::panic::Location::caller().line());
        }
        self.vm_clip = clip.to_string();
        self.vm_t = 0.0;
        self.vm_loop = looping;
        self.vm_speed = 1.0;
        self.vm_queue.clear();
        self.vm_hold = None;
    }

    /// Advance the viewmodel clip and publish the pose for the render thread.
    fn update_viewmodel(&mut self, dt: f32) {
        // Ready = alive, screen not faded / loading, and the Slayer has control (not playing the
        // wake-up / stand-up animation after continuing a save). Becoming ready draws the weapon.
        let alive = game::player().is_some_and(|p| p.chr_ins.modules.data.hp > 0);
        let loading = game::now_loading();
        let clear = alive && self.in_world_for > 0.5 && !game::screen_faded() && !loading;
        // In control = no scripted animation (getting up from a grace / after loading, resting).
        // The old check (anim id 0 only) missed the movement anims right after respawning and the
        // gun waited for the 8 s fallback.
        let in_control = game::player().is_some_and(|p| game::in_event_anim(&p.chr_ins).is_none() && !game::on_ladder(&p.chr_ins));
        // Sitting / resting at a grace (anims 68000-68999): no 8 s fallback - the gun and the
        // Doom HUD came back after ~10 s in the grace menu (user).
        let at_grace = game::player().and_then(|p| game::in_event_anim(&p.chr_ins)).is_some_and(|a| (68000..69000).contains(&(a % 1_000_000)));
        self.clear_for = if clear { self.clear_for + dt } else { 0.0 };
        self.faded_for = if clear { 0.0 } else { self.faded_for + dt };
        // Once drawn, the weapon stays out through short fades / system messages (it used to be
        // holstered and re-drawn); only death, loading or a fade over a second puts it away.
        // Cutscenes: no gun, no HUD (drawn again when control returns).
        let cutscene = game::in_cutscene();
        if cutscene {
            self.clear_for = 0.0;
        }
        // a loading screen puts everything away at once (it isn't a fade plate - user)
        let ready = !cutscene
            && !loading
            && if self.ready {
                alive && self.in_world_for > 0.5 && self.faded_for < 1.0
            } else {
                self.clear_for > 0.3 && (in_control || (self.clear_for > 8.0 && !at_grace))
            };
        if ready && !self.ready {
            self.vm_play("bringup", false);
            self.switched_at = self.time;
            audio::play_vol("weapon_switch", 0.5);
            log::info!("viewmodel ready: drawing weapon (anim {})", game::player().map(|p| current_anim(&p.chr_ins)).unwrap_or(-1));
        }
        if !ready && self.ready {
            // (overlay hide report: why the gun / Doom HUD went away)
            log::info!(
                "overlay hidden: loading {loading} cutscene {cutscene} alive {alive} in_world {:.1}s screen_faded {} faded_for {:.2}s in_control {in_control} anim {} event_anim {:?}",
                self.in_world_for,
                game::screen_faded(),
                self.faded_for,
                game::player().map(|p| current_anim(&p.chr_ins)).unwrap_or(-1),
                game::player().and_then(|p| game::in_event_anim(&p.chr_ins))
            );
        }
        self.ready = ready;

        const SAW_DROP: f32 = 0.3;
        let dropping = self.saw_drop.is_some_and(|t0| self.time < t0 + SAW_DROP);
        if let Some(t0) = self.saw_drop.filter(|t0| self.time >= t0 + SAW_DROP) {
            let _ = t0;
            self.saw_drop = None;
            self.vm_play("bringup", false);
            self.switched_at = self.time;
            // like any weapon switch: the switch delay, and a held button has to be let go first
            self.fire_cd = self.cfg.switch_fire_delay.max(0.0);
            self.fire_lock = self.input.down3(self.cfg.keys.fire, self.cfg.keys.fire_alt, self.cfg.keys.fire_pad);
            self.fire_buffer = 0.0;
        }
        // (lowering: 0 -> 0.3 m down while the saw is put away)
        let saw_down = self.saw_drop.map_or(0.0, |t0| ((self.time - t0) / SAW_DROP).clamp(0.0, 1.0).powi(2) * 0.3);
        let chainsaw = self.time - self.chainsaw_at < 0.6 || self.saw_mode || dropping;
        if self.saw_mode && self.glory.is_none() && self.time - self.chainsaw_at >= 0.6 {
            // Out of ammo: the chainsaw idles in the hands until ammo turns up.
            let len = viewmodel::clip_len(viewmodel::CHAINSAW_FOLDER, &self.vm_clip);
            if len.is_none_or(|l| !self.vm_loop && self.vm_t >= l) {
                self.vm_play("idle", true);
            }
        }
        let melee = self.time < self.melee_until;
        // a chainsaw kill with the Crucible out: the saw plays (it was hidden behind the
        // Crucible - user), then the Crucible is drawn again like after a punch
        let sawing = self.time - self.chainsaw_at < 0.6 || self.glory.as_ref().is_some_and(|g| g.chainsaw);
        if (melee || sawing) && self.crucible_out {
            self.cr_melee = true;
        }
        let folder = if melee {
            "fists"
        } else if chainsaw {
            viewmodel::CHAINSAW_FOLDER
        } else {
            viewmodel::FOLDERS[self.weapon]
        };
        // Menus: lower the weapon out of the way. Esc menu / popups switch ER's HUD state; submenus
        // show up in the menu UI element table (game::menu_window_open). The OS cursor used to
        // count too, but it also shows up mid-gameplay and made the gun dip.
        let menu = game::menu_open() && !self.wheel_open;
        self.menu_t = (self.menu_t + if menu { dt * 6.0 } else { -dt * 6.0 }).clamp(0.0, 1.0);
        let lower = self.menu_t * self.menu_t * 0.45;
        // Precision Bolt scope: looking through it, the gun is out of the picture.
        let scoped = self.weapon == 1 && self.zoom_on && self.time >= self.scope_down_at;
        self.scope_hidden = scoped;
        self.vm_visible = self.ready && (viewmodel::has_model(folder) || self.crucible_out) && self.menu_t < 0.99 && !scoped;
        if !self.vm_visible {
            viewmodel::set_pose(None);
            return;
        }
        if self.crucible_out && !melee && !sawing && viewmodel::has_model(viewmodel::CRUCIBLE_FOLDER) {
            // back from a punch / Blood Punch: Doom's draw again, blade and ignite sound with it
            if std::mem::take(&mut self.cr_melee) && self.cr_away_at.is_none() {
                self.cr_blade = false;
                self.cr_play("bringup", false);
            }
            // swings play at crucible_swing_speed (live)
            let speed = if self.cr_clip.starts_with("swing") { self.cfg.crucible_swing_speed.clamp(0.1, 3.0) } else { 1.0 };
            self.cr_t += dt * speed;
            // the put-away plays as the bringdown (Doom's deactivate: the fold, then down); a
            // finished swing / draw settles into the idle
            if self.cr_away_at.is_some() && self.cr_clip != "bringdown" {
                if self.cr_t >= self.cr_len(self.cr_clip) {
                    self.cr_play("bringdown", false);
                }
            } else if !self.cr_loop && self.cr_away_at.is_none() && self.cr_t >= self.cr_len(self.cr_clip) {
                self.cr_play("idle", true);
            }
            let len = self.cr_len(self.cr_clip).max(0.01);
            // (a loop gets its running time, the renderer wraps it like the guns': wrapped here,
            // the jump back to 0 looked like a restart and crossfaded - the bump in the idle, user)
            let time = if self.cr_loop { self.cr_t } else { self.cr_t.min(len) };
            // The blade lights when the side fangs have dropped in the draw (Doom's
            // bringup_activate: frames 19-22 at 30 fps) and goes out as they fold in the put-away
            // (deactivate: frame 14) - user.
            let blade = match self.cr_clip {
                "bringup" => time >= 22.0 / 30.0,
                "bringdown" => time < 14.0 / 30.0,
                _ => true,
            };
            // Doom's ignite sound when the fangs drop and the blade lights (user), not on the key
            // press: from frame 19, so its peak (0.1 s in) lands as the blade appears at 22
            if self.cr_clip == "bringup" && time >= 19.0 / 30.0 && !self.cr_blade {
                self.cr_blade = true;
                audio::play_vol("crucible_open", 0.9);
            }
            viewmodel::BLADE_ON.store(blade, std::sync::atomic::Ordering::Relaxed);
            viewmodel::set_pose(Some(viewmodel::Pose {
                folder: viewmodel::CRUCIBLE_FOLDER,
                clip: self.cr_clip.to_string(),
                time,
                looping: self.cr_loop,
                offset: [0.0, -lower, 0.0],
                pitch: 0.0,
                yaw: 0.0,
                flash: 0.0,
                visible: true,
                fov_scale: self.cfg.crucible_fov.clamp(0.3, 1.5),
                spin: [0.0; 2],
                mode: 1,
                muzzle_tag: None,
                glows: Vec::new(),
                overlay: None,
                roll: 0.0,
                tip: [0.0; 2],
            }));
            return;
        }
        if melee {
            self.melee_t += dt;
            viewmodel::set_pose(Some(viewmodel::Pose {
                folder,
                clip: self.melee_clip.to_string(),
                time: self.melee_t,
                looping: false,
                offset: [0.0, 0.03 - lower, 0.0],
                pitch: 0.0,
                yaw: 0.0,
                flash: 0.0,
                visible: true,
                // framed a little tighter: a right punch showed the end of the arm past the shoulder
                fov_scale: self.cfg.melee_fov.clamp(0.4, 1.2),
                spin: [0.0; 2],
                mode: 1,
                muzzle_tag: None,
                glows: Vec::new(),
                overlay: None,
                roll: 0.0,
                tip: [0.0; 2],
            }));
            return;
        }
        if self.vm_queue.first().is_some_and(|q| self.time >= q.0) {
            let (_, clip, speed, start, looping) = self.vm_queue.remove(0);
            let rest = std::mem::take(&mut self.vm_queue);
            self.vm_play(clip, looping);
            self.vm_speed = speed;
            self.vm_t = start;
            self.vm_queue = rest;
        }
        // Hold the clip at a given time for a while (SSG: pause after the kick).
        let mut advance = dt * self.vm_speed;
        if let Some((at, left)) = self.vm_hold {
            if self.vm_t + advance >= at && left > 0.0 {
                advance = (at - self.vm_t).max(0.0);
                self.vm_hold = Some((at, left - dt));
            }
        }
        self.vm_t += advance;
        if self.vm_loop && (self.vm_clip == "fire" || self.vm_clip == "turret_fire") && self.time - self.last_shot_at > WEAPONS[self.weapon].interval * 2.0 + 0.05 {
            let rec = if self.vm_clip == "turret_fire" { "turret_recover" } else { "recover" };
            if viewmodel::clip_len(folder, rec).is_some() {
                self.vm_play(rec, false);
            } else {
                let idle = self.idle_clip();
                self.vm_play(idle, true);
            }
        }
        // The empty-magazine idle loops: leave it as soon as the bombs are back (the reload clip
        // can end a frame before the magazine counts as full - the gun then stayed empty).
        if self.vm_clip == "sticky_empty" && (self.sticky_mag > 0 || self.weapon != 0) {
            let idle = self.idle_clip();
            self.vm_play(idle, true);
        }
        if !self.vm_loop && self.vm_queue.is_empty() {
            // the sticky reload holds its last (loaded) frame until the magazine is full
            if self.vm_clip == "sticky_reload" && self.sticky_mag == 0 {
                self.vm_t = self.vm_t.min(viewmodel::clip_len(folder, "sticky_reload").unwrap_or(1.8) - 0.001);
            }
            if let Some(len) = viewmodel::clip_len(folder, &self.vm_clip) {
                if self.vm_t >= len {
                    let idle = self.idle_clip();
                    self.vm_play(idle, true);
                }
            }
        }
        // The model is still waiting on its textures: hold the clip at its start so the equip
        // animation plays once the gun actually appears (it used to pop in at the end of it).
        if !viewmodel::is_loaded(folder) {
            self.vm_t = 0.0;
            self.switched_at = self.time;
        }
        // Gun kick: a stiff spring the shots push (see kick); it snaps back between shots and
        // builds up a little under automatic fire.
        let sdt = dt.min(0.05);
        let acc = -260.0 * self.recoil - 26.0 * self.recoil_v;
        self.recoil_v += acc * sdt;
        self.recoil += self.recoil_v * sdt;
        self.snap *= (-dt * 45.0).exp();
        if self.push_goal > self.push {
            self.push += (self.push_goal - self.push) * (dt / 0.035).min(1.0);
            if self.push >= self.push_goal * 0.95 {
                self.push_goal = 0.0;
            }
        } else {
            self.push_goal = 0.0;
            // Ballista settles slowly (quick kick, slow return - user); Heavy Cannon quicker.
            let tau = if self.weapon == 5 { 0.5 } else { 0.22 };
            self.push *= (-dt / tau).exp();
        }
        let (_, back, lift, side, snap_back) = RECOIL[self.weapon];
        let kick = if melee || chainsaw { 0.0 } else { (self.recoil + self.push).max(-0.3) };
        let kick_pitch = lift.to_radians() * kick;
        let kick_yaw = side.to_radians() * kick * self.recoil_yaw;
        // Turn about the grip, not the eye: offset by pivot - R * pivot.
        let pivot = Vec3::new(0.1, -0.13, 0.3);
        let rot = glam::Mat3::from_rotation_y(kick_yaw) * glam::Mat3::from_rotation_x(-kick_pitch);
        let pivot_fix = pivot - rot * pivot;
        // Chaingun parts: the turret's set while it is out or folding away (Doom swaps the shown
        // meshes with the mode); its shots come out of the four turret barrels in turn.
        let turret_clip = self.weapon == 6 && (self.turret || self.vm_clip.starts_with("turret_"));
        // The turret-mod Chaingun (Doom: chaingun_turret_primary/secondary) shows the turret parts
        // in both modes; folded, its four barrels are the rotary cluster.
        let turret_parts = self.weapon == 6;
        // Rocket Launcher: the loaded rocket's thruster burns in the chamber (Doom's rocket_thurst
        // on the "rocket" tag) while there is a rocket; firing flashes the chamber.
        let mut glows = Vec::new();
        if self.weapon == 3 && !melee && !chainsaw {
            let since = self.time - self.last_shot_at;
            if since < 0.45 {
                glows.push(("rocket_chamber", "fire", 0.16 * (1.0 - since / 0.45), 1.4 * (1.0 - since / 0.45)));
            }
            let loaded = self.ammo[Ammo::Rockets.index()] > 0 || self.cfg.infinite_ammo;
            if loaded && since > 0.5 {
                let a = ((since - 0.5) / 0.25).min(1.0);
                glows.push(("rocket", "fire", 0.09, 1.3 * a));
            }
        }
        // Spinning barrels: the rotary cluster spins up while the chaingun fires; in turret mode each
        // unfolded barrel spins on its own and the cluster settles square so they fold back in place.
        let firing = self.weapon == 6 && self.time - self.last_shot_at < 0.15;
        let target = if turret_clip { [0.0, if firing && self.turret { 32.0 } else { 0.0 }] } else { [if firing { 28.0 } else { 0.0 }, 0.0] };
        for g in 0..2 {
            // While the turret is out (or unfolding / folding) the cluster never turns: it is
            // brought to rest below before the barrels move.
            if g == 0 && turret_clip {
                continue;
            }
            if target[g] > 0.0 {
                self.spin_v[g] = (self.spin_v[g] + 90.0 * dt).min(target[g]);
                self.spin[g] = (self.spin[g] + self.spin_v[g] * dt).rem_euclid(std::f32::consts::TAU);
                continue;
            }
            // Spinning down: brake, but never so hard that it stops short - once the speed is
            // down to what can stop exactly at the rest angle (0), follow that curve to 0, so the
            // barrels always finish their turn and end where they started.
            if self.spin[g] == 0.0 && self.spin_v[g] == 0.0 {
                continue;
            }
            let brake = 36.0;
            let rem = (std::f32::consts::TAU - self.spin[g]).rem_euclid(std::f32::consts::TAU);
            let cap = (2.0 * brake * rem).sqrt().max(4.0);
            let mut v = (self.spin_v[g] - brake * dt).max(0.0);
            if v <= cap {
                v = v.max(4.0).min(cap);
            }
            if v * dt >= rem && v <= cap + 1e-3 {
                self.spin[g] = 0.0;
                self.spin_v[g] = 0.0;
            } else {
                self.spin_v[g] = v;
                self.spin[g] = (self.spin[g] + v * dt).rem_euclid(std::f32::consts::TAU);
            }
        }
        if turret_clip {
            // Finish the turn forward (to a whole revolution) quickly, then hold at 0; the unfold
            // waits at its first frame until then (it put the barrels in each other's places).
            if self.vm_clip == "turret_into" && self.spin[0] != 0.0 {
                self.vm_t = 0.0;
                if let Some(q) = self.vm_queue.first_mut() {
                    q.0 += dt;
                }
            }
            self.spin_v[0] = 0.0;
            if self.spin[0] > 1e-3 {
                let left = std::f32::consts::TAU - self.spin[0];
                let step = (40.0 * dt).min(left);
                self.spin[0] = if step >= left { 0.0 } else { self.spin[0] + step };
            }
        }
        let bob = (self.speed / 6.0).clamp(0.0, 1.5);
        // Aiming (Ballista right click, scopes) steadies the bob and sway (user: tuned down a lot).
        let aim = self.arb_zoom.max(self.zoom).clamp(0.0, 1.0);
        let steady = 1.0 - (1.0 - self.cfg.aim_steady.clamp(0.0, 1.0)) * aim;
        // No walking bob in the air (air speed made it run twice as fast) or during the landing
        // bounce: it fades out on takeoff and back in once the gun has settled.
        self.bob_hold = (self.bob_hold - dt).max(0.0);
        let gate_to = if self.sway_air > 0.0 || self.bob_hold > 0.0 { 0.0 } else { 1.0 };
        let rate = if gate_to > self.bob_gate { 12.0 } else { 8.0 };
        self.bob_gate += (gate_to - self.bob_gate) * (dt * rate).min(1.0);
        let gate = self.bob_gate;
        // Smoothing: the bob eases in and out when you start / stop.
        self.bob_amp += (bob * gate - self.bob_amp) * (dt * 4.0).min(1.0);
        let bob = self.bob_amp * steady;
        // Noise: every step rolls a new size and pace, blended in smoothly.
        let noise = self.cfg.bob_noise.clamp(0.0, 1.0);
        let pace = 1.0 + self.bob_var[2] * 0.15 * noise;
        self.bob_phase = (self.bob_phase + dt * 3.65 * self.cfg.bob[2] * pace * gate) % std::f32::consts::TAU;
        let step = (self.bob_phase / std::f32::consts::FRAC_PI_2) as i32;
        if step != self.bob_step {
            self.bob_step = step;
            for v in self.bob_var_to.iter_mut() {
                self.bob_rng ^= self.bob_rng << 13;
                self.bob_rng ^= self.bob_rng >> 17;
                self.bob_rng ^= self.bob_rng << 5;
                *v = (self.bob_rng as f32 / u32::MAX as f32) * 2.0 - 1.0;
            }
        }
        for k in 0..3 {
            self.bob_var[k] += (self.bob_var_to[k] - self.bob_var[k]) * (dt * 5.0).min(1.0);
        }
        let (side_k, dip_k) = (1.0 + self.bob_var[0] * noise, 1.0 + self.bob_var[1] * noise);
        // One smooth dip per step (sin^2 rounds off the hard bottom |sin| had).
        let dip = (self.bob_phase * 2.0).sin().powi(2);
        let (bob_h, bob_s) = (self.cfg.bob[0], self.cfg.bob[1]);
        // Doom's own weaponBob (bob_mode 1): every part its own sine, phase from distance walked.
        let doom = self.cfg.bob_mode == 1;
        let (mut d_off, mut d_tip, mut d_roll) = ([0.0f32; 3], [0.0f32; 2], 0.0f32);
        if doom {
            let db = &DOOM_BOB[if melee || chainsaw { 3 } else { self.weapon.min(7) }];
            let k = self.cfg.doom_bob;
            // (dash / hook speed doesn't hurry the steps: capped at the run speed)
            let walk = self.speed.min(self.cfg.ground_speed);
            self.doom_bob_phase = (self.doom_bob_phase + walk * gate * dt / DOOM_BOB_STRIDE * k[2]) % (std::f32::consts::TAU * 100.0);
            let ph = self.doom_bob_phase;
            let amt = (bob / 1.5).min(1.0); // full at running speed, eased in/out
            let m = self.cfg.doom_bob_move;
            let r = self.cfg.doom_bob_rot;
            let t = |i: usize| (ph * db.tv[i]).sin() * db.ta[i] * m[i] * k[0] * amt;
            let a = |i: usize| ((ph * db.rv[i]) + db.rp[i].to_radians()).sin() * db.ra[i].to_radians() * r[i] * k[1] * amt;
            d_off = [t(1), t(2), t(0)];
            d_tip = [a(1), a(0)];
            d_roll = a(2);
        }
        let (bob_h, bob_s) = if doom { (0.0, 0.0) } else { (bob_h, bob_s) };
        let roll = self.sway_step(dt) * steady;
        let since_switch = self.time - self.switched_at;
        let gun_off = if melee || chainsaw { [0.0; 3] } else { self.cfg.gun_offset[self.weapon] };
        // Super Shotgun no-target hook: 0.6 s layer (tips snap open, close slowly).
        const TWITCH: f32 = 0.6;
        let overlay = match self.hook_twitch {
            Some(t0) if self.weapon == SUPER_SHOTGUN && !melee && !chainsaw && self.time - t0 < TWITCH => {
                let len = viewmodel::clip_len(folder, "hook_notarget").unwrap_or(TWITCH);
                Some(("hook_notarget", (self.time - t0) / TWITCH * len))
            }
            _ => {
                self.hook_twitch = None;
                None
            }
        };
        let swap = if (0.0..0.2).contains(&since_switch) { (1.0 - since_switch / 0.2).powi(2) * -0.09 } else { 0.0 };
        viewmodel::set_pose(Some(viewmodel::Pose {
            folder,
            clip: self.vm_clip.clone(),
            time: self.vm_t,
            looping: self.vm_loop,
            offset: [
                // Walking bob, shaped like Doom Eternal's (measured from a recording: one dip per
                // step, barely any sideways sway) but slower and at a quarter of its height.
                self.bob_phase.sin() * 0.0006 * bob_s * side_k * bob + d_off[0] + pivot_fix.x + gun_off[0] - self.sway[0] * self.cfg.sway[1] * steady,
                -dip * 0.00125 * bob_h * dip_k * bob + swap - lower - saw_down + pivot_fix.y + gun_off[1] + self.sway_y[0] * steady + d_off[1],
                -back * kick - snap_back * self.snap + pivot_fix.z + gun_off[2] + d_off[2],
            ],
            pitch: kick_pitch,
            yaw: kick_yaw,
            flash: (1.0 - (self.time - self.last_shot_at) / 0.06).max(0.0),
            visible: true,
            // chainsaw: Doom's own handsFovScale (weapon/base/chainsaw.decl 0.65; at 1.0 the hand
            // filled the view - user), live: chainsaw_fov
            fov_scale: if chainsaw { self.cfg.chainsaw_fov.clamp(0.3, 1.5) } else if melee { 1.0 } else { HANDS_FOV[self.weapon] },
            spin: self.spin,
            // (the reload layer brings the new bombs in at its frame 44)
            mode: if turret_parts || (self.weapon == 0 && self.sticky_mag == 0 && !(self.vm_clip == "sticky_reload" && self.vm_t > 44.0 / 30.0)) { 2 } else { 1 },
            muzzle_tag: if turret_clip { Some(TURRET_MUZZLES[(self.shots as usize) % 4]) } else { None },
            glows,
            overlay,
            roll: roll + d_roll,
            // Doom's rotational bob: the tip swings side to side once per stride (yaw, phase
            // -90) and nods with each step.
            tip: if doom {
                d_tip
            } else {
                [
                    self.bob_phase.cos() * self.cfg.bob_tip[0].to_radians() * side_k * bob,
                    -dip * self.cfg.bob_tip[1].to_radians() * dip_k * bob,
                ]
            },
        }));
    }

    /// Hit every hostile in a cone in front of the camera with a point-blank damage bullet
    /// (goes through ER's own damage, stagger and death handling).
    /// A hostile within a punch's reach (the same range / aim cone melee_hit uses).
    fn in_punch_reach(&self, enemies: &[Enemy], range: f32, cone: f32) -> bool {
        let Some((cam, fwd)) = game::camera() else { return false };
        enemies
            .iter()
            .filter(|e| is_hostile(e) && e.hp() > 0 && e.dist <= range)
            .any(|e| (e.pos() + Vec3::Y * 1.1 - cam).normalize_or_zero().dot(fwd) >= cone)
    }

    fn melee_hit(&mut self, enemies: &[Enemy], range: f32, cone: f32, bullet_id: i32, max_targets: usize, amount: f32) -> usize {
        let Some((cam, fwd)) = game::camera() else { return 0 };
        let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        let mut hit = 0;
        for e in enemies.iter().filter(|e| is_hostile(e) && e.dist <= range) {
            let chest = e.pos() + Vec3::Y * 1.1;
            if (chest - cam).normalize_or_zero().dot(fwd) < cone {
                continue;
            }
            let from = chest - (chest - cam).normalize_or_zero() * 0.9;
            // Reaction through the persistent melee hitters (no per-hit spawns: pool-safe).
            // Only the Blood Punch staggers (user: normal punches just do damage).
            if bullet_id == params::BLOOD_PUNCH_DAMAGE_BULLET {
                self.hitters.hit(crate::hitter::BLOOD_PUNCH, chest, (chest - from).normalize_or_zero());
            }
            let _ = owner;
            // Blood Punch kills outright (user: it left them in the stagger state) - except
            // bosses and mini bosses, which just take the hit.
            let lethal = bullet_id == params::BLOOD_PUNCH_DAMAGE_BULLET && !self.chainsaw_immune(e);
            let dmg = if lethal { (e.max_hp().max(e.hp()) as f32 * 2.0 + 1000.0).max(amount * self.dmg_mult()) } else { amount * self.dmg_mult() };
            damage::NO_CATCH.store(lethal, std::sync::atomic::Ordering::Relaxed);
            let _ = damage::apply_handle(e.chr.field_ins_handle, dmg);
            damage::NO_CATCH.store(false, std::sync::atomic::Ordering::Relaxed);
            self.last_combat = self.time;
            hit += 1;
            if hit >= max_targets {
                break;
            }
        }
        hit
    }

    /// Doom melee: alternate left/right punches (3D fists), light damage + stagger.
    fn punch(&mut self, enemies: &[Enemy]) {
        self.punch_alt = !self.punch_alt;
        self.melee_clip = if self.punch_alt { "punch_r" } else { "punch_l" };
        self.melee_t = 0.0;
        // The fist is out of frame ~0.3 s in (recorded); then the gun slides back up.
        self.melee_until = self.time + 0.36;
        self.switched_at = self.melee_until;
        self.melee_cd = 0.4;
        let n = self.melee_hit(enemies, 2.8, 0.55, params::MELEE_BULLET, 1, damage::MELEE);
        audio::play_vol(if n > 0 { "melee" } else { "glory_whoosh" }, 0.8);
        // Punching something breakable smashes it - the same things a dash breaks.
        if let Some((cam, fwd)) = game::camera() {
            self.break_along(cam, fwd, 2.8, 0.3);
        }
    }

    /// Blood Punch: Doom's charged punch - big shockwave hitting everything in front.
    fn blood_punch_blast(&mut self, enemies: &[Enemy]) {
        self.melee_clip = "bloodpunch";
        self.melee_t = 0.0;
        // shown for the clip's own length (0.37 s): 0.6 held its last frame still on screen (user)
        self.melee_until = self.time + viewmodel::clip_len("fists", "bloodpunch").unwrap_or(0.6);
        self.switched_at = self.melee_until;
        self.melee_cd = 0.6;
        let n = self.melee_hit(enemies, 5.5, 0.35, params::BLOOD_PUNCH_DAMAGE_BULLET, 8, damage::BLOOD_PUNCH);
        // Visual: fiery shockwave burst in front of the fist.
        if let Some((cam, fwd)) = game::camera() {
            let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
            // One shockwave pushed out ahead of the fist; three at the lens whited out the screen.
            let _ = bullet::spawn_capped(bullet::CAP_BLOOD_PUNCH, owner, params::BLOOD_PUNCH_BULLET, cam + fwd * 2.5 - Vec3::Y * 0.4, fwd);
            self.break_area(cam + fwd * 2.5 - Vec3::Y * 0.6, 3.0);
        }
        audio::play("blood_punch");
        self.msg(format!("BLOOD PUNCH! ({n} hit)"));
    }

    /// Health bars over hostiles you can see (close ones always; far ones once hurt), plus a boss
    /// bar. Line of sight is re-checked a few times a second per enemy.
    fn update_bars(&mut self, enemies: &[Enemy]) {
        // Hostiles only: untouched ones within 20 m, damaged ones within 45 m, and only in clear
        // line of sight (chest or head - an enemy still rising out of the ground has its chest
        // below the surface). Enemies marked with the mark key (Z) always show theirs. Bosses
        // the game shows a bar for keep only that big bar.
        let cam = game::camera().map(|(p, _)| p);
        let now = self.time;
        let mut bars = Vec::new();
        let active_bosses = game::active_boss_handles();
        let mut seen = HashMap::new();
        self.marked.retain(|h| enemies.iter().any(|e| e.chr.field_ins_handle == *h && e.dist < 60.0));
        for e in enemies.iter().filter(|e| is_hostile(e) && e.dist < 60.0) {
            if active_bosses.contains(&e.chr.field_ins_handle) {
                continue;
            }
            let marked = self.marked.contains(&e.chr.field_ins_handle);
            // Ambient wildlife shares the enemy team but is harmless - no automatic bar (Z can
            // still mark one). Bears (c6030/6031) and boars (c6050) fight back: they keep theirs.
            let wildlife = AMBIENT_WILDLIFE.contains(&e.chr.character_id);
            if wildlife && !marked {
                continue;
            }
            let hurt = e.hp() < e.max_hp();
            let in_range = e.dist < if hurt { 45.0 } else { 20.0 };
            if !marked && !in_range {
                continue;
            }
            let key = &*e.chr as *const _ as usize;
            let ph = &e.chr.modules.physics;
            let body = ph.chr_hit_height.max(ph.hit_height);
            let body = if body.is_finite() && body > 0.2 && body < 30.0 { body } else { 1.6 };
            let head = e.pos() + Vec3::Y * (body.max(1.2) + 0.35);
            let (mut vis, mut at) = self.bar_los.get(&key).copied().unwrap_or((false, -10.0));
            if !marked && now - at > 0.25 {
                if let Some(c) = cam {
                    // Aim inside the body at its own size (middle and near the top): a fixed
                    // 1.2 m aimed over small creatures (dogs, rats, imps) and they never got bars.
                    // Plus a point just above the ground: crawling creatures are far lower than
                    // their standing size and their bar only showed when they reared up to attack.
                    vis = [0.35f32, body * 0.5, body * 0.85].iter().any(|h| {
                        let p = e.pos() + Vec3::Y * *h;
                        damage::trace(c, p - c, (p - c).length() + 0.5, 0.05).chr == Some(e.chr.field_ins_handle)
                    });
                }
                at = now;
            }
            seen.insert(key, (vis, at));
            if vis || marked {
                bars.push(EnemyBar {
                    pos: head,
                    frac: e.hp() as f32 / e.max_hp() as f32,
                    dist: e.dist,
                    boss: false,
                    staggered: self.is_staggered(e),
                    on_screen: true,
                });
            }
        }
        self.bar_los = seen;
        self.bars = bars;
    }

    /// Mark key: give the enemy under the crosshair a bar (press again on it to unmark).
    fn toggle_mark(&mut self) {
        let Some((cam, fwd)) = game::camera() else { return };
        let hit = damage::trace(cam, fwd, 60.0, 0.3);
        let Some(h) = hit.chr else {
            self.msg("NO TARGET");
            return;
        };
        if let Some(i) = self.marked.iter().position(|m| *m == h) {
            self.marked.remove(i);
            self.msg("UNMARKED");
        } else {
            self.marked.push(h);
            self.msg("MARKED");
        }
    }

    /// Doom loot tokens picked up from the ground become Doom ammo / health and leave the
    /// inventory again (one stays as a placeholder entry, never shown in our HUD).
    /// A boss (boss bar) seen alive that is now at 0 HP was defeated: one Crucible charge is owed.
    fn track_bosses(&mut self, dt: f32) {
        let Some(w) = game::world() else { return };
        let mut now = HashMap::new();
        for h in game::active_boss_handles() {
            if let Some(c) = w.chr_ins_by_handle(&h).filter(|c| game::chr_ok(*c as *const _)) {
                now.insert(h, c.modules.data.hp);
            }
        }
        if self.ready {
            self.boss_stage_window = (self.boss_stage_window - dt).max(0.0);
        }
        for h in now.keys() {
            if self.boss_stage_window > 0.0 && !self.boss_seen.contains_key(h) && self.boss_stages.insert(*h) {
                log::info!("boss: a new bar right after a boss died - the next stage (no extra charge)");
            }
        }
        for (h, hp) in &self.boss_seen {
            // (the bar can vanish the frame the boss dies: look the character up directly)
            let dead = match now.get(h) {
                Some(&n) => n <= 0,
                None => w.chr_ins_by_handle(h).is_some_and(|c| game::chr_ok(c as *const _) && c.modules.data.hp <= 0),
            };
            if *hp > 0 && dead {
                if self.boss_stages.contains(h) {
                    log::info!("boss defeated: last stage of the same fight - no extra charge");
                } else {
                    self.boss_crucible += 1;
                    self.boss_crucible_at = self.time;
                    log::info!("boss defeated: a Crucible charge comes with the next drop");
                }
                self.boss_stage_window = BOSS_STAGE;
            }
        }
        self.boss_seen = now.into_iter().filter(|(_, hp)| *hp > 0).collect();
        if self.boss_seen.is_empty() && self.boss_stage_window <= 0.0 {
            self.boss_stages.clear();
        }
        if self.boss_crucible > 0 && self.time - self.boss_crucible_at > 30.0 {
            self.grant_boss_crucible();
        }
    }

    fn grant_boss_crucible(&mut self) {
        let n = std::mem::take(&mut self.boss_crucible);
        let before = self.crucible_charges;
        self.crucible_charges = (self.crucible_charges + n).min(CRUCIBLE_MAX);
        let got = self.crucible_charges - before;
        log::info!("boss Crucible: +{got} (charges {})", self.crucible_charges);
        if got > 0 {
            audio::play("crucible_pickup");
            self.msg(format!("+{got} CRUCIBLE"));
        }
    }

    fn collect_tokens(&mut self) {
        let Some(p) = game::player() else { return };
        let gd = unsafe { p.player_game_data.as_mut() };
        let inv = &gd.equipment.equip_inventory_data.items_data;
        let mut ammo = 0;
        let mut health = 0;
        for e in inv.items_mut() {
            let id = e.item_id.param_id() as i32;
            let cat_goods = e.item_id.category() == eldenring::cs::ItemCategory::Goods;
            if !cat_goods || e.quantity <= 1 {
                continue;
            }
            if id == params::DOOM_AMMO_GOODS {
                ammo += e.quantity - 1;
                e.quantity = 1;
            } else if id == params::DOOM_HEALTH_GOODS {
                health += e.quantity - 1;
                e.quantity = 1;
            }
        }
        // Tokens already in the inventory when the save loads aren't pickups: note them silently.
        if !self.token_init {
            self.token_init = true;
            let has = |id: i32| inv.items().any(|e| e.item_id.param_id() as i32 == id && e.item_id.category() == eldenring::cs::ItemCategory::Goods);
            self.token_seen = (has(params::DOOM_AMMO_GOODS), has(params::DOOM_HEALTH_GOODS));
            return;
        }
        // First pickup of a token creates the entry with quantity 1: count it once.
        for (id, seen) in [(params::DOOM_AMMO_GOODS, &mut self.token_seen.0), (params::DOOM_HEALTH_GOODS, &mut self.token_seen.1)] {
            let has = inv.items().any(|e| e.item_id.param_id() as i32 == id && e.item_id.category() == eldenring::cs::ItemCategory::Goods);
            if has && !*seen {
                *seen = true;
                if id == params::DOOM_AMMO_GOODS { ammo += 1 } else { health += 1 }
            }
        }
        // a defeated boss's Crucible comes with its drop (the next one picked up - user)
        if (ammo > 0 || health > 0) && self.boss_crucible > 0 {
            self.grant_boss_crucible();
        }
        if ammo > 0 {
            // Doom: ammo for what you're holding (per pickup: 6 shells / 15 bullets / 20 cells /
            // 3 rockets) plus half that of a second type. BFG ammo is a rare single round.
            let a = WEAPONS[self.weapon].ammo;
            let per = |t: Ammo| match t {
                Ammo::Shells => 6,
                Ammo::Bullets => 25,
                Ammo::Cells => 20,
                Ammo::Rockets => 3,
                Ammo::Bfg => 0,
            };
            let mut got = [0i32; 5];
            let mut crucible_got = 0;
            // A Crucible kill gives no ammo (user): its drops are taken, nothing is added.
            let no_loot = self.time < self.cr_no_loot_until;
            for k in 0..(if no_loot { 0 } else { ammo }) {
                let roll = (self.kills as usize * 7 + self.shots as usize * 13 + k as usize * 31) % 20;
                let mut give = |t: Ammo, n: i32| {
                    let before = self.ammo[t.index()];
                    self.ammo[t.index()] = (before + n).min(t.max());
                    got[t.index()] += self.ammo[t.index()] - before;
                };
                give(a, per(a));
                let mut other = [Ammo::Shells, Ammo::Bullets, Ammo::Cells, Ammo::Rockets][(self.kills as usize + self.shots as usize + k as usize) % 4];
                if other == a {
                    other = [Ammo::Bullets, Ammo::Cells, Ammo::Rockets, Ammo::Shells][a.index().min(3)];
                }
                give(other, (per(other) + 1) / 2);
                // ~1 in 20 pickups carries a BFG round.
                if roll == 0 {
                    give(Ammo::Bfg, 1);
                }
                // The Crucible: 1 in 80 - rarer than BFG ammo (user).
                if fastrand::u32(..80) == 0 && self.crucible_charges < CRUCIBLE_MAX {
                    self.crucible_charges += 1;
                    crucible_got += 1;
                }
            }
            if !no_loot {
                audio::play("pickup_ammo");
                let mut parts: Vec<String> = [Ammo::Shells, Ammo::Bullets, Ammo::Cells, Ammo::Rockets, Ammo::Bfg]
                    .iter()
                    .filter(|t| got[t.index()] > 0)
                    .map(|t| format!("+{} {}", got[t.index()], t.name().to_uppercase()))
                    .collect();
                if crucible_got > 0 {
                    parts.push(format!("+{crucible_got} CRUCIBLE"));
                    audio::play("crucible_pickup");
                }
                self.msg(if parts.is_empty() { "AMMO FULL".to_string() } else { parts.join("  ") });
            }
        }
        if health > 0 && self.time >= self.cr_no_loot_until {
            let d = &mut p.chr_ins.modules.data;
            let heal = (d.max_hp as f32 * 0.12) as i32 * health as i32;
            d.hp = (d.hp + heal.max(1)).min(d.max_hp);
            audio::play("pickup_health");
            self.msg(format!("HEALTH +{heal}"));
        }
    }

    /// Detect hostile deaths since last frame and drop Doom loot where they fell.
    fn track_deaths(&mut self, enemies: &[Enemy]) {
        // Kills from Doom damage are known exactly (one-shots never show up as "hurt" below).
        let direct: Vec<_> = std::mem::take(&mut *damage::KILLS.lock().unwrap_or_else(|e| e.into_inner()));
        for (ptr, at, max_hp) in direct {
            self.enemy_track.remove(&ptr);
            self.kills += 1;
            self.drop_loot(at, max_hp);
        }
        let mut seen = HashMap::new();
        for e in enemies.iter().filter(|e| is_hostile(e)) {
            seen.insert(&*e.chr as *const _ as usize, (e.hp(), e.pos(), e.max_hp()));
        }
        // game::enemies() skips dead characters, so a tracked one that vanished while hurt died.
        let dead: Vec<(Vec3, i32)> = self
            .enemy_track
            .iter()
            .filter(|(k, (hp, _, max))| !seen.contains_key(k) && *hp < *max && *hp > 0)
            .map(|(_, (_, pos, max))| (*pos, *max))
            .collect();
        self.enemy_track = seen;
        for (pos, max_hp) in dead {
            self.kills += 1;
            self.drop_loot(pos + Vec3::Y * 1.2, max_hp);
        }
    }

    fn drop_loot(&mut self, at: Vec3, max_hp: i32) {
        if self.cfg.doom_loot {
            // Enemies drop a real Elden Ring item instead (see collect_tokens).
            let _ = (at, max_hp);
            return;
        }
        let base = max_hp as f32 / crate::params::applied_hp_mult();
        let big = base > 1500.0;
        // Ammo for the weapon in hand (Doom: what you need), plus a random other type.
        let w = &WEAPONS[self.weapon];
        let a = w.ammo;
        let low = self.ammo[a.index()] < a.max() / 4;
        let n = if big { 6 } else if low { 4 } else { 2 };
        let per = ((a.max() as f32 * 0.06).ceil() as i32).max(1);
        if a != Ammo::Bfg {
            self.pickups.burst(at, Kind::Ammo(a), n, per, false);
        }
        let other = [Ammo::Shells, Ammo::Bullets, Ammo::Cells, Ammo::Rockets][self.kills as usize % 4];
        if other != a {
            self.pickups.burst(at, Kind::Ammo(other), 1, ((other.max() as f32 * 0.06).ceil() as i32).max(1), false);
        }
        // A little health when you're hurt (Doom drops health on low HP).
        if let Some(p) = game::player() {
            let d = &p.chr_ins.modules.data;
            if d.hp * 2 < d.max_hp || big {
                self.pickups.burst(at, Kind::Health, if big { 4 } else { 2 }, (d.max_hp / 40).max(5), false);
            }
        }
    }

    fn collect(&mut self, got: pickups::Collected) {
        if got.health > 0 {
            if let Some(p) = game::player() {
                let d = &mut p.chr_ins.modules.data;
                // Never refill a dead player: the game's death is already under way, and the
                // health only made you look alive for a few seconds.
                if d.hp > 0 {
                    d.hp = (d.hp + got.health).min(d.max_hp);
                }
                self.prev_hp = d.hp;
            }
            audio::play_vol("pickup_health", 0.7);
        }
        if got.armor > 0 {
            self.armor = (self.armor + got.armor).min(self.cfg.armor_max);
            audio::play_vol("pickup_armor", 0.6);
        }
        if got.ammo.iter().any(|&a| a > 0) {
            for a in Ammo::ALL {
                let i = a.index();
                self.ammo[i] = (self.ammo[i] + got.ammo[i]).min(a.max());
            }
            audio::play_vol("pickup_ammo", 0.6);
        }
    }

    // ---------------------------------------------------------------- movement

    /// WASD direction on the ground plane, relative to where the camera looks.
    /// Jump: the gun lifts a little (impulse on the vertical sway spring).
    fn sway_kick(&mut self, k: f32) {
        self.sway_y[1] += self.cfg.sway[2] * 12.0 * k;
    }

    /// Doom-style weapon sway (weaponLag pendulum, values measured from Doom footage): strafing
    /// or turning tilts the gun toward that direction and lets it trail a little behind; jumps
    /// and landings bob it up / down. Springs with a slight overshoot. Returns the tilt (rad);
    /// self.sway[0] (trail) and self.sway_y[0] (lift) feed the gun offset.
    fn sway_step(&mut self, dt: f32) -> f32 {
        let dt = dt.clamp(0.0, 0.05);
        let fwd = game::camera().map(|(_, f)| Vec3::new(f.x, 0.0, f.z).normalize_or(Vec3::NEG_Z));
        if self.in_world_for < 1.0 {
            // loading / respawn / fast travel: the camera jumps - start at rest, no whip
            self.sway = [0.0; 2];
            self.sway_y = [0.0; 2];
            self.sway_lat = 0.0;
            self.sway_fwd = None;
        }
        let mut turn = 0.0; // rad/s, + = turning right
        if let (Some(f), Some(pf), true) = (fwd, self.sway_fwd, dt > 0.0) {
            let pr = Vec3::Y.cross(pf).normalize_or(Vec3::X);
            turn = (f.dot(pr).clamp(-1.0, 1.0).asin() / dt).clamp(-20.0, 20.0);
        }
        self.sway_fwd = fwd;
        // 1 = full strafe (~7 m/s) or a fast turn (~200 deg/s); both feel the same, like Doom.
        let drive = (self.sway_lat / 7.0 + turn / 200f32.to_radians()).clamp(-3.0, 3.0).tanh();
        let spring = |s: &mut [f32; 2], target: f32, w: f32, z: f32| {
            let a = w * w * (target - s[0]) - 2.0 * z * w * s[1];
            s[1] += a * dt;
            s[0] += s[1] * dt;
        };
        spring(&mut self.sway, drive, 11.0, 0.55);
        spring(&mut self.sway_y, 0.0, 9.0, 0.45);
        self.sway[0] = self.sway[0].clamp(-1.6, 1.6);
        self.sway_y[0] = self.sway_y[0].clamp(-0.05, 0.05);
        self.sway[0] * self.cfg.sway[0].to_radians()
    }

    fn wish_dir(&self) -> Vec3 {
        let Some((_, fwd)) = game::camera() else { return Vec3::ZERO };
        let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
        // ER world is left-handed (DirectX): right = up x forward.
        let right = Vec3::Y.cross(flat_fwd).normalize_or(Vec3::X);
        if let Some((x, z, until)) = *WALKTO.lock().unwrap_or_else(|e| e.into_inner()) {
            if let (true, Some(p)) = (Instant::now() < until, game::player()) {
                let at = game::chr_pos(&p.chr_ins);
                let to = Vec3::new(x - at.x, 0.0, z - at.z);
                if to.length() > 0.3 {
                    return to.normalize();
                }
            }
        }
        if let Some((deg, until)) = *AUTOWALK.lock().unwrap_or_else(|e| e.into_inner()) {
            if Instant::now() < until {
                let (sn, cs) = deg.to_radians().sin_cos();
                return (flat_fwd * cs + right * sn).normalize_or_zero();
            }
        }
        let (sx, sz) = input::move_axes(&self.input);
        (flat_fwd * sz + right * sx).normalize_or_zero()
    }

    /// Out of ammo with a boss bar up: the HUD tells you the chainsaw refills from the boss.
    pub fn boss_saw_hint(&self) -> bool {
        self.saw_mode && self.glory.is_none() && !game::active_boss_handles().is_empty()
    }

    /// Dashes you can use right now (the HUD icon is dark at 0).
    pub fn dash_ready(&self) -> u32 {
        if self.dash_lock {
            return 0;
        }
        let n = self.dash_charges.floor();
        (if self.dash_ground { n } else { n.min(self.air_dashes) }).max(0.0) as u32
    }

    fn cr_play(&mut self, clip: &'static str, looping: bool) {
        self.cr_clip = clip;
        self.cr_t = 0.0;
        self.cr_loop = looping;
    }

    fn cr_len(&self, clip: &str) -> f32 {
        viewmodel::clip_len(viewmodel::CRUCIBLE_FOLDER, clip).unwrap_or(0.5)
    }

    /// Charges a Crucible kill on this enemy takes (user): bosses (boss bar) never; base HP under
    /// the chainsaw limit 1, under twice it 2, anything else all 3 - no HP cap (DLC mini bosses
    /// reach 90k HP; the boss bar is the only line - user).
    fn crucible_cost(&self, e: &Enemy) -> Option<u32> {
        // a boss bar up, or a registered boss whose bar isn't up yet
        if Self::is_boss(&e.chr) {
            return None;
        }
        let base = e.max_hp() as f32 / crate::params::applied_hp_mult();
        let lim = self.cfg.crucible_hp.max(1.0);
        if base < lim {
            Some(1)
        } else if base < 2.0 * lim {
            Some(2)
        } else {
            Some(3)
        }
    }

    /// Straight back to the gun that was in hand before the Crucible (quick swap / scroll wheel):
    /// the last-weapon chain isn't touched - the Crucible is no part of it (user).
    fn crucible_to_gun(&mut self) {
        self.crucible_out = false;
        self.cr_away_at = None;
        self.vm_play("bringup", false);
        self.switched_at = self.time;
        self.fire_cd = self.cfg.switch_fire_delay.max(0.0);
        audio::play_vol("weapon_switch", 0.5);
    }

    /// The whole put-away: Doom's deactivate already folds and lowers it (an extra lowering
    /// clip brought it back up first - user).
    fn cr_away_len(&self) -> f32 {
        self.cr_len("bringdown")
    }

    fn crucible_put_away(&mut self) {
        if !self.crucible_out || self.cr_away_at.is_some() {
            return;
        }
        // the whole of Doom's deactivate plays (it was cut at 0.6 s - user)
        self.cr_play("bringdown", false);
        self.cr_away_at = Some(self.time + self.cr_away_len());
        audio::play_vol("crucible_close", 0.8);
    }

    /// The Crucible (user spec): V draws it (charges or not) and puts it away; left
    /// click swings. A demon in reach dies outright for its charges; nothing in reach = Doom's
    /// swing, no charge. Out of charges after a kill: it goes away.
    fn update_crucible(&mut self, enemies: &[Enemy], armed: bool) {
        let k = self.cfg.keys.clone();
        if !armed && self.crucible_out {
            // cutscene / loading: gone at once. Menus, the settings window and grabs keep it out,
            // lowered like a gun (it used to vanish and show the gun lowering instead - user).
            if game::now_loading() || game::in_cutscene() {
                self.crucible_out = false;
                self.cr_away_at = None;
            }
            return;
        }
        if let Some(t) = self.cr_away_at {
            if self.time >= t {
                self.crucible_out = false;
                self.cr_away_at = None;
                self.vm_play("bringup", false);
                self.switched_at = self.time;
            }
            return;
        }
        if armed && self.glory.is_none() && !self.saw_mode && self.input.pressed3(k.crucible, k.crucible_alt, k.crucible_pad) {
            if self.crucible_out {
                self.crucible_put_away();
            } else {
                // drawn with or without charges (user); a swing with none gives the gun back
                self.crucible_out = true;
                self.cr_blade = false;
                self.cr_play("bringup", false);
            }
            return;
        }
        if !self.crucible_out {
            return;
        }
        if self.saw_mode {
            self.crucible_out = false;
            return;
        }
        // swing: after the draw, or most of the way through the last swing
        let swinging = self.cr_clip.starts_with("swing");
        let ready = if self.cr_clip == "bringup" {
            self.cr_t >= self.cr_len("bringup") * 0.8
        } else if swinging {
            self.cr_t >= self.cr_len(self.cr_clip) * 0.6
        } else {
            true
        };
        if !(ready && self.input.pressed3(k.fire, k.fire_alt, k.fire_pad) && !self.wheel_open) {
            return;
        }
        // no charges: left click brings back the last gun (user)
        if self.crucible_charges == 0 {
            self.crucible_to_gun();
            return;
        }
        const SWINGS: [&str; 4] = ["swing_r1", "swing_l1", "swing_r2", "swing_l2"];
        // reach (live: crucible_range) to the enemy's body, not its centre - big ones count too
        let target = game::camera().and_then(|(cam, fwd)| {
            enemies
                .iter()
                .enumerate()
                .filter(|(_, e)| is_hostile(e) && e.hp() > 0)
                .filter(|(_, e)| e.dist <= self.cfg.crucible_range + e.chr.modules.physics.hit_radius.max(0.3))
                .filter(|(_, e)| (e.pos() - cam).normalize_or_zero().dot(fwd) > 0.35)
                .min_by(|a, b| a.1.dist.total_cmp(&b.1.dist))
                .map(|(i, _)| i)
        });
        if let Some(t) = target {
            let e = &enemies[t];
            match self.crucible_cost(e) {
                None => {
                    log::info!("crucible: npc {} entity {} refused (boss)", e.chr.npc_param_id, e.chr.event_entity_id);
                    self.msg("INVALID TARGET");
                    audio::play("crucible_no_energy");
                    return;
                }
                Some(cost) if cost > self.crucible_charges => {
                    self.msg(format!("CRUCIBLE NEEDS {cost} CHARGES"));
                    audio::play("crucible_no_energy");
                    return;
                }
                Some(cost) => {
                    let swing = SWINGS[self.cr_swing % SWINGS.len()];
                    self.cr_swing += 1;
                    self.cr_play(swing, false);
                    audio::play("crucible_swing");
                    let key = &*e.chr as *const _ as usize;
                    // (the same handle-to-pointer step the glory kill uses: the enemy list is
                    // shared, the kill writes its HP)
                    let chr = unsafe { &mut *(std::hint::black_box(key) as *mut eldenring::cs::ChrIns) };
                    // gore: the glory kill's darts
                    let chest = game::chr_pos(chr) + Vec3::Y * 1.2;
                    let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
                    for i in 0..5 {
                        let a = i as f32 * 2.1;
                        let from = chest + Vec3::new(a.cos(), 0.2, a.sin()) * 1.2;
                        let _ = bullet::spawn_capped(bullet::CAP_FX, owner, params::FX_GORE_BULLET, from, chest - from);
                    }
                    log::info!("crucible: npc {} entity {} killed for {cost} charge(s), base HP {:.0}", e.chr.npc_param_id, e.chr.event_entity_id, e.max_hp() as f32 / crate::params::applied_hp_mult());
                    chr.modules.data.hp = 0;
                    self.kills += 1;
                    // no Doom loot, and its ER item drops give nothing (burning enemies still shed
                    // their armor shards through the burn tracker)
                    self.enemy_track.remove(&key);
                    self.cr_no_loot_until = self.time + 4.0;
                    self.crucible_charges -= cost;
                    self.last_combat = self.time;
                    audio::play("crucible_hit");
                    // (no on-screen text on a Crucible kill - user)
                    if self.crucible_charges == 0 {
                        // out of charges: away once the swing has played
                        // (the swing plays at crucible_swing_speed: real time = clip time / speed)
                        let swing_s = self.cr_len(swing) / self.cfg.crucible_swing_speed.clamp(0.1, 3.0);
                        self.cr_away_at = Some(self.time + swing_s + self.cr_away_len());
                        self.later.push((swing_s, "crucible_close"));
                    }
                    return;
                }
            }
        }
        // nothing in reach: Doom's swing, no charge
        let swing = SWINGS[self.cr_swing % SWINGS.len()];
        self.cr_swing += 1;
        self.cr_play(swing, false);
        audio::play("crucible_swing");
    }

    fn start_dash(&mut self) {
        let Some((_, fwd)) = game::camera() else { return };
        let flat_fwd = Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(Vec3::NEG_Z);
        // ER world is left-handed (DirectX): right = up x forward.
        let right = Vec3::Y.cross(flat_fwd).normalize_or(Vec3::X);
        let (sx, sz) = input::move_axes(&self.input);
        let mut dir = flat_fwd * sz + right * sx;
        if dir.length_squared() < 1e-3 {
            dir = flat_fwd;
        }
        self.dash_dir = dir.normalize();
        self.dash_t = self.cfg.dash_time;
        self.dash_charges -= 1.0;
        audio::play("dash");
    }

    /// Stairs: the body climbs in uneven per-frame steps (0-16 cm at a steady 15 cm stride,
    /// following the step edges) - the stair jitter. The first-person camera follows a filtered
    /// height instead: an alpha-beta filter tracks a steady slope without lag and irons out the
    /// steps. Loose in the air (jumps/falls stay direct), reset on teleports and big drops.
    fn smooth_camera(&mut self, y: f32, dt: f32, grounded: bool) {
        if dt <= 0.0 {
            return;
        }
        // In the air the camera rides the body directly (no lag on jumps/falls); after landing
        // the filter restarts at rest, so the fall speed can't overshoot into a bob.
        // Landing: the body comes down the last ~35 cm over ~0.1 s after touchdown (land_vy) and
        // the stair smoothing made the camera trail that and keep sinking after the body stopped
        // - a two-stage landing (user, camera log). The camera rides the body directly through
        // the landing (land_cam_time, live; 0 = off).
        if grounded && self.air_t >= 0.15 && self.cfg.land_cam_time > 0.0 && self.time - self.real_land_at < 0.1 {
            self.cam_land_until = self.time + self.cfg.land_cam_time;
        }
        // Doom's landing: a quick dip of the view, then back up (user, watching Doom Eternal):
        // depth scales with how fast you came down (a hop dips a little, a big fall more).
        if grounded && self.air_t >= 0.15 && self.time - self.real_land_at < 0.1 {
            let k = (-self.air_vy_meas / 8.0).clamp(0.4, 1.0);
            self.land_dip = Some((self.time, self.cfg.land_bounce[0].max(0.0) * k));
        }
        let dip = match self.land_dip {
            Some((t0, depth)) => {
                let (down, up) = (self.cfg.land_bounce[1].max(0.01), self.cfg.land_bounce[2].max(0.01));
                let t = self.time - t0;
                if t < down {
                    let a = t / down;
                    -depth * (1.0 - (1.0 - a) * (1.0 - a))
                } else if t < down + up {
                    let a = (t - down) / up;
                    -depth * (1.0 - a * a * (3.0 - 2.0 * a))
                } else {
                    self.land_dip = None;
                    0.0
                }
            }
            None => 0.0,
        };
        self.air_t = if grounded { 0.0 } else { self.air_t + dt };
        if self.time < self.cam_land_until {
            self.cam_filter = Some((y, 0.0, y));
            camera_step(dip);
            return;
        }
        let (mut sy, vy, last) = match self.cam_filter {
            Some(f) if self.air_t < 0.15 => f,
            _ => {
                self.cam_filter = Some((y, 0.0, y));
                camera_step(dip);
                return;
            }
        };
        // Map tile changes re-base the local position (z jumps 32 m) and teleports jump: carry
        // the filter along so the camera doesn't pop.
        if (y - last).abs() > 0.6 {
            sy += y - last;
        }
        let frames = dt * 60.0;
        // Tuned on recorded stair runs: frame-to-frame jitter 6.6 cm -> ~1 cm, lag <= ~0.25 m.
        // 0.2: tuned offline on recorded stair runs and bumpy ground (camera log): jitter ~6x
        // lower than the body's, lag <= ~0.3 m briefly at stair ends, ~1 cm overshoot on stops.
        let a60: f32 = if grounded { 0.2 } else { 0.6 };
        let alpha = 1.0 - (1.0 - a60).powf(frames);
        let beta = alpha * alpha / (2.0 - alpha);
        let pred = sy + vy * dt;
        let r = y - pred;
        let sy = pred + alpha * r;
        let vy = vy + beta * r / dt;
        let sy = y + (sy - y).clamp(-0.35, 0.35);
        self.cam_filter = Some((sy, vy, y));
        camera_step(sy - y + dip);
    }

    /// Mod movement upgrades: Fast Gunner (turret), Mobility (scope), Full Speed (arbalest).
    fn move_slow(&self) -> f32 {
        if self.turret {
            0.6
        } else if self.zoom > 0.5 {
            0.6 * 1.15
        } else if self.weapon == 5 && self.arb != Arb::Idle {
            0.65 * 1.3
        } else {
            1.0
        }
    }

    fn post_physics(&mut self) {
        let now = Instant::now();
        // The weapon wheel's slow motion only slows what the game simulates: our own movement
        // (walk, air, jumps, dash) runs on this clock, so it slows down with it (user).
        let slow = if self.slowed { self.cfg.wheel_slowmo.clamp(0.05, 1.0) } else { 1.0 };
        let dt = (now - self.post_last).as_secs_f32().min(0.05) * slow;
        self.post_last = now;
        // Only once the character has been in the world for a while (model pointers settle).
        if self.cfg.hide_body && self.in_world_for > 2.0 && !self.doom_off {
            game::hide_player_body(true);
        }
        let Some(p) = game::player() else { return };
        let ladder = game::on_ladder(&p.chr_ins);
        let dead = p.chr_ins.modules.data.hp <= 0;
        let model_y = p.chr_ins.chr_ctrl.model_matrix.3.1;
        let ph = &mut p.chr_ins.modules.physics;
        let pos = game::hpos(&ph.position);
        let mut delta = Vec3::ZERO;

        if self.doom_off {
            camera_eye(f32::NAN);
            camera_eye_xz(f32::NAN, f32::NAN);
            camera_step(0.0);
            return;
        }
        if dead {
            // Dying (e.g. off a cliff): the game's death camera and fall, nothing of ours - air
            // control kept steering the body and the view drifted.
            self.cam_filter = None;
            camera_step(0.0);
            camera_eye(f32::NAN);
            camera_eye_xz(f32::NAN, f32::NAN);
            self.air_vy = None;
            self.air_carry = Vec3::ZERO;
            self.air_push = Vec3::ZERO;
            self.dash_t = 0.0;
            self.last_safe = None;
            if self.gravity_off_by_us {
                ph.gravity_disabled = false;
                self.gravity_off_by_us = false;
            }
            return;
        }
        if self.on_ladder || ladder {
            if self.gravity_off_by_us {
                ph.gravity_disabled = false;
                self.gravity_off_by_us = false;
            }
            // The game's climb / interaction moves us. On a ladder the camera follows the
            // climbing head; in an interaction (fog wall, door, grace) it stays on the eye lock.
            self.cam_filter = None;
            camera_step(0.0);
            if ladder {
                camera_eye(f32::NAN);
                camera_eye_xz(f32::NAN, f32::NAN);
            }
            self.air_carry = Vec3::ZERO;
            self.ground_vel = Vec3::ZERO;
            self.last_safe = None;
            return;
        }
        // Lift detection: the floor under our feet is sampled each frame; when that map object's
        // height changes we're on a moving lift until 0.5 s after it stops (lift_until). While
        // that's set (and we're idle) the game's motion multiplier stays at 1 so the lift carries
        // us - at 0 (Doom movement) the lift's carry was lost: jitter going down, falling through
        // going up (user). Copying the lift's motion ourselves / stepping our movement out were
        // tried first and removed (MODLOG "Lift fix bookkeeping").
        {
            let floor = |at: Vec3| {
                let from = at + Vec3::Y * 0.6;
                let len = 2.0;
                let hit = raycast::cast_sphere(from, Vec3::NEG_Y * len, 0.25, WORLD_FILTER, |h| !raycast::is_chr_hit(h))?;
                if hit.normal.y < 0.97 || hit.segment < 0.01 {
                    return None;
                }
                let h = hit.field_ins_handle()?;
                let name = game::geom_by_handle(&h)?.name;
                Some(((h.block_id.0, h.selector.index()), from.y - hit.segment * len - 0.25, name))
            };
            if let Some((key, at, gy)) = self.lift_floor {
                if let Some((k2, gy2, name)) = floor(at) {
                    let dy = gy2 - gy;
                    if k2 == key && dy.abs() > 0.002 && dy.abs() < 1.0 {
                        if self.time >= self.lift_until {
                            log::info!("lift: riding {name} (floor moving {dy:+.3} m/frame)");
                        }
                        self.lift_until = self.time + 0.5;
                    }
                }
            }
            self.lift_floor = floor(pos).map(|(k, gy, _)| (k, pos, gy));
        }
        // Fell through the floor (position writes can push the capsule into geometry): if we're
        // dropping and there is a floor *above* us right where we were standing a moment ago,
        // put us back on it. Real falls off ledges have no floor overhead and are left alone.
        let t_now = self.time;
        if ph.is_touching_ground {
            self.last_safe = Some((pos, t_now));
        } else if let Some((safe, at)) = self.last_safe.filter(|(s, _)| Vec3::new(s.x - pos.x, 0.0, s.z - pos.z).length() < 10.0) {
            // (a stored spot more than 10 m away sideways is from before a teleport / tile
            // re-base, never a fall-through: ignored)
            let drop = safe.y - pos.y;
            let near = Vec3::new(safe.x - pos.x, 0.0, safe.z - pos.z).length() < 3.0;
            if drop > 1.5 && near && t_now - at < 4.0 {
                let up = raycast::cast_sphere(pos + Vec3::Y * 0.3, Vec3::Y * (drop + 1.0), 0.15, WORLD_FILTER, |h| !raycast::is_chr_hit(h));
                if up.is_some_and(|h| h.normal.y < -0.3) {
                    log::warn!("fell through the floor ({drop:.1} m) - restoring");
                    ph.position.0 = safe.x;
                    ph.position.1 = safe.y + 0.3;
                    ph.position.2 = safe.z;
                    ph.chr_proxy_pos_update_requested = true;
                    self.air_vy = None;
                    self.air_carry = Vec3::ZERO;
                    self.ground_vel = Vec3::ZERO;
                    self.last_safe = None;
                    return;
                }
            }
        }

        if self.dash_t > 0.0 {
            // Dashing into something breakable (crates, wall chunks, gravestones) smashes it and
            // carries on through (user); walls, pillars and rocks still stop the dash.
            {
                let ahead = self.cfg.dash_distance / self.cfg.dash_time * dt.min(self.dash_t) + 0.6;
                // ankle height too: rocks, skulls and pots lie low (the knee/chest checks passed
                // over them and the body just kicked them away - user)
                for h in [0.3f32, 0.75, 1.35] {
                    self.break_along(pos + Vec3::Y * h, self.dash_dir, ahead, 0.3);
                }
            }
            DASHING.store(true, std::sync::atomic::Ordering::Relaxed);
            let step_t = dt.min(self.dash_t);
            self.dash_t -= dt;
            if self.dash_t <= 0.0 && !ph.is_touching_ground {
                // An air dash replaces the momentum: you carry on along the dash, not the way
                // you jumped (that kept sliding you sideways after turning).
                self.air_carry = self.dash_dir * self.cfg.ground_speed;
                self.air_push = Vec3::ZERO;
            }
            let mv = collide_slide(pos, self.dash_dir * (self.cfg.dash_distance / self.cfg.dash_time) * step_t);
            DASHING.store(false, std::sync::atomic::Ordering::Relaxed);
            delta += mv;
            // Dashing on the ground follows it (uphill dashes used to push into the slope).
            if ph.is_touching_ground && self.air_vy.is_none() {
                if let Some(gy) = ground_height(pos + Vec3::new(mv.x, 0.0, mv.z), 0.9) {
                    delta.y += gy - pos.y;
                }
            }
        }

        self.jump_grace = (self.jump_grace - dt).max(0.0);
        // Right after a jump the game still reports ground contact: treat it as airborne.
        let on_ground = ph.is_touching_ground && self.jump_grace <= 0.0;
        if on_ground {
            self.air_push = Vec3::ZERO;
        }
        // A real landing (jumped, or dropped more than land_min_drop): only these get the landing
        // parts 1/2. Stepping down stairs leaves the ground for a moment too - with parts 1/2 on
        // every one of those the stairs went wrong (user).
        if !on_ground {
            self.air_peak_y = self.air_peak_y.max(pos.y);
        } else if !self.was_on_ground {
            let drop = self.air_peak_y - pos.y;
            // (falling: jumping up stairs touches a step on the way up - that's no landing)
            if (self.air_jumped || drop > self.cfg.land_min_drop) && self.air_vy_meas < -1.0 {
                self.real_land_at = self.time;
                if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
                    log::info!("landing: real (jumped {} drop {drop:.2} m, falling {:.1} m/s)", self.air_jumped, self.air_vy_meas);
                }
            } else if TRACE.load(std::sync::atomic::Ordering::Relaxed) {
                log::info!("landing: step (drop {drop:.2} m) - no landing handling");
            }
        }
        if on_ground {
            self.air_peak_y = pos.y;
            self.air_jumped = false;
        }
        // Doom ground movement: near-instant acceleration to a fixed speed, instant stop.
        if self.cfg.doom_move {
            let busy = self.dash_t > 0.0 || self.glory.is_some() || self.hook.is_some();
            if on_ground && !busy {
                if !self.was_on_ground {
                    // touchdown: the last bit down comes at the fall speed (see the ground follow)
                    if self.air_vy_meas < -1.0 && self.cfg.land_settle && self.real_land_at == self.time {
                        self.land_vy = Some(self.air_vy_meas.min(-2.0));
                    }
                    // Landing keeps the momentum from the jump.
                    self.ground_vel = Vec3::new(self.air_carry.x, 0.0, self.air_carry.z);
                    self.air_carry = Vec3::ZERO;
                }
                let wish = self.wish_dir();
                // Mod movement upgrades: Fast Gunner (turret), Mobility (scope), Full Speed (arbalest).
                let slow = if self.turret {
                    0.6
                } else if self.zoom > 0.5 {
                    0.6 * 1.15
                } else if self.weapon == 5 && self.arb != Arb::Idle {
                    0.65 * 1.3
                } else {
                    1.0
                };
                let target = wish * self.cfg.ground_speed * slow;
                let rate = if wish == Vec3::ZERO { self.cfg.ground_friction } else { self.cfg.ground_accel };
                let diff = target - self.ground_vel;
                let step = rate * dt;
                self.ground_vel = if diff.length() <= step { target } else { self.ground_vel + diff.normalize() * step };
                let mv = collide_slide(pos, self.ground_vel * dt);
                if dt > 0.0 {
                    // Walls eat the blocked part of the velocity (no sticking).
                    self.ground_vel = mv / dt;
                }
                delta += mv;
                // Follow the ground ourselves (direct position writes skip ER's ground follow, so
                // slopes swallowed the capsule): snap to the floor under the new spot, up or down
                // a step, and leave real drops to gravity.
                let mut climb = 0.0;
                if mv.length_squared() > 1e-8 {
                    if let Some(gy) = ground_height(pos + Vec3::new(mv.x, 0.0, mv.z), STEP_HEIGHT + 0.1) {
                        let mut dy = gy - pos.y;
                        // Just landed: the game says "on the ground" ~35 cm above it and this
                        // snapped the rest in one frame - a two-stage landing (user, camera log).
                        // Come down the last bit at the fall speed instead.
                        if let Some(v) = self.land_vy {
                            let cap = v * dt; // negative
                            if dy < cap {
                                dy = cap;
                            } else {
                                self.land_vy = None;
                            }
                        }
                        delta.y += dy;
                        climb = dy / dt.max(1e-3);
                    }
                }
                // Landed standing still: the game said "on the ground" ~30 cm above it and our
                // per-frame position write then held the body there while the model eased down -
                // the camera sank 25-30 cm after every landing (user, camera log). Bring the body
                // the rest of the way down at the fall speed, moving or not.
                if mv.length_squared() <= 1e-8 {
                    if let Some(v) = self.land_vy {
                        match ground_height(pos, STEP_HEIGHT + 0.1) {
                            Some(gy) if gy < pos.y - 0.005 => delta.y += (gy - pos.y).max(v * dt),
                            _ => self.land_vy = None,
                        }
                    }
                }
                // Smoothed vertical speed along the ground (carried into jumps).
                self.ground_vy += (climb.clamp(-12.0, 12.0) - self.ground_vy) * (dt * 12.0).min(1.0);
            } else if !on_ground && self.was_on_ground && self.air_carry == Vec3::ZERO {
                // Walked off a ledge: keep running speed in the air.
                self.air_carry = self.ground_vel;
            } else if busy {
                self.ground_vel = Vec3::ZERO;
            }
            if !on_ground && !busy {
                // Doom air control: the momentum turns to wherever you steer (camera-relative)
                // within a fraction of a second, keeping its speed; no keys = keep drifting.
                let wish = self.wish_dir();
                if wish != Vec3::ZERO {
                    let speed = Vec3::new(self.air_carry.x, 0.0, self.air_carry.z).length().max(self.cfg.ground_speed * self.move_slow());
                    let diff = wish * speed - self.air_carry;
                    let step = AIR_STEER * dt;
                    self.air_carry = if diff.length() <= step { wish * speed } else { self.air_carry + diff.normalize() * step };
                } else {
                    self.air_carry *= (1.0 - 0.3 * dt).max(0.0);
                }
                delta += collide_slide(pos, self.air_carry * dt);
            }
        }
        if on_ground && !self.was_on_ground && self.sway_air > 0.25 {
            // Landing: the gun dips and springs back, more after a longer fall.
            let k = (self.sway_air / 0.9).clamp(0.4, 1.0);
            self.sway_y[1] -= self.cfg.sway[3] * 12.0 * k;
            // (the walking bob starts again right away - user)
            self.bob_hold = 0.0;
        }
        self.sway_air = if on_ground { 0.0 } else { self.sway_air + dt };
        // landing bookkeeping: measured vertical speed in the air; at touchdown it becomes the
        // cap for the last bit down (falling at least 2 m/s so it never hangs)
        if !self.last_body_y.is_nan() && dt > 0.0 && !on_ground {
            self.air_vy_meas = (pos.y - self.last_body_y) / dt;
        }
        if !on_ground {
            self.land_vy = None;
        }
        self.last_body_y = pos.y;
        self.was_on_ground = on_ground;

        // Double jump: we integrate the rise ourselves (ballistic, Doom gravity) and hand the
        // fall back to the game at the apex so landing/fall damage/animations stay native.
        if let Some(vy) = self.air_vy {
            let nvy = vy - self.cfg.air_gravity * dt;
            delta += Vec3::Y * (vy + nvy) * 0.5 * dt;
            self.air_vy = (nvy > 0.0 && !on_ground).then_some(nvy);
            ph.gravity_disabled = true;
            self.gravity_off_by_us = true;
        } else if self.gravity_off_by_us {
            ph.gravity_disabled = false;
            self.gravity_off_by_us = false;
        }


        if self.cfg.glory_teleport && self.glory.as_ref().is_some_and(|g| !g.placed) {
            let to = self.glory.as_ref().map(|g| g.to).unwrap_or(pos);
            ph.position.0 = to.x;
            ph.position.1 = to.y + 0.05;
            ph.position.2 = to.z;
            ph.chr_proxy_pos_update_requested = true;
            self.air_vy = None;
            // a deliberate move: the old standing spot is no reference for the floor rescues
            self.last_safe = None;
            if let Some(g) = self.glory.as_mut() {
                g.placed = true;
            }
            delta = Vec3::ZERO;
        } else if self.cfg.glory_teleport && self.glory.is_some() {
            delta = Vec3::ZERO;
        } else if let Some((spot, t0)) = self.glory_hold {
            if self.time < t0 + self.cfg.glory_hold {
                // held at the demon for the finisher (sideways only; the ground keeps its own height)
                delta = Vec3::new(spot.x - pos.x, 0.0, spot.z - pos.z);
            } else {
                self.glory_hold = None;
            }
        } else if let Some(g) = &self.glory {
            // (lunge mode, glory_teleport = false) Lunge along the ground, not in a straight line: on a slope the straight line went
            // into the hill and dropped you through the world. Walls stop it.
            let a = (g.t / GLORY_LUNGE_TIME).clamp(0.0, 1.0);
            let want = g.from.lerp(g.to, a);
            // Breakables in the way are smashed and lunged through while they break, like a dash
            // (church walls still stop you) - user.
            let (gfrom, gto, gt) = (g.from, g.to, g.t);
            if gt < GLORY_LUNGE_TIME {
                let flat = Vec3::new(gto.x - gfrom.x, 0.0, gto.z - gfrom.z);
                let left = Vec3::new(gto.x - pos.x, 0.0, gto.z - pos.z).length() + 0.6;
                for h in [0.3f32, 0.75, 1.35] {
                    self.break_along(pos + Vec3::Y * h, flat, left, 0.3);
                }
            }
            DASHING.store(true, std::sync::atomic::Ordering::Relaxed);
            let mv = collide_slide(pos, Vec3::new(want.x - pos.x, 0.0, want.z - pos.z));
            DASHING.store(false, std::sync::atomic::Ordering::Relaxed);
            let next = pos + Vec3::new(mv.x, 0.0, mv.z);
            // In the air the lunge pulls you down to the ground as well (user): the floor up to
            // 30 m below, reached by the end of the lunge. On the ground: the usual step follow.
            let y = match ground_height(next, 1.5) {
                Some(gy) => gy,
                None => raycast::cast_sphere(next + Vec3::Y * 0.5, Vec3::NEG_Y * 30.0, 0.25, WORLD_FILTER, |h| !raycast::is_chr_hit(h))
                    .filter(|h| h.normal.y > 0.35 && h.segment > 0.01)
                    .map(|h| {
                        let gy = next.y + 0.5 - h.segment * 30.0 - 0.25;
                        let left = ((GLORY_LUNGE_TIME - gt) / dt.max(1e-3)).max(1.0);
                        pos.y + (gy - pos.y) / left
                    })
                    .unwrap_or(pos.y),
            };
            self.air_vy = None;
            delta = Vec3::new(mv.x, y - pos.y, mv.z);
        }

        if self.hook.is_none() && self.hook_miss_until > 0.0 && self.time >= self.hook_miss_until {
            self.hook_miss_until = 0.0;
            if let Ok(mut g) = crate::fx::HOOK.lock() {
                *g = None;
            }
        }
        if let Some(h) = self.hook.as_mut() {
            h.t += dt;
            let enemies = game::enemies(self.cfg.meathook_range + 5.0);
            match enemies.iter().find(|e| &*e.chr as *const _ as usize == h.target) {
                Some(e) if h.t < h.dur + 0.3 && e.hp() > 0 => {
                    if let Ok(mut g) = crate::fx::HOOK.lock() {
                        if let Some(h) = g.as_mut() {
                            h.0 = e.pos() + Vec3::Y * hook_grip(e.chr);
                        }
                    }
                    // Pull toward where the demon is *now* and stop just in front of its body
                    // (never past it: the old lerp to a precomputed point overshot moving or big
                    // enemies and dropped the Slayer behind them).
                    let tgt = e.pos();
                    let to = Vec3::new(tgt.x - pos.x, 0.0, tgt.z - pos.z);
                    let stop = e.chr.modules.physics.hit_radius.max(0.4) + 0.9;
                    let remaining = to.length() - stop;
                    if remaining <= 0.05 {
                        h.t = h.dur + 1.0; // arrived
                        delta = Vec3::ZERO;
                    } else {
                        let speed = self.cfg.meathook_speed;
                        let step = (speed * dt).min(remaining);
                        let mut mv = to.normalize_or_zero() * step;
                        // Little hop over the ground (Doom's arc), and match the target's height.
                        let a = (h.t / h.dur).clamp(0.0, 1.0);
                        let lift = (a * std::f32::consts::PI).cos() * 1.2 * std::f32::consts::PI / h.dur * dt;
                        // Big bodies (trolls): fly up to their chest, not their feet.
                        let lift_to = tgt.y + (hook_grip(e.chr) - 1.3).max(0.0);
                        let dy = ((lift_to - pos.y) * (dt * 6.0).min(1.0)).clamp(-speed * dt, speed * dt);
                        mv.y = lift + dy;
                        delta = collide_slide(pos, mv);
                        // The side sweeps don't see the floor: a pull down toward a lower demon
                        // went through the terrain (user ended up under the map). Never below the
                        // floor under the new spot.
                        if delta.y < 0.0 {
                            let from = pos + Vec3::new(delta.x, 0.6, delta.z);
                            let len = 0.65 - delta.y;
                            if let Some(g) = raycast::cast_sphere(from, Vec3::NEG_Y * len, 0.3, WORLD_FILTER, |h| !raycast::is_chr_hit(h)) {
                                if g.normal.y > 0.2 && g.segment > 0.01 {
                                    let gy = from.y - g.segment * len - 0.3;
                                    if pos.y + delta.y < gy {
                                        delta.y = gy - pos.y;
                                    }
                                }
                            }
                        }
                        h.vel = delta / dt.max(1e-3);
                    }
                    ph.gravity_disabled = true;
                    self.gravity_off_by_us = true;
                }
                _ => {
                    self.hook_end_at = self.time;
                    // Killed (or lost) mid-pull: keep flying the way the hook was pulling (user) -
                    // a little lift so it is a short airborne carry, then normal air control.
                    let (vel, cut) = (h.vel, h.t < h.dur + 0.3);
                    if cut && vel.length_squared() > 1.0 {
                        let flat = Vec3::new(vel.x, 0.0, vel.z);
                        self.air_carry = flat.normalize_or_zero() * flat.length().min(self.cfg.meathook_speed * 0.6);
                        self.air_vy = Some(vel.y.clamp(2.0, 6.0));
                        self.jump_grace = 0.15;
                        log::info!("meathook target lost mid-pull: carrying {:.1} m/s", self.air_carry.length());
                    }
                    self.hook = None;
                    if self.weapon == SUPER_SHOTGUN && (self.vm_clip.starts_with("hook_") || self.vm_clip == "idle") {
                        self.vm_play("hook_retract", false);
                    }
                    if let Ok(mut g) = crate::fx::HOOK.lock() {
                        *g = None;
                    }
                }
            }
        }

        if TRACE.load(std::sync::atomic::Ordering::Relaxed) || WALKTO.lock().is_ok_and(|w| w.is_some_and(|w| Instant::now() < w.2)) {
            let cam_y = game::camera().map_or(f32::NAN, |c| c.0.y);
            log::info!(
                "trace {:.3} pos {:.3} {:.3} {:.3} dy {:.3} ground {} cam {:.3} vel {:.2} head {:.3} model {:.3} hxyz {:.3}",
                self.time, pos.x, pos.y, pos.z, delta.y, ph.is_touching_ground as u8, cam_y, self.ground_vel.length(),
                camera_head().map_or(f32::NAN, |h| h.y), model_y, camera_head().unwrap_or(Vec3::NAN)
            );
        }
        let grounded = on_ground && self.air_vy.is_none() && self.glory.is_none() && self.hook.is_none();
        // The camera this frame is built from the model at `pos` (our move below only shows up
        // next frame), so filter and offset against `pos` - using the post-move height put every
        // stair step into the camera twice (a dip, then a jump).
        // Filter the model's height: erfps2 builds the camera from the model matrix, which is
        // already at this frame's position here (our move below shows next frame).
        self.smooth_camera(model_y, dt, grounded);
        // Eye height: learn the standing head height while idle on the ground, then hold the
        // camera there (the head bone bobbed with stairs, landings and knockdowns).
        // Learn the standing head position (only while idle and upright, not sitting/kneeling).
        if let Some(h) = camera_head() {
            let upright = (h.y - DEFAULT_EYE.y).abs() < 0.25;
            let settled = self.eye_learn.is_some_and(|(_, n)| n >= 120.0);
            if grounded && upright && !settled && self.ground_vel.length() < 0.1 && self.dash_t <= 0.0 {
                self.eye_learn = Some(match self.eye_learn {
                    Some((avg, n)) => (avg + (h - avg) / (n + 1.0).min(120.0), n + 1.0),
                    None => (h, 1.0),
                });
            }
        }
        // The camera sits at that standing eye position on the body - height and forward/side -
        // from the first frame (measured default until learned), so getting up from a grace,
        // knockdowns and landings don't move the view.
        let mut eye = self.eye_learn.filter(|(_, n)| *n >= 30.0).map_or(DEFAULT_EYE, |(avg, _)| avg);
        if self.cfg.eye_height > 0.0 {
            eye.y = self.cfg.eye_height;
        }
        camera_eye(eye.y);
        camera_eye_xz(eye.x, eye.z);
        if delta.length_squared() < 1e-8 {
            return;
        }

        // Horizontal moves were already slid along walls (collide_slide). A second, wider chest
        // cast here used to stop the whole move on any touch - door frames froze you in place -
        // so it now only guards the glory lunge (unclipped) and rises into ceilings.
        let chest = pos + Vec3::Y * 1.0;
        let world = |h: &raycast::hknpHit| !crate::raycast::is_chr_hit(h);
        if self.glory.is_some() {
            if let Some(hit) = raycast::cast_sphere(chest, delta, 0.3, WORLD_FILTER, world) {
                delta *= (hit.segment - 0.05).clamp(0.0, 1.0);
            }
        } else if delta.y > 0.0 && (self.air_vy.is_some() || !ph.is_touching_ground) {
            let rise = Vec3::Y * delta.y;
            if let Some(hit) = raycast::cast_sphere(chest + Vec3::Y * 0.5, rise, 0.22, WORLD_FILTER, world) {
                if hit.normal.y < -0.3 {
                    // Bonked a ceiling: stop the rise and the push.
                    delta.y *= (hit.segment - 0.05).clamp(0.0, 1.0);
                    self.air_vy = None;
                    self.air_push = Vec3::ZERO;
                }
            }
        }

        ph.position.0 += delta.x;
        ph.position.1 += delta.y;
        ph.position.2 += delta.z;
        ph.chr_proxy_pos_update_requested = true;

        log::debug!("move {delta}");
    }
}

/// World position of the viewmodel's barrel tip (along the screen ray through it, ~1 m out).
fn muzzle_world() -> Option<Vec3> {
    let [nx, ny, aspect] = (*crate::viewmodel::MUZZLE_NDC.lock().ok()?)?;
    let c = game::camera_full()?;
    let t = (c.fov * 0.5).tan();
    let dir = (c.fwd + c.right * nx * t * aspect + c.up * ny * t).normalize_or_zero();
    Some(c.pos + dir * 1.0)
}

/// Floor height under `at` within `step` metres up or down (map collision only).
///
/// The height is where a 0.3 m ball dropped on the spot comes to rest (its centre minus the
/// radius), not the contact point: on flat ground that's the floor, and over a step edge the ball
/// rolls, so stairs and lips become a smooth ramp instead of tread-to-tread snaps (the stair
/// jitter), and an edge contact is never rejected as "not walkable" (snagging on small lips).
pub fn ground_height(at: Vec3, step: f32) -> Option<f32> {
    const R: f32 = 0.3;
    let from = at + Vec3::Y * (step + R);
    let len = 2.0 * step;
    let hit = raycast::cast_sphere(from, Vec3::NEG_Y * len, R, 0x2000058, |h| {
        !raycast::is_chr_hit(h)
            && h.field_ins_handle().is_none_or(|f| {
                !matches!(f.selector.field_ins_type(), Some(eldenring::cs::FieldInsType::Bullet))
            })
    })?;
    // Started inside something (a wall beside us): no answer rather than a false step up.
    if hit.segment < 0.01 {
        return None;
    }
    // Walls brushed on the way down have flat normals; floors, slopes and edges point up.
    if hit.normal.y <= 0.2 {
        return None;
    }
    let gy = from.y - hit.segment * len - R;
    // A step up must land on something you can stand on: just past the contact (away from the
    // ball's centre) a stair or sill has a flat top, a rough rock wall doesn't - without this the
    // ball found bumps on cliffs and walls and you walked straight up them.
    if gy > at.y + 0.05 {
        let contact = Vec3::from(hit.pos);
        let centre = Vec3::new(at.x, gy + R, at.z);
        let out = Vec3::new(contact.x - centre.x, 0.0, contact.z - centre.z).normalize_or_zero() * 0.06;
        let top = raycast::cast_sphere(contact + out + Vec3::Y * 0.25, Vec3::NEG_Y * 0.5, 0.03, 0x2000058, |h| !raycast::is_chr_hit(h));
        if !top.is_some_and(|t| t.normal.y > 0.7) {
            return None;
        }
    }
    Some(gy)
}

#[derive(Clone, Debug)]
pub struct EnemyBar {
    pub pos: Vec3,
    pub frac: f32,
    pub dist: f32,
    pub boss: bool,
    pub staggered: bool,
    /// Visible (line of sight); a boss off-screen still gets the top bar.
    pub on_screen: bool,
}

/// Clip a horizontal move against walls/characters (direct position writes skip ER collision):
/// sphere casts at knee and chest height, sliding along whatever is hit.
/// A dash is moving us this frame: things mid-break (state 4) don't stop it - you smash through
/// what's breaking, but what's still standing afterwards blocks again (church walls, pillars and
/// stone walls only lose a chunk; a 0.6 s pass-through let the dash go through the rest - user).
static DASHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn dashed_through(f: &eldenring::cs::FieldInsHandle) -> bool {
    DASHING.load(std::sync::atomic::Ordering::Relaxed)
        // not partial-static assets (behavior 2: church walls lose a chunk and stay up - dashing
        // through them left you stuck inside, user)
        && game::geom_by_handle(f).is_some_and(|g| g.behavior != 2 && game::geom_state(g.ptr) == 4)
}

fn collide_slide(pos: Vec3, mv: Vec3) -> Vec3 {
    let me = game::player().map(|p| p.chr_ins.field_ins_handle);
    let solid_plants = crate::config::get_cached().solid_plants.clone();
    let solid = |h: &raycast::hknpHit| match h.field_ins_handle() {
        None => true,
        Some(f) if Some(f) == me => false,
        Some(f) => match f.selector.field_ins_type() {
            Some(eldenring::cs::FieldInsType::Chr | eldenring::cs::FieldInsType::Map
                | eldenring::cs::FieldInsType::Geom | eldenring::cs::FieldInsType::Hit
                | eldenring::cs::FieldInsType::HitGeom | eldenring::cs::FieldInsType::Obj) => true,
            // Fog walls (boss fog, Stonesword Key seals - AEG099_272) and other system barriers
            // are AEG099 assets of field type 6: ER's own character can't pass them, so neither
            // can we (user walked through a Stonesword fog). Opened fogs leave the world.
            // Placed assets of field type 6: gravestones, church walls, fences, gates, pillars,
            // crates, furniture (AEG217/219/801/810/700/004 ... - user walked through them all).
            // Solid while standing; broken debris, things that shatter on contact (pots) and plants
            // (AEG7xx flowers, AEG8xx bushes / small trees, AEG001 small plants) aren't.
            // something the dash just smashed: carry on through it while it breaks
            Some(eldenring::cs::FieldInsType::ReplayEnemy) if dashed_through(&f) => false,
            Some(eldenring::cs::FieldInsType::ReplayEnemy) => game::geom_by_hit(h).is_some_and(|g| {
                if !damage::is_prop(&g.name) {
                    // (which plant models we pass through, once each: big trees go in solid_plants)
                    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
                    if let Ok(mut seen) = SEEN.lock() {
                        let fam = g.name.get(..10).unwrap_or(&g.name).to_string();
                        if seen.get_or_insert_with(Default::default).insert(fam) {
                            log::info!("walk-through plant {}", g.name);
                        }
                    }
                }
                let plant_ok = damage::is_prop(&g.name) || solid_plants.iter().any(|p| g.name.starts_with(p.as_str()));
                if plant_ok && !g.standing() && !g.break_by_player {
                    // (debug: walked through because of its state - lift cage sides, user)
                    static SEEN_S: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
                    if let Ok(mut seen) = SEEN_S.lock() {
                        if seen.get_or_insert_with(Default::default).insert(g.name.clone()) {
                            log::info!("walk-through by state: {} state {} behavior {} hp {}", g.name, game::geom_state(g.ptr), g.behavior, g.hp);
                        }
                    }
                }
                (!g.loose && g.name.starts_with("AEG099")) || (g.standing() && !g.break_by_player && !g.loose && plant_ok)
            }),
            other => {
                // (debug, walk-through report): other non-solid types, once per type
                static SEEN_T: std::sync::Mutex<Option<std::collections::HashSet<String>>> = std::sync::Mutex::new(None);
                let t = format!("{other:?}");
                if let Ok(mut seen) = SEEN_T.lock() {
                    if seen.get_or_insert_with(Default::default).insert(t.clone()) {
                        let name = game::geom_by_handle(&f).map(|g| g.name);
                        log::info!("walk-through type {t} {name:?} filter {:#x}", h.filter);
                    }
                }
                false
            }
        },
    };
    // Body half-width = R + SKIN = 0.3 m (was 0.4): fits doorways like ER's own character.
    const R: f32 = 0.22;
    const SKIN: f32 = 0.08;
    let mut flat = Vec3::new(mv.x, 0.0, mv.z);
    for _ in 0..2 {
        let len = flat.length();
        if len < 1e-5 {
            return Vec3::new(0.0, mv.y, 0.0);
        }
        let dir = flat / len;
        let mut blocked: Option<(f32, Vec3)> = None;
        for h in [0.75f32, 1.35] {
            let reach = len + SKIN;
            if let Some(hit) = raycast::cast_sphere(pos + Vec3::Y * h, dir * reach, R, 0x2000058, solid) {
                let d = hit.segment * reach;
                let n = Vec3::new(hit.normal.x, 0.0, hit.normal.z).normalize_or_zero();
                // Ignore walkable floors/slopes (up to ~53 deg) and anything low enough to step
                // onto (stair risers, door sills, lips): the ground follow climbs those. Steeper
                // faces block - big rounded rocks used to count as "floor" (anything over 0.35) and
                // a dash went straight into one and out under the map (user); 0.7 then blocked a
                // ~50 deg path you're meant to walk up (normal.y 0.64-0.69, user).
                if hit.normal.y > 0.6 || hit.pos.y - pos.y < STEP_HEIGHT {
                    continue;
                }
                // A wall we're brushing alongside (normal across the move) doesn't block it: it
                // used to stop you dead hugging corridor walls and door frames.
                if n.dot(dir) > -0.1 {
                    continue;
                }
                if blocked.is_none_or(|(bd, _)| d < bd) {
                    blocked = Some((d, n));
                }
                {
                    static LAST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
                    let mut l = LAST.lock().unwrap_or_else(|e| e.into_inner());
                    if l.is_none_or(|t| t.elapsed().as_secs_f32() > 1.0) {
                        *l = Some(Instant::now());
                        log::info!(
                            "move blocked at h {h}: d {d:.2} normal {:?} by {:?}",
                            hit.normal,
                            hit.field_ins_handle().map(|f| f.selector.field_ins_type())
                        );
                    }
                }
            }
        }
        match blocked {
            None => return Vec3::new(flat.x, mv.y, flat.z),
            Some((d, _)) => {
                // Stop at the wall - no sliding along it (user: walls you walk into hold you).
                // Walls running alongside the move were skipped above, so hallways stay free.
                let ahead = dir * (d - SKIN).max(0.0).min(len);
                return Vec3::new(ahead.x, mv.y, ahead.z);
            }
        }
    }
    Vec3::new(0.0, mv.y, 0.0)
}

/// Anything that isn't the player, a co-op/summon ally or a neutral/friendly NPC.
/// (Observed: 6/7 regular & strong enemies, 48 Godrick soldiers at Stormgate.)
/// ER team types that never fight the player offline (player, co-op, spirit ashes, friendly NPCs).
/// Red phantoms (NPC invaders and red spectral enemies) are 3 "black phantom" / 13 "intruder":
/// hostile. Anything the player has hit directly counts as hostile from then on (red phantoms on
/// team 0, provoked NPCs). Each team seen is logged once so new ones can be checked.
pub fn is_hostile(e: &Enemy) -> bool {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEEN: [AtomicU64; 4] = [const { AtomicU64::new(0) }; 4];
    let t = e.chr.team_type as usize;
    // 11 is not friendly: the open-world dragon Agheel (npc 45000010) uses it.
    let hostile = !matches!(t, 0 | 1 | 2 | 4 | 5 | 8 | 10 | 14) || damage::provoked(e.chr);
    let (word, bit) = (t / 64, 1u64 << (t % 64));
    if SEEN[word].fetch_or(bit, Ordering::Relaxed) & bit == 0 {
        log::info!("team {t} first seen (npc {}, hp {}): hostile {hostile}", e.chr.npc_param_id, e.chr.modules.data.max_hp);
    }
    hostile
}

/// Doom: fodder 1 pip, heavies more. Scaled by the HP multiplier so tiers stay stable.
fn chainsaw_cost(_max_hp: i32, _mult: f32) -> f32 {
    // One fuel for any demon the chainsaw may take (boss bar / the HP limit in the settings decide
    // that). An old 4000 base-HP cap here asked "inf" fuel for a bear under the limit (user).
    1.0
}

/// Test bridge (lab mode): the driven, unfocused-mouse session shows the cursor - ignore it.
pub static IGNORE_CURSOR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn play_time() -> u32 {
    unsafe { <eldenring::cs::GameDataMan as fromsoftware_shared::FromStatic>::instance() }
        .map(|g| g.play_time)
        .unwrap_or(0)
}

/// The loaded character's name (PlayerGameData.character_name, private in the bindings: +0x9C).
pub fn character_name() -> Option<String> {
    let p = game::player()?;
    let base = unsafe { p.player_game_data.as_ref() } as *const _ as *const u8;
    let raw = unsafe { std::slice::from_raw_parts(base.add(0x9C) as *const u16, 17) };
    let end = raw.iter().position(|&c| c == 0).unwrap_or(17);
    let name = String::from_utf16(&raw[..end]).ok()?;
    (!name.is_empty() && name.chars().all(|c| !c.is_control())).then_some(name)
}

static SLAYER: Mutex<Option<Slayer>> = Mutex::new(None);

pub fn with<R>(f: impl FnOnce(&mut Slayer) -> R) -> R {
    let mut g = SLAYER.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Slayer::new))
}

pub fn frame() {
    with(|s| s.frame());
}

pub fn post_physics() {
    with(|s| s.post_physics());
}

#[allow(dead_code)]
pub const CHAINSAW_MAX: f32 = CHAINSAW_PIPS;

pub fn current_anim(chr: &eldenring::cs::ChrIns) -> i32 {
    let ta = &chr.modules.time_act;
    let i = (ta.read_idx as usize) % ta.anim_queue.len();
    ta.anim_queue[i].anim_id
}
