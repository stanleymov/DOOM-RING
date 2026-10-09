//! The DOOM RING settings window (F1): difficulty, chainsaw limit, sound and music volume, HUD
//! on/off and two keys per action. Built from Doom Eternal's own settings-menu pieces
//! (tools/convert_hud_textures.py: st_* textures) and its letter fonts.
//!
//! The game is still running underneath: while the window is open the game and our own controls
//! get no input (remap::SETTINGS_OPEN), and the mouse moves the window's own cursor. Every change
//! is written into doomslayer.toml at once (config::save_values), which reloads like a hand edit.
//! Layout is live while it gets tuned: [hud] settings / set_* entries (see `lay`), set_textx for text x.

use std::sync::{Mutex, atomic::Ordering};

use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;

use crate::config::{Config, Keys, get_cached, save_values};
use crate::doomhud::{Ctx, Rgba, alpha, col, key_label};
use crate::remap::{CURSOR_DX, CURSOR_DY, SETTINGS_OPEN};

const LIME: Rgba = [0.80, 0.93, 0.22, 1.0];
const WHITE: Rgba = [0.93, 0.93, 0.90, 1.0];
const GREY: Rgba = [0.59, 0.60, 0.59, 1.0];
const ROW: Rgba = [0.37, 0.38, 0.37, 1.0];
const RED: Rgba = [0.96, 0.16, 0.10, 1.0];
const TAG: Rgba = [0.96, 0.86, 0.33, 0.95];
const DARK: Rgba = [0.07, 0.06, 0.02, 1.0];

/// (label, toml key, toml key of the second slot)
const ACTIONS: [(&str, &str, &str); 13] = [
    ("FIRE", "fire", "fire_alt"),
    ("WEAPON MOD", "alt_fire", "alt_fire_alt"),
    ("DASH", "dash", "dash_alt"),
    ("JUMP", "jump", "jump_alt"),
    ("MELEE", "melee", "melee_alt"),
    ("CHAINSAW", "chainsaw", "chainsaw_alt"),
    ("FLAME BELCH", "flame_belch", "flame_belch_alt"),
    ("CRUCIBLE", "crucible", "crucible_alt"),
    ("WEAPON WHEEL", "weapon_wheel", "weapon_wheel_alt"),
    ("MARK ENEMY", "mark", "mark_alt"),
    ("DOOM MODE ON / OFF", "doom_toggle", "doom_toggle_alt"),
    ("UNSTICK", "unstick", "unstick_alt"),
    ("SETTINGS", "settings", "settings_alt"),
];

/// The controller column: the same 13 actions plus INTERACT (the pad's own interact button).
const PAD_ROWS: usize = 14;

fn pads_of(k: &Keys) -> [u16; PAD_ROWS] {
    [
        k.fire_pad, k.alt_fire_pad, k.dash_pad, k.jump_pad, k.melee_pad, k.chainsaw_pad, k.flame_belch_pad,
        k.crucible_pad, k.weapon_wheel_pad, k.mark_pad, k.doom_toggle_pad, k.unstick_pad, k.settings_pad, k.interact_pad,
    ]
}

fn save_pads(p: &[u16; PAD_ROWS]) {
    let mut ups: Vec<(&str, String, String)> = Vec::new();
    for (i, (_, a, _)) in ACTIONS.iter().enumerate() {
        ups.push(("keys", format!("{a}_pad"), format!("0x{:03X}", p[i])));
    }
    ups.push(("keys", "interact_pad".into(), format!("0x{:03X}", p[PAD_ROWS - 1])));
    let refs: Vec<(&str, &str, String)> = ups.iter().map(|(s, k, v)| (*s, k.as_str(), v.clone())).collect();
    save_values(&refs);
}

fn keys_of(k: &Keys) -> [[u16; 2]; 13] {
    [
        [k.fire, k.fire_alt],
        [k.alt_fire, k.alt_fire_alt],
        [k.dash, k.dash_alt],
        [k.jump, k.jump_alt],
        [k.melee, k.melee_alt],
        [k.chainsaw, k.chainsaw_alt],
        [k.flame_belch, k.flame_belch_alt],
        [k.crucible, k.crucible_alt],
        [k.weapon_wheel, k.weapon_wheel_alt],
        [k.mark, k.mark_alt],
        [k.doom_toggle, k.doom_toggle_alt],
        [k.unstick, k.unstick_alt],
        [k.settings, k.settings_alt],
    ]
}

#[derive(Clone, Copy)]
struct Vals {
    enemy_hp: f32,
    dmg: f32,
    saw_hp: f32,
    cr_hp: f32,
    sfx: f32,
    music_vol: f32,
    music_on: bool,
    show_hud: bool,
    glory: bool,
}

impl Vals {
    fn of(c: &Config) -> Self {
        Vals {
            enemy_hp: c.enemy_hp_mult,
            dmg: c.weapon_damage_mult,
            saw_hp: c.glory_heavy_hp,
            cr_hp: c.crucible_hp,
            sfx: c.volume,
            music_vol: c.music_volume,
            music_on: c.music,
            show_hud: c.show_hud,
            glory: c.glory_kills,
        }
    }
}

/// Sliders: (min, max, step)
const ENEMY_HP: (f32, f32, f32) = (0.5, 5.0, 0.1);
const DMG: (f32, f32, f32) = (0.25, 3.0, 0.05);
const SAW_HP: (f32, f32, f32) = (100.0, 10000.0, 100.0);
const VOL: (f32, f32, f32) = (0.0, 1.0, 0.01);

struct State {
    cursor: [f32; 2],
    lmb: bool,
    toggle: bool,
    esc: bool,
    vals: Vals,
    keys: [[u16; 2]; 13],
    pads: [u16; PAD_ROWS],
    /// Waiting for a key: (action, slot, keys that were down when it started, a controller
    /// binding?). A keyboard binding only takes keys / mouse buttons, a controller binding only
    /// pad buttons (user: no mixing the two).
    capture: Option<(usize, usize, [bool; 256], bool)>,
    /// The pad last frame (its own edges: open / close, A clicks, B closes).
    pad_prev: crate::gamepad::Gamepad,
    /// Last frame's time (the stick moves the cursor at a speed).
    last: Option<std::time::Instant>,
    /// Slider being dragged (0 enemy hp, 1 damage, 2 effects, 3 music)
    drag: Option<usize>,
    /// Arrow held down: (row, direction, pressed at, last step) - it repeats after a moment (user)
    hold: Option<(usize, i32, std::time::Instant, std::time::Instant)>,
    /// The game window had focus last frame (on getting it back the cursor jumps to the pointer)
    focused: bool,
}

/// Where the Windows mouse pointer is, in render pixels of the game window (None if outside it).
fn os_pointer(w: f32, h: f32) -> Option<[f32; 2]> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowInfo, WINDOWINFO};
    let mut pt = POINT::default();
    let mut wi = WINDOWINFO { cbSize: size_of::<WINDOWINFO>() as u32, ..Default::default() };
    unsafe {
        GetCursorPos(&mut pt).ok()?;
        GetWindowInfo(GetForegroundWindow(), &mut wi).ok()?;
    }
    let rc = wi.rcClient;
    let (cw, ch) = ((rc.right - rc.left) as f32, (rc.bottom - rc.top) as f32);
    if cw <= 0.0 || ch <= 0.0 {
        return None;
    }
    let (x, y) = ((pt.x - rc.left) as f32 / cw, (pt.y - rc.top) as f32 / ch);
    ((0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)).then_some([x * w, y * h])
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

fn async_down(vk: u16) -> bool {
    vk != 0 && unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000 != 0
}

pub fn is_open() -> bool {
    SETTINGS_OPEN.load(Ordering::Relaxed)
}

/// Close (no Doom HUD: menus, loading, death).
pub fn close() {
    if SETTINGS_OPEN.swap(false, Ordering::Relaxed) {
        if let Ok(mut s) = STATE.lock() {
            if let Some(s) = s.as_mut() {
                s.capture = None;
                s.drag = None;
            }
        }
    }
}

fn fmt_f(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.contains('.') { s.to_string() } else { format!("{s}.0") }
}

fn save_vals(v: &Vals) {
    save_values(&[
        ("", "enemy_hp_mult", fmt_f(v.enemy_hp)),
        ("", "weapon_damage_mult", fmt_f(v.dmg)),
        ("", "glory_heavy_hp", fmt_f(v.saw_hp)),
        ("", "crucible_hp", fmt_f(v.cr_hp)),
        ("", "volume", fmt_f(v.sfx)),
        ("", "music_volume", fmt_f(v.music_vol)),
        ("", "music", v.music_on.to_string()),
        ("", "show_hud", v.show_hud.to_string()),
        ("", "glory_kills", v.glory.to_string()),
    ]);
}

fn save_keys(k: &[[u16; 2]; 13]) {
    let mut ups: Vec<(&str, &str, String)> = Vec::new();
    for (i, (_, a, b)) in ACTIONS.iter().enumerate() {
        ups.push(("keys", a, format!("0x{:02X}", k[i][0])));
        ups.push(("keys", b, format!("0x{:02X}", k[i][1])));
    }
    save_values(&ups);
}

/// Live layout value: [hud] `name` = [a, b, c, d], defaults when not set.
fn lay(name: &str, d: [f32; 4]) -> [f32; 4] {
    get_cached().hud.get(name).copied().unwrap_or(d)
}

struct Frame<'a, 'b> {
    c: &'a Ctx<'b>,
    k: f32,
    ox: f32,
    oy: f32,
    a: f32,
}

impl Frame<'_, '_> {
    /// 1080p layout point -> screen
    fn p(&self, x: f32, y: f32) -> [f32; 2] {
        [self.c.w * 0.5 + (x - 960.0 + self.ox) * self.k, self.c.h * 0.5 + (y - 540.0 + self.oy) * self.k]
    }

    fn tex(&self, name: &str, x0: f32, y0: f32, x1: f32, y1: f32, tint: Rgba) {
        if let Some(t) = self.c.tex.get(name) {
            self.c.dl.add_image(t.id, self.p(x0, y0), self.p(x1, y1)).col(col(alpha(tint, tint[3] * self.a))).build();
        }
    }

    fn tex_flip(&self, name: &str, x0: f32, y0: f32, x1: f32, y1: f32, tint: Rgba) {
        if let Some(t) = self.c.tex.get(name) {
            self.c.dl.add_image(t.id, self.p(x0, y0), self.p(x1, y1)).uv_min([1.0, 0.0]).uv_max([0.0, 1.0]).col(col(alpha(tint, tint[3] * self.a))).build();
        }
    }

    /// Upside down.
    fn tex_vflip(&self, name: &str, x0: f32, y0: f32, x1: f32, y1: f32, tint: Rgba) {
        if let Some(t) = self.c.tex.get(name) {
            self.c.dl.add_image(t.id, self.p(x0, y0), self.p(x1, y1)).uv_min([0.0, 1.0]).uv_max([1.0, 0.0]).col(col(alpha(tint, tint[3] * self.a))).build();
        }
    }

    fn rect(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, filled: bool) {
        self.c.dl.add_rect(self.p(x0, y0), self.p(x1, y1), col(alpha(c, c[3] * self.a))).filled(filled).thickness(2.0 * self.k).build();
    }

    /// Doom letters, `cap` px tall (1080p), centred on y. align: 0 left, 1 centre, 2 right.
    fn text(&self, x: f32, y: f32, cap: f32, c: Rgba, t: &str, heavy: bool, align: u8) -> f32 {
        let Some(f) = self.c.doom_font(heavy) else { return 0.0 };
        let cap_px = cap * self.k;
        let tr = cap * 0.06 * self.k;
        let (l, r) = self.c.doom_text_span(f, cap_px, t, tr, 1.0);
        let w = r - l;
        let [sx, sy] = self.p(x, y);
        let sx = match align {
            1 => sx - w * 0.5,
            2 => sx - w,
            _ => sx,
        };
        self.c.doom_text(f, sx, sy, cap_px, alpha(c, c[3] * self.a), t, tr, 1.0, false);
        w / self.k
    }

    /// Width of a text in layout units (what `text` would return), without drawing it.
    fn width(&self, cap: f32, t: &str, heavy: bool) -> f32 {
        let Some(f) = self.c.doom_font(heavy) else { return 0.0 };
        let (l, r) = self.c.doom_text_span(f, cap * self.k, t, cap * 0.06 * self.k, 1.0);
        (r - l) / self.k
    }

    fn hit(&self, cur: [f32; 2], x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
        let a = self.p(x0, y0);
        let b = self.p(x1, y1);
        cur[0] >= a[0] && cur[0] <= b[0] && cur[1] >= a[1] && cur[1] <= b[1]
    }

    /// screen x -> layout x
    fn lx(&self, sx: f32) -> f32 {
        (sx - self.c.w * 0.5) / self.k + 960.0 - self.ox
    }
}

/// Draw and run the window (render thread, every frame the Doom HUD is up).
pub fn frame(c: &Ctx) {
    let cfg = get_cached();
    let mut guard = match STATE.lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let st = guard.get_or_insert_with(|| State {
        cursor: [c.w * 0.5, c.h * 0.5],
        lmb: false,
        toggle: false,
        esc: false,
        vals: Vals::of(&cfg),
        keys: keys_of(&cfg.keys),
        pads: pads_of(&cfg.keys),
        capture: None,
        pad_prev: crate::gamepad::Gamepad::default(),
        last: None,
        drag: None,
        hold: None,
        focused: true,
    });

    // the pad: its own edges (render thread; the game thread polls it)
    let pad = crate::gamepad::current();
    let pad_prev = std::mem::replace(&mut st.pad_prev, pad);
    let pad_hit = |code: u16| code != 0 && pad.down(code) && !pad_prev.down(code);
    let now = std::time::Instant::now();
    let dt = st.last.map_or(0.0, |t| now.duration_since(t).as_secs_f32()).min(0.1);
    st.last = Some(now);

    // open / close: the settings key(s) or its pad button, Esc closes
    let toggle = async_down(cfg.keys.settings) || async_down(cfg.keys.settings_alt);
    let pad_toggle = pad_hit(cfg.keys.settings_pad) && st.capture.is_none() && crate::input::game_has_focus();
    let esc = async_down(0x1B);
    let toggle_pressed = toggle && !st.toggle && st.capture.is_none() && crate::input::game_has_focus();
    let esc_pressed = esc && !st.esc;
    if !esc {
        crate::remap::ESC_SWALLOW.store(false, Ordering::Relaxed);
    }
    st.toggle = toggle;
    st.esc = esc;
    let mut open = is_open();
    if toggle_pressed || pad_toggle {
        open = !open;
        if open {
            st.vals = Vals::of(&cfg);
            st.keys = keys_of(&cfg.keys);
            st.pads = pads_of(&cfg.keys);
            st.cursor = [c.w * 0.5, c.h * 0.5];
            CURSOR_DX.store(0, Ordering::Relaxed);
            CURSOR_DY.store(0, Ordering::Relaxed);
            st.lmb = true; // the first click counts only after a fresh press
            crate::audio::play_vol("weapon_switch", 0.4);
        }
        SETTINGS_OPEN.store(open, Ordering::Relaxed);
        if !open {
            // the button that closed it isn't a fresh press for the game
            crate::gamepad::latch_held();
        }
    }
    if !open {
        return;
    }
    // B closes (or cancels a binding); Menu cancels a controller binding
    if pad_hit(crate::gamepad::B) && st.capture.is_none() {
        SETTINGS_OPEN.store(false, Ordering::Relaxed);
        crate::gamepad::latch_held();
        return;
    }
    if pad_hit(crate::gamepad::START) && st.capture.is_some_and(|c| c.3) {
        st.capture = None;
    }
    if esc_pressed {
        if st.capture.is_some() {
            st.capture = None;
        } else {
            // the game must not see this Esc (it opened Elden Ring's menu)
            crate::remap::ESC_SWALLOW.store(true, Ordering::Relaxed);
            SETTINGS_OPEN.store(false, Ordering::Relaxed);
            return;
        }
    }

    // the window's cursor
    let cur_lay = lay("set_cursor", [1.0, 1.0, 0.0, 0.0]);
    let speed = cur_lay[0].clamp(0.2, 5.0) * c.s.max(1.0);
    // Only while the game is the active window: raw mouse motion keeps arriving when the mouse is
    // over another program, and moved this cursor from there (user). Clicks too.
    let focused = crate::input::game_has_focus();
    // back in the game (alt-tab, a click into the window): the cursor starts where the real
    // pointer came in (user)
    if focused && !st.focused {
        if let Some(p) = os_pointer(c.w, c.h) {
            st.cursor = p;
        }
    }
    st.focused = focused;
    let (dx, dy) = (CURSOR_DX.swap(0, Ordering::Relaxed), CURSOR_DY.swap(0, Ordering::Relaxed));
    if focused {
        st.cursor[0] = (st.cursor[0] + dx as f32 * speed).clamp(0.0, c.w);
        st.cursor[1] = (st.cursor[1] + dy as f32 * speed).clamp(0.0, c.h);
    }
    // the left stick moves it (live speed: [hud] set_cursor[2] px/s at 1080p, 0 = 1100)
    let (sx, sy) = pad.left();
    if focused && (sx != 0.0 || sy != 0.0) {
        let v = if cur_lay[2] > 0.0 { cur_lay[2] } else { 1100.0 } * c.s * dt;
        st.cursor[0] = (st.cursor[0] + sx * v).clamp(0.0, c.w);
        st.cursor[1] = (st.cursor[1] - sy * v).clamp(0.0, c.h);
    }
    // A clicks like the left mouse button (not while a controller binding waits: A is a button
    // to bind then)
    let pad_a = pad.down(crate::gamepad::A) && !st.capture.is_some_and(|c| c.3);
    let lmb = focused && (async_down(0x01) || pad_a);
    let click = lmb && !st.lmb && st.capture.is_none();
    let released = !lmb && st.lmb;
    let cur = st.cursor;
    // keyboard or controller bindings: the one used last (frozen while a binding waits, so the
    // list can't swap under it)
    let pad_mode = st.capture.map_or(crate::gamepad::pad_mode(), |c| c.3);

    // ---- layout (live: [hud] settings = [dx, dy, scale, alpha], set_* below)
    let win = lay("settings", [0.0, 0.0, 1.0, 1.0]);
    let f = Frame { c, k: c.s * win[2].clamp(0.3, 3.0), ox: win[0], oy: win[1], a: win[3].clamp(0.0, 1.0) };
    let txt = lay("set_text", [19.0, 19.0, 14.0, 17.0]); // label, value, key tag, section
    let title = lay("set_title", [30.0, 22.0, 18.0, 0.0]); // title, subtitle, close hint
    let rows = lay("set_rows", [58.0, 43.0, 80.0, 52.0]); // left step, keys step, section gap, row height
    let cols = lay("set_cols", [300.0, 222.0, 405.0, 505.0]); // control x, value x, key slot 1 / 2 centre
    let foot = lay("set_footer", [17.0, 20.0, 14.0, 70.0]); // reset, warning, autosave, y from bottom
    // text x offsets: left labels, section titles, hint under SHOW HUD, key-binding labels
    let tx = lay("set_textx", [34.0, 18.0, 34.0, 24.0]);
    // widths: left rows / sections, key-binding column x (from the panel's left edge), its width,
    // slider length
    let wd = lay("set_width", [560.0, 700.0, 560.0, 0.0]);
    let (px0, py0, px1, py1) = (300.0, 110.0, 1620.0, 990.0);

    // dim the game behind
    c.dl.add_rect([0.0, 0.0], [c.w, c.h], col([0.0, 0.0, 0.0, 0.55 * f.a])).filled(true).build();
    // panel
    f.tex("st_frame_backplate", px0, py0, px1, py1, [1.0, 1.0, 1.0, 0.97]);
    // the whole title bar moves together: [hud] set_header = [y offset, -, -, -]
    let hy = py0 + lay("set_header", [0.0, 0.0, 0.0, 0.0])[0];
    // upside down (user)
    f.tex_vflip("st_frame_header_2500w", px0 + 30.0, hy + 22.0, px1 - 30.0, hy + 92.0, [0.47, 0.48, 0.47, 0.9]);
    // (Doom's accent strip along the top edge: removed - user)
    // (the green corner mark: removed - user)
    let tw = f.text(px0 + 110.0, hy + 57.0, title[0], WHITE, "DOOM RING", true, 0);
    let sub_end = px0 + 130.0 + tw + f.text(px0 + 130.0 + tw, hy + 57.0, title[1], GREY, "//  SETTINGS", false, 0);
    let close_key = key_label(cfg.keys.settings);
    let close_lbl = if close_key.is_empty() || pad_mode { "CLOSE".to_string() } else { format!("[{close_key}] CLOSE") };
    // [F1] CLOSE: its right edge this far in from the panel's (set_title[3], 0 = 60)
    let close_r = px1 - if title[3] > 0.0 { title[3] } else { 60.0 };
    let close_w = f.text(close_r, hy + 57.0, title[2], GREY, &close_lbl, false, 2);
    if pad_mode {
        // controller: B closes - its icon before the word
        if let Some(icon) = crate::gamepad::icon(crate::gamepad::B) {
            let x = close_r - close_w - 22.0;
            f.tex(&icon, x - 19.0, hy + 38.0, x + 19.0, hy + 76.0, WHITE);
        }
    }

    // The body (sections, rows, key bindings) scales on its own about the top centre of the
    // content; the title bar and the footer keep their size (user: it got cramped).
    // live: [hud] set_body = [scale, anchor dy, -, -]
    let fw = &f;
    let body = lay("set_body", [0.9, 0.0, 0.0, 0.0]);
    let bsc = body[0].clamp(0.3, 2.0);
    let (ax, ay) = ((px0 + px1) * 0.5, py0 + 120.0 + body[1]);
    let f = Frame { c, k: fw.k * bsc, ox: (ax - 960.0 + fw.ox) / bsc - ax + 960.0, oy: (ay - 540.0 + fw.oy) / bsc - ay + 540.0, a: fw.a };

    let section = |x: f32, y: f32, w: f32, t: &str| {
        // upside down like the title bar (user)
        f.tex_vflip("st_frame_header_2500w", x, y - 14.0, x + w, y + 14.0, [0.35, 0.36, 0.35, 0.9]);
        f.text(x + tx[1], y, txt[3], LIME, t, true, 0);
    };
    let row_bg = |x: f32, y: f32, hot: bool| {
        let h = rows[3] * 0.5;
        // hover: the bar itself tints a little green (no glowing outline - user). Live tint:
        // [hud] set_hover = [r, g, b, alpha]
        let hc = lay("set_hover", [0.45, 0.55, 0.25, 0.75]);
        f.tex("st_button_default_list_base", x, y - h, x + wd[0], y + h, if hot { hc } else { alpha(ROW, 0.55) });
    };

    let mut changed_vals = false;
    let mut changed_keys = false;
    let mut changed_pads = false;
    // the restart marks only show once enemy health differs from what the game started with (user)
    let needs_restart = (st.vals.enemy_hp - crate::params::applied_hp_mult()).abs() > 0.001;
    if needs_restart {
        // in the title bar after "// SETTINGS" (user); live: [hud] set_warn = [dx from the
        // subtitle's end, dy, text size (0 = the footer's set_footer[1]), -]
        let wn = lay("set_warn", [30.0, 0.0, 0.0, 0.0]);
        let size = if wn[2] > 0.0 { wn[2] } else { foot[1] };
        fw.text(sub_end + wn[0], hy + 57.0 + wn[1], size, RED, "* SOME CHANGES REQUIRE A GAME RESTART", true, 0);
    }
    let mut vals = st.vals;
    let mut drag = st.drag;
    let mut hold = if lmb { st.hold } else { None };
    let now = std::time::Instant::now();
    let v = &mut vals;

    // ---- left column
    let lx = px0 + 60.0;
    let cx = lx + cols[0];
    let mut y = py0 + 150.0;
    let row_hot = |y: f32| f.hit(cur, lx, y - rows[3] * 0.5, lx + wd[0], y + rows[3] * 0.5);

    // a slider row: returns the new value while dragged
    let mut slider = |id: usize, y: f32, label: &str, star: bool, val: &mut f32, range: (f32, f32, f32), show: fn(f32) -> String| -> bool {
        let shown = show(*val);
        let hot = row_hot(y) || drag == Some(id);
        row_bg(lx, y, hot);
        let w = f.text(lx + tx[0], y, txt[0], WHITE, label, false, 0);
        if star {
            f.text(lx + tx[0] + 6.0 + w, y - 6.0, txt[0], RED, "*", true, 0);
        }
        // slider length: [hud] set_width[3] (0 = 200)
        let (t0, t1) = (cx, cx + if wd[3] > 0.0 { wd[3] } else { 200.0 });
        f.tex("st_keybinding_bg_bar", t0, y - 7.0, t1, y + 7.0, [0.27, 0.28, 0.27, 1.0]);
        let frac = ((*val - range.0) / (range.1 - range.0)).clamp(0.0, 1.0);
        let fx = t0 + 4.0 + (t1 - t0 - 8.0) * frac;
        f.rect(t0 + 4.0, y - 3.0, fx, y + 3.0, LIME, true);
        f.rect(fx - 5.0, y - 12.0, fx + 5.0, y + 12.0, WHITE, true);
        f.text(cx + cols[1], y, txt[1], WHITE, &shown, false, 0);
        if click && f.hit(cur, t0 - 10.0, y - 18.0, t1 + 10.0, y + 18.0) {
            drag = Some(id);
        }
        let mut changed = false;
        if drag == Some(id) && lmb {
            let fr = ((f.lx(cur[0]) - t0 - 4.0) / (t1 - t0 - 8.0)).clamp(0.0, 1.0);
            let nv = ((range.0 + fr * (range.1 - range.0)) / range.2).round() * range.2;
            *val = nv.clamp(range.0, range.1);
        }
        if drag == Some(id) && released {
            drag = None;
            changed = true;
        }
        changed
    };

    let mut checkbox = |y: f32, label: &str, on: &mut bool| -> bool {
        let hot = row_hot(y);
        row_bg(lx, y, hot);
        f.text(lx + tx[0], y, txt[0], WHITE, label, false, 0);
        // checkboxes line up in the value column (under the X / % values), live nudge:
        // [hud] set_check = [dx, -, -, -]
        let bx = cx + cols[1] + lay("set_check", [0.0, 0.0, 0.0, 0.0])[0];
        f.rect(bx, y - 13.0, bx + 26.0, y + 13.0, WHITE, false);
        if *on {
            f.tex("st_active_item_checkmark", bx + 2.0, y - 12.0, bx + 26.0, y + 12.0, LIME);
        }
        if click && hot {
            *on = !*on;
            return true;
        }
        false
    };
    section(lx, y, wd[0], "DIFFICULTY");
    y += 50.0;
    changed_vals |= slider(0, y, "ENEMY HEALTH", needs_restart, &mut v.enemy_hp, ENEMY_HP, |x| format!("{}X", fmt_f(x)));
    y += rows[0];
    changed_vals |= slider(1, y, "WEAPON DAMAGE", false, &mut v.dmg, DMG, |x| format!("{}X", fmt_f(x)));
    y += rows[0];
    // HP limits: arrows step them
    let mut arrows = |id: usize, y: f32, label: &str, val: &mut f32| -> bool {
        let hot = row_hot(y);
        row_bg(lx, y, hot);
        f.text(lx + tx[0], y, txt[0], WHITE, label, false, 0);
        // live: [hud] set_arrows = [distance of each arrow from the number's centre, the number's
        // centre from the control column (0 = 100), -, -]
        let ar = lay("set_arrows", [90.0, 100.0, 0.0, 0.0]);
        let gap = ar[0];
        let mid = cx + if ar[1] != 0.0 { ar[1] } else { 100.0 };
        f.tex_flip("st_arrows", mid - gap - 10.0, y - 14.0, mid - gap + 10.0, y + 14.0, LIME);
        f.tex("st_arrows", mid + gap - 10.0, y - 14.0, mid + gap + 10.0, y + 14.0, LIME);
        f.text(mid, y, txt[1], WHITE, &format!("{:.0}", *val), false, 1);
        let dir = if f.hit(cur, mid - gap - 25.0, y - 20.0, mid - 25.0, y + 20.0) {
            -1
        } else if f.hit(cur, mid + 25.0, y - 20.0, mid + gap + 25.0, y + 20.0) {
            1
        } else {
            0
        };
        let step = |v: &mut f32| *v = (*v + dir as f32 * SAW_HP.2).clamp(SAW_HP.0, SAW_HP.1);
        if click && dir != 0 {
            step(val);
            hold = Some((id, dir, now, now));
            return true;
        }
        // held: repeats after 0.4 s, 16 steps a second (user)
        if let Some((hid, hdir, t0, last)) = hold {
            if lmb && hid == id && hdir == dir && dir != 0 && now.duration_since(t0).as_secs_f32() > 0.4 && now.duration_since(last).as_secs_f32() >= 0.06 {
                step(val);
                hold = Some((hid, hdir, t0, now));
                return true;
            }
        }
        false
    };
    changed_vals |= arrows(0, y, "ENEMY CHAINSAW HP LIMIT", &mut v.saw_hp);
    y += rows[0];
    // the Crucible's own limit (user): 1 charge under it, 2 under twice it, 3 above
    changed_vals |= arrows(1, y, "CRUCIBLE HIT DAMAGE", &mut v.cr_hp);
    y += rows[0];
    changed_vals |= checkbox(y, "GLORY KILL STAGGER ON / OFF", &mut v.glory);
    y += rows[2];
    section(lx, y, wd[0], "AUDIO");
    y += 50.0;
    changed_vals |= slider(2, y, "SOUND EFFECTS", false, &mut v.sfx, VOL, |x| format!("{:.0}%", x * 100.0));
    y += rows[0];
    changed_vals |= slider(3, y, "DOOM MUSIC", false, &mut v.music_vol, VOL, |x| format!("{:.0}%", x * 100.0));
    y += rows[0];
    changed_vals |= checkbox(y, "MUSIC ON / OFF", &mut v.music_on);
    y += rows[2];
    section(lx, y, wd[0], "HUD");
    y += 50.0;
    changed_vals |= checkbox(y, "SHOW HUD", &mut v.show_hud);
    f.text(lx + tx[2], y + 40.0, txt[2], GREY, "CROSSHAIR, SCOPE AND INTERACT PROMPT STAY ON", false, 0);

    // ---- key bindings (two per action)
    let rx = px0 + wd[1];
    let mut y = py0 + 150.0;
    section(rx, y, wd[2], "KEY BINDINGS");
    if pad_mode {
        f.text(rx + cols[2], y, txt[2], GREY, "CONTROLLER", false, 1);
    } else {
        f.text(rx + cols[2], y, txt[2], GREY, "PRIMARY", false, 1);
        f.text(rx + cols[3], y, txt[2], GREY, "SECONDARY", false, 1);
    }
    y += 48.0;
    let n_rows = if pad_mode { PAD_ROWS } else { ACTIONS.len() };
    for i in 0..n_rows {
        let label = ACTIONS.get(i).map_or("INTERACT", |a| a.0);
        let capturing_row = st.capture.is_some_and(|(r, _, _, _)| r == i);
        // hover / waiting for a key: the bar tints a little green, like the settings rows
        let row_hover = f.hit(cur, rx, y - 21.0, rx + wd[2], y + 21.0);
        let hc = lay("set_hover", [0.45, 0.55, 0.25, 0.75]);
        f.tex("st_keybinding_bg_bar", rx, y - 21.0, rx + wd[2], y + 21.0, if capturing_row || row_hover { [hc[0], hc[1], hc[2], 0.9] } else { [0.31, 0.32, 0.31, 0.9] });
        f.text(rx + tx[3], y, txt[0] * 0.95, WHITE, label, false, 0);
        for slot in 0..if pad_mode { 1 } else { 2 } {
            let sx = rx + if slot == 0 { cols[2] } else { cols[3] };
            let hot = f.hit(cur, sx - 50.0, y - 18.0, sx + 50.0, y + 18.0);
            let bound = if pad_mode { st.pads[i] } else { st.keys[i][slot] };
            if st.capture.is_some_and(|(r, s, _, _)| r == i && s == slot) {
                f.text(sx, y, txt[2], LIME, if pad_mode { "PRESS A BUTTON" } else { "PRESS A KEY" }, true, 1);
            } else if pad_mode && bound != 0 {
                // Doom's own button icon
                if let Some(icon) = crate::gamepad::icon(bound) {
                    let r = if hot { 22.0 } else { 20.0 };
                    f.tex(&icon, sx - r, y - r, sx + r, y + r, WHITE);
                }
            } else if bound == 0 {
                f.text(sx, y, txt[2], if hot { WHITE } else { GREY }, "-", true, 1);
            } else {
                let lbl = key_label(st.keys[i][slot]);
                let kw = f.c.doom_font(true).map_or(30.0, |ff| {
                    let (l, r) = f.c.doom_text_span(ff, txt[2] * f.k, &lbl, txt[2] * 0.06 * f.k, 1.0);
                    (r - l) / f.k
                }) + 16.0;
                f.rect(sx - kw * 0.5, y - 12.0, sx + kw * 0.5, y + 12.0, if hot { [1.0, 0.95, 0.6, 1.0] } else { TAG }, true);
                f.text(sx, y, txt[2], DARK, &lbl, true, 1);
            }
            if click && hot {
                let mut held = [false; 256];
                if pad_mode {
                    // pad buttons down now (the A that clicked) wait to be let go
                    for &code in crate::gamepad::ALL.iter() {
                        held[(code - crate::gamepad::DPAD_UP) as usize] = pad.down(code);
                    }
                } else {
                    for (vk, h) in held.iter_mut().enumerate() {
                        *h = async_down(vk as u16);
                    }
                }
                st.capture = Some((i, slot, held, pad_mode));
            }
        }
        y += rows[1];
    }
    // centred under the key rows; live nudge: [hud] set_hint = [dx, dy, -, -]
    let hint = lay("set_hint", [0.0, 0.0, 0.0, 0.0]);
    let hint_txt = if pad_mode { "SELECT AN ACTION TO CHANGE  -  MENU TO CANCEL  -  DEL TO CLEAR" } else { "SELECT A KEY TO CHANGE  -  ESC TO CANCEL  -  DEL TO CLEAR" };
    f.text(rx + wd[2] * 0.5 + hint[0], y - 10.0 + hint[1], txt[2] * 0.9, GREY, hint_txt, false, 1);

    // controller binding: the first pad button pressed after the click (Menu cancels; keys and
    // mouse buttons don't count - user: no mixing)
    if let Some((r, _, mut held, true)) = st.capture.filter(|_| focused) {
        let mut got: Option<u16> = None;
        for &code in crate::gamepad::ALL.iter() {
            if code == crate::gamepad::START {
                continue;
            }
            let idx = (code - crate::gamepad::DPAD_UP) as usize;
            let d = pad.down(code);
            if d && !held[idx] {
                got = Some(code);
                break;
            }
            if !d {
                held[idx] = false;
            }
        }
        if async_down(0x2E) || async_down(0x08) {
            // Delete / Backspace: empty slot
            st.pads[r] = 0;
            st.capture = None;
            changed_pads = true;
        } else if let Some(code) = got {
            // one button, one action: it leaves wherever else it was
            for b in st.pads.iter_mut() {
                if *b == code {
                    *b = 0;
                }
            }
            st.pads[r] = code;
            st.capture = None;
            changed_pads = true;
            crate::audio::play_vol("weapon_switch", 0.4);
        } else {
            st.capture = Some((r, 0, held, true));
        }
    }

    // key capture: the first key pressed after the click (mouse buttons count)
    if let Some((r, slot, mut held, false)) = st.capture.filter(|_| focused) {
        let mut got: Option<u16> = None;
        for vk in 1u16..255 {
            // generic Shift / Ctrl / Alt: the left / right ones are reported too
            if matches!(vk, 0x10 | 0x11 | 0x12 | 0x1B) {
                continue;
            }
            // the settings action can't take the left mouse button (every click would toggle the
            // window, and it pulled LMB off Fire - user)
            if vk == 0x01 && r == ACTIONS.len() - 1 {
                continue;
            }
            let d = async_down(vk);
            if d && !held[vk as usize] {
                got = Some(vk);
                break;
            }
            if !d {
                held[vk as usize] = false;
            }
        }
        match got {
            Some(0x2E | 0x08) => {
                // Delete / Backspace: empty slot
                st.keys[r][slot] = 0;
                st.capture = None;
                changed_keys = true;
            }
            Some(vk) => {
                // one key, one action: it leaves wherever else it was
                for row in st.keys.iter_mut() {
                    for k in row.iter_mut() {
                        if *k == vk {
                            *k = 0;
                        }
                    }
                }
                st.keys[r][slot] = vk;
                st.capture = None;
                changed_keys = true;
                crate::audio::play_vol("weapon_switch", 0.4);
            }
            None => st.capture = Some((r, slot, held, false)),
        }
    }

    // ---- footer (full size)
    let f = fw;
    let fy = py1 - foot[3];
    // the reset button's x offset, live: [hud] set_reset = [dx (+ = right), -, -, -]
    let rx0 = px0 + 60.0 + lay("set_reset", [0.0, 0.0, 0.0, 0.0])[0];
    let reset_hot = f.hit(cur, rx0, fy - 28.0, rx0 + 340.0, fy + 28.0);
    f.tex("st_root_norm_base", rx0, fy - 28.0, rx0 + 340.0, fy + 28.0, [0.47, 0.48, 0.47, 1.0]);
    f.tex("st_root_norm_stroke", rx0, fy - 28.0, rx0 + 340.0, fy + 28.0, if reset_hot { LIME } else { WHITE });
    f.text(rx0 + 170.0, fy, foot[0], WHITE, "RESET TO DEFAULTS", true, 1);
    // live: [hud] set_autosave = [y offset (- = up), green tint 0..1, x offset (- = left), -]
    let au = lay("set_autosave", [0.0, 0.0, 0.0, 0.0]);
    let t = au[1].clamp(0.0, 1.0);
    let au_col: Rgba = [GREY[0] + (LIME[0] - GREY[0]) * t, GREY[1] + (LIME[1] - GREY[1]) * t, GREY[2] + (LIME[2] - GREY[2]) * t, 1.0];
    let au_r = px1 - 60.0 + au[2];
    let au_w = f.text(au_r, fy + au[0], foot[2], au_col, "CHANGES SAVE AUTOMATICALLY", false, 2);
    // the level's recommended limits (user), between the reset button and the autosave text:
    // chainsaw orange, Crucible red. live: [hud] set_rec = [dx, dy, text size (0 = 14), -]
    let rec = crate::slayer::REC_LIMITS.load(Ordering::Relaxed);
    let (rec_saw, rec_cr) = ((rec >> 32) as f32, (rec & 0xFFFF_FFFF) as f32);
    if rec != 0 {
        let rl = lay("set_rec", [0.0, 0.0, 0.0, 0.0]);
        let size = if rl[2] > 0.0 { rl[2] } else { 14.0 };
        let orange: Rgba = [1.0, 0.5, 0.15, 1.0];
        let red: Rgba = [1.0, 0.25, 0.2, 1.0];
        // two lines (user), each word drawn on its own with a clear gap (the font's spaces are
        // narrow); the tool's name and number in its colour. live gap: [hud] set_rec[3] (0 = 0.45 x size)
        let gap = size * if rl[3] > 0.0 { rl[3] } else { 0.45 };
        let cx = (rx0 + 340.0 + au_r - au_w) * 0.5 + rl[0];
        let line = |y: f32, name: &str, n: f32, c: Rgba| {
            let words: Vec<(String, Rgba)> = vec![(name.into(), c), ("RECOMMENDED".into(), GREY), ("DAMAGE:".into(), GREY), (format!("{n:.0}"), c)];
            let total: f32 = words.iter().map(|(w, _)| f.width(size, w, false)).sum::<f32>() + gap * (words.len() - 1) as f32;
            let mut x = cx - total * 0.5;
            for (w, c) in &words {
                x += f.text(x, y, size, *c, w, false, 0) + gap;
            }
        };
        let dy = size * 0.75;
        line(fy + rl[1] - dy, "CHAINSAW", rec_saw, orange);
        line(fy + rl[1] + dy, "CRUCIBLE", rec_cr, red);
    }
    if click && reset_hot {
        let d = Config::default();
        vals = Vals::of(&d);
        // the limits reset to the level's recommended values, not the level-1 defaults
        if rec != 0 {
            vals.saw_hp = rec_saw;
            vals.cr_hp = rec_cr;
        }
        st.keys = keys_of(&d.keys);
        st.pads = pads_of(&d.keys);
        changed_vals = true;
        changed_keys = true;
        changed_pads = true;
        crate::audio::play_vol("weapon_switch", 0.4);
    }

    // ---- cursor
    let cs = 22.0 * c.s * cur_lay[1].clamp(0.3, 3.0);
    let [x, y] = cur;
    // a plain triangle: the notched arrow was concave, which ImGui can't fill - it drew a ghost
    // outline behind it and a dot at the tip (user)
    let (p0, p1, p2) = ([x, y], [x, y + cs * 0.85], [x + cs * 0.6, y + cs * 0.6]);
    c.dl.add_triangle(p0, p1, p2, col([0.05, 0.05, 0.05, 1.0])).thickness(3.0 * c.s).build();
    c.dl.add_triangle(p0, p1, p2, col([0.95, 0.95, 0.9, 1.0])).filled(true).build();

    st.lmb = lmb;
    st.vals = vals;
    st.drag = drag;
    st.hold = hold;
    let keys = st.keys;
    let pads = st.pads;
    drop(guard);
    if changed_vals {
        save_vals(&vals);
    }
    if changed_keys {
        save_keys(&keys);
    }
    if changed_pads {
        save_pads(&pads);
    }
}
