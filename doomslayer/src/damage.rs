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

/// The boss-bar character a hit character belongs to, when the bar isn't its own. Some bosses are
/// two characters: the Fire Giant's body (entity 1052520801) takes the hits and the game passes its
/// damage to an invisible boss character (1052520800) that owns the bar. Doom damage writes HP
/// directly, so it skipped that hand-off and the bar never moved (user, 1.3). Same team, entity ids
/// next to each other (same map block), and that character's bar on screen.
/// The HP that counts for this character: its boss bar's when it's a boss's body (burn / shard
/// tracking: the body's own HP no longer moves, user 1.3).
pub fn effective_hp(chr: &eldenring::cs::ChrIns) -> i32 {
    bar_owner(chr).map(|o| o.modules.data.hp).unwrap_or(chr.modules.data.hp)
}

/// Part of a boss whose bar belongs to another character (the Fire Giant's body).
pub fn boss_part(chr: &eldenring::cs::ChrIns) -> bool {
    bar_owner(chr).is_some()
}

fn bar_owner(chr: &eldenring::cs::ChrIns) -> Option<&'static mut eldenring::cs::ChrIns> {
    let bars = game::active_boss_handles();
    if chr.event_entity_id <= 0 || bars.contains(&chr.field_ins_handle) {
        return None;
    }
    // Malenia's five helper characters (npc 21202xxx, same team, ids next to hers) are no body of
    // hers: pairing them sent hits on them into her bar and killed her mid phase change (user, 1.3).
    if chr.npc_param_id / 1000 == 21202 {
        return None;
    }
    let w = game::world()?;
    for h in bars {
        let Some(b) = w.chr_ins_by_handle_mut(&h) else { continue };
        if !game::chr_ok(b as *const _) || b.team_type != chr.team_type || b.event_entity_id <= 0 {
            continue;
        }
        let gap = (b.event_entity_id as i64 - chr.event_entity_id as i64).abs();
        // (also while its HP is 0: at a phase change the game keeps the bar and refills it - the
        // body must stay part of the boss meanwhile, not turn into a normal enemy - user, 1.3)
        if gap <= 10 {
            let b: *mut eldenring::cs::ChrIns = b;
            return Some(unsafe { &mut *b });
        }
    }
    None
}

/// One real Elden Ring hit on Malenia (see apply): a dart spawned at her chest carrying the Fire
/// Pot's goods id, which makes the game compute real damage (bullets without one deal 0). At most
/// one every 0.3 s.
fn malenia_finisher(chr: &eldenring::cs::ChrIns) {
    use std::sync::Mutex;
    use std::time::Instant;
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    if last.is_some_and(|t| t.elapsed().as_secs_f32() < 0.3) {
        return;
    }
    *last = Some(Instant::now());
    let Some(p) = game::player() else { return };
    let chest = game::chr_pos(chr) + Vec3::Y * 1.2;
    let from = game::chr_pos(&p.chr_ins) + Vec3::Y * 1.2;
    let dir = (chest - from).normalize_or(Vec3::X);
    let r = crate::bullet::spawn_ex(Some(p.chr_ins.field_ins_handle), crate::params::FINISHER_BULLET, chest - dir * 0.4, dir, crate::params::FINISHER_GOODS);
    let mut held = HELD.lock().unwrap_or_else(|e| e.into_inner());
    let tries = match held.as_mut() {
        Some((h, _, n)) if *h == chr.field_ins_handle => {
            *n += 1;
            *n
        }
        _ => {
            *held = Some((chr.field_ins_handle, Instant::now(), 1));
            1
        }
    };
    if tries == 1 || tries % 10 == 0 {
        log::info!("Malenia npc {} at {} HP: the game's own hit ends the phase (try {tries}, {r:?})", chr.npc_param_id, chr.modules.data.hp);
    }
}

/// Malenia held at 2 HP by our damage, waiting for the finishing hit: (who, since, tries).
static HELD: std::sync::Mutex<Option<(eldenring::cs::FieldInsHandle, std::time::Instant, u32)>> = std::sync::Mutex::new(None);

/// Every frame: a finishing hit that missed (she dodged, jumped, was mid-move) is fired again until
/// it lands - she sat at 2 HP until the player's next hit before (user). Gives up after 15 s or once
/// she's no longer at 2 HP (1 = the phase change, more = healed / refilled, 0 = dead).
pub fn tick_finisher() {
    let Some((h, since, _)) = *HELD.lock().unwrap_or_else(|e| e.into_inner()) else { return };
    let Some(w) = game::world() else { return };
    let alive = w.chr_ins_by_handle_mut(&h).filter(|c| game::chr_ok(*c as *const _));
    match alive {
        Some(c) if c.modules.data.hp == 2 && since.elapsed().as_secs_f32() < 15.0 => malenia_finisher(c),
        _ => *HELD.lock().unwrap_or_else(|e| e.into_inner()) = None,
    }
}

pub fn apply(chr: &mut eldenring::cs::ChrIns, amount: f32) -> bool {
    // A boss's body (phase 1) whose bar belongs to the next phase's character: like the game, the
    // body takes the hit and the bar character loses exactly what the body lost. The body's own
    // death starts the phase change (Godfrey -> Hoarah Loux, who holds the whole bar); the bar
    // reaches 0 only with the last phase. Sending the damage only to the bar character killed
    // Hoarah while Godfrey stood ("LEGEND FELLED", user 1.3).
    if let Some(owner) = bar_owner(chr) {
        static LOGGED: std::sync::Mutex<Vec<(i32, i32)>> = std::sync::Mutex::new(Vec::new());
        let pair = (chr.npc_param_id as i32, owner.npc_param_id as i32);
        if let Ok(mut l) = LOGGED.lock() {
            if !l.contains(&pair) {
                l.push(pair);
                log::info!("damage to npc {} also comes off its boss bar's npc {} (entity {} -> {})", pair.0, pair.1, chr.event_entity_id, owner.event_entity_id);
            }
        }
        let before = chr.modules.data.hp;
        if before <= 0 {
            return false;
        }
        let after = (before - amount.round().max(1.0) as i32).max(0);
        chr.modules.data.hp = after;
        let dealt = before - after;
        let o = &mut owner.modules.data;
        if o.hp > 0 {
            // (the bar keeps at least 1 while a body is still standing: the game ends the fight)
            o.hp = (o.hp - dealt).max(if after > 0 { 1 } else { 0 });
        }
        if after == 0 {
            log::info!("boss body npc {} down (bar npc {} at {}): the game takes the next phase", chr.npc_param_id, owner.npc_param_id, owner.modules.data.hp);
        }
        return after == 0;
    }
    let no_catch = NO_CATCH.load(std::sync::atomic::Ordering::Relaxed);
    let d = &mut chr.modules.data;
    if d.hp <= 0 {
        return false;
    }
    let before = d.hp;
    // Malenia (npc 212000xx): the end of each phase must come from the game's own damage. Her
    // phase-1 end is a real hit that leaves her at 1 HP (the game holds her there and plays the
    // Goddess of Rot change); a plain HP write - 0 or 1 - skips it: 0 counts as killed for good, 1
    // just leaves her fighting (user 1.3; tested 2026-10-22: a 14-damage game hit at 5 HP ->
    // held at 1 -> change; the same hit in phase 2 -> real death and drop). So our damage stops
    // her at 2 HP and fires one real ER hit (a dart with the Fire Pot's goods id, which makes
    // the game deal damage) that finishes the phase the game's way. At exactly 1 HP she's in
    // the change: no damage from us.
    if chr.npc_param_id / 100 == 212000 {
        if before == 1 {
            return false;
        }
        let after = before - amount.round().max(1.0) as i32;
        if after <= 1 {
            chr.modules.data.hp = before.min(2);
            malenia_finisher(chr);
            return false;
        }
        chr.modules.data.hp = after;
        return false;
    }
    let d = &mut chr.modules.data;
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
    /// The direct hit (target, amount), dealt when the projectile arrives - not on the trigger
    /// pull (rockets hurt enemies before they visibly hit, user 1.3). Dropped on remote detonation.
    pub direct: Option<(FieldInsHandle, f32)>,
}
