//! Runtime param patches, applied once when the param repository is ready (before the world loads).
//! No regulation.bin edits: all data changes live here, in code.
//!
//! Doom weapon projectiles hijack the "Glintstone Stars" bullet rows (10404001..) and the
//! "Carian Slicer" / shard AtkParam_Pc rows. Each slot is a byte copy of a template bullet
//! (pebble = hitscan-ish pellet, fire pot = explosive) re-tuned for one Doom weapon.

use std::sync::atomic::{AtomicBool, Ordering};

use eldenring::{
    cs::{AtkParam_Pc, Bullet, CharaInitParam, ItemLotParam_enemy, NpcParam, PlayerCommonParam, SoloParamRepository, SpEffectParam},
    param::{ATK_PARAM_ST, BULLET_PARAM_ST, SP_EFFECT_PARAM_ST},
};
use fromsoftware_shared::FromStatic;

use crate::{config::Config, weapons::WEAPONS};

/// Bullet rows taken over for Doom weapons, one per weapon slot.
pub const BULLET_SLOTS: [i32; 9] = [
    10404001, 10404002, 10404003, 10404004, 10404005, 10404006, 10404007, 10404008, 10404009,
];
/// AtkParam_Pc rows taken over for Doom weapons, one per weapon slot.
pub const ATK_SLOTS: [u32; 9] = [44400, 44401, 44405, 44406, 44407, 44408, 40010, 40070, 47100];

/// Crystal Dart (a throwable goods bullet): fixed damage, no catalyst scaling. Sorcery bullets
/// (e.g. Glintstone Pebble 10400000) deal 0 when spawned without a staff - verified in game.
/// Visual-only utility bullets (no meaningful damage).
pub const FX_BELCH_BULLET: i32 = 10404009;
/// Muzzle flashes made from Elden Ring's own spell effects (no damage): fire burst / blue flash.
pub const FX_MUZZLE_FIRE: i32 = 10404052;
pub const FX_SILENT_ROUND: i32 = 10404053;

/// ER bullets whose particle effects stand in for Doom's (trail, impact), by weapon slot:
/// shotguns: Rock Sling debris; Heavy Cannon / Chaingun: Lightning Spear (golden tracers);
/// Plasma: Glintstone Pebble (blue bolts); Rocket: Flame Sling fireball; Ballista: Frozen
/// Lightning Spear (blue lightning); BFG: Rennala's Comet + Elden Stars burst.
/// Fire-family ER effects only (lightning filled the screen), picked by look-dev in game
/// (`fxtest`): 524002 Flame Sling fireball = small clean fire ball (muzzle flash, tracers),
/// 524081 Fell God fireball = compact bright ball (rocket, ballista, BFG), 523002 Glintstone
/// Pebble = small blue bolt (plasma). Big bursts (524003, 300122, 524121) and the Pebble impact
/// starburst (523003) fill the screen - not used. 0 = keep the weapon's own impact.
const LOOK_TRAIL: [i32; 8] = [-1, 524002, 524002, 524081, -1, 524081, 524002, 524081];
const LOOK_HIT: [i32; 8] = [0, 0, -1, 0, 0, 0, 0, 0];
const SFX_FIRE_BURST: i32 = 524002;

/// ER projectile each Doom weapon fires (its bullet carries the arrow/bolt/pot model as its
/// effect): Combat/Super Shotgun = Bolts, Heavy Cannon = Great Arrows, Plasma = Magicbone
/// Arrows, Rocket = Fire Pot, Ballista = Ballista Bolt, Chaingun = Arrows, BFG = Giantsflame Pot.
const PROJECTILE: [u32; 8] = [20007000, 20005000, 20002400, 10030000, 20007000, 20009000, 20000000, 10030200];
/// Stagger per weapon (ATKPARAM_DMGTYPE_NEW).
const DMG_LEVEL: [u8; 8] = [2, 1, 1, 3, 3, 4, 1, 10];
/// ER blast bullet for an explosive weapon slot (spawned at the impact point).
pub fn blast_bullet(slot: usize) -> Option<i32> {
    let mut i = 0;
    for (k, w) in WEAPONS.iter().enumerate() {
        if w.explosive {
            if k == slot {
                return BLAST_SLOTS.get(i).map(|b| b.0 as i32);
            }
            i += 1;
        }
    }
    None
}

/// How many pellets of a shotgun blast are visible bolts (the rest are silent hitscan-only).
pub const VISIBLE_PELLETS: [u32; 8] = [3, 1, 1, 1, 5, 1, 1, 1];
pub const FX_GORE_BULLET: i32 = 10404010;
/// Blood Punch shockwave: short-range explosive, big damage.
pub const BLOOD_PUNCH_BULLET: i32 = 10404011;
/// Invisible point-blank damage bullets for Doom melee / Blood Punch (goods template: direct hits count).
pub const MELEE_BULLET: i32 = 10404055;
pub const BLOOD_PUNCH_DAMAGE_BULLET: i32 = 10404056;
/// Meathook "bite": an invisible hit with no flinch (damage level 0, no poise / stamina damage,
/// 1 damage) - the demon only notices you and turns on you.
pub const NOTICE_BULLET: i32 = 10404057;
const NOTICE_ATK: u32 = 43811;
/// Breaks map assets (crates, barrels, furniture: AssetGeometryParam hp 1 / def 0): an invisible
/// short dart with object attack power and no character damage, fired into what a shot hit.
pub const BREAKER_BULLET: i32 = 10404054;
/// Explosion-shaped breaker for assets that ignore projectile hits (fences).
pub const BREAKER_BLAST_BULLET: i32 = 10404058;
const BREAKER_ATK: u32 = 43810;
const MELEE_ATK: u32 = 43805;
const BLOOD_PUNCH_DAMAGE_ATK: u32 = 43806;
const BLOOD_PUNCH_ATK: u32 = 41000;
const FX_BELCH_ATK: u32 = 47100;
const FX_GORE_ATK: u32 = 79981;

pub const PEBBLE_BULLET: u32 = 10174000;
/// Doom loot tokens (real ER goods so the drop looks like any Elden Ring item):
/// Sliver of Meat = ammo, Rowa Raisin = health. Converted and removed on pickup.
pub const DOOM_AMMO_GOODS: i32 = 15000;
pub const DOOM_HEALTH_GOODS: i32 = 810;
pub const FIRE_POT_BULLET: u32 = 10030000;
/// Fire Pot's explosion child bullet: the template for our own explosions.
const FIRE_POT_BLAST: u32 = 10030001;
/// Explosion slots for explosive weapons (rocket, BFG): (bullet, atk) per weapon slot.
const BLAST_SLOTS: [(u32, u32); 2] = [(10404050, 43000), (10404051, 43001)];

static APPLIED: AtomicBool = AtomicBool::new(false);

pub fn applied() -> bool {
    APPLIED.load(Ordering::Relaxed)
}

unsafe fn copy_row<T>(dst: &mut T, src: *const T) {
    unsafe { std::ptr::copy_nonoverlapping(src, dst as *mut T, 1) };
}

/// Try to apply; returns true once done. Safe to call every frame.
pub fn try_apply(cfg: &Config) -> bool {
    if applied() {
        return true;
    }
    let Ok(repo) = (unsafe { SoloParamRepository::instance_mut() }) else {
        return false;
    };
    // Holders exist before their res caps are loaded; the bindings panic on an empty holder.
    let ready = [0usize, 6, 7, 8, 10, 15]
        .iter()
        .all(|&i| repo.solo_param_holders[i].get_res_cap(0).is_some());
    if !ready || repo.get::<Bullet>(PEBBLE_BULLET).is_none() {
        return false;
    }

    // Enemy HP scaling (every NPC row, bosses included). Player character rows (< 1000) untouched.
    let mut scaled = 0;
    for (_, row) in repo.rows_mut::<NpcParam>() {
        let hp = row.hp();
        if hp > 1 {
            let new = ((hp as f64) * cfg.enemy_hp_mult as f64).min(u32::MAX as f64 / 2.0) as u32;
            row.set_hp(new);
            scaled += 1;
        }
    }
    log::info!("NpcParam: scaled HP of {scaled} rows by {}", cfg.enemy_hp_mult);
    APPLIED_HP_MULT.store(cfg.enemy_hp_mult.to_bits(), std::sync::atomic::Ordering::Relaxed);

    // Doom loot: every hostile drops a real Elden Ring item (the glowing ground item), either an
    // ammo token or a health token; autoloot collects it and slayer.rs converts it to Doom
    // ammo/health. Vanilla enemy drops are replaced (user's choice).
    if cfg.doom_loot {
        let friendly = |t: u8| matches!(t, 0 | 1 | 2 | 4 | 5 | 8 | 10 | 14);
        let lot = repo
            .rows_mut::<NpcParam>()
            .find(|(_, r)| !friendly(r.team_type()) && r.item_lot_id_enemy() > 0)
            .map(|(_, r)| r.item_lot_id_enemy());
        if let Some(lot) = lot.filter(|l| repo.get::<ItemLotParam_enemy>(*l as u32).is_some()) {
            if let Some(row) = repo.get_mut::<ItemLotParam_enemy>(lot as u32) {
                row.set_lot_item_id01(DOOM_AMMO_GOODS);
                row.set_lot_item_category01(1);
                row.set_lot_item_base_point01(60);
                row.set_lot_item_num01(1);
                row.set_get_item_flag_id01(0);
                row.set_enable_luck01(false);
                row.set_lot_item_id02(DOOM_HEALTH_GOODS);
                row.set_lot_item_category02(1);
                row.set_lot_item_base_point02(40);
                row.set_lot_item_num02(1);
                row.set_get_item_flag_id02(0);
                for k in 3..=8 {
                    match k {
                        3 => { row.set_lot_item_id03(0); row.set_lot_item_category03(0); row.set_lot_item_base_point03(0); }
                        4 => { row.set_lot_item_id04(0); row.set_lot_item_category04(0); row.set_lot_item_base_point04(0); }
                        5 => { row.set_lot_item_id05(0); row.set_lot_item_category05(0); row.set_lot_item_base_point05(0); }
                        6 => { row.set_lot_item_id06(0); row.set_lot_item_category06(0); row.set_lot_item_base_point06(0); }
                        7 => { row.set_lot_item_id07(0); row.set_lot_item_category07(0); row.set_lot_item_base_point07(0); }
                        _ => { row.set_lot_item_id08(0); row.set_lot_item_category08(0); row.set_lot_item_base_point08(0); }
                    }
                }
                row.set_cumulate_num_flag_id(0);
            }
            let mut n = 0;
            for (_, r) in repo.rows_mut::<NpcParam>() {
                if !friendly(r.team_type()) && r.hp() > 1 {
                    r.set_item_lot_id_enemy(lot);
                    n += 1;
                }
            }
            log::info!("doom loot: lot {lot} (ammo {DOOM_AMMO_GOODS} / health {DOOM_HEALTH_GOODS}) on {n} hostile NpcParam rows");
        } else {
            log::warn!("doom loot: no enemy item lot to repurpose");
        }
    }

    for (slot, w) in WEAPONS.iter().enumerate() {
        // Every weapon fires a goods-type projectile (direct-hit damage works); explosive ones get
        // their own blast child below (the vanilla fire pot's blast does ~26 damage).
        // Real ER projectiles so hits land like arrows/bolts/pots (visible, stick, hit reactions);
        // falls back to the dart if a row is missing.
        // Reaction hit only: the plain dart (never sticks, no model); visuals are ours.
        let template_bullet = PEBBLE_BULLET;
        let Some(src_bullet) = repo.get::<Bullet>(template_bullet).map(|r| r as *const BULLET_PARAM_ST)
        else {
            log::error!("template bullet {template_bullet} missing");
            continue;
        };
        let src_atk_id = unsafe { (*src_bullet).atk_id_bullet() } as u32;
        let Some(src_atk) = repo.get::<AtkParam_Pc>(src_atk_id).map(|r| r as *const ATK_PARAM_ST)
        else {
            log::error!("template atk {src_atk_id} missing");
            continue;
        };

        let bullet_id = BULLET_SLOTS[slot] as u32;
        let atk_id = ATK_SLOTS[slot];

        if let Some(atk) = repo.get_mut::<AtkParam_Pc>(atk_id) {
            unsafe { copy_row(atk, src_atk) };
            // Token ER damage only (hit reaction + aggro); damage.rs deals the real Doom damage.
            let power = (w.damage * 0.1).round().clamp(1.0, 65535.0) as u16;
            atk.set_atk_phys_correction(power);
            atk.set_atk_mag_correction(0);
            atk.set_atk_fire_correction(if w.explosive { power / 2 } else { 0 });
            atk.set_atk_thun_correction(0);
            atk.set_atk_stam(w.stagger);
            atk.set_atk_super_armor(w.poise_damage);
            // How hard the target reacts (ATKPARAM_DMGTYPE: 1 small, 2 medium, 3 large, 4 launch...).
            atk.set_dmg_level(DMG_LEVEL[slot]);
            harden_atk(atk);
        } else {
            log::error!("atk slot {atk_id} missing");
            continue;
        }

        if let Some(b) = repo.get_mut::<Bullet>(bullet_id) {
            unsafe { copy_row(b, src_bullet) };
            b.set_atk_id_bullet(atk_id as i32);
            b.set_init_vellocity(w.speed);
            b.set_max_vellocity(w.speed);
            b.set_life(w.range / w.speed);
            b.set_dist(w.range);
            b.set_gravity_in_range(0.0);
            b.set_hit_radius(w.hit_radius);
            b.set_hit_radius_max(w.hit_radius);
            b.set_num_shoot(1);
            harden_bullet(b);
            // Persistent hitter (hitter.rs): invisible, never expires, passes through what it hits,
            // can hit the same demon again a moment later. Explosives hit everything in the blast.
            let _ = (w.sfx_bullet, w.sfx_hit);
            let radius = if w.explosive { if w.damage > 3000.0 { crate::damage::BFG_RADIUS } else { 3.5 } } else { 0.35 };
            make_hitter(b, radius);
            log::info!(
                "weapon slot {slot} '{}': bullet {bullet_id} atk {atk_id} dmg {}",
                w.name,
                w.damage
            );
        }
    }

    // Explosions for explosive weapons: copy the fire pot blast, give it correction-based damage and
    // a Doom-sized radius, and make the weapon's projectile spawn it on impact.
    let mut blast_i = 0;
    for (slot, w) in WEAPONS.iter().enumerate() {
        if !w.explosive || blast_i >= BLAST_SLOTS.len() {
            continue;
        }
        let (blast_id, blast_atk) = BLAST_SLOTS[blast_i];
        blast_i += 1;
        let radius = if w.damage > 3000.0 { crate::damage::BFG_RADIUS } else { 3.5 };
        fx_bullet(repo, FIRE_POT_BLAST, blast_id, blast_atk, -2, 0.0, 0.25, 0);
        // fx_bullet copies the pebble atk only for damage-less FX; give the blast a real one.
        let dart_atk = repo.get::<Bullet>(PEBBLE_BULLET).map(|b| b.atk_id_bullet() as u32);
        if let Some(src) = dart_atk.and_then(|id| repo.get::<AtkParam_Pc>(id).map(|r| r as *const ATK_PARAM_ST)) {
            if let Some(atk) = repo.get_mut::<AtkParam_Pc>(blast_atk) {
                unsafe { copy_row(atk, src) };
                let power = (w.damage * 0.06).clamp(1.0, 65535.0) as u16;
                atk.set_atk_phys_correction(power);
                atk.set_atk_mag_correction(0);
                atk.set_atk_fire_correction(power / 2);
                atk.set_atk_stam(w.stagger);
                atk.set_atk_super_armor(w.poise_damage);
                harden_atk(atk);
            }
        }
        if let Some(b) = repo.get_mut::<Bullet>(blast_id) {
            b.set_hit_radius(radius * 0.4);
            b.set_hit_radius_max(radius);
            b.set_spread_time(0.15);
        }
        if let Some(b) = repo.get_mut::<Bullet>(BULLET_SLOTS[slot] as u32) {
            b.set_hit_bullet_id(blast_id as i32);
        }
        log::info!("weapon slot {slot} '{}': blast {blast_id} r {radius}", w.name);
    }

    // Flame Belch: fire pot with the ground-fire effect (leaves fire burning on the ground).
    fx_bullet(repo, FIRE_POT_BULLET, FX_BELCH_BULLET as u32, FX_BELCH_ATK, 530405, 14.0, 0.55, 60);
    // Muzzle flash: Flame Sling's small fire burst on a plain dart (no emitter children, nothing
    // lingers - the spell emitters' lingering children filled ER's effect budget).
    fx_bullet(repo, PEBBLE_BULLET, FX_MUZZLE_FIRE as u32, FX_GORE_ATK, SFX_FIRE_BURST, 0.5, 0.07, 0);
    // Invisible round for automatics between tracer rounds.
    fx_bullet(repo, PEBBLE_BULLET, FX_SILENT_ROUND as u32, FX_GORE_ATK, -1, 200.0, 0.5, 0);
    // Glory / chainsaw gore: an invisible dart fired into the demon; its impact plays the Bloodbone
    // arrow's blood splash (5031002). No damage.
    fx_bullet(repo, PEBBLE_BULLET, FX_GORE_BULLET as u32, FX_GORE_ATK, -1, 40.0, 0.2, 0);
    if let Some(b) = repo.get_mut::<Bullet>(FX_GORE_BULLET as u32) {
        b.set_sfx_id_hit(5031002);
        b.set_hit_radius(0.3);
    }

    // Self-check: every row taken over must exist (a missing one silently did nothing before:
    // the melee reaction darts pointed at rows that aren't in the regulation).
    {
        let bullets = BULLET_SLOTS.iter().copied().chain([FX_GORE_BULLET, BLOOD_PUNCH_BULLET, MELEE_BULLET, BLOOD_PUNCH_DAMAGE_BULLET, NOTICE_BULLET, BREAKER_BULLET, BREAKER_BLAST_BULLET, FX_MUZZLE_FIRE, FX_SILENT_ROUND]).chain(BLAST_SLOTS.iter().map(|b| b.0 as i32));
        let missing_b: Vec<i32> = bullets.filter(|id| repo.get::<Bullet>(*id as u32).is_none()).collect();
        let atks = ATK_SLOTS.iter().copied().chain([MELEE_ATK, BLOOD_PUNCH_DAMAGE_ATK, NOTICE_ATK, BLOOD_PUNCH_ATK, FX_GORE_ATK, BREAKER_ATK]).chain(BLAST_SLOTS.iter().map(|b| b.1));
        let missing_a: Vec<u32> = atks.filter(|id| repo.get::<AtkParam_Pc>(*id).is_none()).collect();
        if missing_b.is_empty() && missing_a.is_empty() {
            log::info!("param rows: all bullet / atk rows present");
        } else {
            log::warn!("param rows MISSING: bullets {missing_b:?} atks {missing_a:?}");
        }
    }

    // Object breaker: invisible, short, no damage to characters, big object attack.
    let have_rows = (repo.get::<Bullet>(BREAKER_BULLET as u32).is_some(), repo.get::<AtkParam_Pc>(BREAKER_ATK).is_some());
    fx_bullet(repo, PEBBLE_BULLET, BREAKER_BULLET as u32, BREAKER_ATK, -1, 40.0, 0.08, 0);
    if let Some(atk) = repo.get_mut::<AtkParam_Pc>(BREAKER_ATK) {
        atk.set_atk_obj(500);
        atk.set_atk_stam(0);
        atk.set_atk_super_armor(0.0);
    }
    if let Some(b) = repo.get_mut::<Bullet>(BREAKER_BULLET as u32) {
        make_hitter(b, 0.25);
        b.set_sfx_id_hit(-1);
        b.set_life(0.08);
    }
    // Second breaker for assets that ignore projectile hits (fences AEG217_013: behavior type 0,
    // sliding bullet hit type 0) but break from explosions - rockets and Blood Punch broke them
    // (user). Shaped like the rocket blast (fire pot explosion row, 0.25 s, expanding), invisible,
    // carrying the breaker's attack (object damage, nothing else).
    let blast_src = repo.get::<Bullet>(FIRE_POT_BLAST).map(|r| r as *const BULLET_PARAM_ST);
    if let (Some(src), Some(b)) = (blast_src, repo.get_mut::<Bullet>(BREAKER_BLAST_BULLET as u32)) {
        unsafe { copy_row(b, src) };
        b.set_atk_id_bullet(BREAKER_ATK as i32);
        b.set_sfx_id_bullet(-1);
        b.set_sfx_id_hit(-1);
        b.set_hit_bullet_id(-1);
        b.set_init_vellocity(0.0);
        b.set_max_vellocity(0.0);
        b.set_life(0.25);
        b.set_gravity_in_range(0.0);
        b.set_num_shoot(1);
        b.set_hit_radius(0.3);
        b.set_hit_radius_max(0.9);
        b.set_spread_time(0.15);
        harden_bullet(b);
    }
    log::info!("breaker bullet rows present: bullet {} atk {}", have_rows.0, have_rows.1);

    // Melee damage carriers (invisible, short, wide).
    for (bid, aid, power, stagger, hit_sfx) in [
        (MELEE_BULLET as u32, MELEE_ATK, cfg.melee_power, 60u16, 300002),
        (BLOOD_PUNCH_DAMAGE_BULLET as u32, BLOOD_PUNCH_DAMAGE_ATK, cfg.blood_punch_power, 250u16, 5031002),
    ] {
        fx_bullet(repo, PEBBLE_BULLET, bid, aid, -1, 40.0, 0.12, 0);
        let dart_atk = repo.get::<Bullet>(PEBBLE_BULLET).map(|b| b.atk_id_bullet() as u32);
        if let Some(src) = dart_atk.and_then(|id| repo.get::<AtkParam_Pc>(id).map(|r| r as *const ATK_PARAM_ST)) {
            if let Some(atk) = repo.get_mut::<AtkParam_Pc>(aid) {
                unsafe { copy_row(atk, src) };
                atk.set_atk_phys_correction((power * 0.1) as u16);
                atk.set_atk_mag_correction(0);
                atk.set_atk_fire_correction(0);
                atk.set_atk_stam(stagger);
                atk.set_atk_super_armor(stagger as f32);
                atk.set_atk_obj(500); // punches smash crates and barrels too
                harden_atk(atk);
            }
        }
        if let Some(b) = repo.get_mut::<Bullet>(bid) {
            make_hitter(b, 0.6);
            b.set_sfx_id_hit(hit_sfx);
        }
    }

    // Meathook notice: no reaction animation, no blood, 1 damage - only aggro.
    fx_bullet(repo, PEBBLE_BULLET, NOTICE_BULLET as u32, NOTICE_ATK, -1, 40.0, 0.12, 0);
    let dart_atk = repo.get::<Bullet>(PEBBLE_BULLET).map(|b| b.atk_id_bullet() as u32);
    if let Some(src) = dart_atk.and_then(|id| repo.get::<AtkParam_Pc>(id).map(|r| r as *const ATK_PARAM_ST)) {
        if let Some(atk) = repo.get_mut::<AtkParam_Pc>(NOTICE_ATK) {
            unsafe { copy_row(atk, src) };
            atk.set_atk_phys_correction(1);
            atk.set_atk_mag_correction(0);
            atk.set_atk_fire_correction(0);
            atk.set_atk_stam(0);
            atk.set_atk_super_armor(0.0);
            atk.set_dmg_level(0);
            atk.set_atk_obj(0);
            harden_atk(atk);
        }
    }
    if let Some(b) = repo.get_mut::<Bullet>(NOTICE_BULLET as u32) {
        make_hitter(b, 0.6);
        b.set_sfx_id_hit(-1);
    }

    // Blood Punch: fire-pot blast (explodes on contact) with Bloodflame visuals, heavy physical.
    fx_bullet(repo, FIRE_POT_BULLET, BLOOD_PUNCH_BULLET as u32, BLOOD_PUNCH_ATK, 524121, 30.0, 0.12, 0);
    if let Some(atk) = repo.get_mut::<AtkParam_Pc>(BLOOD_PUNCH_ATK) {
        atk.set_atk_phys_correction(100);
        atk.set_atk_stam(120);
        atk.set_atk_super_armor(80.0);
    }

    // "Doomslayer" starting class (replaces the Vagabond): keeps the armour, starts with no weapon
    // or shield equipped (the longsword was back for a test only - user), the Vagabond's own Vigor
    // 15 (user, 1.2; was 30 = ~1000 HP, too tanky). Only new characters start with it.
    // The level stays the Vagabond's own (it was raised to 24 to match the stat total, which
    // started the class 15 levels ahead - user).
    if let Some(c) = repo.get_mut::<CharaInitParam>(3000) {
        c.set_base_vit(15);
        c.set_equip_wep_right(110000);
        c.set_equip_wep_left(110000);
        c.set_equip_subwep_right(-1);
        c.set_equip_subwep_left(-1);
        c.set_equip_subwep_right3(-1);
        c.set_equip_subwep_left3(-1);
        log::info!("class 3000 -> Doomslayer: vigor {}, level {}", c.base_vit(), c.soul_lv());
    }
    let renamed: Vec<_> = crate::game::find_msg("Vagabond")
        .into_iter()
        .filter(|(cat, i)| crate::game::set_msg_by_index(*cat, *i, "Doomslayer"))
        .collect();
    log::info!("class name Vagabond -> Doomslayer in {renamed:?}");

    // Doom pickups: no crouch-and-grab animation when collecting items (autoloot.rs).
    if let Some(pc) = repo.get_mut::<PlayerCommonParam>(0) {
        pc.set_anime_id_drop_item_pick(999999);
        pc.set_anime_id_material_item_pick(999999);
    }

    // Shield = extra max HP: an unused SpEffect row (7280 "Area Scaling - (Unused)") becomes a
    // copy of the Crimson Amber Medallion effect (permanent max HP multiplier) whose rate the
    // slayer sets to (health + shield) / health. The game then carries the shield in its own HP.
    let amber = repo.get::<SpEffectParam>(AMBER_SPEFFECT).map(|r| r as *const SP_EFFECT_PARAM_ST);
    match (amber, repo.get_mut::<SpEffectParam>(SHIELD_SPEFFECT as u32)) {
        (Some(src), Some(dst)) => {
            unsafe { copy_row(dst, src) };
            dst.set_max_hp_rate(1.0);
            log::info!("shield speffect {SHIELD_SPEFFECT} ready (from {AMBER_SPEFFECT})");
        }
        _ => log::warn!("shield speffect rows missing ({AMBER_SPEFFECT} / {SHIELD_SPEFFECT})"),
    }

    APPLIED.store(true, Ordering::Relaxed);
    true
}

/// SpEffect rows for the shield (see try_apply).
pub const SHIELD_SPEFFECT: i32 = 7280;
const AMBER_SPEFFECT: u32 = 310000;

/// Max-HP multiplier of the shield effect.
pub fn set_shield_rate(rate: f32) {
    if let Ok(repo) = unsafe { SoloParamRepository::instance_mut() } {
        if let Some(row) = repo.get_mut::<SpEffectParam>(SHIELD_SPEFFECT as u32) {
            row.set_max_hp_rate(rate);
        }
    }
}

/// Doom shots go through shields and are never parried.
fn harden_atk(atk: &mut ATK_PARAM_ST) {
    atk.set_disable_guard(true);
    atk.set_is_disable_parry(true);
}

/// Every pellet is its own hit: no shared hit list / hit record (otherwise a target becomes
/// immune to that bullet id for the record's lifetime - the "stops damaging" bug), no creation cap.
/// Turn a bullet row into a persistent hitter (see hitter.rs).
fn make_hitter(b: &mut BULLET_PARAM_ST, radius: f32) {
    // Short-lived invisible reaction dart, spawned right at the body (slayer.rs, rate-limited).
    b.set_sfx_id_bullet(-1);
    b.set_sfx_id_hit(5031002);
    b.set_init_vellocity(25.0);
    b.set_max_vellocity(25.0);
    b.set_life(0.12);
    b.set_dist(3.0);
    b.set_gravity_in_range(0.0);
    b.set_hit_radius(radius);
    b.set_hit_radius_max(radius);
    b.set_hit_bullet_id(-1);
    b.set_is_use_shared_hit_list(false);
    b.set_dmg_hit_record_life_time(0.0);
    b.set_is_attack_sfx(false);
}

fn harden_bullet(b: &mut BULLET_PARAM_ST) {
    b.set_is_use_shared_hit_list(false);
    b.set_dmg_hit_record_life_time(0.0);
    b.set_create_limit_group_id(0);
    b.set_is_hit_both_team(false);
    // Never stay stuck in the character like an arrow (stuck darts held pool slots forever and
    // after a while hits stopped registering).
    b.set_is_attack_sfx(false);
}

#[allow(clippy::too_many_arguments)]
fn fx_bullet(
    repo: &mut SoloParamRepository,
    template: u32,
    bullet_id: u32,
    atk_id: u32,
    sfx: i32,
    speed: f32,
    life: f32,
    power: u16,
) {
    let Some(src_bullet) = repo.get::<Bullet>(template).map(|r| r as *const BULLET_PARAM_ST) else {
        return;
    };
    let src_atk_id = unsafe { (*src_bullet).atk_id_bullet() } as u32;
    let Some(src_atk) = repo.get::<AtkParam_Pc>(src_atk_id).map(|r| r as *const ATK_PARAM_ST) else {
        return;
    };
    if let Some(atk) = repo.get_mut::<AtkParam_Pc>(atk_id) {
        unsafe { copy_row(atk, src_atk) };
        atk.set_atk_phys_correction(0);
        atk.set_atk_mag_correction(0);
        atk.set_atk_fire_correction(power);
        atk.set_atk_thun_correction(0);
    }
    if let Some(b) = repo.get_mut::<Bullet>(bullet_id) {
        unsafe { copy_row(b, src_bullet) };
        b.set_atk_id_bullet(atk_id as i32);
        if sfx != -2 {
            b.set_sfx_id_bullet(sfx);
            b.set_sfx_id_hit(-1);
        }
        b.set_init_vellocity(speed);
        b.set_max_vellocity(speed);
        b.set_life(life);
        b.set_gravity_in_range(0.0);
        b.set_num_shoot(1);
        harden_bullet(b);
        log::info!("fx bullet {bullet_id}: sfx {sfx}");
    }
}

/// A registered boss: Elden Ring's GameAreaParam has a row per boss, keyed by the boss's
/// entity ID - known before its health bar ever shows (a field dragon walked up to was
/// Crucible-killed before its bar came up - user).
pub fn is_area_boss(entity_id: u32) -> bool {
    if entity_id == 0 {
        return false;
    }
    let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else {
        return false;
    };
    repo.get::<eldenring::cs::GameAreaParam>(entity_id).is_some()
}

/// Debug dump of one bullet row (bridge command `bullet <id>`).
pub fn describe_bullet(id: u32) -> String {
    let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else {
        return "no repo".into();
    };
    match repo.get::<Bullet>(id) {
        Some(b) => format!(
            "bullet {id}: shared {} record {} limit {} atk {} sfx {} hit_sfx {} life {} v0 {} r {}",
            b.is_use_shared_hit_list(),
            b.dmg_hit_record_life_time(),
            b.create_limit_group_id(),
            b.atk_id_bullet(),
            b.sfx_id_bullet(),
            b.sfx_id_hit(),
            b.life(),
            b.init_vellocity(),
            b.hit_radius()
        ),
        None => format!("bullet {id}: none"),
    }
}

pub fn describe_atk(id: u32) -> String {
    let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else {
        return "no repo".into();
    };
    match repo.get::<AtkParam_Pc>(id) {
        Some(a) => format!(
            "atk {id}: phys {} mag {} fire {} thun {} stam {} corr p{} m{} f{} t{} s{} add_base {} attr {} type {} sub {} {}",
            a.atk_phys(), a.atk_mag(), a.atk_fire(), a.atk_thun(), a.atk_stam(),
            a.atk_phys_correction(), a.atk_mag_correction(), a.atk_fire_correction(),
            a.atk_thun_correction(), a.atk_stam_correction(), a.is_add_base_atk(),
            a.atk_attribute(), a.atk_type(), a.sub_category1(), a.sub_category2()
        ),
        None => format!("atk {id}: none"),
    }
}

/// The enemy HP multiplier the NpcParam rows were actually scaled by at boot. Base-HP maths
/// (chainsaw limit and fuel, glory, bars) divides by this, not by the live setting: moving the
/// settings window's slider only applies after a restart, so the live value would be wrong until
/// then.
static APPLIED_HP_MULT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn applied_hp_mult() -> f32 {
    match APPLIED_HP_MULT.load(std::sync::atomic::Ordering::Relaxed) {
        0 => crate::config::get_cached().enemy_hp_mult,
        b => f32::from_bits(b),
    }.max(0.1)
}
