//! Doom-style item pickup: everything on the ground (dropped items, map items, crafting materials,
//! corpse loot, lost runes) is collected by walking over it - no button, no pickup animation.
//!
//! Elden Ring asks `CSActionButtonManImp` whether an action button (ActionButtonParam row) should
//! execute; we hook that query and answer "yes" for the pickup rows. Same approach as Erd-Tools
//! (Nordgaren/Erd-Tools-CPP, MIT): signature, ActionButtonParam ids and the register-preserving
//! wrapper (the game passes floats in XMM0-2 alongside RCX/RDX).

use std::{ffi::c_void, sync::atomic::{AtomicBool, Ordering}};

use hudhook::mh::{MH_ApplyQueued, MH_Initialize, MH_QueueEnableHook, MhHook};
use pelite::pe::{Pe, PeObject, PeView};

use crate::program::Program;

pub static ENABLED: AtomicBool = AtomicBool::new(true);

/// The action button Elden Ring is showing a prompt for right now: CSActionButtonMan + 0x2c holds
/// its ActionButtonParam id, -1 when none (found by dumping the manager at an interact point and
/// away from it: 9290 <-> -1 in step with the prompt, 2026-10-07). Covers every kind of prompt -
/// the query hook below missed event-script ones (user).
pub fn shown_button() -> Option<i32> {
    use fromsoftware_shared::FromStatic;
    let m = unsafe { eldenring::cs::CSActionButtonMan::instance() }.ok()?;
    let id = unsafe { *((m as *const _ as *const u8).add(0x2c) as *const i32) };
    (id >= 0).then_some(id)
}

/// An interaction is on offer right now (item pickups excluded: walking over them collects them).
pub fn offered() -> bool {
    match shown_button() {
        // 1000 is in the pickup list (lost runes) but ER also shows it as "Examine" (user): the
        // prompt shows for it; a real pickup gets collected in a moment anyway.
        Some(id) => id == 1000 || !is_pickup(id),
        None => false,
    }
}

/// CSActionButtonManImp "should execute action button param" (ER 1.16, from Erd-Tools).
const SIG: &str = "48 89 5C 24 08 57 48 81 EC 90 00 00 00 48 8B 84 24 E0 00 00 00 41 0F B6 D9 48 8B 0D ?? ?? ?? ?? 8B FA 0F 29 B4 24 80 00 00 00";

#[unsafe(no_mangle)]
static mut DOOMRING_AB_TRAMPOLINE: usize = 0;

/// ActionButtonParam rows that are item pickups.
fn is_pickup(id: i32) -> bool {
    matches!(id,
        // dropped / map items
        4000 | 4110 | 4200 | 4201 | 4202 | 4250..=4253 | 4260 | 4270 | 4280 | 4300 | 4350 | 6361 | 9532
        // corpse loot, lost runes
        | 4100 | 1000
        // gatherable materials (overworld + DLC rows)
        | 7800 | 7810..=7828 | 7850 | 7860..=7878 | 207800 | 207810..=207844)
}

/// -1 = let the game decide, 1 = execute now.
extern "C" fn filter(_man: usize, entry_id: i32) -> i32 {
    static SEEN: std::sync::Mutex<Vec<i32>> = std::sync::Mutex::new(Vec::new());
    if let Ok(mut seen) = SEEN.try_lock() {
        if !seen.contains(&entry_id) && seen.len() < 256 {
            seen.push(entry_id);
            log::info!("autoloot: action button {entry_id} (pickup={})", is_pickup(entry_id));
        }
    }
    if ENABLED.load(Ordering::Relaxed) && is_pickup(entry_id) { 1 } else { -1 }
}

#[unsafe(naked)]
unsafe extern "C" fn wrapper() {
    core::arch::naked_asm!(
        "push rcx",
        "push rdx",
        "push r8",
        "push r9",
        "sub rsp, 0x58",
        "movaps [rsp + 0x20], xmm0",
        "movaps [rsp + 0x30], xmm1",
        "movaps [rsp + 0x40], xmm2",
        "call {filter}",
        "movaps xmm0, [rsp + 0x20]",
        "movaps xmm1, [rsp + 0x30]",
        "movaps xmm2, [rsp + 0x40]",
        "add rsp, 0x58",
        "pop r9",
        "pop r8",
        "pop rdx",
        "pop rcx",
        "cmp eax, -1",
        "je 2f",
        "ret",
        "2:",
        "jmp qword ptr [rip + {tramp}]",
        filter = sym filter,
        tramp = sym DOOMRING_AB_TRAMPOLINE,
    )
}

/// MapItemMan item-get popup (the "Rowa Raisin x1" banner). Signature from The Grand Archives'
/// ER table (ItemPopup_code: AOB - 0x14). Doom pickups never show it.
const POPUP_SIG: &str = "?? 8b fa ?? 8b d9 ?? 8b 81 a8 00 00 00";

pub static NO_BANNER: AtomicBool = AtomicBool::new(true);
static mut POPUP_ORIG: usize = 0;

unsafe extern "C" fn popup_detour(this: usize, data: usize) -> usize {
    if NO_BANNER.load(Ordering::Relaxed) {
        return 0;
    }
    let orig: unsafe extern "C" fn(usize, usize) -> usize = unsafe { std::mem::transmute(POPUP_ORIG) };
    unsafe { orig(this, data) }
}

pub fn parse_sig(s: &str) -> Vec<Option<u8>> {
    s.split_whitespace().map(|b| if b.starts_with('?') { None } else { u8::from_str_radix(b, 16).ok() }).collect()
}

/// Find the signature in the executable's code sections.
fn scan(sig: &[Option<u8>]) -> Option<*const u8> {
    scan_step(sig, 16)
}

pub fn scan_step(sig: &[Option<u8>], step: usize) -> Option<*const u8> {
    let program = Program::current();
    let view: PeView<'static> = program.into();
    let base = view.image().as_ptr();
    for sec in view.section_headers() {
        // IMAGE_SCN_CNT_CODE
        if sec.Characteristics & 0x20 == 0 {
            continue;
        }
        let start = sec.VirtualAddress as usize;
        let len = sec.VirtualSize as usize;
        let bytes = unsafe { std::slice::from_raw_parts(base.add(start), len) };
        // Signature starts on a 16-byte boundary (function entry).
        let mut i = (step - (base as usize + start) % step) % step;
        while i + sig.len() <= bytes.len() {
            if sig.iter().enumerate().all(|(k, b)| b.is_none_or(|v| bytes[i + k] == v)) {
                return Some(unsafe { base.add(start + i) });
            }
            i += step;
        }
    }
    None
}

fn install_popup_hook() {
    let Some(hit) = scan_step(&parse_sig(POPUP_SIG), 1) else {
        log::error!("autoloot: item popup signature not found");
        return;
    };
    let target = unsafe { hit.sub(0x14) } as *mut c_void;
    unsafe {
        match MhHook::new(target, popup_detour as *mut c_void) {
            Ok(hook) => {
                POPUP_ORIG = hook.trampoline() as usize;
                let _ = MH_QueueEnableHook(target);
                let _ = MH_ApplyQueued();
                std::mem::forget(hook);
                log::info!("autoloot: item popup hooked at {target:p}");
            }
            Err(e) => log::error!("autoloot: popup hook failed: {e:?}"),
        }
    }
}

pub fn install() {
    install_popup_hook();
    let Some(target) = scan(&parse_sig(SIG)) else {
        log::error!("autoloot: action button signature not found (game version?)");
        return;
    };
    unsafe {
        let _ = MH_Initialize();
        match MhHook::new(target as *mut c_void, wrapper as *mut c_void) {
            Ok(hook) => {
                DOOMRING_AB_TRAMPOLINE = hook.trampoline() as usize;
                let _ = MH_QueueEnableHook(target as *mut c_void);
                let _ = MH_ApplyQueued();
                std::mem::forget(hook);
                log::info!("autoloot: hooked action button query at {target:p}");
            }
            Err(e) => log::error!("autoloot: hook failed: {e:?}"),
        }
    }
}
