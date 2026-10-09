//! Thin accessors over fromsoftware-rs for the bits DOOM RING touches.

use eldenring::{
    cs::{CSCamera, ChrIns, FieldInsHandle, LockTgtMan, PlayerIns, WorldChrMan},
    position::HavokPosition,
};
use fromsoftware_shared::{FromStatic, Superclass};
use glam::Vec3;

pub fn world() -> Option<&'static mut WorldChrMan> {
    unsafe { WorldChrMan::instance_mut().ok() }
}

/// Player-type characters other than you (NPC invaders / red phantoms are built like players): their
/// real HP lives in PlayerGameData and the game copies it over the ChrIns HP every frame.
pub fn player_type_of(chr: &ChrIns) -> Option<&'static mut PlayerIns> {
    let w = world()?;
    let me = w.main_player.as_ref().map(|p| &p.chr_ins as *const ChrIns);
    let ptr = chr as *const ChrIns;
    if Some(ptr) == me {
        return None;
    }
    // By its real class (invaders may sit in any ChrSet), falling back to the player set.
    if let Some(p) = chr.as_subclass::<PlayerIns>().map(|p| p as *const PlayerIns as *mut PlayerIns) {
        // Game-owned object: the engine itself mutates it every frame.
        return Some(unsafe { &mut *p });
    }
    w.player_chr_set.characters().find(|p| std::ptr::eq(&p.chr_ins, ptr))
}

pub fn player() -> Option<&'static mut PlayerIns> {
    let w = world()?;
    let p = w.main_player.as_mut()?;
    Some(unsafe { &mut *(p.as_mut() as *mut PlayerIns) })
}

pub fn hpos(p: &HavokPosition) -> Vec3 {
    Vec3::new(p.0, p.1, p.2)
}

pub fn chr_pos(chr: &ChrIns) -> Vec3 {
    hpos(&chr.modules.physics.position)
}

/// Camera position and forward vector, as driven by erfps2's first-person camera.
/// The game is playing a scripted interaction: walking through a boss fog wall, opening a door,
/// pulling a lever, opening a chest, resting at a grace. Doom movement must let it run (it moved
/// the body through the fog without the event firing). Every ObjActParam player animation, plus
/// the player's event animation range (a000_06xxxx).
pub fn in_event_anim(chr: &ChrIns) -> Option<i32> {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static SET: OnceLock<HashSet<i32>> = OnceLock::new();
    let a = crate::slayer::current_anim(chr);
    if a <= 0 {
        return None;
    }
    // 60071 isn't a door / lever: it flickers on and off for a frame or two around items, glory
    // kills and enemy contact (192 times in a 1.1 session) and each flicker cut a jump and stopped
    // the run (user, 1.2: move freely through it).
    if a % 1_000_000 == 60071 {
        return None;
    }
    if (60000..70000).contains(&(a % 1_000_000)) {
        return Some(a);
    }
    let set = match SET.get() {
        Some(s) => s,
        None => {
            let repo = unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance() }.ok()?;
            let ids: HashSet<i32> = repo
                .rows::<eldenring::cs::ObjActParam>()
                .map(|(_, r)| r.player_anim_id() as i32)
                .filter(|v| *v > 0)
                .collect();
            if ids.is_empty() {
                return None;
            }
            log::info!("obj act player anims: {} ({:?}...)", ids.len(), ids.iter().take(12).collect::<Vec<_>>());
            let _ = SET.set(ids);
            SET.get()?
        }
    };
    set.contains(&a).then_some(a)
}

/// An NPC's display name (NpcName FMG), if it has one.
pub fn npc_name(npc_param_id: i32) -> Option<String> {
    let repo = unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance() }.ok()?;
    let row = repo.get::<eldenring::cs::NpcParam>(npc_param_id as u32)?;
    let msg = unsafe { <eldenring::cs::MsgRepository as fromsoftware_shared::FromStatic>::instance() }.ok()?;
    msg.get_msg(18, row.name_id() as u32).map(|w| String::from_utf16_lossy(w))
}

/// On a ladder (getting on, climbing, getting off).
pub fn on_ladder(chr: &ChrIns) -> bool {
    chr.modules.ladder.state != eldenring::cs::LadderState::None
}

pub fn camera() -> Option<(Vec3, Vec3)> {
    let cam = unsafe { CSCamera::instance().ok()? };
    let m = &cam.pers_cam_1.matrix;
    let fwd = Vec3::new(m.2.0, m.2.1, m.2.2).normalize_or_zero();
    let pos = Vec3::new(m.3.0, m.3.1, m.3.2);
    (fwd != Vec3::ZERO).then_some((pos, fwd))
}

/// Full camera basis for screen projection.
pub struct Cam {
    pub pos: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    pub fwd: Vec3,
    pub fov: f32,
}

pub fn camera_full() -> Option<Cam> {
    let cam = unsafe { CSCamera::instance().ok()? };
    let c = &cam.pers_cam_1;
    let m = &c.matrix;
    let v = |c: &fromsoftware_shared::F32Vector4| Vec3::new(c.0, c.1, c.2);
    Some(Cam { pos: v(&m.3), right: v(&m.0), up: v(&m.1), fwd: v(&m.2), fov: c.fov })
}

impl Cam {
    /// World point -> screen pixels (None if behind the camera).
    pub fn project(&self, p: Vec3, w: f32, h: f32) -> Option<[f32; 2]> {
        let d = p - self.pos;
        let z = d.dot(self.fwd);
        if z < 0.2 {
            return None;
        }
        let t = (self.fov * 0.5).tan();
        let aspect = w / h;
        let x = d.dot(self.right) / (z * t * aspect);
        let y = d.dot(self.up) / (z * t);
        Some([w * 0.5 * (1.0 + x), h * 0.5 * (1.0 - y)])
    }
}

pub struct Enemy {
    pub chr: &'static mut ChrIns,
    pub dist: f32,
}

impl Enemy {
    pub fn hp(&self) -> i32 {
        self.chr.modules.data.hp
    }
    pub fn max_hp(&self) -> i32 {
        self.chr.modules.data.max_hp.max(1)
    }
    pub fn pos(&self) -> Vec3 {
        chr_pos(self.chr)
    }
}

/// Living, non-player characters nearby, closest first.
///
/// `ChrInsDistanceEntry::distance` is NOT a distance (verified: a troll 107 m away reported 14),
/// so the real 3D distance from the player is computed from physics positions.
/// Time spent in enemies() (ns) and calls, for the soak test's performance check (bridge state).
pub static ENEMY_SCAN_NS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ENEMY_SCAN_CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn enemies(max_dist: f32) -> Vec<Enemy> {
    let t0 = std::time::Instant::now();
    let out = enemies_inner(max_dist);
    ENEMY_SCAN_NS.fetch_add(t0.elapsed().as_nanos() as u64, std::sync::atomic::Ordering::Relaxed);
    ENEMY_SCAN_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    out
}

/// Every character the game knows about, with where it was found: all ChrSets (map sets,
/// holders, open-field generators, debug) plus the game's own per-frame lists of every updated
/// character (by update priority / by distance) - ground-spawned enemies were missing from the
/// sets. Deduplicated.
pub fn all_characters(w: &WorldChrMan) -> Vec<(*mut ChrIns, &'static str)> {
    let mut out: Vec<(*mut ChrIns, &'static str)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut add = |p: *mut ChrIns, src: &'static str, out: &mut Vec<(*mut ChrIns, &'static str)>| {
        if !p.is_null() && seen.insert(p as usize) {
            out.push((p, src));
        }
    };
    let mut sets: Vec<(*const eldenring::cs::ChrSet<ChrIns>, &'static str)> =
        w.chr_sets.iter().flatten().map(|s| (&**s as *const _, "chr_sets")).collect();
    let n = (w.chr_set_holder_count as usize).min(w.chr_set_holders.len());
    sets.extend(w.chr_set_holders[..n].iter().map(|h| (h.chr_set.as_ptr() as *const _, "holder")));
    sets.push((&w.open_field_chr_set.base as *const _, "open_field"));
    sets.push((&w.debug_chr_set as *const _, "debug"));
    for (s, src) in sets {
        for c in unsafe { &*s }.characters() {
            add(c as *mut ChrIns, src, &mut out);
        }
    }
    for p in w.chr_inses_by_update_priority.iter() {
        add(p.as_ptr(), "update_list", &mut out);
    }
    for e in w.chr_inses_by_distance.iter() {
        add(e.chr_ins.as_ptr(), "distance_list", &mut out);
    }
    out
}

/// Off while the game loads / fades (the slayer sets it each frame): the world is half built then
/// and nothing needs a character scan (1.2 crash fix).
pub static SCAN_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// A character that's fully there: its module container and its data / physics modules exist.
/// One being built or torn down has null module pointers - the 1.1 crash read `modules.data` of
/// one (address 0) while enemies spawned around the player.
pub fn chr_ok(chr: *const ChrIns) -> bool {
    if chr.is_null() {
        return false;
    }
    unsafe {
        let m = *(std::ptr::addr_of!((*chr).modules) as *const usize);
        if m == 0 {
            return false;
        }
        let m = m as *const eldenring::cs::ChrInsModuleContainer;
        let data = *(std::ptr::addr_of!((*m).data) as *const usize);
        let physics = *(std::ptr::addr_of!((*m).physics) as *const usize);
        data != 0 && physics != 0
    }
}

fn enemies_inner(max_dist: f32) -> Vec<Enemy> {
    if !SCAN_OK.load(std::sync::atomic::Ordering::Relaxed) {
        return vec![];
    }
    let Some(w) = world() else { return vec![] };
    let Some(me) = w.main_player.as_ref() else { return vec![] };
    let player_ptr = &me.chr_ins as *const ChrIns;
    let my_pos = chr_pos(&me.chr_ins);
    let mut out = Vec::new();
    // Every live ChrSet, not just chr_inses_by_distance: that list holds only ~100 nearby
    // characters and misses NPC invaders / red phantoms entirely (only ~6 sets, ~1700 slots).
    let mut seen = std::collections::HashSet::new();
    for (ptr, _) in all_characters(w) {
        if ptr as *const ChrIns == player_ptr || !seen.insert(ptr as usize) || !chr_ok(ptr) {
            continue;
        }
        let chr = unsafe { &mut *ptr };
        if chr.team_type == 1 {
            continue;
        }
        let data = &chr.modules.data;
        if data.hp <= 0 || data.max_hp <= 1 {
            continue;
        }
        let dist = chr_pos(chr).distance(my_pos);
        if dist > max_dist {
            continue;
        }
        out.push(Enemy { chr, dist });
    }
    out.sort_by(|a, b| a.dist.total_cmp(&b.dist));
    out
}

/// Point the camera at a character through the game's own lock-on (erfps2 keeps it first person).
/// Returns false when the target isn't in the lock-on candidate list.
pub fn lock_on(handle: FieldInsHandle) -> bool {
    let Ok(lock) = (unsafe { LockTgtMan::instance_mut() }) else { return false };
    let mut next = lock.nodes;
    let mut found = None;
    while let Some(node) = next.map(|mut p| unsafe { p.as_mut() }) {
        next = node.next;
        node.flags &= !32;
        if unsafe { node.value.as_ref().chr_handle == handle } {
            found = Some(node);
        }
    }
    let Some(node) = found else { return false };
    node.flags |= 32;
    lock.is_locked_on = true;
    lock.is_lock_on_requested = true;
    true
}

pub fn release_lock_on() {
    if let Ok(lock) = unsafe { LockTgtMan::instance_mut() } {
        lock.is_locked_on = false;
        lock.is_lock_on_requested = false;
    }
}

/// Hide every visible piece of the player's Elden Ring body (armour, arms, weapons, base model) while
/// keeping the shadow; the Doom Slayer's own arms are drawn by our viewmodel renderer instead.
/// disp_flags1 bit 0 = visible (erfps2 uses the same bit for the face parts).
pub fn hide_player_body(hide: bool) {
    let Some(p) = player() else { return };
    // During loading the model pointers can still be null; never deref those.
    unsafe fn set(e: *mut eldenring::cs::CSModelDispEntity, hide: bool) {
        if !e.is_null() {
            unsafe { (*e).disp_flags1 = (*e).disp_flags1 & !1 | (!hide) as u32 };
        }
    }
    unsafe fn ent(m: *mut eldenring::cs::CSModelIns) -> *mut eldenring::cs::CSModelDispEntity {
        if m.is_null() {
            return std::ptr::null_mut();
        }
        let f = unsafe { std::ptr::addr_of_mut!((*m).model_disp_entity) } as *mut *mut eldenring::cs::CSModelDispEntity;
        unsafe { *f }
    }
    unsafe {
        let base = std::ptr::addr_of_mut!(p.chr_ins.chr_model_ins) as *mut *mut eldenring::cs::CSChrModelIns;
        if !(*base).is_null() {
            set(ent(std::ptr::addr_of_mut!((**base).model_ins)), hide);
        }
        let asm = std::ptr::addr_of_mut!(p.chr_asm_model_ins) as *mut *mut eldenring::cs::CSChrAsmModelIns;
        if !(*asm).is_null() {
            let a = &mut **asm;
            let cm = std::ptr::addr_of_mut!(a.chr_model_ins) as *mut *mut eldenring::cs::CSChrModelIns;
            if !(*cm).is_null() {
                set(ent(std::ptr::addr_of_mut!((**cm).model_ins)), hide);
            }
            let parts = std::ptr::addr_of_mut!(a.parts_model_ins) as *mut [*mut eldenring::cs::CSModelIns; 27];
            for &m in (*parts).iter() {
                set(ent(m), hide);
            }
        }
    }
}

/// Hide Elden Ring's own player HUD (HP/FP/stamina bars + equipment) during normal gameplay so the
/// Doom HUD replaces it. Menus keep their own states; boss bars are separate and stay visible.
pub fn hide_er_hud(hide: bool) {
    use eldenring::cs::{CSFeManHudState, CSFeManImp};
    let Ok(fe) = (unsafe { <CSFeManImp as fromsoftware_shared::FromStatic>::instance_mut() }) else { return };
    if hide {
        if matches!(fe.hud_state, CSFeManHudState::Default) {
            fe.hud_state = CSFeManHudState::HideAll;
        }
        fe.frontend_values.enable_equip_hud = false;
    } else if matches!(fe.hud_state, CSFeManHudState::HideAll) {
        fe.hud_state = CSFeManHudState::Default;
        fe.frontend_values.enable_equip_hud = true;
    }
}

/// True while an ER menu is open: the game switches its HUD state to ShowAll (Esc / pause menu)
/// or PopupMenu (dialogs); in gameplay it's Default (we turn that into HideAll).
pub fn menu_open() -> bool {
    use eldenring::cs::{CSFeManHudState, CSFeManImp};
    use std::sync::Mutex;
    static LAST: Mutex<String> = Mutex::new(String::new());
    let Ok(fe) = (unsafe { <CSFeManImp as fromsoftware_shared::FromStatic>::instance() }) else { return false };
    let state = format!("{:?}", fe.hud_state);
    if let Ok(mut last) = LAST.lock() {
        if *last != state {
            log::info!("hud state {last} -> {state}");
            *last = state;
        }
    }
    // ShowAll is not used: ER also flips to it mid-gameplay (it made the gun dip while walking);
    // the pause menu itself shows up in the menu UI element table.
    matches!(fe.hud_state, CSFeManHudState::PopupMenu) || menu_window_open()
}

/// Submenus (Equipment, Inventory, System...) put the HUD state back to normal, but their window
/// shows up in CSMenuManImp's UI element table. Seen in play: 5 and 8 are always visible in
/// gameplay, and 7 / 23 can stay visible after loading too. Menu windows: pause 28, Equipment 29,
/// Inventory 31-32, System 37. 65 is the tutorial message box (checked 2026-10-08: visible exactly
/// while one was up) - it counts too, so the gun lowers out of it (user).
/// New elements are logged once so the list can be checked against play.
/// A world object (map asset, "AEG...") hit by a ray, with its breakable stats.
#[derive(Clone)]
pub struct GeomHit {
    pub name: String,
    /// AssetGeometryParam hp: > 0 = breakable by attacks (minus `defense` per hit).
    pub hp: i16,
    pub defense: u16,
    pub break_by_player: bool,
    /// AssetGeometryParam behavior type: 0 on fences that ignore projectile hits.
    pub behavior: u8,
    /// A loose physics prop (not in the placed lists: pots, skulls): the game pushes it around,
    /// so it never blocks our movement.
    pub loose: bool,
    /// CSWorldGeomIns pointer (valid while its block is loaded).
    pub ptr: usize,
}

/// Runtime state of an asset instance (+0x44c): 1 while intact, 4 once broken (seen by diffing an
/// asset before / after a break).
pub fn geom_state(ptr: usize) -> u32 {
    unsafe { ((ptr + 0x44c) as *const u32).read_unaligned() }
}

impl GeomHit {
    /// Still standing: not broken yet (its debris must not stop shots or draw more breakers).
    pub fn intact(&self) -> bool {
        geom_state(self.ptr) == 1
    }

    /// Still there to bump into: standing (1), active / moving (3 - the tutorial lift cage reads
    /// 3 and its walls let you through, user) or mid-break (4 - a wall losing a chunk plays its
    /// break for 1-3 s, then goes back to 1 with the rest of it still up). Unbreakable assets
    /// (hp -1) are always there.
    pub fn standing(&self) -> bool {
        self.hp <= 0 || matches!(geom_state(self.ptr), 1 | 3 | 4)
    }
}

/// Find the CSWorldGeomIns behind a ray-hit handle, cached (shots through grass hit many type-6
/// bodies; each uncached lookup walks every loaded asset). The cache is dropped every 10 s so
/// streamed-out / reused slots don't linger.
pub fn geom_by_handle(handle: &FieldInsHandle) -> Option<GeomHit> {
    use std::{collections::HashMap, sync::Mutex, time::Instant};
    static CACHE: Mutex<Option<(Instant, HashMap<(i32, u32), Option<GeomHit>>)>> = Mutex::new(None);
    let key = (handle.block_id.0, handle.selector.index());
    if let Ok(mut g) = CACHE.lock() {
        let (born, map) = g.get_or_insert_with(|| (Instant::now(), HashMap::new()));
        if born.elapsed().as_secs_f32() > 2.0 || map.len() > 8192 {
            *born = Instant::now();
            map.clear();
        }
        if let Some(hit) = map.get(&key) {
            return hit.clone();
        }
        let hit = geom_lookup(handle);
        // only finds are cached: a "not found" kept a loose prop unbreakable for 2 s after the
        // first hit had already registered it (skulls took several shots - user)
        if hit.is_some() {
            map.insert(key, hit.clone());
        }
        return hit;
    }
    geom_lookup(handle)
}

/// Assets that aren't in any block's placed-asset list (loose physics props: pots, glowing skulls
/// - user's shots and punches ignored them): seen through a ray hit, which points at the object
/// itself. Remembered by handle so later lookups by handle find them too.
static DYNAMIC: std::sync::Mutex<Option<std::collections::HashMap<(i32, u32), usize>>> = std::sync::Mutex::new(None);

/// vtables of the assets in the placed lists (CSWorldGeomIns and subclasses), refreshed now and then.
fn geom_vtables() -> std::collections::HashSet<usize> {
    use eldenring::cs::CSWorldGeomMan;
    use std::{sync::Mutex, time::Instant};
    static VT: Mutex<Option<(Instant, std::collections::HashSet<usize>)>> = Mutex::new(None);
    if let Ok(g) = VT.lock() {
        if let Some((at, set)) = g.as_ref() {
            if at.elapsed().as_secs_f32() < 30.0 && !set.is_empty() {
                return set.clone();
            }
        }
    }
    let mut set = std::collections::HashSet::new();
    if let Ok(gm) = unsafe { CSWorldGeomMan::instance() } {
        for pair in gm.blocks.iter() {
            // every one (the first 400 per block missed the loose props' class at times: shots
            // didn't recognise a skull until a lucky refresh - user needed several shots)
            for g in pair.second.geom_ins_vector.iter() {
                set.insert(unsafe { *(&**g as *const _ as *const usize) });
            }
        }
    }
    if let Ok(mut g) = VT.lock() {
        *g = Some((Instant::now(), set.clone()));
    }
    set
}

/// The asset behind a ray hit: the placed lists first, then the hit's own object when it is an
/// asset (same vtable as the placed ones).
pub fn geom_by_hit(hit: &crate::raycast::hknpHit) -> Option<GeomHit> {
    let handle = hit.field_ins_handle()?;
    if let Some(g) = geom_by_handle(&handle) {
        return Some(g);
    }
    let fi = hit.field_ins()?.as_ptr() as usize;
    let vt = unsafe { *(fi as *const usize) };
    if !geom_vtables().contains(&vt) {
        return None;
    }
    if let Ok(mut d) = DYNAMIC.lock() {
        let m = d.get_or_insert_with(Default::default);
        if m.len() > 4096 {
            m.clear();
        }
        m.insert((handle.block_id.0, handle.selector.index()), fi);
    }
    let g = unsafe { &*(fi as *const eldenring::cs::CSWorldGeomIns) };
    Some(GeomHit { loose: true, ..read_geom(g) })
}

fn read_geom(g: &eldenring::cs::CSWorldGeomIns) -> GeomHit {
    let p = unsafe { g.info.asset_geometry_param.as_ref() };
    // runtime-created props may have no MSB part behind them: never read through a null pointer
    let part = unsafe { *(&g.info.msb_parts_geom.msb_parts.msb_part as *const _ as *const usize) };
    let name = if part == 0 { String::new() } else { unsafe { g.info.msb_parts_geom.msb_parts.msb_part.name.to_string() }.unwrap_or_default() };
    GeomHit { name, hp: p.hp(), defense: p.defense(), break_by_player: p.is_break_by_player_collide(), behavior: p.behavior_type() as u8, loose: false, ptr: g as *const _ as usize }
}

fn geom_lookup(handle: &FieldInsHandle) -> Option<GeomHit> {
    use eldenring::cs::CSWorldGeomMan;
    // a loose prop already seen through a ray hit: no walk through every placed asset
    if let Some(ptr) = DYNAMIC.lock().ok().and_then(|d| d.as_ref()?.get(&(handle.block_id.0, handle.selector.index())).copied()) {
        let vt = unsafe { *(ptr as *const usize) };
        let g = unsafe { &*(ptr as *const eldenring::cs::CSWorldGeomIns) };
        if geom_vtables().contains(&vt) && g.field_ins_handle.selector.index() == handle.selector.index() {
            return Some(GeomHit { loose: true, ..read_geom(g) });
        }
    }
    let gm = unsafe { CSWorldGeomMan::instance() }.ok()?;
    let read = |g: &eldenring::cs::CSWorldGeomIns| {
        let p = unsafe { g.info.asset_geometry_param.as_ref() };
        let name = unsafe { g.info.msb_parts_geom.msb_parts.msb_part.name.to_string() }.unwrap_or_default();
        GeomHit { name, hp: p.hp(), defense: p.defense(), break_by_player: p.is_break_by_player_collide(), behavior: p.behavior_type() as u8, loose: false, ptr: g as *const _ as usize }
    };
    // Asset collision answers ray casts as HitGeom: same block and index as the Geom instance.
    let same = |g: &eldenring::cs::CSWorldGeomIns| {
        g.field_ins_handle == *handle || (g.field_ins_handle.block_id == handle.block_id && g.field_ins_handle.selector.index() == handle.selector.index())
    };
    for pair in gm.blocks.iter() {
        let block = &pair.second;
        if let Some(g) = block.geom_ins_vector.iter().find(|g| same(g)) {
            return Some(read(g));
        }
    }
    // a loose prop seen through a ray hit before (still the same object: vtable and handle match)
    let key = (handle.block_id.0, handle.selector.index());
    let ptr = DYNAMIC.lock().ok()?.as_ref()?.get(&key).copied()?;
    let vt = unsafe { *(ptr as *const usize) };
    let g = unsafe { &*(ptr as *const eldenring::cs::CSWorldGeomIns) };
    (geom_vtables().contains(&vt) && g.field_ins_handle.selector.index() == handle.selector.index()).then(|| GeomHit { loose: true, ..read_geom(g) })
}

/// Breakable map assets in the loaded map near the player: (name, hp, world offset from the
/// player in metres). Positions come from each asset's MSB part (+0x20); overworld tiles
/// (area 60) are 256 m apart, so tile coordinates put everything in one frame.
pub fn breakables_near(max_dist: f32) -> Vec<(String, i16, Vec3)> {
    assets_near(max_dist, true)
}

/// Map assets near the player (optionally only breakable ones): (name, hp, offset from player).
pub fn assets_near(max_dist: f32, breakable_only: bool) -> Vec<(String, i16, Vec3)> {
    use eldenring::cs::CSWorldGeomMan;
    let (Some(gm), Some(me)) = (unsafe { CSWorldGeomMan::instance() }.ok(), player()) else { return Vec::new() };
    let tile = |b: &eldenring::cs::BlockId| Vec3::new(b.block() as f32 * 256.0, 0.0, b.region() as f32 * 256.0);
    let pb = me.current_block_id;
    let mine = Vec3::new(me.block_position.x, me.block_position.y, me.block_position.z) + tile(&pb);
    let mut out = Vec::new();
    for pair in gm.blocks.iter() {
        let block = &pair.second;
        let same_area = block.block_id.area() == pb.area() && (pb.area() >= 60 || block.block_id == pb);
        if !same_area {
            continue;
        }
        for g in block.geom_ins_vector.iter() {
            let p = unsafe { g.info.asset_geometry_param.as_ref() };
            if breakable_only && (p.hp() <= 0 || geom_state(&**g as *const _ as usize) != 1) {
                continue;
            }
            let part = &*g.info.msb_parts_geom.msb_parts.msb_part as *const _ as *const u8;
            let pos = unsafe { (part.add(0x20) as *const [f32; 3]).read_unaligned() };
            let world = Vec3::from(pos) + tile(&block.block_id);
            let d = world - mine;
            if d.length() <= max_dist {
                let name = unsafe { g.info.msb_parts_geom.msb_parts.msb_part.name.to_string() }.unwrap_or_default();
                out.push((name, p.hp(), d));
            }
        }
    }
    out.sort_by(|a, b| a.2.length().total_cmp(&b.2.length()));
    out
}

/// CSWorldGeomIns pointer of a loaded asset by (part of) its MSB name (diagnostics).
pub fn geom_ptr_by_name(want: &str) -> Option<usize> {
    use eldenring::cs::CSWorldGeomMan;
    let gm = unsafe { CSWorldGeomMan::instance() }.ok()?;
    for pair in gm.blocks.iter() {
        for g in pair.second.geom_ins_vector.iter() {
            let name = unsafe { g.info.msb_parts_geom.msb_parts.msb_part.name.to_string() }.unwrap_or_default();
            if name.contains(want) {
                return Some(&**g as *const _ as usize);
            }
        }
    }
    None
}

/// Raw MsbPart pointer of the asset behind a ray-hit handle (diagnostics).
pub fn geom_part_ptr(handle: &FieldInsHandle) -> Option<usize> {
    use eldenring::cs::CSWorldGeomMan;
    let gm = unsafe { CSWorldGeomMan::instance() }.ok()?;
    for pair in gm.blocks.iter() {
        let g = pair.second.geom_ins_vector.iter().find(|g| {
            g.field_ins_handle.block_id == handle.block_id && g.field_ins_handle.selector.index() == handle.selector.index()
        });
        if let Some(g) = g {
            return Some(&*g.info.msb_parts_geom.msb_parts.msb_part as *const _ as usize);
        }
    }
    None
}

/// Indices of the menu UI elements visible right now (bridge diagnostics).
pub fn menu_ui_visible() -> Vec<usize> {
    use eldenring::cs::CSMenuManImp;
    let Ok(mm) = (unsafe { <CSMenuManImp as fromsoftware_shared::FromStatic>::instance() }) else { return Vec::new() };
    mm.ui_states.iter().enumerate().filter(|(_, u)| u.visible()).map(|(i, _)| i).collect()
}

pub fn menu_window_open() -> bool {
    use eldenring::cs::CSMenuManImp;
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEEN: AtomicU64 = AtomicU64::new(0);
    let Ok(mm) = (unsafe { <CSMenuManImp as fromsoftware_shared::FromStatic>::instance() }) else { return false };
    let mut open = false;
    for (i, u) in mm.ui_states.iter().enumerate() {
        if !u.visible() {
            continue;
        }
        if i < 64 && SEEN.fetch_or(1 << i, Ordering::Relaxed) & (1 << i) == 0 {
            log::info!("menu ui element {i} visible");
        }
        open |= (28..=65).contains(&i);
    }
    open
}

/// A loading screen is up. CSNowLoadingHelper's bytes +0xec / +0xed are both 1 in play and drop to 0
/// for the whole loading screen (a waygate: 0 from the start of the load to the arrival; the
/// first load after starting the game too) - found by logging its bytes across a waygate load, 2026-10-07. The fade
/// plates alone missed it: the loading picture isn't a fade, so the Doom HUD stayed up (user).
pub fn now_loading() -> bool {
    let Ok(nl) = (unsafe { <eldenring::cs::CSNowLoadingHelper as fromsoftware_shared::FromStatic>::instance() }) else {
        return false;
    };
    let p = nl as *const _ as *const u8;
    let (a, b) = unsafe { (*p.add(0xec), *p.add(0xed)) };
    a == 0 || b == 0
}

/// True while any fade plate covers the screen (loading, warping, death, cutscene transitions).
pub fn screen_faded() -> bool {
    let Ok(fade) = (unsafe { <eldenring::cs::CSFade as fromsoftware_shared::FromStatic>::instance() }) else {
        return true;
    };
    fade.fade_plates.iter().any(|p| p.current_color.a > 0.05)
}

/// The player is in normal control: idle / locomotion rather than a scripted get-up, item pickup
/// or event animation. ER uses anim id 0 for the base locomotion state.
pub fn has_control(chr: &ChrIns) -> bool {
    let a = crate::slayer::current_anim(chr);
    a == 0 || a == -1
}

/// Fire a Havok behavior event on a character (e.g. "W_DamageLv2_Middle": its hit reaction).
/// Function from The Grand Archives' ER table (PlayAnimation_code: AOB - 0xD), called with the
/// character's hkbCharacter and a UTF-16 event name. Returns false if the event isn't in its graph.
pub fn behavior_event(chr: &ChrIns, event: &str) -> bool {
    use std::sync::OnceLock;
    static FN: OnceLock<Option<usize>> = OnceLock::new();
    let f = FN.get_or_init(|| {
        let sig = crate::autoloot::parse_sig("74 ?? 48 85 d2 74 ?? 48 8d 4c 24 50");
        let hit = crate::autoloot::scan_step(&sig, 1).map(|p| p as usize - 0xD);
        log::info!("behavior event fn: {hit:x?}");
        hit
    });
    let Some(f) = *f else { return false };
    unsafe {
        let behavior = &*chr.modules.behavior as *const _ as *const u8;
        let unk = (behavior.add(0x10) as *const usize).read();
        if unk == 0 {
            return false;
        }
        let hkb = unk + 0x30;
        let wide: Vec<u16> = event.encode_utf16().chain(std::iter::once(0)).collect();
        let call: extern "C" fn(usize, *const u16) -> u32 = std::mem::transmute(f);
        call(hkb, wide.as_ptr()) != u32::MAX
    }
}

/// True while an in-engine cutscene ("remo") plays (same test erfps2 uses to drop first person).
pub fn in_cutscene() -> bool {
    unsafe { <eldenring::cs::CSRemo as fromsoftware_shared::FromStatic>::instance().ok() }
        .and_then(|r| r.remo_man.as_ref())
        .is_some_and(|m| m.state != 1)
}

/// Characters the game itself shows a boss health bar for (the fight has started).
pub fn active_boss_handles() -> Vec<FieldInsHandle> {
    let Ok(fe) = (unsafe { <eldenring::cs::CSFeManImp as fromsoftware_shared::FromStatic>::instance() }) else {
        return Vec::new();
    };
    fe.boss_health_displays.iter().filter(|b| b.fmg_id > 0).map(|b| b.field_ins_handle).collect()
}

/// Bosses the game is showing a bar for: (name from NpcName FMG, hp, max hp). Read live.
pub fn boss_bars() -> Vec<(String, i32, i32)> {
    let Ok(fe) = (unsafe { <eldenring::cs::CSFeManImp as fromsoftware_shared::FromStatic>::instance() }) else {
        return Vec::new();
    };
    let msg = unsafe { <eldenring::cs::MsgRepository as fromsoftware_shared::FromStatic>::instance().ok() };
    let enemies = enemies(500.0);
    let mut out = Vec::new();
    for b in fe.boss_health_displays.iter().filter(|b| b.fmg_id > 0) {
        let Some(e) = enemies.iter().find(|e| e.chr.field_ins_handle == b.field_ins_handle) else { continue };
        // NpcName is FMG category 18 in Elden Ring's item msgbnd.
        let name = msg
            .and_then(|m| m.get_msg(18, b.fmg_id as u32))
            .map(|w| String::from_utf16_lossy(w).trim_end_matches('\0').to_string())
            .filter(|n| !n.is_empty())
            .unwrap_or_default();
        out.push((name, e.hp(), e.max_hp()));
    }
    out
}

/// Find every FMG text equal to `text`: (category, index, msg id is not tracked).
pub fn find_msg(text: &str) -> Vec<(u32, u32)> {
    let Ok(m) = (unsafe { <eldenring::cs::MsgRepository as fromsoftware_shared::FromStatic>::instance() }) else { return Vec::new() };
    let want: Vec<u16> = text.encode_utf16().collect();
    let mut out = Vec::new();
    for cat in 0..m.file_capacity {
        let Some(f) = m.get_file(cat) else { continue };
        for i in 0..f.msg_count {
            if f.msg_by_index(i).is_some_and(|w| w == want.as_slice()) {
                out.push((cat, i));
            }
        }
    }
    out
}

/// Point an FMG entry (by index) at our own text (any length; leaked on purpose, lives forever).
pub fn set_msg_by_index(cat: u32, index: u32, text: &str) -> bool {
    let Ok(m) = (unsafe { <eldenring::cs::MsgRepository as fromsoftware_shared::FromStatic>::instance() }) else { return false };
    let Some(f) = m.get_file(cat) else { return false };
    if index >= f.msg_count || f.msg_offsets.is_null() {
        return false;
    }
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let buf: &'static [u16] = Box::leak(wide.into_boxed_slice());
    let header = f.header() as *const _ as isize;
    let off = buf.as_ptr() as isize - header;
    unsafe { *f.msg_offsets.add(index as usize) = std::num::NonZero::new(off as i64) };
    true
}


/// (debug) Prop watcher: loose type-6 objects around `center` (found by a ring of short casts)
/// and every change to them - state, class (vtable), handle - logged with time. For comparing how
/// Elden Ring's own attacks break pots / skulls / rocks in one hit with how ours need two.
pub fn prop_watch(center: Vec3) {
    use std::{collections::HashMap, sync::Mutex};
    struct W {
        vt: usize,
        state: u32,
        sel: u32,
        name: String,
    }
    static SEEN: Mutex<Option<HashMap<usize, W>>> = Mutex::new(None);
    let Ok(mut g) = SEEN.lock() else { return };
    let seen = g.get_or_insert_with(HashMap::new);
    let base = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }.map(|m| m.0 as usize).unwrap_or(0);
    // discover
    for k in 0..32 {
        let a = k as f32 / 32.0 * std::f32::consts::TAU;
        let dir = Vec3::new(a.cos(), -0.05, a.sin());
        for h in [0.2f32, 0.7] {
            let hit = crate::raycast::cast_sphere(center + Vec3::Y * h, dir * 6.0, 0.1, 0x2000058, |x| {
                x.field_ins_handle().is_some_and(|f| f.selector.field_ins_type() == Some(eldenring::cs::FieldInsType::ReplayEnemy) && f.selector.index() >= 20000)
            });
            if let Some(hit) = hit {
                if let Some(fi) = hit.field_ins() {
                    let ptr = fi.as_ptr() as usize;
                    if !seen.contains_key(&ptr) {
                        let vt = unsafe { *(ptr as *const usize) };
                        let gref = unsafe { &*(ptr as *const eldenring::cs::CSWorldGeomIns) };
                        let name = if geom_vtables().contains(&vt) { read_geom(gref).name } else { String::new() };
                        let state = geom_state(ptr);
                        let sel = unsafe { *((ptr + 0xc) as *const u32) };
                        log::info!("prop watch: + {name} sel {sel:#x} class {:#x} state {state}", vt.wrapping_sub(base));
                        seen.insert(ptr, W { vt, state, sel, name });
                    }
                }
            }
        }
    }
    // changes
    let mut gone = Vec::new();
    for (ptr, w) in seen.iter_mut() {
        let vt = unsafe { *(*ptr as *const usize) };
        let sel = unsafe { *((*ptr + 0xc) as *const u32) };
        if vt != w.vt || sel != w.sel {
            log::info!("prop watch: {} replaced/freed (class {:#x} -> {:#x}, sel {:#x} -> {sel:#x})", w.name, w.vt.wrapping_sub(base), vt.wrapping_sub(base), w.sel);
            gone.push(*ptr);
            continue;
        }
        let state = geom_state(*ptr);
        if state != w.state {
            log::info!("prop watch: {} state {} -> {state}", w.name, w.state);
            w.state = state;
        }
    }
    for p in gone {
        seen.remove(&p);
    }
    if seen.len() > 200 {
        seen.clear();
    }
}
