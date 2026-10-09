//! Doom controls on top of Elden Ring's: hooks IDirectInputDevice8::GetDeviceState (the game reads
//! keyboard and mouse through DInput8) and rewrites what the game sees.
//!
//! Doom keys are hidden from the game (LMB fire, RMB alt-fire, Shift dash, Q wheel, G belch,
//! C chainsaw, X quick swap, 1-8 slots) and a few ER actions are moved so Doom muscle memory works:
//!
//! | press    | ER sees           |
//! |----------|-------------------|
//! | Space    | F  (ER jump)      |
//! | F        | LMB (ER attack = melee) |
//! | Left Alt | nothing (blocked)  |
//! | Left Ctrl| X  (ER crouch)    |
//! | M        | G  (ER map)       |
//!
//! Nothing is rewritten while a menu has the cursor up.

use std::{
    ffi::c_void,
    sync::{
        OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

use hudhook::mh::{MH_ApplyQueued, MH_Initialize, MH_QueueEnableHook, MhHook};
use windows::{
    Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress, LoadLibraryW},
    core::{GUID, HRESULT, s, w},
};

pub static ENABLED: AtomicBool = AtomicBool::new(true);
/// When false (Doom movement), Space is not forwarded as ER's jump - doomslayer jumps itself.
pub static ER_JUMP: AtomicBool = AtomicBool::new(true);
/// While the weapon wheel is open the mouse steers the wheel instead of the camera.
pub static WHEEL_OPEN: AtomicBool = AtomicBool::new(false);
/// While the settings window is open the game sees no keys, buttons or mouse motion at all; the
/// motion moves the window's own cursor (CURSOR_DX / CURSOR_DY, taken by the window each frame).
pub static SETTINGS_OPEN: AtomicBool = AtomicBool::new(false);
/// Esc closed the settings window: the game doesn't see Esc until it's let go (it opened Elden
/// Ring's menu right after - user). Cleared by the settings window once Esc is up.
pub static ESC_SWALLOW: AtomicBool = AtomicBool::new(false);
pub static CURSOR_DX: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static CURSOR_DY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Keyboard keys (DIK codes) bound to Doom actions: the game doesn't see them. Follows the key
/// bindings (set_bound_keys, every frame) - a fixed list let a rebound key still trigger its
/// Elden Ring action.
static BOUND: std::sync::Mutex<[bool; 256]> = std::sync::Mutex::new([false; 256]);
/// DIK of the Doom jump key (forwarded as ER's jump when Doom movement is off).
static JUMP_DIK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(dik::SPACE);

/// Windows virtual key -> DirectInput key code (its scan code; extended keys get 0x80).
pub fn vk_to_dik(vk: u16) -> Option<usize> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{MAPVK_VK_TO_VSC_EX, MapVirtualKeyW};
    // mouse buttons (1, 2, 4, 5, 6) have no scan code
    if vk == 0 || matches!(vk, 1 | 2 | 4 | 5 | 6) {
        return None;
    }
    let sc = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
    if sc == 0 {
        return None;
    }
    let dik = (sc & 0xFF) as usize | if sc & 0xFF00 == 0xE000 { 0x80 } else { 0 };
    Some(dik & 0xFF)
}

/// The keys bound to Doom actions (both slots, weapon slots too) and the jump key.
/// The middle mouse button is bound to a Doom action (the weapon wheel by default): hidden from
/// the game, which uses it for lock-on (user: a middle click targeted an enemy).
static MIDDLE_BOUND: AtomicBool = AtomicBool::new(true);

pub fn set_bound_keys(keys: &[u16], jump: u16) {
    MIDDLE_BOUND.store(keys.contains(&0x04), Ordering::Relaxed);
    let mut b = [false; 256];
    for &vk in keys {
        if let Some(d) = vk_to_dik(vk) {
            b[d] = true;
        }
    }
    if let Ok(mut g) = BOUND.lock() {
        *g = b;
    }
    if let Some(d) = vk_to_dik(jump) {
        JUMP_DIK.store(d, Ordering::Relaxed);
    }
}

/// Mouse look scale x1000 (scoped Heavy Cannon: slower aim). 1000 = unchanged.
pub static LOOK_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1000);

/// Scale a relative mouse delta, carrying the remainder so slow movements aren't lost.
fn scale_look(v: i32, carry: &std::sync::atomic::AtomicI32) -> i32 {
    let k = LOOK_SCALE.load(Ordering::Relaxed);
    if k >= 1000 {
        return v;
    }
    let total = v * k as i32 + carry.load(Ordering::Relaxed);
    let out = total / 1000;
    carry.store(total - out * 1000, Ordering::Relaxed);
    out
}
static CARRY_X: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static CARRY_Y: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static WHEEL_DX: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
pub static WHEEL_DY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

mod dik {
    pub const N1: usize = 0x02;
    pub const N8: usize = 0x09;
    pub const Q: usize = 0x10;
    pub const F: usize = 0x21;
    pub const G: usize = 0x22;
    pub const LCTRL: usize = 0x1D;
    pub const LSHIFT: usize = 0x2A;
    pub const X: usize = 0x2D;
    pub const C: usize = 0x2E;
    pub const V: usize = 0x2F;
    pub const M: usize = 0x32;
    pub const LALT: usize = 0x38;
    pub const SPACE: usize = 0x39;
}

/// Keyboard keys the game must never see besides the bound ones: Left Ctrl (becomes ER's crouch
/// X), X itself (so only Ctrl crouches), Left Alt (ER's backstep rolled the hidden body).
const HIDE_KEYS: &[usize] = &[dik::LCTRL, dik::X, dik::LALT];
/// (physical key, key the game sees)
const KEY_TO_KEY: &[(usize, usize)] = &[
    (dik::SPACE, dik::F),
    // Left Alt used to become Space (ER dodge / backstep): blocked now - it rolled the hidden
    // body by accident (user). Still in HIDE_KEYS, so the game doesn't see it at all.
    (dik::LCTRL, dik::X),
    // M is no longer turned into G (ER's default map key; G is Flame Belch): the map is bound
    // to M in ER's own settings, so M reaches the game as itself.
];
/// Physical key that becomes a mouse button for the game: F -> LMB (ER attack, Doom melee).
/// F is Doom melee now (handled by doomslayer), so nothing is forwarded as a mouse button.
const KEY_TO_MOUSE: &[(usize, usize)] = &[];

/// Mouse-wheel notches seen since the game thread last read them (+ = up).
pub static WHEEL_NOTCHES: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
/// Which input path delivered the wheel (diagnostics): bit0 raw input, bit1 DInput, bit2 WM_MOUSEWHEEL.
pub static WHEEL_SOURCES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

static LAST_KEYS: std::sync::Mutex<[u8; 256]> = std::sync::Mutex::new([0; 256]);

type GetDeviceState = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> HRESULT;
static ORIGINAL: OnceLock<usize> = OnceLock::new();
type GetDeviceData = unsafe extern "system" fn(*mut c_void, u32, *mut u8, *mut u32, u32) -> HRESULT;
static ORIGINAL_DATA: OnceLock<usize> = OnceLock::new();

/// Buffered input (the game reads mouse *motion* this way). While the weapon wheel is open, X/Y
/// deltas (DIMOFS_X = 0, DIMOFS_Y = 4) feed the wheel and are zeroed so the camera holds still.
unsafe extern "system" fn get_device_data(
    this: *mut c_void,
    stride: u32,
    data: *mut u8,
    count: *mut u32,
    flags: u32,
) -> HRESULT {
    let original: GetDeviceData = unsafe { std::mem::transmute(*ORIGINAL_DATA.get().unwrap()) };
    let hr = unsafe { original(this, stride, data, count, flags) };
    let settings = SETTINGS_OPEN.load(Ordering::Relaxed);
    if hr.is_ok() && !data.is_null() && !count.is_null() && ESC_SWALLOW.load(Ordering::Relaxed) && stride >= 8 {
        for i in 0..unsafe { *count } as usize {
            let rec = unsafe { data.add(i * stride as usize) };
            if unsafe { (rec as *const u32).read_unaligned() } == 1 {
                unsafe { (rec.add(4) as *mut i32).write_unaligned(0) };
            }
        }
    }
    if hr.is_err() || data.is_null() || count.is_null() || !(WHEEL_OPEN.load(Ordering::Relaxed) || settings) {
        return hr;
    }
    // Only the mouse device reports offsets 0/4 as axes; keyboard offsets are DIK codes, and DIK 0 /
    // DIK 4 (unused / '3') would only be touched while the wheel is held.
    let n = unsafe { *count } as usize;
    for i in 0..n {
        let rec = unsafe { data.add(i * stride as usize) };
        let ofs = unsafe { (rec as *const u32).read_unaligned() };
        if settings && ofs < 0x100 && stride >= 8 {
            // keyboard / mouse records: nothing reaches the game while the window is open
            unsafe { (rec.add(4) as *mut i32).write_unaligned(0) };
            continue;
        }
        if ofs == 0 || ofs == 4 {
            let val = unsafe { (rec.add(4) as *const i32).read_unaligned() };
            let (a, b) = if ofs == 0 { (&WHEEL_DX, val) } else { (&WHEEL_DY, val) };
            let v = (a.load(Ordering::Relaxed) + b).clamp(-400, 400);
            a.store(v, Ordering::Relaxed);
            unsafe { (rec.add(4) as *mut i32).write_unaligned(0) };
        }
    }
    hr
}

/// Until when (ms on `clock_ms`) the movement keys / left stick are hidden from the game: set every
/// frame we're in the air (Doom movement). Elden Ring kept the hidden body in its run animation
/// mid-jump and its footsteps played (user). A deadline, so it can't stick if frames stop.
pub static AIR_HIDE_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub fn clock_ms() -> u64 {
    static T0: OnceLock<std::time::Instant> = OnceLock::new();
    T0.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Set from the game thread each frame: in the air or not.
pub fn set_air_hide(on: bool) {
    AIR_HIDE_UNTIL.store(if on { clock_ms() + 150 } else { 0 }, Ordering::Relaxed);
}

pub fn air_hide() -> bool {
    clock_ms() < AIR_HIDE_UNTIL.load(Ordering::Relaxed)
}

fn active() -> bool {
    ENABLED.load(Ordering::Relaxed) && !crate::input::cursor_visible()
}

unsafe extern "system" fn get_device_state(this: *mut c_void, size: u32, data: *mut c_void) -> HRESULT {
    let original: GetDeviceState = unsafe { std::mem::transmute(*ORIGINAL.get().unwrap()) };
    let hr = unsafe { original(this, size, data) };
    if hr.is_err() || data.is_null() {
        return hr;
    }
    // the settings window blocks everything, whatever the Windows pointer is doing (a visible
    // pointer used to switch the rewriting off - and the block with it)
    if SETTINGS_OPEN.load(Ordering::Relaxed) && matches!(size, 256 | 16 | 20) {
        let bytes = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, size as usize) };
        if size != 256 {
            CURSOR_DX.fetch_add(i32::from_le_bytes(bytes[0..4].try_into().unwrap()), Ordering::Relaxed);
            CURSOR_DY.fetch_add(i32::from_le_bytes(bytes[4..8].try_into().unwrap()), Ordering::Relaxed);
        }
        bytes.fill(0);
        return hr;
    }
    if size == 256 && ESC_SWALLOW.load(Ordering::Relaxed) {
        unsafe { *(data as *mut u8).add(1) = 0 };
    }
    if !active() {
        return hr;
    }
    match size {
        // Keyboard: 256 DIK bytes.
        256 => {
            let keys = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, 256) };
            let real: [u8; 256] = keys.try_into().unwrap();
            *LAST_KEYS.lock().unwrap() = real;
            if SETTINGS_OPEN.load(Ordering::Relaxed) {
                keys.fill(0);
                return hr;
            }
            for &k in HIDE_KEYS {
                keys[k] = 0;
            }
            if let Ok(bound) = BOUND.lock() {
                for (k, hide) in bound.iter().enumerate() {
                    if *hide {
                        keys[k] = 0;
                    }
                }
            }
            for k in dik::N1..=dik::N8 {
                keys[k] = 0;
            }
            // in the air: no W A S D for the game (its run animation's footsteps)
            if air_hide() {
                for k in [0x11usize, 0x1E, 0x1F, 0x20] {
                    keys[k] = 0;
                }
            }
            // Gun inspect mode (F10): the arrows, 9 and 0 move the gun, not the game.
            if crate::viewmodel::INSPECT_ON.load(Ordering::Relaxed) {
                for k in [0xC8usize, 0xD0, 0xCB, 0xCD, 0x0A, 0x0B] {
                    keys[k] = 0;
                }
            }
            for &(from, to) in KEY_TO_KEY {
                // (the jump entry follows the bound jump key)
                let from = if from == dik::SPACE { JUMP_DIK.load(Ordering::Relaxed) } else { from };
                if from == JUMP_DIK.load(Ordering::Relaxed) && !ER_JUMP.load(Ordering::Relaxed) {
                    continue;
                }
                keys[to] |= real[from] & 0x80;
            }
        }
        // Mouse: DIMOUSESTATE (16) or DIMOUSESTATE2 (20); buttons start at offset 12.
        16 | 20 => {
            let bytes = unsafe { std::slice::from_raw_parts_mut(data as *mut u8, size as usize) };
            if SETTINGS_OPEN.load(Ordering::Relaxed) {
                let lx = i32::from_le_bytes(bytes[0..4].try_into().unwrap());
                let ly = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
                CURSOR_DX.fetch_add(lx, Ordering::Relaxed);
                CURSOR_DY.fetch_add(ly, Ordering::Relaxed);
                bytes.fill(0);
                return hr;
            }
            if WHEEL_OPEN.load(Ordering::Relaxed) {
                let lx = i32::from_le_bytes(bytes[0..4].try_into().unwrap());
                let ly = i32::from_le_bytes(bytes[4..8].try_into().unwrap());
                // Accumulate, clamped to a "stick" radius, and keep the camera still.
                let clamp = |v: i32| v.clamp(-400, 400);
                let nx = (WHEEL_DX.load(Ordering::Relaxed) + lx).clamp(-720, 720);
                let ny = clamp(WHEEL_DY.load(Ordering::Relaxed) + ly);
                WHEEL_DX.store(nx, Ordering::Relaxed);
                WHEEL_DY.store(ny, Ordering::Relaxed);
                bytes[0..8].fill(0);
            }
            // lX / lY: slower look while scoped.
            for (o, carry) in [(0usize, &CARRY_X), (4, &CARRY_Y)] {
                let v = i32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
                bytes[o..o + 4].copy_from_slice(&scale_look(v, carry).to_le_bytes());
            }
            bytes[12] = 0; // LMB = Doom fire
            bytes[13] = 0; // RMB = Doom alt-fire
            if MIDDLE_BOUND.load(Ordering::Relaxed) {
                bytes[14] = 0; // middle = Doom weapon wheel (hold)
            }
            if size == 20 {
                bytes[15] = 0; // mouse back = Doom melee
                bytes[16] = 0; // mouse forward = Flame Belch
            }
            // lZ = wheel: Doom weapon cycling, hidden from the game.
            let lz = i32::from_le_bytes(bytes[8..12].try_into().unwrap());
            if lz != 0 {
                if WHEEL_SOURCES.load(Ordering::Relaxed) & 1 == 0 {
                    WHEEL_NOTCHES.fetch_add(if lz > 0 { 1 } else { -1 }, Ordering::Relaxed);
                }
                WHEEL_SOURCES.fetch_or(2, Ordering::Relaxed);
                bytes[8..12].fill(0);
            }
            let real = *LAST_KEYS.lock().unwrap();
            for &(from, button) in KEY_TO_MOUSE {
                bytes[12 + button] |= real[from] & 0x80;
            }
        }
        _ => {}
    }
    hr
}

// ---------------------------------------------------------------- raw input (mouse motion)

type GetRawInputBufferFn = unsafe extern "system" fn(*mut u8, *mut u32, u32) -> u32;
type GetRawInputDataFn = unsafe extern "system" fn(isize, u32, *mut u8, *mut u32, u32) -> u32;
static ORIG_RIB: OnceLock<usize> = OnceLock::new();
static ORIG_RID: OnceLock<usize> = OnceLock::new();
pub static RAW_SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
/// Mouse movement (|dx| + |dy|) and button presses since last read: the keyboard / mouse is in use.
pub static MOUSE_ACT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

const RIM_TYPEMOUSE: u32 = 0;
/// RAWINPUTHEADER is 24 bytes on x64; RAWMOUSE: usFlags u16, pad, buttons u32, ulRawButtons u32,
/// lLastX i32 @ +12, lLastY i32 @ +16.
const HDR: usize = 24;

unsafe fn steer_wheel(rec: *mut u8) {
    let kind = unsafe { (rec as *const u32).read_unaligned() };
    if kind != RIM_TYPEMOUSE {
        return;
    }
    RAW_SEEN.fetch_add(1, Ordering::Relaxed);
    let m = unsafe { rec.add(HDR) };
    // mouse use (keyboard / mouse mode vs the pad): how far it moved, or a button
    {
        let flags = unsafe { (m as *const u16).read_unaligned() };
        let bf = unsafe { (m.add(4) as *const u16).read_unaligned() };
        if flags & 1 == 0 {
            let (x, y) = unsafe { ((m.add(12) as *const i32).read_unaligned(), (m.add(16) as *const i32).read_unaligned()) };
            MOUSE_ACT.fetch_add((x.unsigned_abs() + y.unsigned_abs()).min(1000), Ordering::Relaxed);
        }
        if bf & 0x0555 != 0 {
            MOUSE_ACT.fetch_add(1000, Ordering::Relaxed);
        }
    }
    if SETTINGS_OPEN.load(Ordering::Relaxed) {
        // the settings window's cursor; the game gets nothing (buttons, wheel, motion)
        let flags = unsafe { (m as *const u16).read_unaligned() };
        let px = unsafe { m.add(12) as *mut i32 };
        let py = unsafe { m.add(16) as *mut i32 };
        if flags & 1 == 0 {
            CURSOR_DX.fetch_add(unsafe { px.read_unaligned() }, Ordering::Relaxed);
            CURSOR_DY.fetch_add(unsafe { py.read_unaligned() }, Ordering::Relaxed);
        }
        unsafe {
            (m.add(4) as *mut u16).write_unaligned(0);
            px.write_unaligned(0);
            py.write_unaligned(0);
        }
        return;
    }
    if active() {
        // RAWMOUSE.usButtonFlags @ +4, usButtonData @ +6.
        let bf = unsafe { m.add(4) as *mut u16 };
        let mut b = unsafe { bf.read_unaligned() };
        // Left/right button down/up (0x1,0x2,0x4,0x8) are Doom fire / alt-fire, not ER attacks.
        b &= !0x000F;
        // Side buttons 4/5 down/up (0x40,0x80,0x100,0x200): Doom melee / Flame Belch.
        b &= !0x03C0;
        // Middle down/up (0x10, 0x20): the Doom weapon wheel - Elden Ring's lock-on otherwise.
        if MIDDLE_BOUND.load(Ordering::Relaxed) {
            b &= !0x0030;
        }
        // RI_MOUSE_WHEEL = 0x0400: weapon cycling.
        if b & 0x0400 != 0 {
            let data = unsafe { (m.add(6) as *const i16).read_unaligned() };
            WHEEL_NOTCHES.fetch_add(if data > 0 { 1 } else { -1 }, Ordering::Relaxed);
            WHEEL_SOURCES.fetch_or(1, Ordering::Relaxed);
            b &= !0x0400;
        }
        unsafe { bf.write_unaligned(b) };
    }
    let flags = unsafe { (m as *const u16).read_unaligned() };
    if !WHEEL_OPEN.load(Ordering::Relaxed) {
        // Slower look while scoped (relative motion only).
        if active() && flags & 1 == 0 && LOOK_SCALE.load(Ordering::Relaxed) < 1000 {
            for (o, carry) in [(12usize, &CARRY_X), (16, &CARRY_Y)] {
                let p = unsafe { m.add(o) as *mut i32 };
                unsafe { p.write_unaligned(scale_look(p.read_unaligned(), carry)) };
            }
        }
        return;
    }
    if flags & 1 != 0 {
        return; // absolute (tablets / RDP)
    }
    let px = unsafe { m.add(12) as *mut i32 };
    let py = unsafe { m.add(16) as *mut i32 };
    let (x, y) = unsafe { (px.read_unaligned(), py.read_unaligned()) };
    let c = |v: i32| v.clamp(-400, 400);
    WHEEL_DX.store((WHEEL_DX.load(Ordering::Relaxed) + x).clamp(-720, 720), Ordering::Relaxed);
    WHEEL_DY.store(c(WHEEL_DY.load(Ordering::Relaxed) + y), Ordering::Relaxed);
    unsafe {
        px.write_unaligned(0);
        py.write_unaligned(0);
    }
}

unsafe extern "system" fn get_raw_input_buffer(data: *mut u8, size: *mut u32, header: u32) -> u32 {
    let orig: GetRawInputBufferFn = unsafe { std::mem::transmute(*ORIG_RIB.get().unwrap()) };
    let n = unsafe { orig(data, size, header) };
    if n == 0 || n == u32::MAX || data.is_null() {
        return n;
    }
    let mut p = data;
    for _ in 0..n {
        unsafe { steer_wheel(p) };
        let sz = unsafe { (p.add(4) as *const u32).read_unaligned() } as usize;
        // NEXTRAWINPUTBLOCK: 8-byte aligned on x64.
        p = unsafe { p.add((sz + 7) & !7) };
    }
    n
}

unsafe extern "system" fn get_raw_input_data(h: isize, cmd: u32, data: *mut u8, size: *mut u32, header: u32) -> u32 {
    let orig: GetRawInputDataFn = unsafe { std::mem::transmute(*ORIG_RID.get().unwrap()) };
    let n = unsafe { orig(h, cmd, data, size, header) };
    // RID_INPUT = 0x10000003
    if cmd == 0x1000_0003 && !data.is_null() && n != u32::MAX && n as usize >= HDR + 20 {
        unsafe { steer_wheel(data) };
    }
    n
}

pub(crate) unsafe fn hook_export(dll: windows::core::PCWSTR, name: windows::core::PCSTR, detour: *mut c_void, slot: &OnceLock<usize>) -> Result<(), String> {
    unsafe {
        let module = LoadLibraryW(dll).map_err(|e| e.to_string())?;
        let target = GetProcAddress(module, name).ok_or("export missing")? as *mut c_void;
        let hook = MhHook::new(target, detour).map_err(|e| format!("{e:?}"))?;
        slot.set(hook.trampoline() as usize).ok();
        MH_QueueEnableHook(target).ok().map_err(|e| format!("{e:?}"))?;
        std::mem::forget(hook);
        Ok(())
    }
}

const IID_IDIRECTINPUT8W: GUID = GUID::from_u128(0xBF798031_483A_4DA2_AA99_5D64ED369700);
const GUID_SYS_KEYBOARD: GUID = GUID::from_u128(0x6F1D2B61_D5A0_11CF_BFC7_444553540000);

type DirectInput8Create =
    unsafe extern "system" fn(*mut c_void, u32, *const GUID, *mut *mut c_void, *mut c_void) -> HRESULT;
type CreateDevice =
    unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void, *mut c_void) -> HRESULT;
type Release = unsafe extern "system" fn(*mut c_void) -> u32;

/// Find GetDeviceState through a throwaway keyboard device and hook it (shared by all devices).
pub fn install() {
    if let Err(e) = unsafe { try_install() } {
        log::error!("input remap not installed: {e}");
    }
}

unsafe fn try_install() -> Result<(), String> {
    unsafe {
        let module = LoadLibraryW(w!("dinput8.dll")).map_err(|e| e.to_string())?;
        let create: DirectInput8Create = std::mem::transmute(
            GetProcAddress(module, s!("DirectInput8Create")).ok_or("no DirectInput8Create")?,
        );
        let hinst = GetModuleHandleW(None).map_err(|e| e.to_string())?;
        let mut di: *mut c_void = std::ptr::null_mut();
        create(hinst.0, 0x0800, &IID_IDIRECTINPUT8W, &mut di, std::ptr::null_mut())
            .ok()
            .map_err(|e| format!("DirectInput8Create: {e}"))?;
        let di_vtbl = *(di as *const *const usize);
        let create_device: CreateDevice = std::mem::transmute(*di_vtbl.add(3));
        let mut dev: *mut c_void = std::ptr::null_mut();
        create_device(di, &GUID_SYS_KEYBOARD, &mut dev, std::ptr::null_mut())
            .ok()
            .map_err(|e| format!("CreateDevice: {e}"))?;
        let dev_vtbl = *(dev as *const *const usize);
        let target = *dev_vtbl.add(9) as *mut c_void;
        let target_data = *dev_vtbl.add(10) as *mut c_void;

        let _ = MH_Initialize(); // hudhook may have done it already
        let hook = MhHook::new(target, get_device_state as *mut c_void).map_err(|e| format!("{e:?}"))?;
        ORIGINAL.set(hook.trampoline() as usize).ok();
        MH_QueueEnableHook(target).ok().map_err(|e| format!("{e:?}"))?;
        let hook_data =
            MhHook::new(target_data, get_device_data as *mut c_void).map_err(|e| format!("{e:?}"))?;
        ORIGINAL_DATA.set(hook_data.trampoline() as usize).ok();
        MH_QueueEnableHook(target_data).ok().map_err(|e| format!("{e:?}"))?;
        std::mem::forget(hook_data);
        hook_export(w!("user32.dll"), s!("GetRawInputBuffer"), get_raw_input_buffer as *mut c_void, &ORIG_RIB)?;
        hook_export(w!("user32.dll"), s!("GetRawInputData"), get_raw_input_data as *mut c_void, &ORIG_RID)?;
        MH_ApplyQueued().ok().map_err(|e| format!("{e:?}"))?;
        std::mem::forget(hook);

        let release_dev: Release = std::mem::transmute(*dev_vtbl.add(2));
        release_dev(dev);
        let release_di: Release = std::mem::transmute(*di_vtbl.add(2));
        release_di(di);
        log::info!("input remap: GetDeviceState hooked at {target:p}");
        Ok(())
    }
}
