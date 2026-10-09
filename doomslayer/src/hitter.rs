//! Enemy reactions without exhausting Elden Ring's bullet pool.
//!
//! Every bullet the game spawns outside its own character pipeline keeps a manager slot (an entry
//! in CSBulletManager's per-bullet list) until the map area unloads; after 256 spawns all further
//! spawns fail ("construction failed") and hits stop registering. So instead of one bullet per hit,
//! each weapon owns ONE persistent, invisible, penetrating "hitter" bullet that never expires; when
//! a shot lands we move it onto the target for a frame. The game registers a real hit from the
//! player with that weapon's stagger / poise (enemies flinch, bleed and aggro), and the hitter goes
//! back to its parking spot. 8 bullets total, respawned only if the game removes one (area change).

use eldenring::cs::{CSBulletManager, FieldInsHandle};
use fromsoftware_shared::FromStatic;
use glam::Vec3;

use crate::{bullet, game, params};

pub const MELEE: usize = 8;
pub const BLOOD_PUNCH: usize = 9;
/// Meathook bite: aggro only, no flinch; ignores (and doesn't start) the per-enemy cooldown.
pub const NOTICE: usize = 10;
const N: usize = 11;

fn hitter_id(slot: usize) -> i32 {
    match slot {
        MELEE => params::MELEE_BULLET,
        BLOOD_PUNCH => params::BLOOD_PUNCH_DAMAGE_BULLET,
        NOTICE => params::NOTICE_BULLET,
        s => params::BULLET_SLOTS[s],
    }
}

/// Kept for the bridge's look-dev commands.
pub static SWEEP: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(20);
pub static ON_MS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(60);

/// Slots in use in CSBulletManager's per-bullet table (256 max; freed only on area reload).
pub fn pool_used() -> u32 {
    let Ok(man) = (unsafe { CSBulletManager::instance() }) else { return 0 };
    // buffer entries in use (+0x1d0, recounted by the game) + heap entries alive (+0x38).
    let base = man as *const _ as *const u8;
    unsafe { (base.add(0x1d0) as *const u32).read_unaligned() + (base.add(0x38) as *const u32).read_unaligned() }
}

/// Seconds between real ER hits on the same enemy (user: 2 s).
const REACT_COOLDOWN: f32 = 2.0;


#[derive(Default)]
pub struct Hitters {
    /// Last reaction per character (ptr-free key: handle bits) for the per-enemy cooldown.
    last: std::collections::HashMap<u64, f32>,
    time: f32,
}

impl Hitters {
    /// Real ER hit on a character for weapon `slot`: one dart at the body, at most one per enemy
    /// every 2 s, and never into the reserved part of the pool.
    pub fn hit(&mut self, slot: usize, at: Vec3, dir: Vec3) {
        self.hit_on(None, slot, at, dir);
    }

    pub fn hit_on(&mut self, who: Option<FieldInsHandle>, slot: usize, at: Vec3, dir: Vec3) {
        let key = who.map(|h| unsafe { std::mem::transmute::<FieldInsHandle, u64>(h) }).unwrap_or((at.x as i64 as u64) ^ ((at.z as i64 as u64) << 20));
        if slot != NOTICE && self.last.get(&key).is_some_and(|t| self.time - t < REACT_COOLDOWN) {
            return;
        }
        if pool_used() >= bullet::CAP_REACTION {
            return;
        }
        if slot != NOTICE {
            self.last.insert(key, self.time);
        }
        let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
        // Blasts (straight down onto the impact point): start just above it - from 0.8 m up the
        // ER explosion went off above our own fireball (user).
        let lead = if who.is_none() && dir.y < -0.9 { 0.2 } else { 0.8 };
        if let Err(e) = bullet::spawn(owner, hitter_id(slot), at - dir * lead, dir) {
            log::warn!("reaction spawn failed: {e}");
        }
    }

    pub fn update(&mut self, dt: f32, _owner: Option<FieldInsHandle>) {
        self.time += dt;
        if self.last.len() > 256 {
            let t = self.time;
            self.last.retain(|_, v| t - *v < 2.0);
        }
    }
}
