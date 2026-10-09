//! Doom-style damage that does not depend on the Elden Ring character's stats or equipped weapon
//! (ER scales every player-owned bullet by those - a level 5 unarmed Tarnished made Doom guns weak,
//! and some enemies' defenses rounded the hits down to almost nothing).
//!
//! Each pellet is a hitscan sphere cast from the camera; the first character hit takes the weapon's
//! fixed damage. Explosives detonate where the ray ends (after the projectile's travel time) and
//! damage every hostile in the radius with falloff. The ER bullet is still spawned for visuals, hit
//! reactions and aggro; its own damage is cut to a token amount (see params.rs).

use eldenring::cs::{FieldInsHandle, FieldInsType};
use glam::Vec3;

use crate::{game, raycast, slayer::is_hostile};

/// Collision filter that sees both the map and characters (same one erfps2 uses for lock-on).
const FILTER: u32 = 0x2000058;

/// Doom damage per weapon slot: (per pellet, blast radius m, blast damage).
/// Tuned against ER enemies with HP x2: SSG point blank deletes fodder, the Heavy Cannon and
/// Chaingun shred, rockets clear groups, the BFG wipes a room.
/// BFG 9000 blast radius (m): damage, ER's blast hitbox and the green explosion all use it.
/// 9 -> 14 (user: bigger BFG explosion).
pub const BFG_RADIUS: f32 = 14.0;

pub const TABLE: [(f32, f32, f32); 8] = [
    (40.0, 0.0, 0.0),     // Combat Shotgun: 8 pellets (tried 30: too weak - user)
    (60.0, 0.0, 0.0),     // Heavy Cannon
    (30.0, 0.0, 0.0),     // Plasma Rifle
    (150.0, 3.5, 450.0),  // Rocket Launcher
    (38.0, 0.0, 0.0),     // Super Shotgun: 16 pellets (tried 33.25: too weak - user)
    (700.0, 0.0, 0.0),    // Ballista
    (32.0, 0.0, 0.0),     // Chaingun
    (250.0, BFG_RADIUS, 2500.0), // BFG 9000 (halved: user found it too strong)
];

pub const MELEE: f32 = 60.0;
pub const BLOOD_PUNCH: f32 = 900.0;

pub struct Hit {
    pub pos: Vec3,
    pub chr: Option<FieldInsHandle>,
    pub dist: f32,
    /// World object (map asset) that was hit, when it wasn't a character.
    pub geom: Option<FieldInsHandle>,
}

/// First thing along `dir` within `range`, ignoring the player's own capsule.
pub fn trace(origin: Vec3, dir: Vec3, range: f32, radius: f32) -> Hit {
    let me = game::player().map(|p| p.chr_ins.field_ins_handle);
    let v = dir.normalize_or_zero() * range;
    // Skip our own capsule, bullets (ours spawn right in front of the eye) and replay ghosts /
    // bloodstain phantoms, which have bodies near the player and swallowed wide shotgun pellets.
    // Breakable assets (crates, barrels, furniture) answer ray casts as type 6 "ReplayEnemy":
    // those stop the shot (and get broken); plants / bushes of that type stay see-through.
    let solid = |h: &raycast::hknpHit| match h.field_ins_handle() {
        None => true,
        Some(f) if Some(f) == me => false,
        Some(f) => match f.selector.field_ins_type() {
            Some(FieldInsType::Bullet | FieldInsType::ReplayGhost) => false,
            // standing or mid-break (a wall losing a chunk stays up), breakable or not (pillars);
            // only breakable ones get broken (Slayer::break_geom checks hp)
            Some(FieldInsType::ReplayEnemy) => game::geom_by_hit(h).is_some_and(|g| (is_prop(&g.name) || g.loose) && (g.standing() || g.loose)),
            _ => true,
        },
    };
    match raycast::cast_sphere(origin, v, radius, FILTER, solid) {
        Some(h) => {
            let chr = if raycast::is_chr_hit(&h) { h.field_ins_handle() } else { None };
            let geom = h.field_ins_handle().filter(|f| {
                chr.is_none() && matches!(f.selector.field_ins_type(), Some(FieldInsType::Geom | FieldInsType::HitGeom | FieldInsType::ReplayEnemy))
            });
            Hit { pos: Vec3::from(h.pos), chr, dist: h.segment * range, geom }
        }
        None => Hit { pos: origin + v, chr: None, dist: range, geom: None },
    }
}

/// Characters the player has hit directly (ChrIns pointers): hostile from then on.
static PROVOKED: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

pub fn provoked(chr: &eldenring::cs::ChrIns) -> bool {
    let p = chr as *const _ as usize;
    PROVOKED.lock().is_ok_and(|v| v.contains(&p))
}

/// Props (solid, shootable): not plants (AEG8xx bushes / small trees, AEG001 small plants, AEG7xx
/// flowers), which would soak up shots and spend the game's limited bullet slots on grass.
/// AEG700 is NOT a plant: it's stone walls / ruins - the walls at the church and a gate the user
/// walked through were all AEG700 (an old note had all of AEG7 as flowers).
pub fn is_prop(name: &str) -> bool {
    name.contains("AEG700") || !["AEG7", "AEG8", "AEG001"].iter().any(|p| name.contains(p))
}

/// Damage a character the player hit directly. Like ER's own weapons this hurts anyone but the
/// player, co-op phantoms / spirit ashes (team 2) and allies (team 8); whoever is hit counts as
/// hostile afterwards. Returns Some(killed) when it was a valid target.
pub fn apply_handle(handle: FieldInsHandle, amount: f32) -> Option<bool> {
    let Some(e) = game::enemies(400.0).into_iter().find(|e| e.chr.field_ins_handle == handle) else {
        // Not in the scan: ask the game for it directly (its own handle -> ChrSet lookup) and
        // say what it is, then hurt it anyway if it's a foe.
        let w = game::world()?;
        let Some(chr) = w.chr_ins_by_handle_mut(&handle) else {
            log_no_damage(handle, None, "not in the character scan, not in chr_sets[container]");
            return None;
        };
        let d = &chr.modules.data;
        log::info!(
            "hit outside the scan: npc {} {:?} team {} hp {}/{} (applying damage)",
            chr.npc_param_id,
            game::npc_name(chr.npc_param_id),
            chr.team_type,
            d.hp,
            d.max_hp
        );
        if matches!(chr.team_type, 1 | 2 | 8) {
            return None;
        }
        return Some(apply(chr, amount));
    };
    if matches!(e.chr.team_type, 1 | 2 | 8) {
        log_no_damage(handle, Some(&e), "friendly team");
        return None;
    }
    if !is_hostile(&e) {
        log::info!("provoked npc {} team {}", e.chr.npc_param_id, e.chr.team_type);
        if let Ok(mut v) = PROVOKED.lock() {
            v.push(e.chr as *const _ as usize);
            if v.len() > 256 {
                v.remove(0);
            }
        }
    }
    Some(apply(e.chr, amount))
}

/// A shot hit a character but dealt no damage: say who and why (once a second per character),
/// so "this enemy doesn't take damage" reports can be traced from the log.
fn log_no_damage(handle: FieldInsHandle, e: Option<&game::Enemy>, why: &str) {
    static LAST: std::sync::Mutex<Option<std::collections::HashMap<u64, std::time::Instant>>> = std::sync::Mutex::new(None);
    let key = unsafe { std::mem::transmute::<FieldInsHandle, u64>(handle) };
    let mut g = LAST.lock().unwrap_or_else(|p| p.into_inner());
    let map = g.get_or_insert_with(Default::default);
    if map.get(&key).is_some_and(|t| t.elapsed().as_secs_f32() < 1.0) {
        return;
    }
    map.insert(key, std::time::Instant::now());
    match e {
        Some(e) => log::info!(
            "no damage ({why}): npc {} {:?} team {} hp {}/{}",
            e.chr.npc_param_id,
            game::npc_name(e.chr.npc_param_id),
            e.chr.team_type,
            e.hp(),
            e.max_hp()
        ),
        None => log::info!("no damage ({why}): handle {:?} type {:?}", handle, handle.selector.field_ins_type()),
    }
}

/// Enemies just caught by the Doom-style stagger: (ChrIns ptr) -> (floor HP, when). Protected
/// for CATCH_GRACE so the rest of the same shot can't kill them.
static CAUGHT: std::sync::Mutex<Vec<(usize, i32, std::time::Instant)>> = std::sync::Mutex::new(Vec::new());
const CATCH_GRACE: f32 = 0.25;

fn catch(key: usize, floor: i32) {
    let mut g = CAUGHT.lock().unwrap_or_else(|e| e.into_inner());
    g.retain(|(k, _, t)| *k != key && t.elapsed().as_secs_f32() < CATCH_GRACE);
    g.push((key, floor, std::time::Instant::now()));
}

fn caught_floor(key: usize) -> Option<i32> {
    let g = CAUGHT.lock().unwrap_or_else(|e| e.into_inner());
    g.iter().find(|(k, _, t)| *k == key && t.elapsed().as_secs_f32() < CATCH_GRACE).map(|(_, f, _)| *f)
}

/// Kills dealt through this module since the slayer last drained them: (ChrIns ptr, chest, max hp).
pub static KILLS: std::sync::Mutex<Vec<(usize, Vec3, i32)>> = std::sync::Mutex::new(Vec::new());

/// Apply raw damage. Returns true if this hit killed it.
/// Set while BFG damage is applied: no stagger catch - what it can kill dies (user).
pub static NO_CATCH: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn apply(chr: &mut eldenring::cs::ChrIns, amount: f32) -> bool {
    let no_catch = NO_CATCH.load(std::sync::atomic::Ordering::Relaxed);
    let d = &mut chr.modules.data;
    if d.hp <= 0 {
        return false;
    }
    let before = d.hp;
    d.hp = (d.hp - amount.round().max(1.0) as i32).max(0);
    // A caught enemy (below) is protected for the rest of that shot: the other shotgun pellets,
    // the rocket's blast after its direct hit, the ballista's second hit. The next shot kills.
    let key = chr as *const _ as usize;
    if let Some(floor) = caught_floor(key).filter(|_| !no_catch) {
        let d = &mut chr.modules.data;
        if d.hp < floor {
            d.hp = floor.min(before);
        }
    }
    let d = &mut chr.modules.data;
    // Doom-style stagger: a shot that takes a normal enemy or mini boss from above the glory-kill
    // window into it (or straight to dead) leaves it staggered in the window, protected for the
    // rest of that shot (shotgun pellets, rocket blast, ballista second hit): a glory kill is
    // always on offer and the next shot kills. Not for bosses with a boss bar, boss-class HP or
    // harmless wildlife.
    let after = chr.modules.data.hp;
    let max = chr.modules.data.max_hp.max(1) as f32;
    let cfg = crate::config::get_cached();
    let window = max * cfg.stagger_hp_frac;
    if !no_catch && before as f32 > window && (after as f32) <= window {
        let base_hp = max / crate::params::applied_hp_mult();
        // (boss = boss bar only: the 4000 base-HP cap kept big field enemies like bears out of
        // glory kills - user)
        let _ = base_hp;
        let eligible = cfg.glory_kills
            && !crate::slayer::AMBIENT_WILDLIFE.contains(&chr.character_id)
            && !game::active_boss_handles().contains(&chr.field_ins_handle);
        if eligible {
            let floor = if after > 0 { after } else { ((window * 0.5) as i32).max(1) };
            chr.modules.data.hp = floor;
            catch(key, floor);
            log::info!("stagger catch: npc {} {} -> {} (glory window)", chr.npc_param_id, before, floor);
        }
    }
    let d = &mut chr.modules.data;
    let hp = d.hp;
    // Red phantoms / NPC invaders: write their PlayerGameData HP too, or the game restores it.
    if let Some(p) = game::player_type_of(chr) {
        let pgd = unsafe { p.player_game_data.as_mut() };
        static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            log::info!("damage to player-type chr npc {} team {}: game data hp {} -> {hp}", chr.npc_param_id, chr.team_type, pgd.current_hp);
        }
        pgd.current_hp = hp.max(0) as u32;
    }
    let d = &mut chr.modules.data;
    let killed = d.hp == 0;
    if killed {
        log::info!("kill: npc {} hp {} -> 0 (hit {:.0})", chr.npc_param_id, before, amount);
        let max = d.max_hp;
        let at = game::chr_pos(chr) + Vec3::Y * 1.2;
        KILLS.lock().unwrap_or_else(|e| e.into_inner()).push((chr as *const _ as usize, at, max));
    }
    killed
}

/// Area damage with linear falloff to 25% at the edge. Returns how many hostiles were hit.
pub fn explode(at: Vec3, radius: f32, amount: f32) -> usize {
    let mut n = 0;
    for e in game::enemies(400.0) {
        if !is_hostile(&e) {
            continue;
        }
        // Distance to the nearest point of a 2 m tall body.
        let p = e.pos();
        let y = at.y.clamp(p.y, p.y + 2.0);
        let d = Vec3::new(p.x, y, p.z).distance(at);
        if d > radius {
            continue;
        }
        let k = 1.0 - (d / radius.max(0.1)) * 0.75;
        apply(e.chr, amount * k);
        n += 1;
    }
    n
}

/// Explosion waiting for its projectile to arrive.
pub struct Pending {
    pub at: Vec3,
    pub t: f32,
    pub radius: f32,
    pub amount: f32,
    /// ER blast bullet spawned on detonation (enemy reactions).
    pub er_blast: Option<i32>,
    /// Where it was fired from and its total flight time (remote detonation).
    pub origin: Vec3,
    pub total: f32,
}
