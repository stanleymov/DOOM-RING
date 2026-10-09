//! Doom Eternal style HUD drawn with Dear ImGui (hudhook, D3D12 present hook).
//!
//! Layout follows Doom Eternal: health + armor bottom-left, equipment (chainsaw fuel, flame belch)
//! above them, ammo bottom-right, dash pips under the crosshair, weapon wheel on hold, and flashing
//! markers on staggered (glory-killable) demons.

use hudhook::{
    BeforeWndProc, ImguiRenderLoop, RenderContext,
    imgui::{self, Context, FontConfig, FontId, FontSource, ImColor32, Ui},
    windows::Win32::Foundation::{HWND, LPARAM, WPARAM},
};

use crate::{
    game,
    slayer::{self, CHAINSAW_MAX},
    weapons::{Ammo, WEAPONS},
};

const GREEN: [f32; 4] = [0.45, 1.0, 0.35, 1.0];
const ARMOR: [f32; 4] = [0.35, 0.95, 0.75, 1.0];
const ORANGE: [f32; 4] = [1.0, 0.55, 0.1, 1.0];
const RED: [f32; 4] = [1.0, 0.18, 0.12, 1.0];
const DIM: [f32; 4] = [0.0, 0.0, 0.0, 0.55];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const DASH: [f32; 4] = [0.35, 0.8, 1.0, 1.0];

pub struct Hud {
    ui_tex: std::collections::HashMap<String, crate::doomhud::Tex>,
    fonts: crate::doomhud::Fonts,
    num: Option<crate::doomhud::NumFont>,
    /// Doom's letter font (eternal_regular) and its heavy bake
    letters: Option<crate::doomhud::NumFont>,
    letters_heavy: Option<crate::doomhud::NumFont>,
    big: Option<FontId>,
    mid: Option<FontId>,
    small: Option<FontId>,
    last_hp: i32,
    hurt_flash: f32,
    /// Seconds since the Doom HUD came back (respawn / load): no hurt flash yet.
    shown_for: f32,
    last_time: f32,
}

// FontId is a raw pointer into the imgui atlas; only the render thread ever touches it.
unsafe impl Send for Hud {}
unsafe impl Sync for Hud {}

impl Hud {
    pub fn new() -> Self {
        Self { ui_tex: Default::default(), fonts: crate::doomhud::Fonts { num_big: None, num_mid: None, label: None, small: None }, num: None, letters: None, letters_heavy: None, big: None, mid: None, small: None, last_hp: -1, hurt_flash: 0.0, shown_for: 0.0, last_time: 0.0 }
    }
}

fn col(c: [f32; 4]) -> ImColor32 {
    ImColor32::from_rgba_f32s(c[0], c[1], c[2], c[3])
}

fn with_alpha(c: [f32; 4], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], c[3] * a]
}

impl ImguiRenderLoop for Hud {
    fn initialize<'a>(&'a mut self, ctx: &mut Context, rc: &'a mut dyn RenderContext) {
        let ui_dir = crate::config::mod_dir().join("doom_ui");
        if let Ok(files) = std::fs::read_dir(&ui_dir) {
            for f in files.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "png") {
                    if let Some((id, w, h)) = load_texture(rc, &p) {
                        let name = p.file_stem().unwrap().to_string_lossy().into_owned();
                        self.ui_tex.insert(name, crate::doomhud::Tex { id, w: w as f32, h: h as f32 });
                    }
                }
            }
        }
        log::info!("Doom UI textures loaded: {}", self.ui_tex.len());
        self.num = crate::doomhud::load_numfont(&ui_dir, "font_eternal_numeral_regular");
        log::info!("Doom numeral font: {}", self.num.as_ref().map_or(0, |n| n.glyphs.len()));
        self.letters = crate::doomhud::load_numfont(&ui_dir, "font_eternal_regular");
        self.letters_heavy = crate::doomhud::load_numfont(&ui_dir, "font_eternal_regular_heavy");
        log::info!("Doom letter font: {}", self.letters.as_ref().map_or(0, |n| n.glyphs.len()));
        ctx.set_ini_filename(None);
        // our overlay never changes the Windows pointer (the settings window draws its own)
        ctx.io_mut().config_flags |= imgui::ConfigFlags::NO_MOUSE_CURSOR_CHANGE;
        let ttf = std::fs::read("C:\\Windows\\Fonts\\bahnschrift.ttf")
            .or_else(|_| std::fs::read("C:\\Windows\\Fonts\\impact.ttf"));
        if let Ok(data) = ttf {
            let data: &'static [u8] = Box::leak(data.into_boxed_slice());
            let mut add = |size: f32| {
                ctx.fonts().add_font(&[FontSource::TtfData {
                    data,
                    size_pixels: size,
                    config: Some(FontConfig { oversample_h: 2, ..FontConfig::default() }),
                }])
            };
            self.small = Some(add(20.0));
            self.mid = Some(add(30.0));
            self.big = Some(add(64.0));
        }
        // Doom-style chamfered numerals (Chakra Petch, OFL) for the Doom HUD, sized for the screen.
        if let Ok(data) = std::fs::read(ui_dir.join("ChakraPetch-SemiBold.ttf")) {
            let data: &'static [u8] = Box::leak(data.into_boxed_slice());
            let k = screen_scale();
            let mut add = |size: f32| {
                ctx.fonts().add_font(&[FontSource::TtfData {
                    data,
                    size_pixels: size * k,
                    config: Some(FontConfig { oversample_h: 2, ..FontConfig::default() }),
                }])
            };
            self.fonts.num_mid = Some(add(44.0));
            self.fonts.num_big = Some(add(64.0));
            self.fonts.label = Some(add(26.0));
            self.fonts.small = Some(add(20.0));
        }
        log::info!("HUD initialised (font loaded: {})", self.big.is_some());
    }

    fn before_wnd_proc(&self, _hwnd: HWND, umsg: u32, wparam: WPARAM, lparam: LPARAM) -> BeforeWndProc {
        // WM_INPUT: the game reads mouse motion as raw input. While the weapon wheel is open the
        // motion steers the wheel and is swallowed so the camera stays put.
        // The settings window draws its own cursor: no Windows pointer over the game while it's
        // open (it showed as a second cursor outline behind ours - user). WM_SETCURSOR = 0x20.
        // WM_SETCURSOR isn't sent while the game holds the mouse, so the pointer is also hidden on
        // every mouse message (WM_INPUT, WM_MOUSEMOVE) - these arrive on the window's thread.
        if crate::remap::SETTINGS_OPEN.load(std::sync::atomic::Ordering::Relaxed) && matches!(umsg, 0x0020 | 0x00FF | 0x0200) {
            unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursor(None) };
            if umsg == 0x0020 {
                return BeforeWndProc::Break;
            }
        }
        // Esc that closed the settings window: not for the game (Elden Ring's menu opened).
        if crate::remap::ESC_SWALLOW.load(std::sync::atomic::Ordering::Relaxed) && matches!(umsg, 0x0100 | 0x0101 | 0x0102) && wparam.0 == 0x1B {
            return BeforeWndProc::Break;
        }
        // The settings window: the motion moves its cursor; the game gets none of it.
        if umsg == 0x00FF && crate::remap::SETTINGS_OPEN.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some((dx, dy)) = raw_mouse_delta(lparam.0) {
                use std::sync::atomic::Ordering::Relaxed;
                crate::remap::CURSOR_DX.fetch_add(dx, Relaxed);
                crate::remap::CURSOR_DY.fetch_add(dy, Relaxed);
            }
            return BeforeWndProc::Break;
        }
        if umsg == 0x00FF && crate::remap::WHEEL_OPEN.load(std::sync::atomic::Ordering::Relaxed) {
            if let Some((dx, dy)) = raw_mouse_delta(lparam.0) {
                use std::sync::atomic::Ordering::Relaxed;
                let c = |v: i32| v.clamp(-400, 400);
                let nx = c(crate::remap::WHEEL_DX.load(Relaxed) + dx);
                let ny = c(crate::remap::WHEEL_DY.load(Relaxed) + dy);
                crate::remap::WHEEL_DX.store(nx, Relaxed);
                crate::remap::WHEEL_DY.store(ny, Relaxed);
                return BeforeWndProc::Break;
            }
        }
        // Mouse wheel cycles weapons (WM_MOUSEWHEEL = 0x020A) when no lower-level path saw it -
        // not while the settings window is open (it switched weapons - user).
        if umsg == 0x020A && crate::remap::SETTINGS_OPEN.load(std::sync::atomic::Ordering::Relaxed) {
            return BeforeWndProc::Break;
        }
        if umsg == 0x020A {
            use std::sync::atomic::Ordering::Relaxed;
            let delta = ((wparam.0 >> 16) & 0xFFFF) as i16;
            if crate::remap::WHEEL_SOURCES.load(Relaxed) & 3 == 0 {
                crate::remap::WHEEL_NOTCHES.fetch_add(if delta > 0 { 1 } else { -1 }, Relaxed);
            }
            crate::remap::WHEEL_SOURCES.fetch_or(4, Relaxed);
            return BeforeWndProc::Break;
        }
        BeforeWndProc::Continue
    }

    fn render(&mut self, ui: &mut Ui) {
        let [w, h] = ui.io().display_size;
        let s = 1.0f32.max(h / 1080.0); // scale for 1440p/4K

        let snap = slayer::with(|s| Snapshot::take(s));
        let Some(snap) = snap else {
            // no Doom HUD (menus, loading, dead): the settings window goes too
            crate::settings_ui::close();
            // and the health from before isn't compared with the next one (a respawn flashed
            // red - user)
            self.last_hp = -1;
            self.hurt_flash = 0.0;
            self.shown_for = 0.0;
            return;
        };
        // HUD off (settings window): only the crosshair, the scope and the interact prompt
        let show = crate::config::get_cached().show_hud;
        let settings_open = crate::settings_ui::is_open();

        let dt = (snap.time - self.last_time).clamp(0.0, 0.1);
        self.last_time = snap.time;
        self.shown_for += dt;
        // (health settles over the first moments after a respawn / load: no flash for that)
        if self.last_hp > 0 && snap.hp < self.last_hp && self.shown_for > 1.5 {
            self.hurt_flash = 1.0;
        }
        self.last_hp = snap.hp;
        self.hurt_flash = (self.hurt_flash - dt * 2.5).max(0.0);

        let dl = ui.get_background_draw_list();
        let big = self.big;
        let mid = self.mid;
        let small = self.small;
        let text = |font: Option<FontId>, pos: [f32; 2], c: [f32; 4], t: &str| {
            let _f = font.map(|f| ui.push_font(f));
            // Drop shadow for readability over bright skies.
            dl.add_text([pos[0] + 2.0, pos[1] + 2.0], col([0.0, 0.0, 0.0, 0.8 * c[3]]), t);
            dl.add_text(pos, col(c), t);
        };
        let text_w = |font: Option<FontId>, t: &str| {
            let _f = font.map(|f| ui.push_font(f));
            ui.calc_text_size(t)[0]
        };

        // ---- hurt vignette
        if self.hurt_flash > 0.0 {
            let a = self.hurt_flash * 0.35;
            let edge = 140.0 * s;
            let r = col([0.8, 0.0, 0.0, a]);
            let z = col([0.8, 0.0, 0.0, 0.0]);
            dl.add_rect_filled_multicolor([0.0, 0.0], [edge, h], r, z, z, r);
            dl.add_rect_filled_multicolor([w - edge, 0.0], [w, h], z, r, r, z);
        }

        // ---- glory kill markers
        if let Some(cam) = game::camera_full().filter(|_| show) {
            let blink = (snap.time * 8.0).sin() * 0.5 + 0.5;
            for (pos, in_range) in &snap.staggered {
                if let Some([x, y]) = cam.project(*pos + glam::Vec3::Y * 2.1, w, h) {
                    let c = if *in_range { ORANGE } else { [0.3, 0.6, 1.0, 1.0] };
                    let r = (14.0 + 6.0 * blink) * s;
                    let pts = [[x, y - r], [x + r, y], [x, y + r], [x - r, y]];
                    dl.add_polyline(pts.to_vec(), col(with_alpha(c, 0.6 + 0.4 * blink)))
                        .filled(true)
                        .build();
                    if *in_range {
                        let kk = &crate::config::get_cached().keys;
                        let key = if crate::gamepad::pad_mode() {
                            crate::gamepad::label(kk.melee_pad).to_string()
                        } else {
                            crate::doomhud::key_label(kk.melee)
                        };
                        let t = if key.is_empty() { "GLORY KILL".to_string() } else { format!("GLORY KILL [{key}]") };
                        text(small, [x - text_w(small, &t) / 2.0, y + r + 4.0], ORANGE, &t);
                    }
                }
            }
        }


        // ---- DOOM Eternal HUD (vitals, equipment/ammo, reticle)
        let st = crate::doomhud::State {
            time: snap.time,
            hp: snap.hp,
            max_hp: snap.max_hp,
            armor: snap.armor,
            armor_max: snap.armor_max,
            weapon: snap.weapon,
            ammo: snap.ammo,
            dash: snap.dash,
            dash_ready: snap.dash_ready,
            fuel: snap.fuel,
            belch_cd: snap.belch_cd,
            belch_max: crate::slayer::BELCH_COOLDOWN,
            blood_punch: snap.blood_punch,
            wheel_open: snap.wheel_open,
            wheel_pick: snap.wheel_pick,
            wheel_t: snap.wheel_t,
            last_shot_at: snap.last_shot_at,
            bolt_at: snap.bolt_at,
            // The hook icon goes to any demon the meathook can grab (it may leave the brackets).
            hook_icon: snap.hook_icon.and_then(|p| game::camera_full()?.project(p, w, h)),
            zoom: snap.zoom,
            heat: snap.heat,
            sticky_ready: snap.sticky_ready,
            sticky_charge: snap.sticky_charge,
            sticky_reload: snap.sticky_reload,
            sticky_flash: snap.sticky_flash,
            arb: snap.arb,
            bfg_charge: snap.bfg_charge,
            hook_ready_for: snap.hook_ready_for,
            keys: snap.keys,
            crucible_out: snap.crucible_out,
            crucible_charges: snap.crucible_charges,
        };
        let dctx = crate::doomhud::Ctx { ui, dl: &dl, tex: &self.ui_tex, fonts: &self.fonts, num: self.num.as_ref(), letters: self.letters.as_ref(), letters_heavy: self.letters_heavy.as_ref(), w, h, s: h / 1080.0 };
        if show {
            crate::doomhud::pickups(&dctx, &snap.pickups, snap.time);
            crate::doomhud::enemy_bars(&dctx, &snap.bars, snap.time);
            crate::doomhud::boss_bars(&dctx, &snap.bosses);
        }
        // Precision Bolt scope: only the scope (its own ammo readout), no vitals / equipment.
        let scoped = st.weapon == 1 && st.zoom > 0.5;
        if !scoped && show {
            crate::doomhud::vitals(&dctx, &st, self.hurt_flash);
            crate::doomhud::equipment(&dctx, &st);
        }
        crate::doomhud::mod_overlay(&dctx, &st);
        if !snap.wheel_open {
            // out of ammo with the chainsaw idling in the hands: no crosshair (user)
            if snap.crucible_out && !settings_open {
                // the Crucible's own reticle (its charges) instead of the gun's
                crate::doomhud::crucible_reticle(&dctx, snap.crucible_charges);
            } else if !snap.saw_mode && !settings_open {
                crate::doomhud::reticle(&dctx, &st);
            }
            if crate::autoloot::offered() {
                let kk = &crate::config::get_cached().keys;
                let key = crate::doomhud::key_label(crate::gamepad::shown(kk.interact, kk.interact_pad));
                crate::doomhud::interact_prompt(&dctx, &key);
            }
            if snap.boss_saw_hint && show {
                crate::doomhud::center_hint(&dctx, "CHAINSAW TO GET AMMO");
            }
            // music test: the track, where it is, its start point and the keys (top left)
            if crate::audio::MUSIC_TEST.load(std::sync::atomic::Ordering::Relaxed) {
                let t = crate::audio::music_status().unwrap_or_else(|| "NO MUSIC".into());
                text(small, [20.0 * s, 20.0 * s], [0.9, 0.95, 0.5, 1.0], &format!("MUSIC TEST   {t}"));
                text(small, [20.0 * s, 44.0 * s], [0.75, 0.75, 0.75, 1.0], crate::audio::music_keys_hint());
            }
        }
        let cx = w / 2.0;

        // ---- messages (right of crosshair, Doom pickup style)
        let mut my = h / 2.0 - 20.0 * s;
        for (m, t) in snap.messages.iter().rev().filter(|_| show) {
            let a = t.clamp(0.0, 1.0);
            text(mid, [cx + 120.0 * s, my], with_alpha([1.0, 0.85, 0.2, 1.0], a), m);
            my -= 34.0 * s;
        }

        // ---- weapon wheel
        crate::doomhud::wheel(&dctx, &st);
        // ---- settings window (F1), over everything
        crate::settings_ui::frame(&dctx);
    }
}

/// Relative mouse motion from a WM_INPUT lParam (None for keyboard / absolute input).
fn raw_mouse_delta(lparam: isize) -> Option<(i32, i32)> {
    use windows::Win32::UI::Input::{
        GetRawInputData, HRAWINPUT, RAWINPUT, RAWINPUTHEADER, RID_INPUT, RIM_TYPEMOUSE,
    };
    let mut raw = RAWINPUT::default();
    let mut size = size_of::<RAWINPUT>() as u32;
    let n = unsafe {
        GetRawInputData(
            HRAWINPUT(lparam as _),
            RID_INPUT,
            Some(&mut raw as *mut _ as *mut _),
            &mut size,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    if n == u32::MAX || raw.header.dwType != RIM_TYPEMOUSE.0 {
        return None;
    }
    let m = unsafe { raw.data.mouse };
    // MOUSE_MOVE_ABSOLUTE = 1
    (m.usFlags.0 & 1 == 0).then_some((m.lLastX, m.lLastY))
}

/// Screen height / 1080 of the game window, for font sizes (fonts are built once).
fn screen_scale() -> f32 {
    use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetClientRect};
    let mut r = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = GetClientRect(GetForegroundWindow(), &mut r);
    }
    ((r.bottom - r.top) as f32 / 1080.0).clamp(1.0, 2.5)
}

fn load_texture(rc: &mut dyn RenderContext, path: &std::path::Path) -> Option<(imgui::TextureId, u32, u32)> {
    let file = std::fs::File::open(path).ok()?;
    let mut dec = png::Decoder::new(std::io::BufReader::new(file));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::ALPHA | png::Transformations::STRIP_16);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buf).ok()?;
    let px = &buf[..info.buffer_size()];
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    let id = rc.load_texture(&rgba, info.width, info.height).ok()?;
    Some((id, info.width, info.height))
}

fn bar(dl: &imgui::DrawListMut, at: [f32; 2], w: f32, h: f32, frac: f32, c: [f32; 4]) {
    dl.add_rect(at, [at[0] + w, at[1] + h], col(with_alpha(c, 0.2))).filled(true).build();
    dl.add_rect(at, [at[0] + w * frac.clamp(0.0, 1.0), at[1] + h], col(c)).filled(true).build();
}

/// Copy of what the HUD needs, taken under the slayer lock and released immediately.
struct Snapshot {
    time: f32,
    hp: i32,
    max_hp: i32,
    armor: i32,
    armor_max: i32,
    weapon: usize,
    ammo: [i32; 5],
    dash: f32,
    dash_ready: u32,
    fuel: f32,
    belch_cd: f32,
    wheel_open: bool,
    messages: Vec<(String, f32)>,
    staggered: Vec<(glam::Vec3, bool)>,
    wheel_pick: Option<usize>,
    wheel_t: f32,
    pickups: Vec<crate::pickups::Visual>,
    bars: Vec<crate::slayer::EnemyBar>,
    bosses: Vec<(String, i32, i32)>,
    keys: [u16; 5],
    crucible_out: bool,
    crucible_charges: u32,
    blood_punch: u32,
    last_shot_at: f32,
    bolt_at: f32,
    hook_icon: Option<glam::Vec3>,
    zoom: f32,
    heat: f32,
    sticky_ready: f32,
    sticky_charge: f32,
    sticky_reload: f32,
    sticky_flash: (f32, bool),
    arb: f32,
    bfg_charge: f32,
    hook_ready_for: f32,
    saw_mode: bool,
    boss_saw_hint: bool,
}

impl Snapshot {
    fn take(s: &slayer::Slayer) -> Option<Self> {
        // Hidden during loading / fades / death, like the viewmodel.
        // Hidden in menus too (pause / options / inventory) so it never covers them.
        if !s.ready || s.menu_t > 0.5 {
            return None;
        }
        // (the slayer's copy of the game's HP: no game reads on the render thread)
        let (pool, pool_max) = s.hud_pool;
        if pool_max <= 0 {
            return None;
        }
        Some(Self {
            time: s.time,
            // The game's HP pool carries the shield on top: show health and shield separately.
            hp: if s.base_max > 0 { s.health.min(pool).clamp(0, s.base_max) } else { pool },
            max_hp: if s.base_max > 0 { s.base_max } else { pool_max },
            armor: s.armor,
            armor_max: s.cfg.armor_max,
            weapon: s.weapon,
            ammo: s.ammo,
            dash: s.dash_charges,
            dash_ready: s.dash_ready(),
            fuel: s.chainsaw_fuel,
            belch_cd: s.belch_cd,
            wheel_open: s.wheel_open,
            messages: s.messages.iter().cloned().collect(),
            staggered: s.staggered.clone(),
            wheel_pick: s.wheel_pick,
            wheel_t: s.wheel_t,
            pickups: crate::pickups::visuals(&s.pickups),
            bars: s.bars.clone(),
            bosses: s.boss_bars.clone(),
            // (controller mode: the pad buttons, drawn as Doom's button icons)
            keys: {
                let k = &s.cfg.keys;
                let p = crate::gamepad::shown;
                [p(k.dash, k.dash_pad), p(k.flame_belch, k.flame_belch_pad), p(k.chainsaw, k.chainsaw_pad), p(k.alt_fire, k.alt_fire_pad), p(k.crucible, k.crucible_pad)]
            },
            crucible_out: s.crucible_out,
            crucible_charges: s.crucible_charges,
            blood_punch: s.blood_punch,
            last_shot_at: s.last_shot_at,
            bolt_at: s.bolt_at,
            hook_icon: s.hook_icon,
            zoom: s.zoom,
            heat: s.heat,
            sticky_ready: s.sticky_mag as f32,
            sticky_charge: s.sticky_charge,
            sticky_reload: if s.sticky_reload > 0.0 { 1.0 - s.sticky_reload / crate::slayer::STICKY_RELOAD } else { -1.0 },
            sticky_flash: (s.time - s.sticky_flash_at, s.sticky_flash_all),
            arb: s.arb_charge(),
            bfg_charge: s.bfg_charge_frac(),
            hook_ready_for: s.hook_ready_for,
            saw_mode: s.saw_mode,
            boss_saw_hint: s.boss_saw_hint(),
        })
    }
}

pub fn install(module: usize) {
    use hudhook::{Hudhook, hooks::dx12::ImguiDx12Hooks, windows::Win32::Foundation::HINSTANCE};
    crate::viewmodel::install();
    std::thread::spawn(move || {
        // Give the game time to create its swapchain before hooking Present.
        std::thread::sleep(std::time::Duration::from_secs(5));
        match Hudhook::builder()
            .with::<ImguiDx12Hooks>(Hud::new())
            .with_hmodule(HINSTANCE(module as _))
            .build()
            .apply()
        {
            Ok(()) => log::info!("HUD hooks applied"),
            Err(e) => log::error!("HUD hooks failed: {e:?}"),
        }
        crate::remap::install();
        crate::gamepad::install();
        crate::autoloot::install();
    });
}
