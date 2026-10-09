//! Spawning game bullets for Doom weapons.
//!
//! `eldenring::cs::BulletSpawnData` keeps its fields private, so this mirrors its `repr(C)` layout
//! (0x110 bytes) and reinterprets. Layout cross-checked with the TGA cheat table's BulletSpawn:
//! bullet id at +0x14, position at +0x80; everything else zero is accepted by the game.

use eldenring::cs::{BulletSpawnData, CSBulletManager, FieldInsHandle};
use fromsoftware_shared::{F32Vector4, FromStatic};
use glam::Vec3;

#[repr(C)]
struct RawSpawn {
    owner: FieldInsHandle,
    behavior_id: i32,
    magic_id: i32,
    unk10: u32,
    bullet_id: i32,
    goods_id: i32,
    dummy_poly_id: i32,
    target: [u32; 2],
    unk28: u32,
    unk2c: u32,
    unk30: [f32; 4],
    unk40: u32,
    unk44: u32,
    pad48: [u8; 8],
    acceleration_angle: [f32; 4],
    unk60: [f32; 4],
    angle: [f32; 4],
    position: [f32; 4],
    rest: [u8; 0x80],
}

const _: () = assert!(size_of::<RawSpawn>() == 0x110);
const _: () = assert!(size_of::<RawSpawn>() == size_of::<BulletSpawnData>());

/// Game bullets leave a hit record (CSBulletManager list at +0x20, 64 + 192 entries) that isn't
/// released until the area reloads, so the 256 slots are shared out by priority. Breaking props
/// ranks highest, cosmetic hit reactions lowest; the last 24 stay free for enemy arrows / spells.
pub const CAP_REACTION: u32 = 140;
pub const CAP_FX: u32 = 170;
pub const CAP_BLOOD_PUNCH: u32 = 200;
pub const CAP_BREAK: u32 = 232;

/// Spawn only while fewer than `cap` hit records are in use.
pub fn spawn_capped(cap: u32, owner: Option<FieldInsHandle>, bullet_id: i32, pos: Vec3, dir: Vec3) -> Result<(), i32> {
    if crate::hitter::pool_used() >= cap {
        return Err(-2);
    }
    spawn(owner, bullet_id, pos, dir)
}

pub fn spawn(owner: Option<FieldInsHandle>, bullet_id: i32, pos: Vec3, dir: Vec3) -> Result<(), i32> {
    spawn_ex(owner, bullet_id, pos, dir, GOODS_ID.load(std::sync::atomic::Ordering::Relaxed))
}

/// Goods id attached to every Doom bullet (-1 = none). Set from config.
pub static GOODS_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(-1);

/// Look-dev: which "none" ids leak a manager slot. bit0 behavior, bit1 magic, bit2 goods,
/// bit3 dummy poly, bit4 target: set = write 0 instead of -1.
pub static SPAWN_ZERO_MASK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

pub fn spawn_ex(owner: Option<FieldInsHandle>, bullet_id: i32, pos: Vec3, dir: Vec3, goods: i32) -> Result<(), i32> {
    let Ok(man) = (unsafe { CSBulletManager::instance_mut() }) else {
        return Err(-1);
    };

    // The game builds spawn data with its constructor and releases it with its destructor: the
    // spawn stores ref-counted objects in it (+0x98/+0xa0) that only the destructor drops. Without
    // it every bullet leaked one manager slot and after 256 spawns all bullets failed (code 4).
    if let Some((ctor, dtor)) = spawn_data_fns() {
        let mut buf = SpawnBuf([0; 0x200]);
        let p = buf.0.as_mut_ptr();
        unsafe { ctor(p) };
        let raw = unsafe { &mut *(p as *mut RawSpawn) };
        if let Some(owner) = owner {
            raw.owner = owner;
        }
        raw.bullet_id = bullet_id;
        raw.goods_id = goods;
        let dir = dir.normalize_or(Vec3::NEG_Z);
        raw.angle = [dir.x, dir.y, dir.z, 0.0];
        raw.acceleration_angle = raw.angle;
        raw.position = [pos.x, pos.y, pos.z, 1.0];
        // Bit 3 of +0x44 (set by the ctor) queues a network bullet-sync packet in the manager;
        // offline nothing flushes it and it keeps the bullet referenced. Local bullets only.
        // Bit 0 marks the spawn fire-and-forget, as the game's own one-shot spawns do: the
        // manager's emitter record then frees itself once its last bullet is gone. Without it the
        // record waits for an owner to release it, which never happens, and the 256-entry pool
        // fills until the area reloads.
        unsafe { *(p.add(0x44) as *mut u32) = (*(p.add(0x44) as *const u32) & !8) | 1 };
        let r = man.spawn_bullet(unsafe { &*(p as *const BulletSpawnData) });
        unsafe { dtor(p) };
        return r;
    }

    // Fallback (unknown game version): zero-initialised like the cheat table does.
    let mut raw: RawSpawn = unsafe { std::mem::zeroed() };
    if let Some(owner) = owner {
        raw.owner = owner;
    }
    let zm = SPAWN_ZERO_MASK.load(std::sync::atomic::Ordering::Relaxed);
    let none = |bit: u32| if zm & (1 << bit) != 0 { 0 } else { -1 };
    raw.behavior_id = none(0);
    raw.magic_id = none(1);
    raw.bullet_id = bullet_id;
    raw.goods_id = if zm & 4 != 0 { 0 } else { goods };
    raw.dummy_poly_id = none(3);
    raw.target = if zm & 16 != 0 { [0, 0] } else { [u32::MAX, u32::MAX] };
    let dir = dir.normalize_or(Vec3::NEG_Z);
    raw.angle = [dir.x, dir.y, dir.z, 0.0];
    raw.acceleration_angle = raw.angle;
    raw.position = [pos.x, pos.y, pos.z, 1.0];

    let data = unsafe { &*(&raw as *const RawSpawn as *const BulletSpawnData) };
    man.spawn_bullet(data)
}

#[repr(C, align(16))]
struct SpawnBuf([u8; 0x200]);

type SpawnDataFn = unsafe extern "C" fn(*mut u8) -> *mut u8;

/// BulletSpawnData constructor / destructor (ER 2.7.1.0, found next to the game's own
/// spawn call); verified by their prologue bytes, None if they don't match.
fn spawn_data_fns() -> Option<(SpawnDataFn, SpawnDataFn)> {
    static FNS: std::sync::OnceLock<Option<(usize, usize)>> = std::sync::OnceLock::new();
    let f = FNS.get_or_init(|| {
        let base = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None).ok()?.0 as usize };
        let (ctor, dtor) = (base + 0x38c580, base + 0x38d020);
        let ctor_sig: [u8; 10] = [0x48, 0x89, 0x4C, 0x24, 0x08, 0x57, 0x48, 0x83, 0xEC, 0x30];
        let dtor_sig: [u8; 10] = [0x48, 0x89, 0x4C, 0x24, 0x08, 0x53, 0x48, 0x83, 0xEC, 0x30];
        let ok = unsafe {
            std::slice::from_raw_parts(ctor as *const u8, 10) == ctor_sig
                && std::slice::from_raw_parts(dtor as *const u8, 10) == dtor_sig
        };
        log::info!("bullet spawn data ctor/dtor {}", if ok { "found" } else { "NOT found - bullets may leak" });
        ok.then_some((ctor, dtor))
    });
    f.map(|(c, d)| unsafe { (std::mem::transmute::<usize, SpawnDataFn>(c), std::mem::transmute::<usize, SpawnDataFn>(d)) })
}

pub fn v4(v: Vec3) -> F32Vector4 {
    F32Vector4(v.x, v.y, v.z, 1.0)
}
