//! Gamepad: Xbox pads, and PlayStation pads through Steam Input - Elden Ring reads both with
//! XInput (XINPUT1_4.dll), so one hook covers them (user, 2026-10-08).
//!
//! Pad buttons are their own key category: codes 0x200.. that never collide with keyboard / mouse
//! virtual keys (< 0x100), so a binding can only ever be one or the other. The game sees the pad
//! with every Doom-bound button hidden (like the keyboard's bound keys), and the Doom interact
//! button pressed as Elden Ring's own interact button.
//!
//! Which icons to show (Xbox / PlayStation): a Sony controller connected = PlayStation icons
//! (`pad_icons` = "auto" | "xbox" | "ps"). Keyboard or pad mode: whichever was used last.

use std::sync::{
    atomic::{AtomicBool, AtomicU16, AtomicU32, Ordering},
    Mutex, OnceLock,
};

pub const DPAD_UP: u16 = 0x200;
pub const DPAD_DOWN: u16 = 0x201;
pub const DPAD_LEFT: u16 = 0x202;
pub const DPAD_RIGHT: u16 = 0x203;
pub const START: u16 = 0x204;
pub const BACK: u16 = 0x205;
pub const LS: u16 = 0x206;
pub const RS: u16 = 0x207;
pub const LB: u16 = 0x208;
pub const RB: u16 = 0x209;
pub const A: u16 = 0x20C;
pub const B: u16 = 0x20D;
pub const X: u16 = 0x20E;
pub const Y: u16 = 0x20F;
pub const LT: u16 = 0x210;
pub const RT: u16 = 0x211;

/// Every pad code (buttons, then the two triggers).
pub const ALL: [u16; 16] = [DPAD_UP, DPAD_DOWN, DPAD_LEFT, DPAD_RIGHT, START, BACK, LS, RS, LB, RB, A, B, X, Y, LT, RT];

pub fn is_pad(code: u16) -> bool {
    (DPAD_UP..=RT).contains(&code)
}

/// XInput button bit for a pad button code (triggers have none).
fn bit(code: u16) -> u16 {
    if (DPAD_UP..=Y).contains(&code) { 1 << (code - DPAD_UP) } else { 0 }
}

/// A trigger counts as pressed past this (0-255).
const TRIGGER_ON: u8 = 40;
/// Stick dead zone (XInput's own recommendation for the left stick).
pub const DEAD: f32 = 7849.0 / 32767.0;

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Gamepad {
    pub buttons: u16,
    pub lt: u8,
    pub rt: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}

impl Gamepad {
    pub fn down(&self, code: u16) -> bool {
        match code {
            LT => self.lt > TRIGGER_ON,
            RT => self.rt > TRIGGER_ON,
            c if is_pad(c) => self.buttons & bit(c) != 0,
            _ => false,
        }
    }

    /// Stick as (x, y) in [-1, 1] with the dead zone taken out (y up = +).
    pub fn left(&self) -> (f32, f32) {
        stick(self.lx, self.ly)
    }

    pub fn right(&self) -> (f32, f32) {
        stick(self.rx, self.ry)
    }

    /// Anything pressed or pushed: the pad is being used.
    pub fn active(&self) -> bool {
        let (l, r) = (self.left(), self.right());
        self.buttons != 0 || self.lt > TRIGGER_ON || self.rt > TRIGGER_ON || l.0 != 0.0 || l.1 != 0.0 || r.0 != 0.0 || r.1 != 0.0
    }
}

fn stick(x: i16, y: i16) -> (f32, f32) {
    let (x, y) = (x as f32 / 32767.0, y as f32 / 32767.0);
    let m = (x * x + y * y).sqrt();
    if m < DEAD {
        return (0.0, 0.0);
    }
    // rescale so the edge of the dead zone is 0 (no jump when the stick leaves it)
    let k = ((m - DEAD) / (1.0 - DEAD)).min(1.0) / m;
    (x * k, y * k)
}

#[repr(C)]
struct XState {
    packet: u32,
    pad: Gamepad,
}

type GetState = unsafe extern "system" fn(u32, *mut XState) -> u32;
static ORIG: OnceLock<usize> = OnceLock::new();
/// Calls of the hooked XInputGetState (diagnostics: the game is reading the pad through it).
pub static HOOK_CALLS: AtomicU32 = AtomicU32::new(0);

/// Buttons the game must not see (Doom-bound), and the bound triggers.
static HIDE: AtomicU16 = AtomicU16::new(0);
static HIDE_LT: AtomicBool = AtomicBool::new(false);
static HIDE_RT: AtomicBool = AtomicBool::new(false);
/// Buttons held when the settings window closed (its close / the last click): hidden until let
/// go, so the game doesn't take them as a fresh press.
static LATCH: AtomicU16 = AtomicU16::new(0);
/// (Doom interact button bit << 16) | Elden Ring's interact button bit: pressing ours presses theirs.
static INTERACT: AtomicU32 = AtomicU32::new(0);

/// The pad buttons bound to Doom actions (from the bindings, every frame).
pub fn set_bound(codes: &[u16], interact: u16, er_interact: u16) {
    let mut hide = 0u16;
    let (mut lt, mut rt) = (false, false);
    for &c in codes.iter().chain([&interact]) {
        match c {
            LT => lt = true,
            RT => rt = true,
            c if is_pad(c) => hide |= bit(c),
            _ => {}
        }
    }
    HIDE.store(hide, Ordering::Relaxed);
    HIDE_LT.store(lt, Ordering::Relaxed);
    HIDE_RT.store(rt, Ordering::Relaxed);
    INTERACT.store(((bit(interact) as u32) << 16) | bit(er_interact) as u32, Ordering::Relaxed);
}

/// In gameplay (in the world, no Elden Ring menu, loading screen or cutscene): only then are the
/// Doom buttons hidden from the game - its menus need A / B / X / Y and the rest (user: the pad
/// did nothing in Elden Ring's menu). Set by the game thread every frame.
pub static GAMEPLAY: AtomicBool = AtomicBool::new(false);

/// Hide what's held right now until it's let go (settings window closed).
pub fn latch_held() {
    LATCH.store(current().buttons, Ordering::Relaxed);
}

unsafe extern "system" fn get_state(index: u32, st: *mut XState) -> u32 {
    let orig: GetState = unsafe { std::mem::transmute(*ORIG.get().unwrap()) };
    let r = unsafe { orig(index, st) };
    if r != 0 || st.is_null() {
        return r;
    }
    HOOK_CALLS.fetch_add(1, Ordering::Relaxed);
    // Doom layer off (F9): plain Elden Ring
    if !crate::remap::ENABLED.load(Ordering::Relaxed) {
        return r;
    }
    let s = unsafe { &mut (*st).pad };
    if crate::remap::SETTINGS_OPEN.load(Ordering::Relaxed) {
        // the settings window has the pad (cursor, clicks): the game gets nothing
        *s = Gamepad::default();
        return r;
    }
    if !GAMEPLAY.load(Ordering::Relaxed) {
        // Elden Ring's menus, title screen, loading, cutscenes: the pad is the game's
        LATCH.store(0, Ordering::Relaxed);
        return r;
    }
    let real = *s;
    let latch = LATCH.load(Ordering::Relaxed) & real.buttons;
    LATCH.store(latch, Ordering::Relaxed);
    s.buttons &= !(HIDE.load(Ordering::Relaxed) | latch);
    if HIDE_LT.load(Ordering::Relaxed) {
        s.lt = 0;
    }
    if HIDE_RT.load(Ordering::Relaxed) {
        s.rt = 0;
    }
    let ia = INTERACT.load(Ordering::Relaxed);
    if (ia >> 16) as u16 & real.buttons != 0 {
        s.buttons |= ia as u16;
    }
    // in the air: no left stick for the game (its run animation's footsteps - remap::air_hide)
    if crate::remap::air_hide() {
        s.lx = 0;
        s.ly = 0;
    }
    // the weapon wheel steers with the right stick: the camera holds still
    if crate::remap::WHEEL_OPEN.load(Ordering::Relaxed) {
        s.rx = 0;
        s.ry = 0;
    }
    r
}

/// The real pad (every connected controller merged), read without the game's view of it.
fn read() -> Gamepad {
    let f: GetState = match ORIG.get() {
        Some(p) => unsafe { std::mem::transmute(*p) },
        None => return Gamepad::default(),
    };
    let mut out = Gamepad::default();
    // the slot being used decides the icons (a DualSense plugged in gave the Xbox pad PS icons -
    // user); a real maker (Microsoft / Sony) wins over Steam's virtual copy of the same pad
    let mut used: Option<u16> = None;
    for i in 0..4 {
        let mut s = XState { packet: 0, pad: Gamepad::default() };
        if unsafe { f(i, &mut s) } != 0 {
            continue;
        }
        let p = s.pad;
        if p.active() {
            let vid = SLOT_VID[i as usize].load(Ordering::Relaxed);
            if used.is_none_or(|u| !matches!(u, 0x045E | 0x054C)) {
                used = Some(vid);
            }
        }
        out.buttons |= p.buttons;
        out.lt = out.lt.max(p.lt);
        out.rt = out.rt.max(p.rt);
        let mag = |x: i16, y: i16| (x as i32).pow(2) + (y as i32).pow(2);
        if mag(p.lx, p.ly) > mag(out.lx, out.ly) {
            (out.lx, out.ly) = (p.lx, p.ly);
        }
        if mag(p.rx, p.ry) > mag(out.rx, out.ry) {
            (out.rx, out.ry) = (p.rx, p.ry);
        }
    }
    if let Some(v) = used {
        if ACTIVE_VID.swap(v, Ordering::Relaxed) != v {
            log::info!("gamepad: using a {} pad (vendor {v:04X})", match v { 0x054C => "Sony", 0x045E => "Microsoft", 0x28DE => "Steam virtual", _ => "other" });
        }
    }
    out
}

/// Maker of the pad in each XInput slot (XInputGetCapabilitiesEx: Microsoft 045E, Sony 054C,
/// Steam Input's virtual pad 28DE), and of the one in use.
static SLOT_VID: [AtomicU16; 4] = [AtomicU16::new(0), AtomicU16::new(0), AtomicU16::new(0), AtomicU16::new(0)];
static ACTIVE_VID: AtomicU16 = AtomicU16::new(0);

#[repr(C)]
#[derive(Default)]
struct CapsEx {
    kind: u8,
    sub: u8,
    flags: u16,
    pad: Gamepad,
    vib: [u16; 2],
    vendor: u16,
    product: u16,
    version: u16,
    _u1: u16,
    _u2: u32,
}

type GetCapsEx = unsafe extern "system" fn(u32, u32, u32, *mut CapsEx) -> u32;

fn scan_slots() {
    use windows::{core::PCSTR, Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW}};
    static F: OnceLock<Option<usize>> = OnceLock::new();
    let f = F.get_or_init(|| unsafe {
        let m = LoadLibraryW(windows::core::w!("xinput1_4.dll")).ok()?;
        // XInputGetCapabilitiesEx is exported by ordinal 108 only
        GetProcAddress(m, PCSTR(108 as *const u8)).map(|p| p as usize)
    });
    let Some(f) = f else { return };
    let f: GetCapsEx = unsafe { std::mem::transmute(*f) };
    for i in 0..4u32 {
        let mut c = CapsEx::default();
        let vid = if unsafe { f(1, i, 0, &mut c) } == 0 { c.vendor } else { 0 };
        if SLOT_VID[i as usize].swap(vid, Ordering::Relaxed) != vid {
            log::info!("gamepad: slot {i} vendor {vid:04X} product {:04X}", c.product);
        }
    }
}

static CURRENT: Mutex<Gamepad> = Mutex::new(Gamepad { buttons: 0, lt: 0, rt: 0, lx: 0, ly: 0, rx: 0, ry: 0 });

/// Read the pad for this frame (game thread); everyone else uses `current()`.
pub fn poll() -> Gamepad {
    let p = read();
    if let Ok(mut c) = CURRENT.lock() {
        *c = p;
    }
    scan_devices();
    p
}

pub fn current() -> Gamepad {
    CURRENT.lock().map(|c| *c).unwrap_or_default()
}

/// The pad was used last (not the keyboard / mouse): HUD tags and the binding screen show pad
/// buttons.
pub static PAD_MODE: AtomicBool = AtomicBool::new(false);

pub fn pad_mode() -> bool {
    PAD_MODE.load(Ordering::Relaxed)
}

/// A Sony controller is connected (USB / Bluetooth HID, vendor 0x054C).
static SONY: AtomicBool = AtomicBool::new(false);

/// HID devices rescanned every 2 s (plugging in the PS5 pad switches the icons).
fn scan_devices() {
    use windows::Win32::UI::Input::{GetRawInputDeviceInfoW, GetRawInputDeviceList, RAWINPUTDEVICELIST, RID_DEVICE_INFO, RIDI_DEVICEINFO, RIM_TYPEHID};
    static NEXT: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let now = std::time::Instant::now();
    if let Ok(mut n) = NEXT.lock() {
        if n.is_some_and(|t| now < t) {
            return;
        }
        *n = Some(now + std::time::Duration::from_secs(2));
    }
    scan_slots();
    let mut count = 0u32;
    let sz = size_of::<RAWINPUTDEVICELIST>() as u32;
    unsafe {
        if GetRawInputDeviceList(None, &mut count, sz) != 0 || count == 0 {
            return;
        }
        let mut list = vec![RAWINPUTDEVICELIST::default(); count as usize];
        let n = GetRawInputDeviceList(Some(list.as_mut_ptr()), &mut count, sz);
        if n == u32::MAX {
            return;
        }
        let mut sony = false;
        for d in list.iter().take(n as usize).filter(|d| d.dwType == RIM_TYPEHID) {
            let mut info = RID_DEVICE_INFO { cbSize: size_of::<RID_DEVICE_INFO>() as u32, ..Default::default() };
            let mut isz = info.cbSize;
            if GetRawInputDeviceInfoW(Some(d.hDevice), RIDI_DEVICEINFO, Some(&mut info as *mut _ as *mut _), &mut isz) == u32::MAX {
                continue;
            }
            // usage page 1 (generic desktop), usage 4 / 5 (joystick / gamepad)
            let hid = info.Anonymous.hid;
            if hid.dwVendorId == 0x054C && hid.usUsagePage == 1 && matches!(hid.usUsage, 4 | 5) {
                sony = true;
            }
        }
        // Steam Input hides the pads it translates from the game's own device lists (a wired
        // DualSense never showed up here - user): Windows' device tree isn't hidden
        let sony = sony || sony_in_device_tree();
        if sony != SONY.swap(sony, Ordering::Relaxed) {
            log::info!("gamepad: Sony controller {}", if sony { "connected - PlayStation icons" } else { "gone - Xbox icons" });
        }
    }
}

/// A Sony controller in Windows' device tree (present HID devices "HID\VID_054C..."), read through
/// cfgmgr32 - Steam's in-game hooks don't hide it there.
fn sony_in_device_tree() -> bool {
    use windows::{core::{s, w, PCWSTR}, Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW}};
    type Size = unsafe extern "system" fn(*mut u32, PCWSTR, u32) -> u32;
    type List = unsafe extern "system" fn(PCWSTR, *mut u16, u32, u32) -> u32;
    // CM_GETIDLIST_FILTER_ENUMERATOR | CM_GETIDLIST_FILTER_PRESENT
    const FLAGS: u32 = 0x1 | 0x100;
    unsafe {
        let Ok(m) = LoadLibraryW(w!("cfgmgr32.dll")) else { return false };
        let (Some(size), Some(list)) = (GetProcAddress(m, s!("CM_Get_Device_ID_List_SizeW")), GetProcAddress(m, s!("CM_Get_Device_ID_ListW"))) else {
            return false;
        };
        let size: Size = std::mem::transmute(size);
        let list: List = std::mem::transmute(list);
        let mut len = 0u32;
        if size(&mut len, w!("HID"), FLAGS) != 0 || len == 0 {
            return false;
        }
        let mut buf = vec![0u16; len as usize];
        if list(w!("HID"), buf.as_mut_ptr(), len, FLAGS) != 0 {
            return false;
        }
        String::from_utf16_lossy(&buf).to_uppercase().contains("VID_054C")
    }
}

/// PlayStation icons? (`pad_icons` = "ps" / "xbox" forces it; "auto" = a Sony pad is connected)
pub fn ps_icons() -> bool {
    match crate::config::get_cached().pad_icons.as_str() {
        "ps" => true,
        "xbox" => false,
        // the pad in use: Sony / Microsoft; Steam's virtual pad (or unknown): a Sony pad connected
        _ => match ACTIVE_VID.load(Ordering::Relaxed) {
            0x054C => true,
            0x045E => false,
            _ => SONY.load(Ordering::Relaxed),
        },
    }
}

/// doom_ui texture of a pad button's icon in the current set ("pad_xb_joy1", "pad_ps_trigger_left").
pub fn icon(code: u16) -> Option<String> {
    let n = match code {
        DPAD_UP => "dpad_up",
        DPAD_DOWN => "dpad_down",
        DPAD_LEFT => "dpad_left",
        DPAD_RIGHT => "dpad_right",
        START => "joy9",
        BACK => "joy10",
        LS => "joy7",
        RS => "joy8",
        LB => "joy5",
        RB => "joy6",
        A => "joy1",
        B => "joy2",
        X => "joy3",
        Y => "joy4",
        LT => "trigger_left",
        RT => "trigger_right",
        _ => return None,
    };
    Some(format!("pad_{}_{n}", if ps_icons() { "ps" } else { "xb" }))
}

/// What an action shows: its pad button in pad mode, its key otherwise.
pub fn shown(kb: u16, pad: u16) -> u16 {
    if pad_mode() { pad } else { kb }
}

/// Short text for a button in the current set (text-only places like "GLORY KILL [RS]").
pub fn label(code: u16) -> &'static str {
    if !ps_icons() {
        return name(code);
    }
    match code {
        START => "OPTIONS",
        BACK => "CREATE",
        LS => "L3",
        RS => "R3",
        LB => "L1",
        RB => "R1",
        A => "CROSS",
        B => "CIRCLE",
        X => "SQUARE",
        Y => "TRIANGLE",
        LT => "L2",
        RT => "R2",
        c => name(c),
    }
}

/// Text name (logs, diagnostics).
pub fn name(code: u16) -> &'static str {
    match code {
        DPAD_UP => "DPAD UP",
        DPAD_DOWN => "DPAD DOWN",
        DPAD_LEFT => "DPAD LEFT",
        DPAD_RIGHT => "DPAD RIGHT",
        START => "MENU",
        BACK => "VIEW",
        LS => "LS",
        RS => "RS",
        LB => "LB",
        RB => "RB",
        A => "A",
        B => "B",
        X => "X",
        Y => "Y",
        LT => "LT",
        RT => "RT",
        _ => "?",
    }
}

pub fn install() {
    use windows::core::{s, w};
    match unsafe { crate::remap::hook_export(w!("xinput1_4.dll"), s!("XInputGetState"), get_state as *mut std::ffi::c_void, &ORIG) } {
        Ok(()) => {
            let r = unsafe { hudhook::mh::MH_ApplyQueued() };
            log::info!("gamepad: XInputGetState hooked ({r:?})");
        }
        Err(e) => log::error!("gamepad: XInputGetState not hooked: {e}"),
    }
}
