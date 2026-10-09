//! Agent test bridge (oracle). Every 0.25 s writes `doomslayer_state.json` next to the DLL and runs
//! any lines found in `doomslayer_cmd.txt` (then deletes it). Lets an agent fire, dash, spawn and
//! read HP numbers without a human at the keyboard.

use std::fmt::Write as _;

use serde_json::json;

use crate::{
    config,
    game::{self},
    params,
    slayer::{Slayer, is_hostile},
    weapons::WEAPONS,
};

static mut ACC: f32 = 0.0;

pub fn tick(s: &mut Slayer, dt: f32) {
    if !s.cfg.bridge {
        return;
    }
    unsafe {
        ACC += dt;
        if ACC < 0.25 {
            return;
        }
        ACC = 0.0;
    }
    let dir = config::mod_dir();
    let cmd_path = dir.join("doomslayer_cmd.txt");
    if let Ok(text) = std::fs::read_to_string(&cmd_path) {
        let _ = std::fs::remove_file(&cmd_path);
        let mut out = String::new();
        for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
            let r = run(s, line);
            log::info!("bridge> {line} => {r}");
            let _ = writeln!(out, "{line} => {r}");
        }
        let _ = std::fs::write(dir.join("doomslayer_cmd.out"), out);
    }
    // Write-then-rename so readers never see a half-written file.
    let tmp = dir.join("doomslayer_state.json.tmp");
    if std::fs::write(&tmp, state(s).to_string()).is_ok() {
        let _ = std::fs::rename(&tmp, dir.join("doomslayer_state.json"));
    }
}

fn state(s: &Slayer) -> serde_json::Value {
    let player = game::player().map(|p| {
        let d = &p.chr_ins.modules.data;
        let ph = &p.chr_ins.modules.physics;
        json!({
            "hp": d.hp, "max_hp": d.max_hp, "fp": d.fp, "stamina": d.stamina,
            "pos": [ph.position.0, ph.position.1, ph.position.2],
            "on_ground": ph.is_touching_ground,
            "team": p.chr_ins.team_type,
            "stats": unsafe { let g = p.player_game_data.as_ref(); [g.level, g.vigor, g.strength, g.dexterity] },
            "rh_weapon": unsafe { format!("{:?}", p.player_game_data.as_ref().equipment.equipment_entries.weapon_primary_right) },
            "anim": crate::slayer::current_anim(&p.chr_ins),
            "chunk": [p.chr_ins.chunk_position.0, p.chr_ins.chunk_position.1, p.chr_ins.chunk_position.2],
            "model": model_pos(&p.chr_ins),
            "block": format!("{:?}", p.chr_ins.block_id),
        })
    });
    let cam = game::camera().map(|(p, f)| json!({"pos": [p.x, p.y, p.z], "fwd": [f.x, f.y, f.z]}));
    let enemies: Vec<_> = game::enemies(150.0)
        .iter()
        .take(12)
        .map(|e| {
            json!({
                "npc_param": e.chr.npc_param_id, "chr_id": e.chr.character_id,
                "team": e.chr.team_type, "hostile": is_hostile(e), "player_type": game::player_type_of(e.chr).is_some(),
                "hp": e.hp(), "max_hp": e.max_hp(), "dist": e.dist,
                "staggered": s.is_staggered(e),
                "pos": [e.pos().x, e.pos().y, e.pos().z],
                "anim": crate::slayer::current_anim(e.chr),
                "poise": [e.chr.modules.super_armor.sa_durability, e.chr.modules.super_armor.sa_durability_max],
                "poise_broken": e.chr.modules.super_armor.poise_broken_state,
                "chunk": [e.chr.chunk_position.0, e.chr.chunk_position.1, e.chr.chunk_position.2],
                "model": model_pos(e.chr),
                "block": format!("{:?}", e.chr.block_id),
            })
        })
        .collect();
    json!({
        "t": s.time,
        "params_applied": params::applied(),
        "input_active": s.input.active,
        "weapon": WEAPONS[s.weapon].name,
        "ammo": s.ammo, "armor": s.armor,
        "dash_charges": s.dash_charges, "chainsaw_fuel": s.chainsaw_fuel, "belch_cd": s.belch_cd,
        "combat_age": s.time - s.last_combat,
        "kills": s.kills, "glory_kills": s.glory_kills, "shots": s.shots,
        "blood_punch": s.blood_punch, "pickups": s.pickups.list.len(), "menu_t": s.menu_t, "menu_open": game::menu_open(),
        "ui": game::menu_ui_visible(),
        "pool": crate::hitter::pool_used(),
        "scan": [game::ENEMY_SCAN_NS.load(std::sync::atomic::Ordering::Relaxed), game::ENEMY_SCAN_CALLS.load(std::sync::atomic::Ordering::Relaxed)],
        "vm": [s.vm_clip.clone(), format!("{:.2}", s.vm_t), format!("{:.1}", s.speed)],
        "messages": s.messages.iter().map(|(m, _)| m.clone()).collect::<Vec<_>>(),
        "player": player, "camera": cam, "enemies": enemies,
    })
}

/// The weapon is frozen on screen (bridge `freeze`).
static FROZE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn run(s: &mut Slayer, line: &str) -> String {
    let mut it = line.split_whitespace();
    let cmd = it.next().unwrap_or_default();
    let args: Vec<&str> = it.collect();
    let num = |i: usize| args.get(i).and_then(|a| a.parse::<f64>().ok());
    let k = s.cfg.keys.clone();
    match cmd {
        "fire" => format!("fired={}", s.fire()),
        // fire_ground [dist]: shoot the ground `dist` m ahead (impact visuals).
        "fire_ground" => {
            let Some((_, fwd)) = game::camera() else { return "no camera".into() };
            let d = num(0).unwrap_or(5.0) as f32;
            let flat = glam::Vec3::new(fwd.x, 0.0, fwd.z).normalize_or(glam::Vec3::X);
            s.fire_cd = 0.0;
            format!("fired={}", s.fire_dir(Some((flat * d - glam::Vec3::Y * 1.6).normalize())))
        }
        // Fire at the nearest hostile's chest regardless of where the camera looks.
        "lookat" => {
            // What's under the crosshair: every body along the camera ray (type, asset name, hp).
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let me = game::player().map(|p| p.chr_ins.field_ins_handle);
            let mut out = Vec::new();
            let mut from = cam;
            for _ in 0..4 {
                let Some(h) = crate::raycast::cast_sphere(from, fwd * 40.0, 0.05, 0x2000058, |x| me.is_none() || x.field_ins_handle() != me) else { break };
                let fh = h.field_ins_handle();
                let kind = fh.map(|f| format!("{:?}", f.selector.field_ins_type())).unwrap_or("map".into());
                let geom = fh.and_then(|f| game::geom_by_handle(&f)).map(|g| format!("{} hp {} def {}", g.name, g.hp, g.defense)).unwrap_or_default();
                let at = from + fwd * 40.0 * h.segment;
                out.push(format!("{kind} {geom} at {:.1} m", at.distance(cam)));
                from = at + fwd * 0.3;
            }
            out.join("
")
        }
        "myfilter" => {
            // The collision filter of the player's own body: cast at ourselves from 2 m away and
            // keep only our own capsule.
            let Some(p) = game::player() else { return "no player".into() };
            let me = p.chr_ins.field_ins_handle;
            let chest = game::chr_pos(&p.chr_ins) + glam::Vec3::Y;
            let mut out = Vec::new();
            for f in [0x2000058u32, 0xffffffff, 0] {
                for dir in [glam::Vec3::X, glam::Vec3::Z, glam::Vec3::Y] {
                    let from = chest + dir * 2.0;
                    if let Some(h) = crate::raycast::cast_sphere(from, -dir * 2.5, 0.05, f, |x| x.field_ins_handle() == Some(me)) {
                        out.push(format!("query {f:#x} from {dir:?}: own body filter {:#x}", h.filter));
                        break;
                    }
                }
            }
            if out.is_empty() { "own body not hit".into() } else { out.join("; ") }
        }
        "castf" => {
            // castf FILTER [radius]: cast along the view with a filter, name what it hits.
            let filter = args.first().and_then(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok()).unwrap_or(0x2000058);
            let r = num(1).unwrap_or(0.05) as f32;
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let me = game::player().map(|p| p.chr_ins.field_ins_handle);
            match crate::raycast::cast_sphere(cam, fwd * 30.0, r, filter, |x| me.is_none() || x.field_ins_handle() != me) {
                Some(h) => {
                    let t = h.field_ins_handle().map(|f| f.selector.field_ins_type());
                    let g = game::geom_by_hit(&h).map(|g| format!("{} state {}", g.name, game::geom_state(g.ptr)));
                    format!("filter {filter:#x}: {:.2} m {t:?} {g:?} body filter {:#x}", h.segment * 30.0, h.filter)
                }
                None => format!("filter {filter:#x}: no hit"),
            }
        }
        "whatis" => {
            // whatis: the first thing along the view - field type, handle, class (vtable RVA) and
            // any loaded asset sharing its index (runtime-created objects the asset lookup misses).
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let me = game::player().map(|p| p.chr_ins.field_ins_handle);
            let Some(h) = crate::raycast::cast_sphere(cam, fwd * 30.0, 0.05, 0x2000058, |x| me.is_none() || x.field_ins_handle() != me) else { return "no hit".into() };
            let Some(fi) = h.field_ins() else { return format!("hit at {:.2} m without an owner", h.segment * 30.0) };
            let base = unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) }.map(|m| m.0 as usize).unwrap_or(0);
            let vt = unsafe { *(fi.as_ptr() as *const usize) };
            let fh = unsafe { fi.as_ref().handle };
            let mut same = Vec::new();
            if let Ok(gm) = unsafe { <eldenring::cs::CSWorldGeomMan as fromsoftware_shared::FromStatic>::instance() } {
                for pair in gm.blocks.iter() {
                    for g in pair.second.geom_ins_vector.iter() {
                        if g.field_ins_handle.selector.index() == fh.selector.index() {
                            let name = unsafe { g.info.msb_parts_geom.msb_parts.msb_part.name.to_string() }.unwrap_or_default();
                            same.push(format!("{name} (block {:?})", g.field_ins_handle.block_id.0));
                        }
                    }
                }
            }
            format!(
                "{:.2} m type {:?} block {} selector {:#x} index {} vtable rva {:#x} ptr {:#x}; same-index assets: {:?}",
                h.segment * 30.0, fh.selector.field_ins_type(), fh.block_id.0, fh.selector.0, fh.selector.index(),
                vt.wrapping_sub(base), fi.as_ptr() as usize, same
            )
        }
        "assetparam" => {
            // assetparam ID [ID ...]: AssetEnvironmentGeometryParam rows by asset number
            // (AEG099_200 -> 99200, AEG800_016 -> 800016), written to doomslayer_assetparam.txt.
            use eldenring::cs::{AssetEnvironmentGeometryParam, SoloParamRepository};
            use fromsoftware_shared::FromStatic;
            let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else { return "no params".into() };
            let mut out = Vec::new();
            for a in &args {
                let Ok(id) = a.parse::<u32>() else { continue };
                match repo.get::<AssetEnvironmentGeometryParam>(id) {
                    Some(r) => out.push(format!("{id}: {r:?}")),
                    None => out.push(format!("{id}: no row")),
                }
            }
            let _ = std::fs::write(crate::config::mod_dir().join("doomslayer_assetparam.txt"), out.join("

"));
            format!("{} rows written", out.len())
        }
        "lots" => {
            // lots: the asset lotteries that spawn loose props at runtime (EnvObjLotParam,
            // RollingObjLotParam): every row with its asset ids and weights.
            use eldenring::cs::{EnvObjLotParam, RollingObjLotParam, SoloParamRepository};
            use fromsoftware_shared::FromStatic;
            let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else { return "no params".into() };
            let mut out = Vec::new();
            for i in 0..2000 {
                let Some(r) = repo.get_row_by_index::<EnvObjLotParam>(i) else { break };
                let ids = [r.asset_id_0(), r.asset_id_1(), r.asset_id_2(), r.asset_id_3(), r.asset_id_4(), r.asset_id_5(), r.asset_id_6(), r.asset_id_7()];
                let w = [r.create_weight_0(), r.create_weight_1(), r.create_weight_2(), r.create_weight_3(), r.create_weight_4(), r.create_weight_5(), r.create_weight_6(), r.create_weight_7()];
                let v: Vec<String> = ids.iter().zip(w).filter(|(id, _)| **id >= 0).map(|(id, w)| format!("{id}x{w}")).collect();
                out.push(format!("env[{i}] {}", v.join(" ")));
            }
            for i in 0..2000 {
                let Some(r) = repo.get_row_by_index::<RollingObjLotParam>(i) else { break };
                let ids = [r.asset_id_0(), r.asset_id_1(), r.asset_id_2(), r.asset_id_3(), r.asset_id_4(), r.asset_id_5(), r.asset_id_6(), r.asset_id_7()];
                let w = [r.create_weight_0(), r.create_weight_1(), r.create_weight_2(), r.create_weight_3(), r.create_weight_4(), r.create_weight_5(), r.create_weight_6(), r.create_weight_7()];
                let v: Vec<String> = ids.iter().zip(w).filter(|(id, _)| **id >= 0).map(|(id, w)| format!("{id}x{w}")).collect();
                out.push(format!("rolling[{i}] {}", v.join(" ")));
            }
            let _ = std::fs::write(crate::config::mod_dir().join("doomslayer_lots.txt"), out.join("
"));
            format!("{} rows written to doomslayer_lots.txt", out.len())
        }
        "geomnear" => {
            // Every map asset near the player, breakable or not (finding explosive barrels etc).
            let list = game::assets_near(num(0).unwrap_or(12.0) as f32, false);
            let rows: Vec<String> = list.iter().take(60).map(|(n, hp, d)| format!("{n} hp {hp} d {:.1}", d.length())).collect();
            rows.join("
")
        }
        "breakdef" => {
            // Breakable props near the player with their defense (what object attack they need).
            use eldenring::cs::CSWorldGeomMan;
            use fromsoftware_shared::FromStatic;
            let Ok(gm) = (unsafe { CSWorldGeomMan::instance() }) else { return "no geom man".into() };
            let near: Vec<String> = game::breakables_near(num(0).unwrap_or(40.0) as f32).into_iter().map(|(n, _, _)| n).collect();
            let mut out = Vec::new();
            for pair in gm.blocks.iter() {
                for g in pair.second.geom_ins_vector.iter() {
                    let name = unsafe { g.info.msb_parts_geom.msb_parts.msb_part.name.to_string() }.unwrap_or_default();
                    if near.contains(&name) {
                        let p = unsafe { g.info.asset_geometry_param.as_ref() };
                        out.push(format!("{name} hp {} def {} state {}", p.hp(), p.defense(), game::geom_state(&**g as *const _ as usize)));
                    }
                }
            }
            out.sort();
            out.join("
")
        }
        "breaknear" => {
            // Nearest breakable assets: name, hp, offset (m) from the player.
            let mut list = game::breakables_near(num(0).unwrap_or(400.0) as f32);
            if let Some(prefix) = args.get(1) {
                // "props": skip plants (AEG7xx flowers, AEG8xx bushes, AEG001 small plants)
                if *prefix == "props" {
                    list.retain(|(n, _, _)| !["AEG7", "AEG8", "AEG001"].iter().any(|v| n.contains(v)));
                } else {
                    list.retain(|(n, _, _)| n.contains(prefix));
                }
            }
            let rows: Vec<String> = list
                .iter()
                .take(25)
                .enumerate()
                .map(|(i, (n, hp, d))| format!("{i}: {n} hp {hp} d {:.0} m offset [{:.1}, {:.1}, {:.1}]", d.length(), d.x, d.y, d.z))
                .collect();
            format!("{} breakables
{}", list.len(), rows.join("
"))
        }
        "breaktest" => {
            // breaktest NAME BULLET_ID: fire a bullet row straight into a prop (aim searched like
            // breakshoot) and report its runtime state before / after.
            let (Some(want), Some(bid)) = (args.first(), num(1)) else { return "breaktest NAME BULLET".into() };
            let list = game::assets_near(2000.0, false);
            let Some((name, _, d)) = list.iter().find(|(n, _, _)| n.contains(want)) else { return "no such asset".into() };
            let Some(p) = game::player() else { return "no player".into() };
            let Some((cam, _)) = game::camera() else { return "no camera".into() };
            let Some(ptr) = game::geom_ptr_by_name(name) else { return "no ptr".into() };
            let before = game::geom_state(ptr);
            let base = game::chr_pos(&p.chr_ins) + *d;
            let mut aim = None;
            'search: for h in [0.3f32, 0.6, 0.9, 0.15, 1.2, 1.6, 0.0] {
                for side in [0.0f32, 0.3, -0.3, 0.6, -0.6] {
                    let to = base + glam::Vec3::Y * h;
                    let right = (to - cam).cross(glam::Vec3::Y).normalize_or_zero();
                    let dir = (to + right * side - cam).normalize();
                    let hit = crate::damage::trace(cam, dir, 60.0, 0.05);
                    if hit.geom.and_then(|g| game::geom_by_handle(&g)).is_some_and(|g| g.name == *name) {
                        aim = Some((dir, hit.pos));
                        break 'search;
                    }
                }
            }
            let Some((dir, at)) = aim else { return format!("{name}: no clear line (state {before})") };
            let owner = Some(p.chr_ins.field_ins_handle);
            let r = crate::bullet::spawn(owner, bid as i32, at - dir * 0.6, dir);
            format!("{name}: state {before}, spawned {bid} {r:?} (check state again)")
        }
        "geomparam" => {
            // geomparam NAME: the asset's whole AssetGeometryParam row (what it takes to break it).
            let Some(want) = args.first() else { return "geomparam NAME".into() };
            let Some(ptr) = game::geom_ptr_by_name(want) else { return "not found".into() };
            let g = unsafe { &*(ptr as *const eldenring::cs::CSWorldGeomIns) };
            let p = unsafe { g.info.asset_geometry_param.as_ref() };
            format!("{want} state {}: {p:?}", game::geom_state(ptr))
        }
        "geomstate" => {
            let Some(want) = args.first() else { return "geomstate NAME".into() };
            match game::geom_ptr_by_name(want) {
                Some(p) => format!("{want}: state {}", game::geom_state(p)),
                None => "not found".into(),
            }
        }
        "breakshoot" => {
            // Fire the current gun straight at a breakable by name; report what the ray meets.
            let Some(want) = args.first() else { return "breakshoot NAME".into() };
            let list = game::breakables_near(2000.0);
            let Some((name, _, d)) = list.iter().find(|(n, _, _)| n.contains(want)) else { return "gone (no such breakable)".into() };
            let Some(p) = game::player() else { return "no player".into() };
            let Some((cam, _)) = game::camera() else { return "no camera".into() };
            // Search aim points around the prop until the ray lands on it (origins sit at the
            // base / back of many props).
            let base = game::chr_pos(&p.chr_ins) + *d;
            let mut aim = None;
            'search: for h in [0.3f32, 0.6, 0.9, 0.15, 1.2, 1.6, 0.0] {
                for side in [0.0f32, 0.3, -0.3, 0.6, -0.6] {
                    let to = base + glam::Vec3::Y * h;
                    let right = (to - cam).cross(glam::Vec3::Y).normalize_or_zero();
                    let dir = (to + right * side - cam).normalize();
                    let hit = crate::damage::trace(cam, dir, 60.0, 0.05);
                    if hit.geom.and_then(|g| game::geom_by_handle(&g)).is_some_and(|g| g.name == *name) {
                        aim = Some((dir, hit.dist));
                        break 'search;
                    }
                }
            }
            let Some((dir, dist)) = aim else { return format!("{name}: no clear line to it") };
            s.fire_cd = 0.0;
            let ok = s.fire_dir(Some(dir));
            format!("{name}: fired={ok}, ray on target at {dist:.1} m")
        }
        "poke" => {
            // poke ADDR_HEX VALUE_HEX: write a u32 (reverse engineering only).
            let (Some(a), Some(v)) = (args.first(), args.get(1)) else { return "poke ADDR VAL".into() };
            let (Ok(a), Ok(v)) = (usize::from_str_radix(a.trim_start_matches("0x"), 16), u32::from_str_radix(v.trim_start_matches("0x"), 16)) else { return "bad hex".into() };
            let p = a as *mut u32;
            let old = unsafe { p.read_unaligned() };
            unsafe { p.write_unaligned(v) };
            format!("{a:#x}: {old:#x} -> {v:#x}")
        }
        "hitrec" => {
            // hitrec BULLET_ID [seconds]: read / set a bullet row's dmg hit record lifetime.
            use eldenring::cs::{Bullet, SoloParamRepository};
            use fromsoftware_shared::FromStatic;
            let Ok(repo) = (unsafe { SoloParamRepository::instance_mut() }) else { return "no params".into() };
            let id = num(0).unwrap_or(crate::params::FX_SILENT_ROUND as f64) as u32;
            let Some(b) = repo.get_mut::<Bullet>(id) else { return "no row".into() };
            let old = (b.dmg_hit_record_life_time(), b.is_use_shared_hit_list());
            if let Some(v) = num(1) {
                b.set_dmg_hit_record_life_time(v as f32);
            }
            format!("bullet {id}: hit record life {:?} (shared list {}) -> {}", old.0, old.1, b.dmg_hit_record_life_time())
        }
        "sfxlist" => {
            // Dump the bullet-sfx list (CSBulletManager +0x20: buffer ptr, head, empty, count):
            // each entry's first 0x60 bytes as u64s, to a file per entry.
            let Ok(man) = (unsafe { <eldenring::cs::CSBulletManager as fromsoftware_shared::FromStatic>::instance() }) else { return "no man".into() };
            let base = man as *const _ as *const u8;
            let rd = |o: usize| unsafe { (base.add(o) as *const usize).read_unaligned() };
            let (buf, head, empty) = (rd(0x20), rd(0x28), rd(0x30));
            let count = unsafe { (base.add(0x38) as *const u32).read_unaligned() };
            let mut out = vec![format!("buf {buf:#x} head {head:#x} empty {empty:#x} alloc {count}")];
            let mut p = head;
            for i in 0..num(0).unwrap_or(4.0) as usize {
                if p == 0 { break; }
                let bytes = unsafe { std::slice::from_raw_parts(p as *const u8, 0x9d0) };
                let _ = std::fs::write(crate::config::mod_dir().join(format!("sfx_{i}.bin")), bytes);
                let in_buf = p >= buf && p < buf + 64 * 0x9d0;
                out.push(format!("{i}: {p:#x} in_buf {in_buf}"));
                // next pointer: first u64 in the entry that points into the buffer or looks like a heap entry
                let next = (0..0x9d0 / 8).map(|k| unsafe { (p as *const usize).add(k).read_unaligned() }).find(|v| *v != p && (*v >= buf && *v < buf + 64 * 0x9d0));
                p = next.unwrap_or(0);
            }
            out.join("
")
        }
        "geomwrite" => {
            // geomwrite NAME OFFSET_HEX VALUE_HEX: poke a u32 into an asset instance (RE only).
            let (Some(want), Some(off), Some(val)) = (args.first(), args.get(1), args.get(2)) else { return "geomwrite NAME OFF VAL".into() };
            let (Ok(off), Ok(val)) = (usize::from_str_radix(off.trim_start_matches("0x"), 16), u32::from_str_radix(val.trim_start_matches("0x"), 16)) else { return "bad hex".into() };
            let Some(ptr) = game::geom_ptr_by_name(want) else { return "not found".into() };
            let p = (ptr + off) as *mut u32;
            let old = unsafe { p.read_unaligned() };
            unsafe { p.write_unaligned(val) };
            format!("{want} +{off:#x}: {old:#x} -> {val:#x}")
        }
        "geommem" => {
            // Dump 0x800 bytes of an asset instance to a file (find its runtime break state).
            let (Some(want), Some(path)) = (args.first(), args.get(1)) else { return "geommem NAME FILE".into() };
            let Some(ptr) = game::geom_ptr_by_name(want) else { return "not found".into() };
            let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, 0x800) };
            match std::fs::write(crate::config::mod_dir().join(path), bytes) {
                Ok(()) => format!("dumped {want} @ {ptr:#x}"),
                Err(e) => format!("write failed: {e}"),
            }
        }
        "breakprobe" => {
            // Which collision filter sees a breakable: cast at it through single filter bits.
            let Some(want) = args.first() else { return "breakprobe NAME".into() };
            let list = game::breakables_near(2000.0);
            let Some((name, _, d)) = list.iter().find(|(n, _, _)| n.contains(want)) else { return "no such breakable".into() };
            let Some(p) = game::player() else { return "no player".into() };
            let Some((cam, _)) = game::camera() else { return "no camera".into() };
            let target = game::chr_pos(&p.chr_ins) + *d + glam::Vec3::Y * num(1).unwrap_or(0.3) as f32;
            let v = (target - cam) * 1.3;
            let me = p.chr_ins.field_ins_handle;
            let mut out = vec![format!("{name} at {:.1} m", (target - cam).length())];
            for bit in 0..32u32 {
                let f = 1u32 << bit;
                let Some(h) = crate::raycast::cast_sphere(cam, v, 0.05, f, |h| h.field_ins_handle() != Some(me)) else { continue };
                let who = match h.field_ins_handle() {
                    Some(fh) => format!("{:?} {}", fh.selector.field_ins_type(), game::geom_by_handle(&fh).map(|g| g.name).unwrap_or_default()),
                    None => "no-owner".into(),
                };
                out.push(format!("bit {bit} ({f:#x}): {who} at {:.1} m", h.segment * v.length()));
            }
            out.join("
")
        }
        "breaktp" => {
            // Stand 2.5 m from a breakable by name (from breaknear).
            let Some(want) = args.first() else { return "breaktp NAME".into() };
            let list = game::breakables_near(2000.0);
            let Some((name, _, d)) = list.iter().find(|(n, _, _)| n.contains(want)) else { return "no such breakable".into() };
            let Some(p) = game::player() else { return "no player".into() };
            // Stand back along the current view direction so the prop is straight ahead.
            let flat = match game::camera() {
                Some((_, f)) => glam::Vec3::new(f.x, 0.0, f.z),
                None => glam::Vec3::new(d.x, 0.0, d.z),
            };
            let stand = game::chr_pos(&p.chr_ins) + *d - flat.normalize_or_zero() * 3.0 + glam::Vec3::Y * 0.5;
            let ph = &mut p.chr_ins.modules.physics;
            ph.position.0 = stand.x;
            ph.position.1 = stand.y;
            ph.position.2 = stand.z;
            ph.chr_proxy_pos_update_requested = true;
            format!("tp next to {name} ({:.0} m away)", d.length())
        }
        "rows" => {
            // Which Bullet / AtkParam_Pc rows exist in a range (finding free rows to take over).
            use eldenring::cs::{AtkParam_Pc, Bullet, SoloParamRepository};
            use fromsoftware_shared::FromStatic;
            let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else { return "no params".into() };
            let (a0, n) = (num(1).unwrap_or(0.0) as u32, num(2).unwrap_or(50.0) as u32);
            let have: Vec<u32> = (a0..a0 + n)
                .filter(|id| if args.first() == Some(&"bullet") { repo.get::<Bullet>(*id).is_some() } else { repo.get::<AtkParam_Pc>(*id).is_some() })
                .collect();
            format!("{have:?}")
        }
        "geomdump" => {
            // Find where a map asset's position lives: the MSB part memory of the asset the
            // camera ray hits, as floats, plus the player's block-vs-havok offset.
            let Some((cam, _)) = game::camera() else { return "no camera".into() };
            let Some(me) = game::player() else { return "no player".into() };
            let skip = num(0).unwrap_or(0.0) as usize;
            let hit = (0..120)
                .map(|k| {
                    let a = k as f32 * std::f32::consts::TAU / 120.0;
                    crate::damage::trace(cam, glam::Vec3::new(a.cos(), -0.15, a.sin()), 60.0, 0.05)
                })
                .filter(|h| h.geom.is_some())
                .nth(skip);
            let Some(hit) = hit else { return "no geom".into() };
            let g = hit.geom.unwrap();
            let Some(ptr) = game::geom_part_ptr(&g) else { return "no part".into() };
            let hp = game::chr_pos(&me.chr_ins);
            let bp = me.block_position;
            let fl: Vec<String> = (0..48).map(|i| format!("{:.1}", unsafe { (ptr as *const f32).add(i).read_unaligned() })).collect();
            format!(
                "{} hit havok [{:.1},{:.1},{:.1}] player havok [{:.1},{:.1},{:.1}] block [{:.1},{:.1},{:.1}]
{}",
                game::geom_by_handle(&g).map(|i| i.name).unwrap_or_default(),
                hit.pos.x, hit.pos.y, hit.pos.z, hp.x, hp.y, hp.z, bp.x, bp.y, bp.z, fl.join(" ")
            )
        }
        "fire_down" => {
            // Fire the current gun at the ground `d` m ahead of the player (breakable tests);
            // also report what the ray meets there.
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let Some(me) = game::player() else { return "no player".into() };
            let d = num(0).unwrap_or(3.0) as f32;
            let flat = glam::Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
            let target = game::chr_pos(&me.chr_ins) + flat * d + glam::Vec3::Y * num(1).unwrap_or(0.3) as f32;
            let dir = (target - cam).normalize();
            let hit = crate::damage::trace(cam, dir, 40.0, 0.05);
            let what = hit
                .geom
                .and_then(|g| game::geom_by_handle(&g))
                .map(|g| format!("{} hp {}", g.name, g.hp))
                .unwrap_or_else(|| format!("chr {}", hit.chr.is_some()));
            s.fire_cd = 0.0;
            let ok = s.fire_dir(Some(dir));
            format!("fired={ok} ray hits {what} at {:.1} m", hit.dist)
        }
        "fire_at" => {
            let Some((cam, _)) = game::camera() else { return "no camera".into() };
            let want = num(0).map(|v| v as i32);
            match game::enemies(150.0)
                .into_iter()
                .filter(is_hostile)
                .find(|e| want.is_none_or(|w| e.chr.npc_param_id == w))
            {
                Some(e) => {
                    let before = e.hp();
                    let ptr = &*e.chr as *const eldenring::cs::ChrIns;
                    let dir = (e.pos() + glam::Vec3::Y * 1.0 - cam).normalize();
                    s.fire_cd = 0.0;
                    let ok = s.fire_dir(Some(dir));
                    // Damage is applied synchronously: read the same character right after.
                    let after = unsafe { (&(*ptr).modules).data.hp };
                    format!("fired={ok} at npc {} dist {:.1} hp_before {before} hp_after {after}", e.chr.npc_param_id, e.dist)
                }
                None => "no hostile".into(),
            }
        }
        "select" => {
            s.select(num(0).unwrap_or(4.0) as usize);
            WEAPONS[s.weapon].name.into()
        }
        // Synthetic key presses go through the same input path as the keyboard.
        "press" => {
            let vk = match args.first().copied().unwrap_or("") {
                "dash" => k.dash,
                "jump" => k.jump,
                "melee" => k.melee,
                "chainsaw" => k.chainsaw,
                "belch" => k.flame_belch,
                "fire" => k.fire,
                other => u16::from_str_radix(other.trim_start_matches("0x"), 16).unwrap_or(0),
            };
            s.input.inject(vk);
            format!("vk 0x{vk:02x}")
        }
        "ammo" => {
            for a in crate::weapons::Ammo::ALL {
                s.ammo[a.index()] = a.max();
            }
            "full".into()
        }
        // freeze: hold the weapon on screen as it is now (again: unfreeze); freeze <s>: the frozen
        // clip at another time; freeze [model] <clip> [s]: that clip (of that model: fists,
        // chainsaw, crucible or a gun folder) at that time; freeze off.
        "freeze" => {
            use std::sync::atomic::Ordering::Relaxed;
            let models = crate::viewmodel::FOLDERS.iter().copied().chain(["fists", crate::viewmodel::CHAINSAW_FOLDER, crate::viewmodel::CRUCIBLE_FOLDER]);
            let mut a = args.clone();
            let folder = a.first().and_then(|f| models.clone().find(|m| m == f));
            if folder.is_some() {
                a.remove(0);
            }
            let t = a.last().and_then(|v| v.parse::<f32>().ok());
            let clip = a.first().filter(|c| c.parse::<f32>().is_err()).copied();
            if a.first() == Some(&"off") || (args.is_empty() && FROZE.load(Relaxed)) {
                FROZE.store(false, Relaxed);
                crate::viewmodel::freeze(true, None, None, None)
            } else {
                FROZE.store(true, Relaxed);
                crate::viewmodel::freeze(false, folder, clip, if clip.is_some() { Some(t.unwrap_or(0.0)) } else { t })
            }
        }
        // noammo: empty every ammo pool (out-of-ammo chainsaw test).
        "noammo" => {
            s.ammo = [0; 5];
            "empty".into()
        }
        "fuel" => {
            s.chainsaw_fuel = num(0).unwrap_or(3.0) as f32;
            "ok".into()
        }
        // lab [0|1]: enemies can't move/attack, player can't die (debug flags).
        "lab" => {
            let on = num(0).map(|v| v != 0.0).unwrap_or(true);
            match unsafe { <eldenring::cs::WorldChrManDbgFlags as fromsoftware_shared::FromStatic>::instance_mut() } {
                Ok(f) => {
                    f.player_no_dead = on;
                    f.all_no_attack = on;
                    f.all_no_move = on;
                    if on {
                        crate::slayer::IGNORE_CURSOR.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    format!("lab={on}")
                }
                Err(e) => format!("no dbg flags: {e:?}"),
            }
        }
        "watch" => {
            s.watch_anims = num(0).unwrap_or(5.0) as f32;
            "watching anims".into()
        }
        "anim" => match game::player() {
            Some(p) => {
                p.chr_ins.modules.event.request_animation_id = num(0).unwrap_or(-1.0) as i32;
                "requested".into()
            }
            None => "no player".into(),
        },
        // hold <key> <0|1>: keep a synthetic key down (e.g. hold wheel 1, then hold wheel 0).
        "hold" => {
            let vk = match args.first().copied().unwrap_or("") {
                "wheel" => k.weapon_wheel,
                "fire" => k.fire,
                other => u16::from_str_radix(other.trim_start_matches("0x"), 16).unwrap_or(0),
            };
            let on = num(1).unwrap_or(1.0) != 0.0;
            s.input.hold(vk, on);
            format!("hold 0x{vk:02x} {on}")
        }
        // wheelmove dx dy: feed the wheel as if the mouse moved.
        "wheelmove" => {
            use std::sync::atomic::Ordering::Relaxed;
            crate::remap::WHEEL_DX.fetch_add(num(0).unwrap_or(0.0) as i32, Relaxed);
            crate::remap::WHEEL_DY.fetch_add(num(1).unwrap_or(0.0) as i32, Relaxed);
            "ok".into()
        }
        // face [npc]: lock the camera onto the nearest hostile (or that NpcParam); "face off" releases.
        // trace [radius]: what the Doom hitscan sees along the camera (damage debugging).
        "trace" => {
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let r = num(0).unwrap_or(0.15) as f32;
            let h = crate::damage::trace(cam, fwd, 150.0, r);
            let me = game::player().map(|p| p.chr_ins.field_ins_handle);
            let who = h.chr.and_then(|c| {
                game::enemies(400.0).into_iter().find(|e| e.chr.field_ins_handle == c).map(|e| e.chr.npc_param_id)
            });
            let raw = crate::raycast::cast_sphere(cam, fwd * 150.0, r, 0x2000058, |x| me.is_none() || x.field_ins_handle() != me)
                .map(|x| {
                    let g = game::geom_by_hit(&x);
                    format!(
                        "{:?} at {:.2} m {}",
                        x.field_ins_handle().map(|h| h.selector.field_ins_type()),
                        x.segment * 150.0,
                        g.map(|g| format!("{} state {} behavior {} hp {}", g.name, game::geom_state(g.ptr), g.behavior, g.hp)).unwrap_or_default()
                    )
                });
            format!("dist {:.2} chr {:?} npc {:?} raw {:?}", h.dist, h.chr.is_some(), who, raw)
        }
        "breakables" => {
            // Asset types that break (hp > 0 and break on player contact), as AEGxxx_yyy names.
            use eldenring::cs::{AssetEnvironmentGeometryParam, SoloParamRepository};
            use fromsoftware_shared::FromStatic;
            let Ok(repo) = (unsafe { SoloParamRepository::instance() }) else { return "no params".into() };
            let rows: Vec<String> = repo
                .rows::<AssetEnvironmentGeometryParam>()
                .filter(|(_, r)| r.hp() > 0 && r.is_break_by_player_collide())
                .take(80)
                .map(|(id, r)| format!("AEG{:03}_{:03} hp {} def {}", id / 1000, id % 1000, r.hp(), r.defense()))
                .collect();
            rows.join("
")
        }
        "spawngeom" => {
            // Spawn a map asset `dist` m in front of the player (breakable test targets).
            use eldenring::{cs::{CSWorldGeomMan, GeometrySpawnParameters}, position::BlockPosition};
            use fromsoftware_shared::FromStatic;
            let Some(name) = args.first().map(|s| s.to_string()) else { return "spawngeom NAME [dist]".into() };
            let dist = num(1).unwrap_or(4.0) as f32;
            let Some(me) = game::player() else { return "no player".into() };
            let Some((_, fwd)) = game::camera() else { return "no camera".into() };
            let bp = me.block_position;
            let c = (bp.x, bp.y, bp.z);
            let block = me.current_block_id;
            let Ok(gm) = (unsafe { CSWorldGeomMan::instance_mut() }) else { return "no geom man".into() };
            let Some(bd) = gm.geom_block_data_by_id_mut(&block) else { return format!("no geom block {block:?}") };
            let f = glam::Vec2::new(fwd.x, fwd.z).normalize_or_zero() * dist;
            let params = GeometrySpawnParameters {
                position: BlockPosition::from_xyz(c.0 + f.x, c.1, c.2 + f.y),
                rot_x: 0.0,
                rot_y: 0.0,
                rot_z: 0.0,
                scale_x: 1.0,
                scale_y: 1.0,
                scale_z: 1.0,
            };
            match bd.spawn_geometry(&name, &params) {
                Some(_) => format!("spawned {name} at [{:.1}, {:.1}, {:.1}] in {block:?}", c.0 + f.x, c.1, c.2 + f.y),
                None => format!("spawn {name} failed"),
            }
        }
        "rayhits" => {
            // What rays hit around the player, by field type, for a collision filter (hex arg).
            use glam::Vec3;
            let Some(me) = game::player() else { return "no player".into() };
            let at = game::chr_pos(&me.chr_ins);
            let filter = args.first().and_then(|a| u32::from_str_radix(a.trim_start_matches("0x"), 16).ok()).unwrap_or(0x2000058);
            let mut tally: std::collections::BTreeMap<String, u32> = Default::default();
            for h in [0.5f32, 1.2] {
                for k in 0..90 {
                    let a = k as f32 * std::f32::consts::TAU / 90.0;
                    let dir = Vec3::new(a.cos(), -0.1, a.sin()) * 25.0;
                    let hit = crate::raycast::cast_sphere(at + Vec3::Y * h, dir, 0.05, filter, |_| true);
                    let key = match hit {
                        None => "miss".to_string(),
                        Some(h) => match h.field_ins_handle() {
                            None => "no-owner".to_string(),
                            Some(f) => format!("{:?}", f.selector.field_ins_type()),
                        },
                    };
                    *tally.entry(key).or_default() += 1;
                }
            }
            format!("filter {filter:#x}: {tally:?}")
        }
        "geomscan" => {
            use glam::Vec3;
            // Breakable objects around the player: rays in a ring at three heights, distinct
            // map assets with hp > 0 listed with where the ray hit them.
            let Some(me) = game::player() else { return "no player".into() };
            let at = game::chr_pos(&me.chr_ins);
            let range = num(0).unwrap_or(25.0) as f32;
            let mut found: Vec<(String, i16, u16, bool, Vec3)> = Vec::new();
            let (mut rays, mut geoms) = (0, 0);
            for h in [0.4f32, 1.0, 1.8] {
                for k in 0..120 {
                    let a = k as f32 * std::f32::consts::TAU / 120.0;
                    for pitch in [-0.25f32, 0.0] {
                        let dir = Vec3::new(a.cos(), pitch, a.sin());
                        rays += 1;
                        let hit = crate::damage::trace(at + Vec3::Y * h, dir, range, 0.05);
                        let Some(g) = hit.geom else { continue };
                        geoms += 1;
                        let Some(info) = game::geom_by_handle(&g) else { continue };
                        if (info.hp > 0 || args.get(1) == Some(&"all")) && !found.iter().any(|f| f.0 == info.name) {
                            found.push((info.name, info.hp, info.defense, info.break_by_player, hit.pos));
                        }
                    }
                }
            }
            let list: Vec<String> = found
                .iter()
                .map(|(n, hp, def, roll, p)| format!("{n} hp {hp} def {def} roll {roll} at [{:.1}, {:.1}, {:.1}] d {:.1}", p.x, p.y, p.z, p.distance(at)))
                .collect();
            format!("{rays} rays, {geoms} geom hits, {} breakable
{}", found.len(), list.join("
"))
        }
        "allchr" => {
            // Every character set near the player: which set red phantoms / invaders live in, and
            // how much the 196 chr_sets cost to scan.
            let Some(w) = game::world() else { return "no world".into() };
            let Some(me) = game::player() else { return "no player".into() };
            let mp = game::chr_pos(&me.chr_ins);
            let listed: std::collections::HashSet<usize> = w.chr_inses_by_distance.iter().map(|e| e.chr_ins.as_ptr() as usize).collect();
            let mut out = Vec::new();
            let mut row = |set: String, c: &eldenring::cs::ChrIns| {
                let d = game::chr_pos(c).distance(mp);
                let ptr = c as *const _ as usize;
                if d < 120.0 && (!listed.contains(&ptr) || !matches!(c.team_type, 6 | 48 | 51)) {
                    out.push(format!(
                        "{set} npc {} team {} hp {}/{} d {:.0} listed {}",
                        c.npc_param_id, c.team_type, c.modules.data.hp, c.modules.data.max_hp, d, listed.contains(&ptr)
                    ));
                }
            };
            for p in w.player_chr_set.characters() { row("player".into(), &p.chr_ins); }
            for c in w.ghost_chr_set.characters() { row("ghost".into(), c); }
            for c in w.summon_buddy_chr_set.characters() { row("buddy".into(), c); }
            for c in w.debug_chr_set.characters() { row("debug".into(), c); }
            let (mut sets, mut cap) = (0, 0u32);
            for (i, set) in w.chr_sets.iter().enumerate() {
                let Some(set) = set.as_ref() else { continue };
                sets += 1;
                cap += set.capacity;
                for c in set.characters() { row(format!("set{i}"), c); }
            }
            format!("by_distance {} sets {sets} capacity {cap}
{}", listed.len(), out.join("
"))
        }
        "menudump" => {
            // Menu detection research: ER's HUD state, popup / window job pointers and which of
            // CSMenuManImp's UI elements are visible.
            use eldenring::cs::{CSFeManImp, CSMenuManImp};
            let hud = unsafe { <CSFeManImp as fromsoftware_shared::FromStatic>::instance() }.map(|f| format!("{:?}", f.hud_state)).unwrap_or_default();
            let Ok(mm) = (unsafe { <CSMenuManImp as fromsoftware_shared::FromStatic>::instance() }) else { return "no menu man".into() };
            let base = mm as *const CSMenuManImp as *const u8;
            let rd = |o: usize| unsafe { (base.add(o) as *const usize).read() };
            let vis: Vec<usize> = mm.ui_states.iter().enumerate().filter(|(_, u)| u.visible()).map(|(i, _)| i).collect();
            let created: Vec<usize> = mm.ui_states.iter().enumerate().filter(|(_, u)| u.created()).map(|(i, _)| i).collect();
            format!(
                "hud {hud} popup {:x} window_job {:x} nocursor {} cursor_os {} visible {vis:?} created {created:?}",
                rd(0x80),
                rd(0x88),
                mm.disable_mouse_cursor,
                crate::input::cursor_visible()
            )
        }
        "hookfx" => {
            // Meathook chain visual test: hooked point `d` metres ahead (or "off").
            let Ok(mut g) = crate::fx::HOOK.lock() else { return "lock".into() };
            if args.first() == Some(&"off") {
                *g = None;
                return "off".into();
            }
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let at = cam + fwd * num(0).unwrap_or(10.0) as f32;
            *g = Some((at, std::time::Instant::now()));
            format!("hook at {at:?}")
        }
        "face" => {
            if args.first() == Some(&"off") {
                game::release_lock_on();
                return "released".into();
            }
            let want = num(0).map(|v| v as i32);
            match game::enemies(150.0)
                .into_iter()
                .filter(is_hostile)
                .find(|e| want.is_none_or(|w| e.chr.npc_param_id == w))
            {
                Some(e) => format!("lock {} -> {}", e.chr.npc_param_id, game::lock_on(e.chr.field_ins_handle)),
                None => "no hostile".into(),
            }
        }
        "combat" => {
            s.last_combat = s.time;
            "combat".into()
        }
        // nodead [0|1]: the player can't die (enemies still fight normally) - for recordings.
        "nodead" => {
            let on = num(0).map(|v| v != 0.0).unwrap_or(true);
            match unsafe { <eldenring::cs::WorldChrManDbgFlags as fromsoftware_shared::FromStatic>::instance_mut() } {
                Ok(f) => {
                    f.player_no_dead = on;
                    format!("nodead={on}")
                }
                Err(e) => format!("no dbg flags: {e:?}"),
            }
        }
        // itemlog [save|diff]: snapshot / diff the FE item-log view model bytes (banner hunting).
        "itemlog" => {
            static SNAP: std::sync::Mutex<Vec<u8>> = std::sync::Mutex::new(Vec::new());
            let Ok(fe) = (unsafe { <eldenring::cs::CSFeManImp as fromsoftware_shared::FromStatic>::instance() }) else { return "no fe".into() };
            let cur = fe.get_item_log_view_model.to_vec();
            let mut snap = SNAP.lock().unwrap();
            if args.first() == Some(&"save") || snap.is_empty() {
                *snap = cur;
                return "saved".into();
            }
            let mut out = String::new();
            let mut n = 0;
            for i in (0..cur.len()).step_by(4) {
                let a = u32::from_le_bytes(snap[i..i + 4].try_into().unwrap());
                let b = u32::from_le_bytes(cur[i..i + 4].try_into().unwrap());
                if a != b && n < 60 {
                    out += &format!("{i:#x}: {a:#x}->{b:#x} ({}->{}) ", a as i32, f32::from_bits(b));
                    n += 1;
                }
            }
            format!("{n} diffs: {out}")
        }
        // fxtest <sfx> [speed] [life] [dist]: fire a damage-less dart wearing an ER effect (look-dev).
        // spam <n> <bullet id>: spawn n short-lived bullets into the air (pool stress test).
        // hitnow <npc> [slot]: queue a hitter hit on that enemy's chest (reaction test).
        "hitnow" => {
            let want = num(0).map(|v| v as i32);
            let slot = num(1).unwrap_or(1.0) as usize;
            let Some((cam, _)) = game::camera() else { return "no camera".into() };
            match game::enemies(150.0).into_iter().filter(is_hostile).find(|e| want.is_none_or(|w| e.chr.npc_param_id == w)) {
                Some(e) => {
                    let chest = e.pos() + glam::Vec3::Y * 1.1;
                    s.hitters.hit(slot, chest, (chest - cam).normalize_or_zero());
                    format!("queued slot {slot} on {}", e.chr.npc_param_id)
                }
                None => "no hostile".into(),
            }
        }
        // anim <npc> <event>: fire a behavior event on that enemy (reaction look-dev).
        "ev" => {
            let want = num(0).map(|v| v as i32);
            let ev = args.get(1).copied().unwrap_or("W_DamageLv2_Middle");
            match game::enemies(150.0).into_iter().filter(is_hostile).find(|e| want.is_none_or(|w| e.chr.npc_param_id == w)) {
                Some(e) => format!("{ev} on {} -> {}", e.chr.npc_param_id, game::behavior_event(e.chr, ev)),
                None => "no hostile".into(),
            }
        }
        // npcname <npc>: the NpcName FMG text for that enemy (boss bar name check).
        "npcname" => {
            let want = num(0).map(|v| v as u32).unwrap_or(43114010);
            let Ok(repo) = (unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance() }) else { return "no repo".into() };
            let Some(row) = repo.get::<eldenring::cs::NpcParam>(want) else { return "no row".into() };
            let id = row.name_id();
            let msg = unsafe { <eldenring::cs::MsgRepository as fromsoftware_shared::FromStatic>::instance().ok() };
            let name = msg.and_then(|m| m.get_msg(18, id as u32)).map(|w| String::from_utf16_lossy(w));
            format!("npc {want} name_id {id} -> {name:?}")
        }
        "findmsg" => {
            let t = args.join(" ");
            format!("{t:?}: {:?}", game::find_msg(&t))
        }
        "playtime" => format!("{:?}", unsafe { <eldenring::cs::GameDataMan as fromsoftware_shared::FromStatic>::instance() }.map(|g| g.play_time)),
        "msg" => {
            let cat = num(0).unwrap_or(18.0) as u32;
            let id = num(1).unwrap_or(902130000.0) as u32;
            let msg = unsafe { <eldenring::cs::MsgRepository as fromsoftware_shared::FromStatic>::instance().ok() };
            format!("fmg {cat}/{id}: {:?}", msg.and_then(|m| m.get_msg(cat, id)).map(String::from_utf16_lossy))
        }
        "sweep" => {
            crate::hitter::SWEEP.store(num(0).unwrap_or(20.0) as u32, std::sync::atomic::Ordering::Relaxed);
            crate::hitter::ON_MS.store(num(1).unwrap_or(60.0) as u32, std::sync::atomic::Ordering::Relaxed);
            "ok".into()
        }
        "zeromask" => {
            let m = num(0).unwrap_or(0.0) as u32;
            crate::bullet::SPAWN_ZERO_MASK.store(m, std::sync::atomic::Ordering::Relaxed);
            format!("zeromask {m:#b}")
        }
        "spam" => {
            let n = num(0).unwrap_or(50.0) as u32;
            let id = num(1).unwrap_or(crate::params::FX_SILENT_ROUND as f64) as i32;
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
            let mut fails = 0;
            let mut code = 0;
            for _ in 0..n {
                if let Err(e) = crate::bullet::spawn(owner, id, cam + fwd * 1.5 + glam::Vec3::Y * 3.0, glam::Vec3::Y) {
                    fails += 1;
                    code = e;
                }
            }
            format!("spam {n} x {id}: {fails} failed (code {code})")
        }
        "fxtest" => {
            let sfx = num(0).unwrap_or(-1.0) as i32;
            let speed = num(1).unwrap_or(20.0) as f32;
            let life = num(2).unwrap_or(0.5) as f32;
            let dist = num(3).unwrap_or(2.0) as f32;
            if let Ok(repo) = unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() } {
                let src = repo.get::<eldenring::cs::Bullet>(crate::params::FX_SILENT_ROUND as u32).map(|r| r as *const eldenring::param::BULLET_PARAM_ST);
                if let Some(b) = repo.get_mut::<eldenring::cs::Bullet>(10404054) {
                    if let Some(src) = src {
                        unsafe { std::ptr::copy_nonoverlapping(src, b as *mut _, 1) };
                    }
                    b.set_sfx_id_bullet(sfx);
                    b.set_init_vellocity(speed);
                    b.set_max_vellocity(speed);
                    b.set_life(life);
                }
            }
            let Some((cam, fwd)) = game::camera() else { return "no camera".into() };
            let owner = game::player().map(|p| p.chr_ins.field_ins_handle);
            format!("{:?}", crate::bullet::spawn(owner, 10404054, cam + fwd * dist - glam::Vec3::Y * 0.2, fwd))
        }
        // bullets: live bullets grouped by param id (count, max time alive) - pool-leak hunting.
        "bullets" => {
            let Ok(man) = (unsafe { <eldenring::cs::CSBulletManager as fromsoftware_shared::FromStatic>::instance() }) else { return "no man".into() };
            let mut by: std::collections::BTreeMap<i32, (u32, f32)> = Default::default();
            let mut hp: Vec<String> = Vec::new();
            let mut n = 0;
            let mut cur = man.bullets.head;
            while let Some(p) = cur {
                let b = unsafe { p.as_ref() };
                if b.fly_state.base.param.param_id / 100 == 104040 {
                    hp.push(format!("{}@({:.1},{:.1},{:.1})", b.fly_state.base.param.param_id % 100, b.physics.position.0, b.physics.position.1, b.physics.position.2));
                }
                let e = by.entry(b.fly_state.base.param.param_id).or_insert((0, 0.0));
                e.0 += 1;
                e.1 = e.1.max(b.time_alive);
                n += 1;
                if n > 1000 {
                    break;
                }
                cur = b.next_bullet;
            }
            // Pool counters (private fields at fixed offsets of CSBulletManager).
            let base = man as *const _ as *const u8;
            let rd = |o: usize| unsafe { (base.add(o) as *const u32).read_unaligned() };
            format!("{n} live: {by:?} | sfx ctr {} / {} | bullets ctr {} / {} | {}", rd(0x1d0), rd(0x1d4), rd(0x1d8), rd(0x1dc), hp.join(" "))
        }
        "flashhold" => {
            let on = num(0).unwrap_or(1.0) != 0.0;
            crate::viewmodel::FLASH_HOLD.store(on, std::sync::atomic::Ordering::Relaxed);
            if on {
                crate::viewmodel::muzzle_flash(s.weapon);
            }
            format!("flashhold {on}")
        }
        "bp" => {
            s.blood_punch = num(0).unwrap_or(1.0) as u32;
            format!("blood_punch={}", s.blood_punch)
        }
        // probe X Y Z: is there loaded floor collision under that point (safe teleport check)?
        "probe" => {
            let (Some(x), Some(y), Some(z)) = (num(0), num(1), num(2)) else { return "probe x y z".into() };
            let from = glam::Vec3::new(x as f32, y as f32, z as f32);
            let down = crate::raycast::cast_sphere(from, glam::Vec3::NEG_Y * 80.0, 0.2, 0x2000058, |h| !crate::raycast::is_chr_hit(h));
            let up = crate::raycast::cast_sphere(from, glam::Vec3::Y * 40.0, 0.2, 0x2000058, |h| !crate::raycast::is_chr_hit(h));
            format!(
                "floor {:?} ceiling {:?}",
                down.map(|h| (h.pos.y, h.normal.y)),
                up.map(|h| h.pos.y)
            )
        }
        // camlog 1|0: erfps2's per-frame camera log (game folder erfps2_camlog.txt).
        "camlog" => {
            let on = num(0).map(|v| v != 0.0).unwrap_or(true);
            unsafe {
                let Ok(m) = windows::Win32::System::LibraryLoader::GetModuleHandleW(windows::core::w!("erfps2.dll")) else { return "no erfps2".into() };
                let Some(f) = windows::Win32::System::LibraryLoader::GetProcAddress(m, windows::core::s!("erfps2_camlog")) else { return "no export".into() };
                let call: extern "C" fn(bool) = std::mem::transmute(f);
                call(on);
            }
            format!("camlog {on}")
        }
        // chrscan: deep scan of every character source; lists characters only the game's update /
        // distance lists know about (missing from every ChrSet) and per-source counts.
        // pad: controller diagnostics (hook calls = the game reads XInput through our hook)
        "pad" => {
            let p = crate::gamepad::current();
            let down: Vec<&str> = crate::gamepad::ALL.iter().filter(|&&c| p.down(c)).map(|&c| crate::gamepad::name(c)).collect();
            format!(
                "hook calls {} | mode {} | icons {} | down {:?} | lt {} rt {} | left {:?} right {:?}",
                crate::gamepad::HOOK_CALLS.load(std::sync::atomic::Ordering::Relaxed),
                if crate::gamepad::pad_mode() { "controller" } else { "keyboard" },
                if crate::gamepad::ps_icons() { "ps" } else { "xbox" },
                down, p.lt, p.rt, p.left(), p.right()
            )
        }
        // classes: the starting classes (CharaInitParam 3000-3009) - level and stats
        "classes" => {
            use eldenring::cs::{CharaInitParam, SoloParamRepository};
            let Ok(repo) = (unsafe { <SoloParamRepository as fromsoftware_shared::FromStatic>::instance() }) else { return "no repo".into() };
            (3000..=3009u32)
                .filter_map(|id| repo.get::<CharaInitParam>(id).map(|c| format!(
                    "{id}: level {} vig {} mind {} end {} str {} dex {} int {} fai {} arc {}",
                    c.soul_lv(), c.base_vit(), c.base_wil(), c.base_end(), c.base_str(), c.base_dex(), c.base_mag(), c.base_fai(), c.base_luc()
                )))
                .collect::<Vec<_>>()
                .join("
")
        }
        // bosses [range]: nearby enemies with their entity ID and whether GameAreaParam lists them
        // as a boss (the Crucible's boss check), plus whether a boss bar is up for them
        "bosses" => {
            let range = args.first().and_then(|a| a.parse().ok()).unwrap_or(300.0);
            let bars = game::active_boss_handles();
            let mut out: Vec<String> = game::enemies(range)
                .iter()
                .filter(|e| e.hp() > 0)
                .map(|e| format!("npc {} entity {} dist {:.0} hp {} area_boss {} bar {}", e.chr.npc_param_id, e.chr.event_entity_id, e.dist, e.max_hp(), crate::params::is_area_boss(e.chr.event_entity_id), bars.contains(&e.chr.field_ins_handle)))
                .collect();
            out.sort();
            if out.is_empty() { "no enemies in range".into() } else { out.join("
") }
        }
        // abman: hex dump of CSActionButtonMan's first 0x100 bytes (find the prompt it's showing:
        // dump at an interact point and away from it, then diff)
        "abman" => {
            use fromsoftware_shared::FromStatic;
            let Ok(m) = (unsafe { eldenring::cs::CSActionButtonMan::instance() }) else { return "no CSActionButtonMan".into() };
            let p = m as *const _ as *const u8;
            let mut lines = Vec::new();
            for row in 0..16usize {
                let bytes: Vec<String> = (0..16usize).map(|i| format!("{:02x}", unsafe { *p.add(row * 16 + i) })).collect();
                let words: Vec<String> = (0..4usize).map(|i| format!("{}", unsafe { *(p.add(row * 16 + i * 4) as *const i32) })).collect();
                lines.push(format!("{:03x}: {}  | {}", row * 16, bytes.join(" "), words.join(" ")));
            }
            lines.join("
")
        }
        // stat <name> <value>: set a player attribute or level (undo a test level-up);
        // runes <+n|-n>: give / take runes. The game recalculates max HP etc. on its next stat
        // refresh (rest at a grace).
        "stat" => {
            let Some(p) = game::player() else { return "no player".into() };
            let (Some(name), Some(v)) = (args.first().copied(), num(1)) else { return "usage: stat vigor|mind|endurance|strength|dexterity|intelligence|faith|arcane|level <value>".into() };
            let g = unsafe { p.player_game_data.as_mut() };
            let v = v.max(1.0) as u32;
            let field = match name {
                "vigor" => &mut g.vigor,
                "mind" => &mut g.mind,
                "endurance" => &mut g.endurance,
                "strength" => &mut g.strength,
                "dexterity" => &mut g.dexterity,
                "intelligence" => &mut g.intelligence,
                "faith" => &mut g.faith,
                "arcane" => &mut g.arcane,
                "level" => &mut g.level,
                _ => return format!("unknown stat {name}"),
            };
            let old = *field;
            *field = v;
            log::info!("stat {name}: {old} -> {v}");
            format!("{name} {old} -> {v}")
        }
        // crucible <0..3>: set the Crucible's charges (testing)
        "crucible" => {
            let n = num(0).unwrap_or(3.0).clamp(0.0, crate::slayer::CRUCIBLE_MAX as f64) as u32;
            s.crucible_charges = n;
            format!("crucible charges {n}")
        }
        "runes" => {
            let Some(p) = game::player() else { return "no player".into() };
            let Some(d) = num(0) else { return "usage: runes <+n|-n>".into() };
            let g = unsafe { p.player_game_data.as_mut() };
            let old = g.rune_count;
            g.rune_count = (old as i64 + d as i64).max(0) as u32;
            log::info!("runes: {old} -> {}", g.rune_count);
            format!("runes {old} -> {}", g.rune_count)
        }
        "chrscan" => {
            let Some(w) = game::world() else { return "no world".into() };
            let me = w.main_player.as_ref().map(|p| game::chr_pos(&p.chr_ins)).unwrap_or_default();
            let all = game::all_characters(w);
            let mut counts: std::collections::BTreeMap<&str, usize> = Default::default();
            let mut lines = Vec::new();
            for (p, src) in &all {
                *counts.entry(*src).or_default() += 1;
                let c = unsafe { &**p };
                if *src == "update_list" || *src == "distance_list" {
                    let d = &c.modules.data;
                    lines.push(format!(
                        "  {src}: npc {} {:?} team {} hp {}/{} container {} dist {:.0}",
                        c.npc_param_id,
                        game::npc_name(c.npc_param_id),
                        c.team_type,
                        d.hp,
                        d.max_hp,
                        c.field_ins_handle.selector.container(),
                        game::chr_pos(c).distance(me)
                    ));
                }
            }
            let mut by_container: std::collections::BTreeMap<u32, usize> = Default::default();
            for (p, _) in &all {
                *by_container.entry(unsafe { &**p }.field_ins_handle.selector.container()).or_default() += 1;
            }
            format!("total {} by source {:?}
by container {:?}
only in game lists ({}):
{}", all.len(), counts, by_container, lines.len(), lines.join("
"))
        }
        "god" => {
            s.god = num(0).map(|v| v != 0.0).unwrap_or(!s.god);
            format!("god={}", s.god)
        }
        // hpdbg: the HP pool as the game and the mod see it (shield-in-health checks).
        "hpdbg" => {
            let Some(p) = game::player() else { return "no player".into() };
            let d = &p.chr_ins.modules.data;
            let pgd = unsafe { p.player_game_data.as_ref() };
            format!(
                "chr hp {}/{} | save hp {}/{} base {} | mod health {} base {} armor {}",
                d.hp, d.max_hp, pgd.current_hp, pgd.current_max_hp, pgd.base_max_hp, s.health, s.base_max, s.armor
            )
        }
        // hurt N: take N damage the way an enemy hit does (the game's HP drops) - shield tests.
        "hurt" => {
            let Some(p) = game::player() else { return "no player".into() };
            let n = num(0).unwrap_or(100.0) as i32;
            let d = &mut p.chr_ins.modules.data;
            d.hp = (d.hp - n).max(1);
            format!("hp now {}", d.hp)
        }
        "armor" => {
            s.armor = num(0).unwrap_or(150.0) as i32;
            "ok".into()
        }
        // Set nearest hostile's HP to a fraction of max (to test stagger / glory kills).
        "wound" => {
            let frac = num(0).unwrap_or(0.1) as f32;
            let want = num(1).map(|v| v as i32);
            match game::enemies(150.0).into_iter().filter(is_hostile).find(|e| want.is_none_or(|w| e.chr.npc_param_id == w)) {
                Some(e) => {
                    let max = e.max_hp();
                    e.chr.modules.data.hp = (max as f32 * frac).max(1.0) as i32;
                    format!("npc {} hp {}", e.chr.npc_param_id, e.chr.modules.data.hp)
                }
                None => "no hostile".into(),
            }
        }
        // tp_enemy [dist] [index]: stand `dist` m from a hostile (line-of-sight tests).
        "tp_enemy" => {
            let dist = num(0).unwrap_or(5.0) as f32;
            let idx = num(1).unwrap_or(0.0) as usize;
            let hostile: Vec<_> = game::enemies(200.0).into_iter().filter(is_hostile).collect();
            // Index < 1000 picks by distance rank, otherwise it is an NpcParam id.
            let pick = if idx >= 1000 {
                hostile.iter().find(|e| e.chr.npc_param_id as usize == idx)
            } else {
                hostile.get(idx)
            };
            let Some(e) = pick else { return "no hostile".into() };
            let Some(p) = game::player() else { return "no player".into() };
            let ph = &mut p.chr_ins.modules.physics;
            let from = game::hpos(&ph.position);
            let target = e.pos();
            let mut away = from - target;
            away.y = 0.0;
            let at = target + away.normalize_or(glam::Vec3::X) * dist + glam::Vec3::Y * 0.5;
            ph.position.0 = at.x;
            ph.position.1 = at.y;
            ph.position.2 = at.z;
            ph.chr_proxy_pos_update_requested = true;
            format!("tp to {at} near npc {} hp {}", e.chr.npc_param_id, e.hp())
        }
        // raw <bullet_id> <owner 0|1> [npc]: spawn a bullet 1.5 m in front of a hostile, aimed at it.
        "raw" => {
            let id = num(0).unwrap_or(10030000.0) as i32;
            let with_owner = num(1).unwrap_or(0.0) != 0.0;
            let want = num(2).map(|v| v as i32);
            let Some(e) = game::enemies(150.0)
                .into_iter()
                .filter(is_hostile)
                .find(|e| want.is_none_or(|w| e.chr.npc_param_id == w))
            else {
                return "no hostile".into();
            };
            let owner = if with_owner { game::player().map(|p| p.chr_ins.field_ins_handle) } else { None };
            let chest = e.pos() + glam::Vec3::Y * 1.2;
            let from = game::player().map(|p| game::chr_pos(&p.chr_ins)).unwrap_or(chest);
            let dir = (chest - from).normalize_or(glam::Vec3::X);
            let at = chest - dir * 1.5;
            let r = crate::bullet::spawn(owner, id, at, dir);
            format!("{r:?} bullet {id} owner={with_owner} at npc {} hp {}", e.chr.npc_param_id, e.hp())
        }
        "setatk" => {
            let (Some(id), Some(v)) = (num(0), num(1)) else { return "usage: setatk id phys".into() };
            match unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() } {
                Ok(repo) => match repo.get_mut::<eldenring::cs::AtkParam_Pc>(id as u32) {
                    Some(a) => {
                        match args.get(2).copied() {
                            Some("corr") => a.set_atk_phys_correction(v as u16),
                            Some("add") => {
                                a.set_atk_phys(v as u16);
                                a.set_is_add_base_atk(true)
                            }
                            _ => a.set_atk_phys(v as u16),
                        }
                        params::describe_atk(id as u32)
                    }
                    None => "no row".into(),
                },
                Err(_) => "no repo".into(),
            }
        }
        // dumpbullets: every Bullet row's sfx ids -> bullets_dump.csv (pick visuals offline).
        "dumpbullets" => {
            let Ok(repo) = (unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() }) else {
                return "no repo".into();
            };
            let mut out = String::from("id,atk,sfx_bullet,sfx_hit,sfx_flick,life,speed,radius
");
            for (id, b) in repo.rows_mut::<eldenring::cs::Bullet>() {
                let _ = writeln!(out, "{id},{},{},{},{},{},{},{}", b.atk_id_bullet(), b.sfx_id_bullet(), b.sfx_id_hit(), b.sfx_id_flick(), b.life(), b.init_vellocity(), b.hit_radius());
            }
            let _ = std::fs::write(config::mod_dir().join("bullets_dump.csv"), out);
            "written".into()
        }
        // sfx <slot> <bullet_sfx> [hit_sfx]: live-swap a weapon slot's visuals.
        "sfx" => {
            let slot = num(0).unwrap_or(4.0) as usize;
            let Ok(repo) = (unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() }) else {
                return "no repo".into();
            };
            match repo.get_mut::<eldenring::cs::Bullet>(crate::params::BULLET_SLOTS[slot] as u32) {
                Some(b) => {
                    if let Some(v) = num(1) { b.set_sfx_id_bullet(v as i32); }
                    if let Some(v) = num(2) { b.set_sfx_id_hit(v as i32); }
                    format!("slot {slot}: sfx {} hit {}", b.sfx_id_bullet(), b.sfx_id_hit())
                }
                None => "no row".into(),
            }
        }
        // aflag <atk id> <disable_guard 0|1> <disable_parry 0|1>
        "aflag" => {
            let id = num(0).unwrap_or(44401.0) as u32;
            let Ok(repo) = (unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() }) else { return "no repo".into() };
            match repo.get_mut::<eldenring::cs::AtkParam_Pc>(id) {
                Some(a) => {
                    if let Some(v) = num(1) { a.set_disable_guard(v != 0.0); }
                    if let Some(v) = num(2) { a.set_is_disable_parry(v != 0.0); }
                    format!("atk {id}: disable_guard {} disable_parry {}", a.disable_guard(), a.is_disable_parry())
                }
                None => "no row".into(),
            }
        }
        // bsfx <bullet id> <trail> <hit> [attach_type] [delete_by_hit] [delete_by_life] [stick 0|1]
        "bsfx" => {
            let id = num(0).unwrap_or(10404002.0) as u32;
            let Ok(repo) = (unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() }) else { return "no repo".into() };
            match repo.get_mut::<eldenring::cs::Bullet>(id) {
                Some(b) => {
                    if let Some(v) = num(1) { b.set_sfx_id_bullet(v as i32); }
                    if let Some(v) = num(2) { b.set_sfx_id_hit(v as i32); }
                    if let Some(v) = num(3) { b.set_attach_effect_type(v as u8); }
                    if let Some(v) = num(4) { b.set_bullet_sfx_delete_type_by_hit(v as i8); }
                    if let Some(v) = num(5) { b.set_bullet_sfx_delete_type_by_life_dead(v as i8); }
                    if let Some(v) = num(6) { b.set_is_attack_sfx(v != 0.0); }
                    format!(
                        "bullet {id}: trail {} hit {} attach {} del_hit {} del_life {} stick {} flick {} hit_bullet {}",
                        b.sfx_id_bullet(), b.sfx_id_hit(), b.attach_effect_type(), b.bullet_sfx_delete_type_by_hit(),
                        b.bullet_sfx_delete_type_by_life_dead(), b.is_attack_sfx(), b.sfx_id_flick(), b.hit_bullet_id()
                    )
                }
                None => "no row".into(),
            }
        }
        // bflag <bullet id> <shared 0|1> <record secs> <both_team 0|1>
        "bflag" => {
            let id = num(0).unwrap_or(10404002.0) as u32;
            let Ok(repo) = (unsafe { <eldenring::cs::SoloParamRepository as fromsoftware_shared::FromStatic>::instance_mut() }) else { return "no repo".into() };
            match repo.get_mut::<eldenring::cs::Bullet>(id) {
                Some(b) => {
                    if let Some(v) = num(1) { b.set_is_use_shared_hit_list(v != 0.0); }
                    if let Some(v) = num(2) { b.set_dmg_hit_record_life_time(v as f32); }
                    if let Some(v) = num(3) { b.set_is_hit_both_team(v != 0.0); }
                    format!("bullet {id}: shared {} record {} both {}", b.is_use_shared_hit_list(), b.dmg_hit_record_life_time(), b.is_hit_both_team())
                }
                None => "no row".into(),
            }
        }
        "goods" => {
            crate::bullet::GOODS_ID.store(num(0).unwrap_or(-1.0) as i32, std::sync::atomic::Ordering::Relaxed);
            "ok".into()
        }
        "atk" => params::describe_atk(num(0).unwrap_or(10174000.0) as u32),
        // autowalk DEG SECS: walk for SECS seconds, DEG right of the camera's forward (0 = ahead).
        "autowalk" => {
            let deg = num(0).unwrap_or(0.0) as f32;
            let secs = num(1).unwrap_or(1.0);
            *crate::slayer::AUTOWALK.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((deg, std::time::Instant::now() + std::time::Duration::from_secs_f64(secs)));
            format!("walking {deg} deg for {secs} s")
        }
        // walkto X Z [SECS]: walk straight toward a world point (movement tests).
        "walkto" => {
            let (Some(x), Some(z)) = (num(0), num(1)) else { return "walkto x z [secs]".into() };
            let secs = num(2).unwrap_or(5.0);
            *crate::slayer::WALKTO.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((x as f32, z as f32, std::time::Instant::now() + std::time::Duration::from_secs_f64(secs)));
            format!("walking to {x} {z}")
        }
        // heightmap R STEP: floor heights (relative to the player, dm) on a grid, rows = +z.
        "heightmap" => {
            let r = num(0).unwrap_or(8.0) as f32;
            let st = num(1).unwrap_or(1.0) as f32;
            let Some(p) = game::player() else { return "no player".into() };
            let at = game::chr_pos(&p.chr_ins);
            let n = (r / st) as i32;
            let mut rows = vec![format!("origin {at} step {st}")];
            for iz in (-n..=n).rev() {
                let mut row = String::new();
                for ix in -n..=n {
                    let q = at + glam::Vec3::new(ix as f32 * st, 0.0, iz as f32 * st);
                    row += &match crate::slayer::ground_height(q, 3.0) {
                        Some(h) => format!("{:>4}", ((h - at.y) * 10.0).round() as i32),
                        None => "   .".into(),
                    };
                }
                rows.push(row);
            }
            rows.join("
")
        }
        "mtrace" => {
            let on = args.first() == Some(&"on");
            crate::slayer::TRACE.store(on, std::sync::atomic::Ordering::Relaxed);
            format!("trace {on}")
        }
        "tp" => {
            let (Some(x), Some(y), Some(z)) = (num(0), num(1), num(2)) else { return "usage: tp x y z".into() };
            let Some(p) = game::player() else { return "no player".into() };
            let ph = &mut p.chr_ins.modules.physics;
            ph.position.0 = x as f32;
            ph.position.1 = y as f32;
            ph.position.2 = z as f32;
            ph.chr_proxy_pos_update_requested = true;
            "ok".into()
        }
        // cast dx dy dz [filter_hex] [radius]: sphere cast from the player's chest.
        "cast" => {
            let (Some(x), Some(y), Some(z)) = (num(0), num(1), num(2)) else { return "usage: cast dx dy dz".into() };
            let filter = args.get(3).and_then(|f| u32::from_str_radix(f.trim_start_matches("0x"), 16).ok()).unwrap_or(0x2000058);
            let r = num(4).unwrap_or(0.35) as f32;
            let Some(p) = game::player() else { return "no player".into() };
            let chest = game::chr_pos(&p.chr_ins) + glam::Vec3::Y;
            let d = glam::Vec3::new(x as f32, y as f32, z as f32);
            match crate::raycast::cast_sphere(chest, d, r, filter, |h| !crate::raycast::is_chr_hit(h)) {
                Some(h) => format!("hit seg {:.3} pos {:?} filter {:#x}", h.segment, h.pos, h.filter),
                None => "no hit".into(),
            }
        }
        // poise <value>: set nearest hostile's super-armor durability (stance tests).
        "poise" => match game::enemies(150.0).into_iter().find(is_hostile) {
            Some(e) => {
                e.chr.modules.super_armor.sa_durability = num(0).unwrap_or(0.01) as f32;
                format!("npc {} poise {}", e.chr.npc_param_id, e.chr.modules.super_armor.sa_durability)
            }
            None => "no hostile".into(),
        },
        // glory [chainsaw]: force a glory kill / chainsaw on the nearest hostile (ignores facing).
        "glory" => {
            let saw = args.first() == Some(&"chainsaw");
            match game::enemies(60.0).iter().position(is_hostile) {
                Some(i) => {
                    s.start_glory(i, saw);
                    "started".into()
                }
                None => "no hostile".into(),
            }
        }
        "bullet" => params::describe_bullet(num(0).unwrap_or(10400000.0) as u32),
        "spawn" => {
            // spawn <chr_id> <npc_param> <think_param> [dist]
            let (Some(chr), Some(npc), Some(think)) = (num(0), num(1), num(2)) else {
                return "usage: spawn chr npc think [dist]".into();
            };
            let dist = num(3).unwrap_or(6.0) as f32;
            let (Some(w), Some((cam, fwd))) = (game::world(), game::camera()) else {
                return "no world".into();
            };
            let Some(p) = w.main_player.as_ref() else { return "no player".into() };
            let ppos = game::chr_pos(&p.chr_ins);
            let flat = glam::Vec3::new(fwd.x, 0.0, fwd.z).normalize_or_zero();
            let at = ppos + flat * dist;
            let _ = cam;
            let req = eldenring::cs::ChrDebugSpawnRequest {
                chr_id: chr as i32,
                chara_init_param_id: -1,
                npc_param_id: npc as i32,
                npc_think_param_id: think as i32,
                event_entity_id: -1,
                talk_id: -1,
                pos_x: at.x,
                pos_y: at.y,
                pos_z: at.z,
            };
            w.spawn_debug_character(&req);
            format!("spawn requested at {at}")
        }
        _ => format!("unknown command '{cmd}'"),
    }
}

fn model_pos(c: &eldenring::cs::ChrIns) -> [f32; 3] {
    let m = &c.chr_ctrl.model_matrix.3;
    [m.0, m.1, m.2]
}
