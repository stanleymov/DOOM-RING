//! DOOM Eternal HUD, laid out from in-game reference screenshots (1080p coordinates, scaled):
//!
//! * bottom-left  : ability hexes (dash charges, blood punch), armor row (lime), health row (blue),
//!                  each a slanted backer with icon, Eternal-style numerals and segmented gauge.
//! * bottom-right : equipment boxes (flame belch / chainsaw / ammo), ammo count + type icon.
//! * centre       : per-weapon reticle, weapon wheel (real wedge textures from the user's install).
//!
//! Ammo icons, health cross and armor shield are vector art in Doom's Flash UI, so they are drawn
//! as shapes here; everything that exists as a texture (wheel, weapon/ability icons) is the original.

use std::{collections::HashMap, f32::consts::PI};

use hudhook::imgui::{DrawListMut, FontId, ImColor32, TextureId, Ui};

use crate::weapons::{Ammo, WEAPONS};

pub type Rgba = [f32; 4];

pub const HEALTH: Rgba = [0.27, 0.78, 1.0, 1.0];
pub const ARMOR: Rgba = [0.66, 0.92, 0.22, 1.0];
pub const YELLOW: Rgba = [0.98, 0.88, 0.25, 1.0];
pub const RED: Rgba = [1.0, 0.25, 0.2, 1.0];
/// Doom's HUD plates are see-through glass: a faint grey tint with a thin light rim.
pub const GLASS: Rgba = [0.62, 0.68, 0.72, 0.10];
pub const RIM: Rgba = [0.85, 0.9, 0.95, 0.22];

/// Doom's wheel order, clockwise from the top: CS, SSG, HC, CG, Plasma, Ballista, Rocket, BFG.
pub const WHEEL_ORDER: [usize; 8] = [0, 4, 1, 6, 2, 5, 3, 7];

/// Weapon icon texture per weapon slot (weapons.rs order).
/// Doom's weapon mod icons per slot (tools/convert_ammo_icons.py); the BFG has none.
pub const MOD_ICONS: [&str; 8] = [
    "mod_sticky", "mod_bolt", "mod_heatblast", "mod_remote", "mod_meathook", "mod_arbalest", "mod_turret", "",
];

pub const WEAPON_ICONS: [&str; 8] = [
    "shotgun_selected", "har_selected", "plasma_selected", "rocket_selected",
    "dbshotgun_selected", "gauss_selected", "chaingun_selected", "bfg_selected",
];

pub fn ammo_color(a: Ammo) -> Rgba {
    match a {
        Ammo::Shells => [0.96, 0.78, 0.36, 1.0],
        Ammo::Bullets => [0.86, 0.86, 0.56, 1.0],
        Ammo::Cells => [0.74, 0.64, 0.96, 1.0],
        Ammo::Rockets => [0.97, 0.56, 0.26, 1.0],
        Ammo::Bfg => [0.45, 0.96, 0.45, 1.0],
    }
}

pub fn col(c: Rgba) -> ImColor32 {
    ImColor32::from_rgba_f32s(c[0], c[1], c[2], c[3])
}

pub fn alpha(c: Rgba, a: f32) -> Rgba {
    [c[0], c[1], c[2], c[3] * a]
}

fn mix(a: Rgba, b: Rgba, t: f32) -> Rgba {
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

#[derive(Clone, Copy)]
pub struct Tex {
    pub id: TextureId,
    pub w: f32,
    pub h: f32,
}

/// Doom's own HUD numeral font (tools/convert_font.py): glyph rects in its atlas texture.
pub struct NumFont {
    pub tex: String,
    pub atlas: [f32; 2],
    /// x, y, w, h, top, left, advance (64 px font units)
    pub glyphs: HashMap<char, [f32; 7]>,
}

pub fn load_numfont(dir: &std::path::Path, name: &str) -> Option<NumFont> {
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(dir.join(format!("{name}.json"))).ok()?).ok()?;
    let atlas = v.get("atlas")?;
    let f = |x: &serde_json::Value, i: usize| x.get(i).and_then(|n| n.as_f64()).unwrap_or(0.0) as f32;
    let glyphs = v
        .get("glyphs")?
        .as_object()?
        .iter()
        .filter_map(|(k, g)| Some((k.chars().next()?, std::array::from_fn(|i| f(g, i)))))
        .collect();
    Some(NumFont { tex: name.to_string(), atlas: [f(atlas, 0), f(atlas, 1)], glyphs })
}

/// Short Doom-style key name for a key tag ("LSHFT", "RMB", "G").
pub fn key_label(vk: u16) -> String {
    // a controller button: "@" + its icon texture (key tags draw the icon)
    if crate::gamepad::is_pad(vk) {
        return crate::gamepad::icon(vk).map(|n| format!("@{n}")).unwrap_or_default();
    }
    match vk {
        // unbound: nothing at all (key tags skip an empty label - it drew a stray "00" tag, user)
        0 => String::new(),
        0x01 => "LMB".into(),
        0x02 => "RMB".into(),
        0x04 => "MMB".into(),
        0x05 => "MB4".into(),
        0x06 => "MB5".into(),
        0x09 => "TAB".into(),
        0x10 => "SHFT".into(),
        0x11 => "CTRL".into(),
        0x12 => "ALT".into(),
        0x14 => "CAPS".into(),
        0x20 => "SPACE".into(),
        0xA0 => "LSHFT".into(),
        0xA1 => "RSHFT".into(),
        0xA2 => "LCTRL".into(),
        0xA3 => "RCTRL".into(),
        0xA4 => "LALT".into(),
        0xA5 => "RALT".into(),
        0x30..=0x39 | 0x41..=0x5A => (vk as u8 as char).to_string(),
        0x70..=0x7B => format!("F{}", vk - 0x6F),
        // every other key: Windows' own name for it on this keyboard layout (Æ, Ø, Å, ;, -, Page
        // Up, End, Print Screen...), shortened - they all showed as hex codes (user)
        _ => os_key_name(vk).unwrap_or_else(|| format!("{vk:02X}")),
    }
}

fn os_key_name(vk: u16) -> Option<String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyNameTextW, MAPVK_VK_TO_CHAR, MAPVK_VK_TO_VSC_EX, MapVirtualKeyW};
    // symbol keys (VK_OEM_*): the character itself - dead keys came out as their names ("TREMA"
    // for the ¨ key on a Norwegian layout - user). Bit 31 marks a dead key.
    if (0xBA..=0xE2).contains(&vk) {
        let ch = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_CHAR) } & 0x7FFF_FFFF;
        if let Some(c) = char::from_u32(ch).filter(|c| !c.is_control() && !c.is_whitespace()) {
            // characters Doom's font doesn't have (½ drew nothing - user)
            let s = match c {
                '½' => "1/2",
                '¼' => "1/4",
                '¾' => "3/4",
                '²' => "2",
                '³' => "3",
                '¹' => "1",
                '¦' => "|",
                '¬' => "NOT",
                '¤' => "CUR",
                'µ' => "MU",
                '¶' => "PARA",
                '¢' => "CENT",
                _ => return Some(c.to_uppercase().collect()),
            };
            return Some(s.to_string());
        }
    }
    let sc = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
    if sc == 0 {
        return None;
    }
    // navigation keys need the extended bit, or Windows names the numpad key (Page Up -> Num 9)
    let ext = sc & 0xFF00 == 0xE000 || matches!(vk, 0x21..=0x28 | 0x2C | 0x2D | 0x2E | 0x5B..=0x5D | 0x6F | 0x90 | 0xA3 | 0xA5);
    let lparam = ((sc & 0xFF) << 16) as i32 | if ext { 1 << 24 } else { 0 };
    let mut buf = [0u16; 64];
    let n = unsafe { GetKeyNameTextW(lparam, &mut buf) };
    if n <= 0 {
        return None;
    }
    let name = String::from_utf16_lossy(&buf[..n as usize]).to_uppercase();
    let short = match name.as_str() {
        "PAGE UP" | "PGUP" => "PGUP".to_string(),
        "PAGE DOWN" | "PGDN" => "PGDN".to_string(),
        "PRNT SCRN" | "PRINT SCREEN" | "PRTSC" | "SYS RQ" => "PRTSC".to_string(),
        "INSERT" => "INS".to_string(),
        "DELETE" => "DEL".to_string(),
        "BACKSPACE" => "BKSP".to_string(),
        "SCROLL LOCK" => "SCRLK".to_string(),
        "NUM LOCK" => "NUMLK".to_string(),
        "CAPS LOCK" => "CAPS".to_string(),
        "APPLICATION" | "APPS" => "MENU".to_string(),
        "LEFT WINDOWS" => "LWIN".to_string(),
        "RIGHT WINDOWS" => "RWIN".to_string(),
        n if n.len() > 6 => n.replace(' ', ""),
        n => n.to_string(),
    };
    Some(short)
}

pub struct Fonts {
    pub num_big: Option<FontId>,
    pub num_mid: Option<FontId>,
    pub label: Option<FontId>,
    pub small: Option<FontId>,
}

/// Everything the HUD reads, copied out of the slayer under its lock.
pub struct State {
    pub time: f32,
    pub hp: i32,
    pub max_hp: i32,
    pub armor: i32,
    pub armor_max: i32,
    pub weapon: usize,
    pub ammo: [i32; 5],
    pub dash: f32,
    /// Dashes usable right now (dark icon while locked: both used / refilled in the air).
    pub dash_ready: u32,
    pub fuel: f32,
    pub belch_cd: f32,
    pub belch_max: f32,
    pub blood_punch: u32,
    pub wheel_open: bool,
    pub wheel_pick: Option<usize>,
    pub wheel_t: f32,
    pub last_shot_at: f32,
    pub bolt_at: f32,
    /// Super Shotgun: screen point of the demon the meathook would grab (None = icon at home).
    pub hook_icon: Option<[f32; 2]>,
    /// Weapon mods: Heavy Cannon scope (0..1), plasma heat, sticky bomb recharge, arbalest charge.
    pub zoom: f32,
    pub heat: f32,
    pub sticky_ready: f32,
    /// Sticky Bombs: 0..1 of the bomb recharging now, 0..1 of a full reload (-1 none),
    /// (seconds since a bomb / the magazine came back, whole magazine?).
    pub sticky_charge: f32,
    pub sticky_reload: f32,
    pub sticky_flash: (f32, bool),
    pub arb: f32,
    /// BFG wind-up 0..1 (0 = not charging).
    pub bfg_charge: f32,
    /// Seconds the meathook has been ready (< 0: recharging).
    pub hook_ready_for: f32,
    /// Key tags: dash, flame belch, chainsaw, weapon mod, crucible.
    pub keys: [u16; 5],
    /// The Crucible: in the hands (its reticle replaces the gun's), its charges.
    pub crucible_out: bool,
    pub crucible_charges: u32,
}

pub struct Ctx<'a> {
    pub ui: &'a Ui,
    pub dl: &'a DrawListMut<'a>,
    pub tex: &'a HashMap<String, Tex>,
    pub fonts: &'a Fonts,
    pub num: Option<&'a NumFont>,
    /// Doom's letter font (regular / heavy), when converted
    pub letters: Option<&'a NumFont>,
    pub letters_heavy: Option<&'a NumFont>,
    pub w: f32,
    pub h: f32,
    /// pixels per 1080p pixel
    pub s: f32,
}

impl Ctx<'_> {
    pub fn text(&self, font: Option<FontId>, pos: [f32; 2], c: Rgba, t: &str) {
        let _f = font.map(|f| self.ui.push_font(f));
        // Doom's numerals carry a soft glow.
        let g = col(alpha(c, 0.18));
        for (dx, dy) in [(-1.5, 0.0), (1.5, 0.0), (0.0, -1.5), (0.0, 1.5)] {
            self.dl.add_text([pos[0] + dx * self.s, pos[1] + dy * self.s], g, t);
        }
        self.dl.add_text(pos, col(c), t);
    }

    /// Doom's segmented numerals, `height` px tall, vertically centred on `cy`, starting at `x`
    /// (or ending there when `right`). Returns the width.
    pub fn digits(&self, x: f32, cy: f32, height: f32, c: Rgba, t: &str, right: bool) -> f32 {
        let Some(nf) = self.num.filter(|nf| self.tex.contains_key(&nf.tex)) else {
            let sz = self.text_size(self.fonts.num_mid, t);
            let x0 = if right { x - sz[0] } else { x };
            self.text(self.fonts.num_mid, [x0, cy - sz[1] * 0.5], c, t);
            return sz[0];
        };
        let tex = self.tex[&nf.tex];
        let k = height / 74.0;
        let width: f32 = t.chars().filter_map(|ch| nf.glyphs.get(&ch)).map(|g| g[6] * k).sum::<f32>() - 4.0 * k;
        let base = cy + 33.0 * k;
        let draw = |color: Rgba, dx: f32, dy: f32| {
            let mut pen = if right { x - width } else { x };
            for ch in t.chars() {
                let Some(g) = nf.glyphs.get(&ch) else { continue };
                let [gx, gy, gw, gh, top, left, adv] = *g;
                let p0 = [pen + left * k + dx, base - top * k + dy];
                let p1 = [p0[0] + gw * k, p0[1] + gh * k];
                self.dl
                    .add_image(tex.id, p0, p1)
                    .uv_min([gx / nf.atlas[0], gy / nf.atlas[1]])
                    .uv_max([(gx + gw) / nf.atlas[0], (gy + gh) / nf.atlas[1]])
                    .col(col(color))
                    .build();
                pen += adv * k;
            }
        };
        // Soft bloom like Doom's HUD, then the crisp digits.
        let g = alpha(c, 0.16);
        let o = (height * 0.05).max(1.0);
        for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
            draw(g, dx, dy);
        }
        draw(c, 0.0, 0.0);
        width
    }

    /// A number right-aligned in a `field`-digit wide slot starting at `x`.
    pub fn display(&self, x: f32, cy: f32, height: f32, c: Rgba, t: &str, field: usize) -> f32 {
        let w = height / 74.0 * 56.0 * field.max(t.len()) as f32;
        self.digits(x + w, cy, height, c, t, true);
        w
    }

    /// Doom's ability hex: thick rim broken in the middle of the left and right sides ("[ ]").
    fn bracket_hex(&self, c: [f32; 2], r: f32, color: Rgba, thick: f32) {
        let v: Vec<[f32; 2]> = (0..6)
            .map(|i| {
                let a = PI / 6.0 + i as f32 * PI / 3.0;
                [c[0] + a.cos() * r, c[1] + a.sin() * r]
            })
            .collect();
        for i in 0..6 {
            let (a, b) = (v[i], v[(i + 1) % 6]);
            // edges 2 (left) and 5 (right) are the vertical sides: keep only their ends
            if i == 2 || i == 5 {
                let at = |t: f32| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t];
                self.dl.add_line(a, at(0.22), col(color)).thickness(thick).build();
                self.dl.add_line(at(0.78), b, col(color)).thickness(thick).build();
            } else {
                self.dl.add_line(a, b, col(color)).thickness(thick).build();
            }
        }
    }

    /// Doom's key tag: a small yellow plate with the binding in dark letters, centred above `at`.
    /// Doom's letter font, if loaded (heavy weight when asked and available).
    pub fn doom_font(&self, heavy: bool) -> Option<&NumFont> {
        let f = if heavy { self.letters_heavy.or(self.letters) } else { self.letters };
        f.filter(|f| self.tex.contains_key(&f.tex))
    }

    /// Ink extent of `t` in Doom's letter font: (left, right) from the pen start, px. Capitals are
    /// `cap` px tall; `tr` px is added between letters.
    pub fn doom_text_span(&self, f: &NumFont, cap: f32, t: &str, tr: f32, sx: f32) -> (f32, f32) {
        let k = cap / 68.0 * sx;
        let (mut pen, mut l, mut r) = (0.0f32, f32::MAX, 0.0f32);
        for ch in t.chars() {
            match f.glyphs.get(&ch) {
                Some(g) => {
                    // glyph rects carry 3 texels of distance-field padding on each side
                    l = l.min(pen + (g[5] + 3.0) * k);
                    r = r.max(pen + (g[5] + g[2] - 3.0) * k);
                    pen += g[6] * k + tr;
                }
                None => pen += 20.0 * k + tr,
            }
        }
        if l == f32::MAX { (0.0, 0.0) } else { (l, r) }
    }

    /// Draw `t` in Doom's letter font with its ink starting at `x`, capitals `cap` px tall and
    /// vertically centred on `cy`; `glow` adds the HUD's soft bloom. Returns the ink width.
    pub fn doom_text(&self, f: &NumFont, x: f32, cy: f32, cap: f32, c: Rgba, t: &str, tr: f32, sx: f32, glow: bool) -> f32 {
        let tex = self.tex[&f.tex];
        let k = cap / 68.0;
        // `sx` squeezes (< 1) or widens the letters horizontally
        let kx = k * sx;
        let (l, r) = self.doom_text_span(f, cap, t, tr, sx);
        let base = cy + 34.0 * k;
        let draw = |color: Rgba, dx: f32, dy: f32| {
            let mut pen = x - l;
            for ch in t.chars() {
                let Some(g) = f.glyphs.get(&ch) else {
                    pen += 20.0 * kx + tr;
                    continue;
                };
                let [gx, gy, gw, gh, top, left, adv] = *g;
                let p0 = [pen + left * kx + dx, base - top * k + dy];
                self.dl
                    .add_image(tex.id, p0, [p0[0] + gw * kx, p0[1] + gh * k])
                    .uv_min([gx / f.atlas[0], gy / f.atlas[1]])
                    .uv_max([(gx + gw) / f.atlas[0], (gy + gh) / f.atlas[1]])
                    .col(col(color))
                    .build();
                pen += adv * kx + tr;
            }
        };
        if glow {
            let o = (cap * 0.08).max(1.0);
            for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
                draw(alpha(c, 0.16), dx, dy);
            }
        }
        draw(c, 0.0, 0.0);
        r - l
    }

    pub fn key_tag(&self, at: [f32; 2], label: &str) {
        self.key_tag_k(at, label, 1.0);
    }

    /// A key tag at `k` times its normal size (it follows the size of the element it labels).
    pub fn key_tag_k(&self, at: [f32; 2], label: &str, k: f32) {
        if label.is_empty() {
            return;
        }
        let s = self.s * k;
        // controller button: Doom's own button icon instead of a key box (live size: [hud]
        // pad_icon = [dx, dy, size, opacity])
        if let Some(tex) = label.strip_prefix('@') {
            let pi = crate::config::get_cached().hud.get("pad_icon").copied().unwrap_or([0.0, 4.0, 1.0, 1.0]);
            let sz = 32.0 * s * pi[2].clamp(0.3, 3.0);
            self.image(tex, [at[0] + pi[0] * s, at[1] - sz * 0.5 + pi[1] * s], sz, [1.0, 1.0, 1.0, pi[3].clamp(0.0, 1.0)]);
            return;
        }
        // Doom's letters (heavy), live: [hud] key_letters = [letter spacing px, -, size, -]
        let kl = crate::config::get_cached().hud.get("key_letters").copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
        if let Some(f) = self.doom_font(true) {
            let tr = kl[0] * s;
            let cap = 12.0 * s * kl[2].clamp(0.3, 3.0);
            // per-key squeeze, live: [hud] key_<label> = [width scale, ..] (e.g. key_lshft)
            let sx = crate::config::get_cached().hud.get(&format!("key_{}", label.to_lowercase())).map_or(1.0, |v| v[0].clamp(0.3, 2.0));
            let (l, r) = self.doom_text_span(f, cap, label, tr, sx);
            let tw = r - l;
            let (pw, ph) = (tw + 10.0 * s, 20.0 * s);
            let p0 = [at[0] - pw * 0.5, at[1] - ph];
            self.dl.add_rect(p0, [p0[0] + pw, at[1]], col([0.96, 0.86, 0.33, 0.95])).filled(true).rounding(2.0 * s).build();
            self.doom_text(f, at[0] - tw * 0.5, at[1] - ph * 0.5, cap, [0.07, 0.06, 0.02, 1.0], label, tr, sx, false);
            return;
        }
        // letter spacing, live: [hud] key_letters = [px added between letters (negative = tighter), ...]
        let tr = crate::config::get_cached().hud.get("key_letters").map(|v| v[0]).unwrap_or(0.0) * s;
        let ws: Vec<[f32; 2]> = label.chars().map(|ch| self.text_size(self.fonts.small, &ch.to_string())).collect();
        let tw = ws.iter().map(|w| w[0]).sum::<f32>() + tr * (ws.len().max(1) - 1) as f32;
        let th = ws.iter().map(|w| w[1]).fold(0.0, f32::max);
        let (pw, ph) = (tw + 10.0 * s, 20.0 * s);
        let p0 = [at[0] - pw * 0.5, at[1] - ph];
        self.dl.add_rect(p0, [p0[0] + pw, at[1]], col([0.96, 0.86, 0.33, 0.95])).filled(true).rounding(2.0 * s).build();
        let _f = self.fonts.small.map(|f| self.ui.push_font(f));
        let mut x = at[0] - tw * 0.5;
        for (ch, w) in label.chars().zip(&ws) {
            self.dl.add_text([x, at[1] - ph * 0.5 - th * 0.5], col([0.07, 0.06, 0.02, 1.0]), ch.to_string());
            x += w[0] + tr;
        }
    }

    fn text_size(&self, font: Option<FontId>, t: &str) -> [f32; 2] {
        let _f = font.map(|f| self.ui.push_font(f));
        self.ui.calc_text_size(t)
    }

    /// 1080p reference coordinates -> screen (anchored to the bottom-left / bottom-right corners).
    /// A texture at its native size x `k`, top-left at `p` (Doom's HUD pieces).
    fn piece(&self, name: &str, p: [f32; 2], k: f32, tint: Rgba) -> Option<[f32; 2]> {
        let t = self.tex.get(name)?;
        let (w, h) = (t.w * k, t.h * k);
        self.dl.add_image(t.id, p, [p[0] + w, p[1] + h]).col(col(tint)).build();
        Some([w, h])
    }

    /// Same, mirrored left-right (Doom builds its hex rings from one half, flipped).
    fn piece_flip(&self, name: &str, p: [f32; 2], k: f32, tint: Rgba) {
        if let Some(t) = self.tex.get(name) {
            let (w, h) = (t.w * k, t.h * k);
            self.dl.add_image(t.id, p, [p[0] + w, p[1] + h]).uv_min([1.0, 0.0]).uv_max([0.0, 1.0]).col(col(tint)).build();
        }
    }

    /// bl / br for Doom's HUD pieces: the positions measured from the user's Doom screenshot sat
    /// 18 px (1080p) too high in game, so they are moved down by that.
    fn dbl(&self, x: f32, y: f32) -> [f32; 2] {
        // live tuning: hud_left = [dx, dy, scale] (scaled about the bottom-left corner)
        let l = crate::config::get_cached().hud_left;
        let sc = l[2].clamp(0.3, 3.0);
        self.bl(x * sc + l[0], 1080.0 - (1080.0 - (y + DOOM_DY)) * sc + l[1])
    }

    fn dbr(&self, x: f32, y: f32) -> [f32; 2] {
        // live tuning: hud_right = [dx, dy, scale] (scaled about the bottom-right corner)
        let r = crate::config::get_cached().hud_right;
        let sc = r[2].clamp(0.3, 3.0);
        self.br(1920.0 - (1920.0 - x) * sc + r[0], 1080.0 - (1080.0 - (y + DOOM_DY)) * sc + r[1])
    }

    fn bl(&self, x: f32, y: f32) -> [f32; 2] {
        [x * self.s, self.h - (1080.0 - y) * self.s]
    }

    fn br(&self, x: f32, y: f32) -> [f32; 2] {
        [self.w - (1920.0 - x) * self.s, self.h - (1080.0 - y) * self.s]
    }

    /// Slanted box (parallelogram leaning right like Doom's panels).
    fn slant(&self, p: [f32; 2], w: f32, h: f32, lean: f32, c: Rgba, filled: bool) {
        let pts = vec![
            [p[0] + lean, p[1]],
            [p[0] + w + lean, p[1]],
            [p[0] + w, p[1] + h],
            [p[0], p[1] + h],
            [p[0] + lean, p[1]],
        ];
        if filled {
            self.dl.add_polyline(pts, col(c)).filled(true).build();
        } else {
            self.dl.add_polyline(pts, col(c)).thickness(1.5 * self.s).build();
        }
    }

    fn hexagon(&self, c: [f32; 2], r: f32, color: Rgba, filled: bool, thick: f32) {
        let pts: Vec<[f32; 2]> = (0..=6)
            .map(|i| {
                let a = PI / 6.0 + i as f32 * PI / 3.0;
                [c[0] + a.cos() * r, c[1] + a.sin() * r]
            })
            .collect();
        if filled {
            self.dl.add_polyline(pts, col(color)).filled(true).build();
        } else {
            self.dl.add_polyline(pts, col(color)).thickness(thick).build();
        }
    }

    fn image(&self, name: &str, center: [f32; 2], size: f32, tint: Rgba) {
        if let Some(t) = self.tex.get(name) {
            let k = size / t.w.max(t.h);
            let (hw, hh) = (t.w * k * 0.5, t.h * k * 0.5);
            self.dl
                .add_image(t.id, [center[0] - hw, center[1] - hh], [center[0] + hw, center[1] + hh])
                .col(col(tint))
                .build();
        }
    }

    // ------------------------------------------------------------------ vector icons

    fn icon_cross(&self, c: [f32; 2], r: f32, color: Rgba) {
        let a = r * 0.32;
        self.dl.add_rect([c[0] - a, c[1] - r], [c[0] + a, c[1] + r], col(color)).filled(true).build();
        self.dl.add_rect([c[0] - r, c[1] - a], [c[0] + r, c[1] + a], col(color)).filled(true).build();
    }

    fn icon_shield(&self, c: [f32; 2], r: f32, color: Rgba) {
        let pts = vec![
            [c[0] - r, c[1] - r * 0.8],
            [c[0] + r, c[1] - r * 0.8],
            [c[0] + r * 0.85, c[1] + r * 0.1],
            [c[0], c[1] + r],
            [c[0] - r * 0.85, c[1] + r * 0.1],
            [c[0] - r, c[1] - r * 0.8],
        ];
        self.dl.add_polyline(pts, col(color)).filled(true).build();
        let dark = col([0.02, 0.05, 0.02, 0.9]);
        self.dl.add_line([c[0] - r * 0.55, c[1] - r * 0.25], [c[0], c[1] + r * 0.35], dark).thickness(2.0 * self.s).build();
        self.dl.add_line([c[0] + r * 0.55, c[1] - r * 0.25], [c[0], c[1] + r * 0.35], dark).thickness(2.0 * self.s).build();
    }

    /// Small ammo-type glyph (shells / bullets / cells / rockets / BFG).
    pub fn icon_ammo(&self, a: Ammo, c: [f32; 2], r: f32, color: Rgba) {
        // Doom's own ammo icons when converted (tools/convert_ammo_icons.py); vector fallback.
        let name = match a {
            Ammo::Shells => "ammo_shells",
            Ammo::Bullets => "ammo_bullets",
            Ammo::Cells => "ammo_cells",
            Ammo::Rockets => "ammo_rockets",
            Ammo::Bfg => "ammo_bfg",
        };
        if self.tex.contains_key(name) {
            self.image(name, c, r * 2.6, color);
            return;
        }
        let cc = col(color);
        match a {
            Ammo::Shells => {
                for dx in [-0.45, 0.45] {
                    let x = c[0] + dx * r;
                    self.dl.add_rect([x - r * 0.3, c[1] - r * 0.8], [x + r * 0.3, c[1] + r * 0.45], cc).filled(true).build();
                    self.dl.add_rect([x - r * 0.38, c[1] + r * 0.5], [x + r * 0.38, c[1] + r * 0.85], cc).filled(true).build();
                }
            }
            Ammo::Bullets => {
                for dx in [-0.6, 0.0, 0.6] {
                    let x = c[0] + dx * r;
                    self.dl.add_rect([x - r * 0.2, c[1] - r * 0.3], [x + r * 0.2, c[1] + r * 0.85], cc).filled(true).build();
                    self.dl.add_triangle([x - r * 0.2, c[1] - r * 0.3], [x + r * 0.2, c[1] - r * 0.3], [x, c[1] - r * 0.85], cc).filled(true).build();
                }
            }
            Ammo::Cells => {
                self.dl.add_rect([c[0] - r * 0.55, c[1] - r * 0.6], [c[0] + r * 0.55, c[1] + r * 0.85], cc).thickness(2.0 * self.s).build();
                self.dl.add_rect([c[0] - r * 0.25, c[1] - r * 0.85], [c[0] + r * 0.25, c[1] - r * 0.6], cc).filled(true).build();
                self.dl.add_rect([c[0] - r * 0.3, c[1] - r * 0.25], [c[0] + r * 0.3, c[1] + r * 0.55], cc).filled(true).build();
            }
            Ammo::Rockets => {
                self.dl.add_triangle([c[0], c[1] - r], [c[0] - r * 0.35, c[1] - r * 0.3], [c[0] + r * 0.35, c[1] - r * 0.3], cc).filled(true).build();
                self.dl.add_rect([c[0] - r * 0.25, c[1] - r * 0.3], [c[0] + r * 0.25, c[1] + r * 0.6], cc).filled(true).build();
                self.dl.add_triangle([c[0] - r * 0.25, c[1] + r * 0.2], [c[0] - r * 0.6, c[1] + r * 0.9], [c[0] - r * 0.25, c[1] + r * 0.9], cc).filled(true).build();
                self.dl.add_triangle([c[0] + r * 0.25, c[1] + r * 0.2], [c[0] + r * 0.6, c[1] + r * 0.9], [c[0] + r * 0.25, c[1] + r * 0.9], cc).filled(true).build();
            }
            Ammo::Bfg => {
                self.dl.add_circle(c, r * 0.75, cc).thickness(2.0 * self.s).num_segments(20).build();
                self.dl.add_circle(c, r * 0.35, cc).filled(true).num_segments(16).build();
            }
        }
    }

    /// Segmented slanted gauge: `segs` pieces, `frac` of them lit (partial last one dimmed).
    fn gauge(&self, p: [f32; 2], segs: usize, frac: f32, seg_w: f32, h: f32, gap: f32, color: Rgba) {
        let lit = frac * segs as f32;
        for i in 0..segs {
            let x = p[0] + i as f32 * (seg_w + gap);
            let f = (lit - i as f32).clamp(0.0, 1.0);
            if f >= 1.0 {
                self.slant([x, p[1]], seg_w, h, h * 0.45, alpha(color, 0.95), true);
            } else {
                self.slant([x, p[1]], seg_w, h, h * 0.45, alpha(color, 0.06), true);
                if f > 0.0 {
                    // partly filled: lit from the left
                    self.slant([x, p[1]], seg_w * f, h, h * 0.45, alpha(color, 0.75), true);
                }
                self.slant([x, p[1]], seg_w, h, h * 0.45, alpha(color, 0.45), false);
            }
        }
    }
}

// ---------------------------------------------------------------------- Doom's own HUD pieces

/// Doom's HUD pieces are drawn at 0.42x their texture size (measured from the user's Doom
/// screenshot: the health shell's 94 px band is 39 px tall on a 1080p screen, a pip slot - one
/// 61 px segment tile - is 24 px).
const DK: f32 = 0.42;
/// Vertical correction for the Doom HUD positions (see Ctx::dbl).
const DOOM_DY: f32 = 18.0;

fn tune() -> std::sync::Arc<crate::config::Config> {
    crate::config::get_cached()
}

/// Live per-element tuning ([hud] table in doomslayer.toml): name = [move x, move y, scale,
/// opacity] in 1080p px, scaled about the element's own anchor. Elements nest: a row's transform
/// also moves / scales its icon, number and pips.
#[derive(Clone, Copy)]
struct Tf {
    sc: f32,
    tx: f32,
    ty: f32,
    a: f32,
}

impl Tf {
    fn id() -> Self {
        Tf { sc: 1.0, tx: 0.0, ty: 0.0, a: 1.0 }
    }
    /// The element `name`'s own transform, anchored at (ax, ay).
    fn el(t: &crate::config::Config, name: &str, ax: f32, ay: f32) -> Self {
        let e = t.hud.get(name).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
        let sc = e[2].clamp(0.1, 5.0);
        Tf { sc, tx: ax * (1.0 - sc) + e[0], ty: ay * (1.0 - sc) + e[1], a: e[3].clamp(0.0, 1.0) }
    }
    /// This transform inside `outer` (applied first, then outer's).
    fn within(self, outer: Tf) -> Self {
        Tf { sc: self.sc * outer.sc, tx: self.tx * outer.sc + outer.tx, ty: self.ty * outer.sc + outer.ty, a: self.a * outer.a }
    }
    fn pt(&self, x: f32, y: f32) -> (f32, f32) {
        (x * self.sc + self.tx, y * self.sc + self.ty)
    }
}

/// A vital row from Doom's pieces (hud_healthinfo.swf): left part with the icon, `tiles` middle
/// tiles, the right end; the number right-aligned at `num_r`; 8 pips from `pip_x` (reference px,
/// visible left edge) - lit for what you have, Doom's empty pip for the rest.
#[allow(clippy::too_many_arguments)]
fn doom_row(c: &Ctx, name: &str, tex_at: [f32; 2], parts: (&str, &str, &str), fills: (&str, &str, &str), tiles: usize, icon: (&str, [f32; 2], f32), color: Rgba, value: i32, max: i32, num_r: f32, cy: f32, pip_x: f32, more: usize) {
    let t = tune();
    let bs = c.s * t.hud_left[2].clamp(0.3, 3.0);
    let row = Tf::el(&t, &format!("{name}_row"), tex_at[0], tex_at[1]);
    // A 4-digit number (Vigor levelled up: 1036 HP) ran into the cells: each digit past three
    // moves the cells one bar segment right and makes the bar one segment longer (user). Sized by
    // the max, so it doesn't jump when the value crosses 1000.
    // (`more` = the health number's extra digits: the shield row follows it, so its cells stay
    // right above the first health cells - user)
    let _ = (value, max);
    let seg = c.tex.get("dh_health_container_segment_shell").map_or(61.0, |t| t.w) * DK;
    let pip_x = pip_x + more as f32 * seg;
    let tiles = tiles + more;
    // pip count per row (live: hud_pip_count = [armor, health])
    let n = (if name == "armor" { t.hud_pip_count[0] } else { t.hud_pip_count[1] }).clamp(1, 16) as usize;
    let s = bs * row.sc;
    let k = DK * s;
    let (rx, ry) = row.pt(tex_at[0], tex_at[1]);
    let at = c.dbl(rx, ry);
    let (l, m, r) = parts;
    let (lf, mf, rf) = fills;
    let fill = alpha(color, 0.65 * row.a);
    let shell = alpha(mix(color, [1.0, 1.0, 1.0, 1.0], 0.15), row.a);
    if l == "dh_ammo_container_new" {
        // Shield: the ammo container flipped left-right and up-down (its tab becomes the icon
        // box), visible from x 62 / y 925 and stretched so the 8 pips fit (0.394 tall).
        if let Some(tx) = c.tex.get(l) {
            // end padding after the last pip, live: [hud] armor_end = [extra px, ..]
            let end = pip_x + (n as f32 - 1.0) * t.hud_pips[1] + 56.0 + t.hud.get("armor_end").map_or(0.0, |v| v[0]);
            let sx = (end - 62.0) / 620.0;
            let sy = 0.394;
            let (x0, y0) = row.pt(62.0 - 12.0 * sx, 925.2 - 21.0 * sy);
            let (x1, y1) = row.pt(62.0 + (652.0 - 12.0) * sx, 925.2 + (136.0 - 21.0) * sy);
            c.dl.add_image(tx.id, c.dbl(x0, y0), c.dbl(x1, y1)).uv_min([1.0, 1.0]).uv_max([0.0, 0.0]).col(col(alpha(mix(color, [1.0, 1.0, 1.0, 1.0], 0.2), row.a))).build();
        }
        let _ = (m, r, lf, mf, rf, tiles);
    } else {
        let mut x = at[0];
        let lw = c.tex.get(l).map_or(0.0, |t| t.w * k);
        let mw = c.tex.get(m).map_or(0.0, |t| t.w * k);
        c.piece(lf, [x, at[1]], k, fill);
        c.piece(l, [x, at[1]], k, shell);
        x += lw;
        for _ in 0..tiles {
            c.piece(mf, [x, at[1]], k, fill);
            c.piece(m, [x, at[1]], k, shell);
            x += mw;
        }
        c.piece(rf, [x, at[1]], k, fill);
        c.piece(r, [x, at[1]], k, shell);
    }
    // icon + its glow
    let (icon, ic, isz) = icon;
    let it = Tf::el(&t, &format!("{name}_icon"), ic[0], ic[1]).within(row);
    let (ix, iy) = it.pt(ic[0], ic[1]);
    let icp = c.dbl(ix, iy);
    let isz = isz * bs * it.sc;
    let gt = Tf::el(&t, &format!("{name}_icon_glow"), ic[0], ic[1]).within(it);
    let (gx, gy) = gt.pt(ic[0], ic[1]);
    c.image(&format!("{icon}_glow"), c.dbl(gx, gy), isz * 1.1 * gt.sc / it.sc, alpha(color, 0.55 * gt.a));
    c.image(icon, icp, isz, alpha(color, it.a));
    // number (Doom's segment digits), right-aligned on a fixed edge: both rows use the same field
    // width (3 digits, 4 once max HP has 4), so units / tens / hundreds always stack, whatever the
    // current values (user)
    let nt = Tf::el(&t, &format!("{name}_number"), num_r, cy).within(row);
    let nh = 25.0 * bs * nt.sc;
    let w = nh / 74.0 * 56.0 * 3.0;
    let (nx, ny) = nt.pt(num_r, cy);
    let np = c.dbl(nx, ny);
    c.display(np[0] - w, np[1], nh, alpha(color, nt.a), &value.max(0).to_string(), 3 + more);
    // pips
    let pt = Tf::el(&t, &format!("{name}_pips"), pip_x, cy).within(row);
    let per = (max.max(1) as f32) / n as f32;
    let have = value.max(0) as f32 / per;
    let pz = 0.436 * pt.sc;
    let pk = pz * bs;
    // the lit cells alone (not the dark backing): `{name}_pip_fill`, inside the pips' transform
    let ft = Tf::el(&t, &format!("{name}_pip_fill"), pip_x, cy).within(pt);
    let fk = 0.436 * ft.sc * bs;
    // the shield row is the flipped ammo container, so its pips are mirrored left-right to match
    let flip = l == "dh_ammo_container_new";
    let draw = |name: &str, p: [f32; 2], k: f32, tint: Rgba| if flip { c.piece_flip(name, p, k, tint) } else { c.piece(name, p, k, tint); };
    for i in 0..n {
        let vx = pip_x + i as f32 * t.hud_pips[1];
        // pip top-left in reference px (before the pips' own transform)
        let (px0, py0) = (vx - 20.0 * 0.436, cy - 47.5 * 0.436);
        let (qx, qy) = pt.pt(px0, py0);
        let px = c.dbl(qx, qy);
        let f = (have - i as f32).clamp(0.0, 1.0);
        draw("dh_health_pip_empty", [px[0] + (if flip { 2.0 } else { -2.0 }) * pk, px[1] - 3.0 * pk], pk, alpha(color, 0.35 * pt.a));
        let (fx, fy) = ft.pt(px0, py0);
        let fp = c.dbl(fx, fy);
        if f >= 1.0 {
            // soft bloom around lit cells, live: `{name}_pip_glow` = [.., .., size, opacity]
            let pg = t.hud.get(&format!("{name}_pip_glow")).copied().unwrap_or([0.0, 0.0, 1.0, 0.3]);
            if pg[3] > 0.0 {
                let o = 2.0 * fk * pg[2];
                for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
                    draw("dh_health_pip_full", [fp[0] + dx, fp[1] + dy], fk, alpha(color, pg[3].min(1.0) * 0.5 * ft.a));
                }
            }
            draw("dh_health_pip_full", fp, fk, alpha(color, ft.a));
        } else if f > 0.0 {
            // a partly full cell is cut like a progress bar: the full cell up to a slanted edge
            // parallel to its own sides (pip art: left side x 49 - 0.5625 (y - 24), 44 px wide)
            if let Some(tx) = c.tex.get("dh_health_pip_full") {
                let (w, h) = (tx.w, tx.h);
                // cut x at the texture's top / bottom rows, in screen-local px (mirrored for the shield)
                let (ct, cb) = if flip { (11.5 + 44.0 * f, 66.1 + 44.0 * f) } else { (62.5 + 44.0 * f, 7.9 + 44.0 * f) };
                let uu = |x: f32| if flip { 1.0 - x / w } else { x / w };
                let q = |x: f32, y: f32| [fp[0] + x * fk, fp[1] + y * fk];
                c.dl.add_image_quad(tx.id, q(0.0, 0.0), q(ct, 0.0), q(cb, h), q(0.0, h))
                    .uv([uu(0.0), 0.0], [uu(ct), 0.0], [uu(cb), 1.0], [uu(0.0), 1.0])
                    .col(col(alpha(color, ft.a)))
                    .build();
            }
        }
    }
}

/// Part of a ring half (`tex` drawn at `at`, scale `k`, mirrored when `flip`) between two angles
/// around `cen` (radians from straight up, anticlockwise on screen): a fan of thin wedges, each
/// ending on the texture's edge so nothing samples outside it.
fn ring_sweep(c: &Ctx, tex: &str, at: [f32; 2], k: f32, flip: bool, cen: [f32; 2], a0: f32, a1: f32, tint: Rgba) {
    let Some(t) = c.tex.get(tex) else { return };
    let (w, h) = (t.w * k, t.h * k);
    let uv = |q: [f32; 2]| {
        let u = ((q[0] - at[0]) / w).clamp(0.0, 1.0);
        [if flip { 1.0 - u } else { u }, ((q[1] - at[1]) / h).clamp(0.0, 1.0)]
    };
    // where the ray at angle `a` leaves the texture's rectangle
    let hit = |a: f32| {
        let (dx, dy) = (-a.sin(), -a.cos());
        let mut tm = f32::MAX;
        if dx < -1e-6 { tm = tm.min((at[0] - cen[0]) / dx); }
        if dx > 1e-6 { tm = tm.min((at[0] + w - cen[0]) / dx); }
        if dy < -1e-6 { tm = tm.min((at[1] - cen[1]) / dy); }
        if dy > 1e-6 { tm = tm.min((at[1] + h - cen[1]) / dy); }
        [cen[0] + dx * tm, cen[1] + dy * tm]
    };
    let n = (((a1 - a0) / std::f32::consts::PI) * 48.0).ceil().max(1.0) as usize;
    let col = col(tint);
    for i in 0..n {
        let (b0, b1) = (a0 + (a1 - a0) * i as f32 / n as f32, a0 + (a1 - a0) * (i + 1) as f32 / n as f32);
        let (q0, q1) = (hit(b0), hit(b1));
        c.dl.add_image_quad(t.id, cen, q0, q1, q1).uv(uv(cen), uv(q0), uv(q1), uv(q1)).col(col).build();
    }
}

/// Doom's dash / Blood Punch ring: its hex from two mirrored "radial fill" halves, the icon inside,
/// the count beside it and the key tag above (positions from the user's Doom screenshot).
#[allow(clippy::too_many_arguments)]
fn doom_hex(c: &Ctx, name: &str, cen: [f32; 2], color: Rgba, icon: &str, count: u32, key: &str, lit: bool, rune: bool, fill: f32) {
    let t = tune();
    let bs = c.s * t.hud_left[2].clamp(0.3, 3.0);
    let e = Tf::el(&t, name, cen[0], cen[1]);
    let s = bs * e.sc;
    let (cx, cy) = e.pt(cen[0], cen[1]);
    let p = c.dbl(cx, cy);
    let ring = alpha(color, if lit { e.a } else { 0.4 * e.a });
    // glow strength, live: `{name}_glow` = [.., .., .., strength] (1 = Doom's, above 1 = brighter)
    let gs = t.hud.get(&format!("{name}_glow")).map(|v| v[3]).unwrap_or(1.0).clamp(0.0, 3.0);
    let glow = alpha(color, (if lit { 0.55 } else { 0.15 } * e.a * gs).min(1.0));
    if rune {
        // Blood Punch: two of Doom's radialfill halves (+ glow) mirrored into a closed hexagon,
        // their straight edges meeting at the centre (213 px tall -> 56 px)
        let k = 0.263 * s;
        let top = p[1] - 128.0 * k; // 256 tall, ring centred
        c.piece("dh_radialfill_glow", [p[0] - 146.0 * k, top], k, glow);
        c.piece_flip("dh_radialfill_glow", [p[0], top], k, glow);
        c.piece("dh_radialfill", [p[0] - 128.0 * k, top], k, ring);
        c.piece_flip("dh_radialfill", [p[0], top], k, ring);
    } else {
        // Dash: Doom's bracket halves (radialfill_halves; 134 px tall -> 56 px), mirrored "[ ]"
        let k = 0.418 * s;
        let top = p[1] - (11.0 + 134.0 * 0.5) * k;
        // texture edges meet at the centre (no overlap: overlapping edges drew a seam line); the
        // halves' own open ends leave Doom's gaps at the top and bottom
        let hw = 91.0 * k;
        // dim halves underneath; the green fills each half from the bottom up as its dash
        // recharges: the left half is dash 1, the right half dash 2 (`fill` = charges, fractional)
        let dim = alpha(color, 0.3 * e.a);
        c.piece("dh_radialfill_halves", [p[0] - hw, top], k, dim);
        c.piece_flip("dh_radialfill_halves", [p[0], top], k, dim);
        // the charge fills like a clock hand sweeping around the ring's centre, anticlockwise from
        // the top: down the left half (dash 1), then up the right half from the bottom (dash 2)
        // A half only shines green when that dash is usable (`count` = usable dashes); while it
        // recharges or is locked (both used / refilled in the air) it fills in a dull grey-green.
        for (side, f) in [(0, fill.clamp(0.0, 1.0)), (1, (fill - 1.0).clamp(0.0, 1.0))] {
            if f <= 0.0 {
                continue;
            }
            let x0 = if side == 0 { p[0] - hw } else { p[0] };
            let a0 = if side == 0 { 0.0 } else { std::f32::consts::PI };
            let ready = count > side as u32;
            let layers: &[(&str, Rgba)] = if ready {
                &[("dh_radialfill_halves_glow", alpha(color, (0.55 * e.a * gs).min(1.0))), ("dh_radialfill_halves", alpha(color, e.a))]
            } else {
                &[("dh_radialfill_halves", alpha(mix(color, [0.55, 0.58, 0.55, 1.0], 0.6), 0.6 * e.a))]
            };
            for &(tex, tint) in layers {
                ring_sweep(c, tex, [x0, top], k, side == 1, p, a0, a0 + std::f32::consts::PI * f, tint);
            }
        }
    }
    // the icon alone: `{name}_icon` = [dx, dy, scale, opacity], inside the ring's transform
    let it = Tf::el(&t, &format!("{name}_icon"), cen[0], cen[1]).within(e);
    let (ix, iy) = it.pt(cen[0], cen[1]);
    // tinted with the ring's green, with a soft bloom, live: `{name}_icon_glow` = [tint (0 white..1 green), -, size, opacity]
    let ig = t.hud.get(&format!("{name}_icon_glow")).copied().unwrap_or([0.7, 0.0, 1.0, 0.3]);
    let ip = c.dbl(ix, iy);
    let isz = 36.0 * bs * it.sc;
    let tint = if lit { alpha(mix([1.0, 1.0, 1.0, 1.0], color, ig[0].clamp(0.0, 1.0)), it.a) } else { alpha(color, 0.5 * it.a) };
    if lit && ig[3] > 0.0 {
        let o = isz * 0.04 * ig[2];
        for (dx, dy) in [(-o, 0.0), (o, 0.0), (0.0, -o), (0.0, o)] {
            c.image(icon, [ip[0] + dx, ip[1] + dy], isz * (1.0 + 0.06 * ig[2]), alpha(color, ig[3].min(1.0) * 0.5 * it.a));
        }
    }
    c.image(icon, ip, isz, tint);
    // the count, live: `{name}_count` = [dx, dy, scale, opacity], inside the ring's transform
    let nt = Tf::el(&t, &format!("{name}_count"), cen[0] + 33.0, cen[1] - 16.0).within(e);
    let (nx, ny) = nt.pt(cen[0] + 33.0, cen[1] - 16.0);
    let np = c.dbl(nx, ny);
    c.digits(np[0], np[1], 15.0 * bs * nt.sc, alpha([0.95, 1.0, 0.85, 1.0], nt.a), &count.to_string(), false);
    let kt = t.hud.get("keys").copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    // this hexagon's own key offset, live: `{name}_key` = [dx, dy, ..]; a controller icon has its
    // own on top (`{name}_padkey`: the icons sat differently from the letters - user)
    let mut kb = t.hud.get(&format!("{name}_key")).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    if key.starts_with('@') {
        let kp = t.hud.get(&format!("{name}_padkey")).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
        kb[0] += kp[0];
        kb[1] += kp[1];
    }
    let (kx, ky) = e.pt(cen[0], cen[1] - 38.0);
    c.key_tag_k(c.dbl(kx + kt[0] + kb[0], ky + kt[1] + kb[1]), key, e.sc);
}

/// Health, armor, dash and Blood Punch from Doom's own HUD textures.
fn vitals_doom(c: &Ctx, st: &State, hurt: f32) {
    let t = tune();
    let s = c.s * t.hud_left[2].clamp(0.3, 3.0);
    let hp_low = st.hp * 4 < st.max_hp;
    let hc = if hp_low { mix(HEALTH, RED, 0.5 + 0.5 * (st.time * 6.0).sin()) } else { HEALTH };
    let hc = mix(hc, RED, hurt);
    let ring_c = [0.80, 0.93, 0.22, 1.0]; // Doom's dash / Blood Punch green-yellow
    // dark plate behind the two hexes (Blood Punch's half tinted when charged)
    let pl = Tf::el(&t, "hex_plate", 96.0, 860.0);
    let (px, py) = pl.pt(96.0, 860.0);
    c.slant(c.dbl(px, py), 150.0 * s * pl.sc, 54.0 * s * pl.sc, 26.0 * s * pl.sc, [0.0, 0.0, 0.0, 0.45 * pl.a], true);
    if st.blood_punch > 0 {
        let (bx, by) = pl.pt(172.0, 860.0);
        c.slant(c.dbl(bx, by), 76.0 * s * pl.sc, 54.0 * s * pl.sc, 26.0 * s * pl.sc, alpha([0.85, 0.55, 0.1, 1.0], 0.25 * pl.a), true);
    }
    // armor: visible box x 62..(end), y 925..963; health: x 55..(end), y 967..1007
    // extra digits of the health number (4-digit HP): both rows' cells move right by that many
    // segments, so the shield cells stay over the first health cells
    let more = st.max_hp.max(st.hp).max(0).to_string().len().saturating_sub(3);
    doom_row(
        c,
        "armor",
        [62.0 - 16.0 * DK, 925.2 - 18.0 * DK],
        ("dh_ammo_container_new", "dh_armor_container_center_shell", "dh_armor_container_right_shell"),
        ("dh_armor_container_left_fill", "dh_armor_container_center_fill", "dh_armor_container_right_fill"),
        6,
        ("dh_icon_armor", [98.4, 949.4], 68.0),
        ARMOR,
        st.armor,
        st.armor_max,
        190.0,
        943.9,
        214.0,
        more,
    );
    doom_row(
        c,
        "health",
        [55.0 - 12.0 * DK, 967.4 - 15.0 * DK],
        ("dh_health_container_leftside_shell", "dh_health_container_segment_shell", "dh_health_container_right_shell"),
        ("dh_health_container_leftside_fill", "dh_health_container_segment_fill", "dh_health_container_right_fill"),
        3,
        ("dh_icon_health_new", [91.2, 990.5], 56.0),
        hc,
        st.hp,
        st.max_hp,
        190.0,
        987.1,
        204.0,
        more,
    );
    // Doom's decor line under the health row (hud_slayer_health_decor_a): its diagonal runs up the
    // row's slanted left end, the bar sits just below it (stretched to our 8-pip row).
    if let Some(tx) = c.tex.get("dh_health_decor_a") {
        let d = Tf::el(&t, "decor", 66.0, 1014.0);
        let (sx, sy) = (0.668, 0.45);
        let (x0, y0) = (66.0, 1014.0);
        let (ax, ay) = d.pt(x0 - 25.0 * sx, y0 - 126.0 * sy);
        // (longer with a 4-digit health number, like the bar)
        let more = st.max_hp.max(st.hp).max(0).to_string().len().saturating_sub(3) as f32;
        let seg = c.tex.get("dh_health_container_segment_shell").map_or(61.0, |t| t.w) * DK;
        let (bx, by) = d.pt(x0 + (tx.w - 25.0) * sx + more * seg, y0 + (tx.h - 126.0) * sy);
        c.dl.add_image(tx.id, c.dbl(ax, ay), c.dbl(bx, by)).col(col(alpha(hc, d.a))).build();
    }
    // dash and Blood Punch
    let charges = st.dash_ready;
    doom_hex(c, "dash", [136.8, 886.8], ring_c, "icon_dash", charges, &key_label(st.keys[0]), charges > 0, false, st.dash);
    let bp_icon = if c.tex.contains_key("icon_rune_bloodpunch") { "icon_rune_bloodpunch" } else { "icon_punch_charged" };
    doom_hex(c, "blood_punch", [204.0, 886.8], ring_c, bp_icon, st.blood_punch, &key_label(crate::gamepad::shown(t.keys.melee, t.keys.melee_pad)), st.blood_punch > 0, true, 0.0);
}

/// Ammo (Doom's ammo container with its mod tab) and, over it, the chainsaw and flame belch boxes
/// (Doom's equipment backers) - placement from the user's Doom screenshot (grenades / Crucible
/// slots are not in the mod).
fn equipment_doom(c: &Ctx, st: &State) {
    let t = tune();
    let bs = c.s * t.hud_right[2].clamp(0.3, 3.0);
    let wd = &WEAPONS[st.weapon];
    let ammo_c = ammo_color(wd.ammo);
    let ammo = st.ammo[wd.ammo.index()];
    let low = ammo < wd.ammo_per_shot;
    let ac = if low { mix(ammo_c, RED, 0.5 + 0.5 * (st.time * 6.0).sin()) } else { ammo_c };
    let mod_icon = MOD_ICONS[st.weapon.min(7)];
    let has_mod = c.tex.contains_key(mod_icon);
    // container: visible x 1509.6..1766, y 969.6..1008 (0.404x)
    let cx0 = 1501.6 - 20.0 * 0.404;
    let cy0 = 969.6 - 20.0 * 0.404;
    let row = Tf::el(&t, "ammo_row", cx0, cy0);
    let k = 0.404 * bs * row.sc;
    let cont = if has_mod { "dh_ammo_container_new" } else { "dh_ammo_container_new_nomod" };
    let (ax, ay) = row.pt(cx0, cy0);
    let cy = 988.8;
    if st.crucible_out {
        // The Crucible out (user, like Doom's BFG): the row turns red - its shard where the ammo
        // icon sits (no charge bar: the box and the reticle show them).
        let red: Rgba = t.hud.get("crucible_color").copied().unwrap_or([1.0, 0.45, 0.38, 1.0]);
        c.piece("dh_ammo_container_new_nomod", c.dbr(ax, ay), k, alpha(mix(red, [1.0, 1.0, 1.0, 1.0], 0.2), row.a));
        let at = Tf::el(&t, "ammo_icon", 1677.6, cy).within(row);
        let (ix, iy) = at.pt(1677.6, cy);
        c.image("st_ico_crucible", c.dbr(ix, iy), 34.0 * bs * at.sc, alpha([1.0, 0.85, 0.82, 1.0], at.a));
        // (no charge segments in the ammo row - user: the box and the reticle show them)
    } else {
    c.piece(cont, c.dbr(ax, ay), k, alpha(mix(ac, [1.0, 1.0, 1.0, 1.0], 0.2), row.a));
    let nt = Tf::el(&t, "ammo_number", 1646.0, cy).within(row);
    let nh = 25.0 * bs * nt.sc;
    let w = nh / 74.0 * 56.0 * 3.0;
    let (nx, ny) = nt.pt(1646.0, cy);
    let np = c.dbr(nx, ny);
    c.display(np[0] - w, np[1], nh, alpha(ac, nt.a), &ammo.to_string(), 3);
    let at = Tf::el(&t, "ammo_icon", 1677.6, cy).within(row);
    let (ix, iy) = at.pt(1677.6, cy);
    c.icon_ammo(wd.ammo, c.dbr(ix, iy), 11.0 * bs * at.sc, alpha(ac, at.a));
    if has_mod {
        let mt = Tf::el(&t, "mod_icon", 1730.4, cy).within(row);
        let (mx, my) = mt.pt(1730.4, cy);
        c.image(mod_icon, c.dbr(mx, my), 34.0 * bs * mt.sc, alpha(ac, 0.95 * mt.a));
    }
    }

    // box colours (Doom: fuel orange, flame belch yellow), live: [hud] chainsaw_color / flame_color = [r, g, b, a]
    let box_color = |k: &str, d: Rgba| t.hud.get(k).copied().unwrap_or(d);
    // chainsaw then flame belch in the first two of Doom's equipment slots
    let boxes = [
        ("chainsaw_box", "dh_icon_ammo_fuel", box_color("chainsaw_color", [1.0, 0.5, 0.15, 1.0]), (st.fuel / crate::slayer::CHAINSAW_MAX).clamp(0.0, 1.0), None::<i32>, st.keys[2], [1591.0, 939.0], 30.0),
        ("flame_box", "icon_ability_flame_belch", box_color("flame_color", [1.0, 0.82, 0.2, 1.0]), 1.0 - (st.belch_cd / st.belch_max.max(0.01)).clamp(0.0, 1.0), None, st.keys[1], [1646.0, 939.0], 36.0),
        // the Crucible, third in the row (user): lit with a charge, its count beside it
        ("crucible_box", "st_icon_crucible", box_color("crucible_color", [1.0, 0.45, 0.38, 1.0]), if st.crucible_charges > 0 { 1.0 } else { 0.0 }, None, st.keys[4], [1701.0, 939.0], 45.0),
    ];
    let kt = t.hud.get("keys").copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
    for (name, icon, color, ready, count, key, cen, isz) in boxes.iter() {
        let e = Tf::el(&t, name, cen[0], cen[1]);
        let s = bs * e.sc;
        let bk = 0.36 * s; // Doom's boxes: backer 131 px visible -> 47 px
        let (bx, by) = e.pt(cen[0], cen[1]);
        let bc = c.dbr(bx, by);
        let full = *ready >= 1.0;
        let (bw, bh) = (259.0 * bk, 189.0 * bk);
        let p = [bc[0] - bw * 0.5, bc[1] - bh * 0.5];
        if *name == "crucible_box" {
            // Doom's Crucible slot (user's Doom capture): the glass, the open left frame
            // (3pip_leftline) and three bars down the right edge - one per charge, lit from the
            // bottom up (bars at art rows 12-42 / 50-81 / 89-119), the sword icon in the middle.
            let charges = st.crucible_charges.min(3);
            c.piece("dh_equipment_backer_new", p, bk, alpha(*color, if charges > 0 { 0.75 } else { 0.4 } * e.a));
            let fp = [bc[0] - 211.0 * bk * 0.5, bc[1] - 133.0 * bk * 0.5];
            c.piece("dh_equipment_fill_3pip_leftline", fp, bk, alpha(*color, if charges > 0 { 0.95 } else { 0.5 } * e.a));
            c.piece("dh_equipment_fill_3pip_backer", fp, bk, alpha(*color, 0.35 * e.a));
            if let Some(tx) = c.tex.get("dh_equipment_fill_3pip_fill") {
                // they fill from the bottom up (user)
                let bands = [(85.0f32, 123.0f32), (46.0, 85.0), (8.0, 46.0)];
                for (i, (y0, y1)) in bands.iter().enumerate() {
                    if (i as u32) < charges {
                        c.dl.add_image(tx.id, [fp[0], fp[1] + y0 * bk], [fp[0] + tx.w * bk, fp[1] + y1 * bk])
                            .uv_min([0.0, y0 / tx.h]).uv_max([1.0, y1 / tx.h])
                            .col(col(alpha(*color, 0.95 * e.a))).build();
                    }
                }
            }
            // live: [hud] crucible_box_icon = [dx, dy, size scale, opacity]
            let iv = t.hud.get(&format!("{name}_icon")).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
            let ia = iv[3].clamp(0.0, 1.0) * e.a;
            // Doom's Crucible icon tilted 45 deg right, along the box's slant (user; live: [hud]
            // crucible_box_icon_rot = [degrees, -, -, -], positive = left), just left of centre
            // (the bars take the right edge)
            let rot = t.hud.get("crucible_box_icon_rot").map_or(-45.0, |v| v[0]).to_radians();
            if let Some(tx) = c.tex.get(*icon) {
                let sz = *isz * iv[2].max(0.05) * s;
                let k = sz / tx.w.max(tx.h);
                let (hw, hh) = (tx.w * k * 0.5, tx.h * k * 0.5);
                let ctr = [bc[0] + (iv[0] - 2.0) * s, bc[1] + iv[1] * s];
                // positive = tilt left (counter-clockwise on screen)
                let (sn, cs) = (-rot).sin_cos();
                let r = |x: f32, y: f32| [ctr[0] + x * cs - y * sn, ctr[1] + x * sn + y * cs];
                // red like the box (user), dimmer with no charges
                let tint = alpha(*color, if charges > 0 { 1.0 } else { 0.5 } * ia);
                c.dl.add_image_quad(tx.id, r(-hw, -hh), r(hw, -hh), r(hw, hh), r(-hw, hh)).col(col(tint)).build();
            }
            let (kx, ky) = e.pt(cen[0] + 8.0, cen[1] - 41.0);
            let kb = t.hud.get(&format!("{name}_key")).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
            c.key_tag_k(c.dbr(kx + kt[0] + kb[0], ky + kt[1] + kb[1]), &key_label(*key), e.sc);
            continue;
        }
        // glass: Doom's backer, flooded with the colour when ready
        c.piece("dh_equipment_backer_new", p, bk, alpha(*color, if full { 0.9 } else { 0.45 } * e.a));
        if full {
            // the ready glow (flash + lit square), live: `{name}_glow` opacity
            let ga = t.hud.get(&format!("{name}_glow")).map(|v| v[3]).unwrap_or(1.0) * e.a;
            c.piece("dh_equipment_backer_flash", p, bk, alpha(*color, 0.35 * ga));
            c.piece("dh_equipment_fill_1pip_fill", [bc[0] - 223.0 * bk * 0.5, bc[1] - 144.0 * bk * 0.5], bk, alpha(*color, 0.95 * ga));
        } else {
            c.piece("dh_equipment_fill_1pip_backer", [bc[0] - 211.0 * bk * 0.5, bc[1] - 133.0 * bk * 0.5], bk, alpha(*color, 0.4 * e.a));
            // recharge fill rising inside the box: Doom's slanted fill (art rows 9..135), cut at the level
            if let Some(tx) = c.tex.get("dh_equipment_fill_1pip_fill") {
                let f = ready.clamp(0.0, 1.0);
                let v0 = (9.0 + 126.0 * (1.0 - f)) / tx.h;
                let p0 = [bc[0] - tx.w * bk * 0.5, bc[1] - tx.h * bk * 0.5];
                c.dl.add_image(tx.id, [p0[0], p0[1] + v0 * tx.h * bk], [p0[0] + tx.w * bk, p0[1] + tx.h * bk]).uv_min([0.0, v0]).uv_max([1.0, 1.0]).col(col(alpha(*color, 0.35 * e.a))).build();
            }
        }
        // icon opacity, live: `{name}_icon` = [.., .., .., opacity]
        let ia = t.hud.get(&format!("{name}_icon")).map_or(1.0, |v| v[3].clamp(0.0, 1.0)) * e.a;
        c.image(icon, bc, *isz * s, if full { alpha([1.0, 0.92, 0.85, 1.0], ia) } else { alpha(*color, 0.6 * ia) });
        let (kx, ky) = e.pt(cen[0] + 8.0, cen[1] - 41.0);
        // this box's own key offset, live: `{name}_key` = [dx, dy, ..]
        let kb = t.hud.get(&format!("{name}_key")).copied().unwrap_or([0.0, 0.0, 1.0, 1.0]);
        c.key_tag_k(c.dbr(kx + kt[0] + kb[0], ky + kt[1] + kb[1]), &key_label(*key), e.sc);
        if let Some(n) = count {
            c.digits(bc[0] + 36.0 * s, bc[1] + 12.0 * s, 13.0 * s, alpha(*color, e.a), &n.to_string(), false);
        }
    }
}

// ---------------------------------------------------------------------- bottom-left

pub fn vitals(c: &Ctx, st: &State, hurt: f32) {
    let s = c.s;

    if crate::config::get_cached().hud_style == 1 {
        vitals_doom(c, st, hurt);
        return;
    } else {
        // Health row (reference: y 965..1010, x 62..330): glass plate, tinted icon tab, segment display.
        let hp_low = st.hp * 4 < st.max_hp;
        let hc = if hp_low { mix(HEALTH, RED, 0.5 + 0.5 * (st.time * 6.0).sin()) } else { HEALTH };
        let hc = mix(hc, RED, hurt);
        let p = c.bl(62.0, 966.0);
        c.slant(p, 300.0 * s, 44.0 * s, 10.0 * s, alpha(hc, 0.10), true);
        c.slant(p, 300.0 * s, 44.0 * s, 10.0 * s, alpha(hc, 0.3), false);
        c.slant(p, 58.0 * s, 44.0 * s, 10.0 * s, alpha(hc, 0.22), true);
        c.slant(p, 58.0 * s, 44.0 * s, 10.0 * s, alpha(hc, 0.6), false);
        let hex = c.bl(94.0, 988.0);
        if c.tex.contains_key("hud_health") {
            // Doom's health icon (argent_cell_health: the cross in a hex)
            c.image("hud_health", hex, 40.0 * s, hc);
        } else {
            c.hexagon(hex, 17.0 * s, hc, false, 2.5 * s);
            c.icon_cross(hex, 8.0 * s, hc);
        }
        // Between the icon tab (ends ~130 with its slant) and the gauge (205).
        let tp = c.bl(134.0, 988.0);
        c.display(tp[0], tp[1], 27.0 * s, hc, &st.hp.max(0).to_string(), 3);
        c.gauge(c.bl(205.0, 974.0), 6, st.hp as f32 / st.max_hp.max(1) as f32, 20.0 * s, 28.0 * s, 5.0 * s, hc);

        // Armor row (y 922..962).
        let ac = ARMOR;
        let p = c.bl(80.0, 922.0);
        c.slant(p, 282.0 * s, 38.0 * s, 9.0 * s, alpha(ac, 0.08), true);
        c.slant(p, 282.0 * s, 38.0 * s, 9.0 * s, alpha(ac, 0.28), false);
        c.slant(p, 52.0 * s, 38.0 * s, 9.0 * s, alpha(ac, 0.3), true);
        c.slant(p, 52.0 * s, 38.0 * s, 9.0 * s, alpha(ac, 0.6), false);
        if c.tex.contains_key("hud_armor") {
            // Doom's armor icon (argent_cell_armor: the shield with the Slayer's helmet)
            c.image("hud_armor", c.bl(108.0, 941.0), 32.0 * s, ac);
        } else {
            c.icon_shield(c.bl(110.0, 941.0), 12.0 * s, ac);
        }
        let tp = c.bl(146.0, 941.0);
        c.display(tp[0], tp[1], 24.0 * s, ac, &st.armor.max(0).to_string(), 3);
        // Armor and health gauges: six equal segments each.
        c.gauge(c.bl(205.0, 929.0), 6, st.armor as f32 / st.armor_max.max(1) as f32, 20.0 * s, 24.0 * s, 5.0 * s, ac);

    }

    // Ability hexes (y 860..915): dash charges, blood punch. Doom: a double hex rim, see-through
    // inside, with faint diagonal hatching on the plate to the left.
    for k in 0..4 {
        let x = c.bl(70.0 + k as f32 * 7.0, 0.0)[0];
        let (y0, y1) = (c.bl(0.0, 900.0)[1], c.bl(0.0, 866.0)[1]);
        c.dl.add_line([x, y0], [x + 18.0 * s, y1], col(alpha(RIM, 0.8))).thickness(1.5 * s).build();
    }
    let dash_c = c.bl(136.0, 884.0);
    let charges = st.dash_ready as i32;
    let dc = if charges > 0 { YELLOW } else { alpha(YELLOW, 0.4) };
    c.hexagon(dash_c, 24.0 * s, alpha(dc, 0.08), true, 0.0);
    c.bracket_hex(dash_c, 25.0 * s, dc, 4.5 * s);
    c.image("icon_dash", dash_c, 30.0 * s, dc);
    // recharge sweep
    let partial = st.dash.fract();
    if st.dash < 2.0 && partial > 0.0 {
        let n = 24;
        let pts: Vec<[f32; 2]> = (0..=n)
            .map(|i| {
                let a = -PI / 2.0 + partial * 2.0 * PI * i as f32 / n as f32;
                [dash_c[0] + a.cos() * 31.0 * s, dash_c[1] + a.sin() * 31.0 * s]
            })
            .collect();
        c.dl.add_polyline(pts, col(alpha(YELLOW, 0.7))).thickness(2.5 * s).build();
    }
    let np = c.bl(166.0, 862.0);
    c.digits(np[0], np[1], 16.0 * s, YELLOW, &charges.to_string(), false);
    c.key_tag(c.bl(136.0, 852.0), &key_label(st.keys[0]));
    // Blood Punch: Doom's fist in a double hex - dim grey while empty, lit and filled when charged.
    let bp_c = c.bl(204.0, 884.0);
    let charged = st.blood_punch > 0;
    let grey = [0.62, 0.62, 0.56, 0.75];
    let bpc = if charged { mix(YELLOW, [1.0, 0.95, 0.6, 1.0], 0.5 + 0.5 * (st.time * 5.0).sin()) } else { grey };
    // Doom's punch runes: the base rune while empty, the max-punch rune when charged, both drawn
    // in the HUD's own colours.
    if charged {
        c.hexagon(bp_c, 24.0 * s, alpha(bpc, 0.18), true, 0.0);
        c.image("icon_punch_charged", bp_c, 42.0 * s, bpc);
    } else {
        c.hexagon(bp_c, 24.0 * s, alpha(grey, 0.08), true, 0.0);
        c.image("icon_punch", bp_c, 40.0 * s, grey);
    }
    c.hexagon(bp_c, 26.0 * s, alpha(bpc, 0.45), false, 1.5 * s);
    c.hexagon(bp_c, 21.0 * s, bpc, false, 2.5 * s);
    let bp = c.bl(234.0, 862.0);
    c.digits(bp[0], bp[1], 16.0 * s, if charged { YELLOW } else { [0.85, 0.85, 0.75, 0.9] }, &st.blood_punch.to_string(), false);
}

// ---------------------------------------------------------------------- bottom-right

pub fn equipment(c: &Ctx, st: &State) {
    if crate::config::get_cached().hud_style == 1 {
        equipment_doom(c, st);
        return;
    }
    let s = c.s;
    let wd = &WEAPONS[st.weapon];
    let ammo_c = ammo_color(wd.ammo);

    // Equipment boxes (reference x 1655..1835, y 925..970): flame belch, chainsaw, current mod/ammo.
    let boxes = [
        // Doom's chainsaw slot shows its fuel can (ico_fuel).
        (if c.tex.contains_key("ammo_fuel") { "ammo_fuel" } else { "icon_ability_chainsaw" }, RED, (st.fuel / crate::slayer::CHAINSAW_MAX).clamp(0.0, 1.0), Some(st.fuel.floor() as i32)),
        ("icon_ability_flame_belch", YELLOW, 1.0 - (st.belch_cd / st.belch_max.max(0.01)).clamp(0.0, 1.0), None),
    ];
    // (the weapon mod sits next to the ammo, like Doom; the two abilities sit over its right end)
    let tags = [st.keys[2], st.keys[1]];
    for (i, (icon, color, ready, count)) in boxes.iter().enumerate() {
        let p = c.br(1746.0 + i as f32 * 58.0, 922.0);
        let full = *ready >= 1.0;
        let fill = if full { 0.26 } else { 0.08 };
        c.slant(p, 46.0 * s, 46.0 * s, 12.0 * s, alpha(*color, fill), true);
        // cooldown fill rising from the bottom
        if !full {
            let fh = 46.0 * s * ready;
            c.dl.add_rect([p[0], p[1] + 46.0 * s - fh], [p[0] + 46.0 * s + 6.0 * s, p[1] + 46.0 * s], col(alpha(*color, 0.25))).filled(true).build();
        }
        // Doom's frames glow when the ability is ready.
        if full {
            c.dl.add_polyline(
                vec![[p[0] + 12.0 * s, p[1]], [p[0] + 58.0 * s, p[1]], [p[0] + 46.0 * s, p[1] + 46.0 * s], [p[0], p[1] + 46.0 * s], [p[0] + 12.0 * s, p[1]]],
                col(alpha(*color, 0.25)),
            )
            .thickness(6.0 * s)
            .build();
        }
        c.slant(p, 46.0 * s, 46.0 * s, 12.0 * s, alpha(*color, if full { 1.0 } else { 0.45 }), false);
        let ic = [p[0] + 29.0 * s, p[1] + 23.0 * s];
        c.image(icon, ic, 36.0 * s, alpha(*color, if full { 1.0 } else { 0.55 }));
        c.key_tag([p[0] + 29.0 * s, p[1] - 5.0 * s], &key_label(tags[i]));
        if let Some(n) = count {
            c.digits(p[0] + 34.0 * s, p[1] + 36.0 * s, 14.0 * s, *color, &n.to_string(), false);
        }
    }

    // Ammo row (reference: "17" at x 1660, y 988 with shell icon to its right): a faint glass strip
    // between two thin lines in the ammo colour that fade out to the left.
    let ammo = st.ammo[wd.ammo.index()];
    let low = ammo < wd.ammo_per_shot;
    let ac = if low { mix(ammo_c, RED, 0.5 + 0.5 * (st.time * 6.0).sin()) } else { ammo_c };
    let p = c.br(1600.0, 968.0);
    let has_mod = c.tex.contains_key(MOD_ICONS[st.weapon.min(7)]);
    let plate_w = if has_mod { 262.0 } else { 190.0 };
    c.slant(p, plate_w * s, 42.0 * s, 10.0 * s, GLASS, true);
    c.slant(p, plate_w * s, 42.0 * s, 10.0 * s, alpha(ac, 0.3), false);
    for y in [968.0, 1010.0] {
        let (l, r) = (c.br(1560.0, y), c.br(1792.0, y));
        c.dl.add_rect_filled_multicolor(l, [r[0], r[1] + 1.5 * s], col(alpha(ac, 0.0)), col(alpha(ac, 0.6)), col(alpha(ac, 0.6)), col(alpha(ac, 0.0)));
    }
    let tp = c.br(1650.0, 989.0);
    c.display(tp[0], tp[1], 30.0 * s, ac, &ammo.to_string(), 3);
    c.icon_ammo(wd.ammo, c.br(1740.0, 989.0), 13.0 * s, ac);
    // The weapon's mod (right-click ability) in its own angled box right of the ammo (Doom).
    let mod_icon = MOD_ICONS[st.weapon.min(7)];
    if has_mod {
        c.slant(c.br(1798.0, 968.0), 64.0 * s, 42.0 * s, 10.0 * s, alpha(ammo_c, 0.22), true);
        c.slant(c.br(1798.0, 968.0), 64.0 * s, 42.0 * s, 10.0 * s, alpha(ammo_c, 0.6), false);
        c.image(mod_icon, c.br(1833.0, 989.0), 34.0 * s, alpha(ammo_c, 0.95));
    }

}

// ---------------------------------------------------------------------- reticle

/// Arc from angle a0 sweeping `frac` of a full turn (clockwise from the top).
fn arc(c: &Ctx, cen: [f32; 2], r: f32, frac: f32, color: Rgba, thick: f32) {
    let n = (48.0 * frac.clamp(0.0, 1.0)).ceil().max(1.0) as usize;
    let pts: Vec<[f32; 2]> = (0..=n)
        .map(|i| {
            let a = -PI / 2.0 + frac.clamp(0.0, 1.0) * 2.0 * PI * i as f32 / n as f32;
            [cen[0] + a.cos() * r, cen[1] + a.sin() * r]
        })
        .collect();
    c.dl.add_polyline(pts, col(color)).thickness(thick).build();
}

/// One of Doom's reticle pieces (tools/convert_reticles.py: white on transparency): the texture
/// point `anchor` (texture pixels) lands on `at`, scaled by `k` screen px per texture px, turned
/// by `angle` (radians, clockwise) and optionally mirrored.
fn piece(c: &Ctx, name: &str, at: [f32; 2], anchor: [f32; 2], k: f32, angle: f32, flip: (bool, bool), tint: Rgba) {
    piece_part(c, name, at, anchor, k, angle, flip, tint, [0.0, 0.0, 1.0, 1.0]);
}

/// Part of a piece: `uv` = [u0, v0, u1, v1] of the texture (progress bars fill this way).
#[allow(clippy::too_many_arguments)]
fn piece_part(c: &Ctx, name: &str, at: [f32; 2], anchor: [f32; 2], k: f32, angle: f32, flip: (bool, bool), tint: Rgba, uv: [f32; 4]) {
    let Some(t) = c.tex.get(name) else { return };
    if uv[2] <= uv[0] || uv[3] <= uv[1] {
        return;
    }
    let (fx, fy) = (if flip.0 { -1.0 } else { 1.0 }, if flip.1 { -1.0 } else { 1.0 });
    let (sn, cs) = angle.sin_cos();
    let p = |u: f32, v: f32| {
        let (dx, dy) = ((u * t.w - anchor[0]) * fx * k, (v * t.h - anchor[1]) * fy * k);
        [at[0] + dx * cs - dy * sn, at[1] + dx * sn + dy * cs]
    };
    let [u0, v0, u1, v1] = uv;
    c.dl.add_image_quad(t.id, p(u0, v0), p(u1, v0), p(u1, v1), p(u0, v1))
        .uv([u0, v0], [u1, v0], [u1, v1], [u0, v1])
        .col(col(tint))
        .build();
}

/// Doom's reticle green.
const RET: Rgba = [0.82, 0.95, 0.32, 0.95];
const RET_DIM: Rgba = [0.45, 0.48, 0.42, 0.6];
/// Doom's reticle pieces are drawn at 0.75 of their texture size at 1080p (matched to footage).
const RET_K: f32 = 0.75;

/// Weapon-mod overlays: Precision Bolt scope (Doom's own scope textures).
pub fn mod_overlay(c: &Ctx, st: &State) {
    if st.weapon == 1 && st.zoom > 0.05 {
        if c.tex.contains_key("ret_heavycannon_scope_larger_ring_top") {
            hc_scope(c, st);
        } else {
            hc_scope_classic(c, st);
        }
    }
}

/// Heavy Cannon Precision Bolt scope from Doom's own pieces (layout matched to a Doom capture):
/// dark edges, green hex glass at the sides, two rings (top/bottom halves), side wings and
/// corner decos; the inner ring's reload glow comes back as the bolt recharges.
fn hc_scope(c: &Ctx, st: &State) {
    let s = c.s;
    let cen = [c.w / 2.0, c.h / 2.0];
    let a = st.zoom;
    let deco = [0.78, 0.92, 0.25, 0.9 * a];
    let ring = [1.0, 1.0, 0.85, 0.95 * a];
    for flip in [false, true] {
        // Dark vignette and green hex glass, left half (mirrored for the right).
        // The halves meet exactly at the centre (clamped sampling: no seam).
        let half = c.w * 0.5;
        if let Some(t) = c.tex.get("ret_heavycannon_scope_black") {
            let k = c.h / t.h;
            piece(c, "ret_heavycannon_scope_black", [cen[0] - half * if flip { -1.0 } else { 1.0 }, 0.0], [0.0, 0.0], k, 0.0, (flip, false), [1.0, 1.0, 1.0, 0.85 * a]);
        }
        if let Some(t) = c.tex.get("ret_heavycannon_scope_green_hex") {
            let k = c.h / t.h;
            piece(c, "ret_heavycannon_scope_green_hex", [cen[0] - half * if flip { -1.0 } else { 1.0 }, 0.0], [0.0, 0.0], k, 0.0, (flip, false), [0.65, 0.85, 0.1, a]);
        }
    }
    // Rings: each texture is the top arc of its circle (fitted centres); mirrored for the bottom.
    // Inner ring (top and bottom halves): after a bolt it turns orange and the orange bar unloads
    // from left to right as the bolt recharges; when ready, a quick green flash (Doom).
    let since = st.time - st.bolt_at;
    let rec = crate::slayer::BOLT_RECOVERY;
    for fy in [false, true] {
        piece(c, "ret_heavycannon_scope_larger_ring_glow", cen, [511.5, 459.9], 1.22 * s, 0.0, (false, fy), [1.0, 1.0, 0.7, 0.75 * a]);
        piece(c, "ret_heavycannon_scope_larger_ring_top", cen, [511.5, 459.9], 1.22 * s, 0.0, (false, fy), ring);
        if since < rec {
            let orange = [1.0, 0.5, 0.12, 0.95 * a];
            piece(c, "ret_heavycannon_scope_smaller_ring_top", cen, [511.5, 526.6], 0.76 * s, 0.0, (false, fy), alpha(orange, 0.4 * a));
            // the arc spans u 0.07..0.93 of the texture: empties fully, from the very start
            let u0 = 0.07 + 0.86 * (since / rec);
            piece_part(c, "ret_heavycannon_scope_smaller_ring_reload_glow", cen, [511.5, 526.6], 0.76 * s, 0.0, (false, fy), alpha(orange, 0.5 * a), [u0, 0.0, 1.0, 1.0]);
            piece_part(c, "ret_heavycannon_scope_smaller_ring_top", cen, [511.5, 526.6], 0.76 * s, 0.0, (false, fy), orange, [u0, 0.0, 1.0, 1.0]);
        } else if since < rec + 0.35 {
            let f = 1.0 - (since - rec) / 0.35;
            piece(c, "ret_heavycannon_scope_smaller_ring_reload_glow", cen, [511.5, 526.6], 0.76 * s, 0.0, (false, fy), [0.6, 1.0, 0.3, 0.8 * a * f]);
            piece(c, "ret_heavycannon_scope_smaller_ring_top", cen, [511.5, 526.6], 0.76 * s, 0.0, (false, fy), mix([0.95, 0.95, 0.9, 0.7 * a], [0.6, 1.0, 0.3, a], f));
        } else {
            piece(c, "ret_heavycannon_scope_smaller_ring_top", cen, [511.5, 526.6], 0.76 * s, 0.0, (false, fy), [0.95, 0.95, 0.9, 0.7 * a]);
        }
    }
    // Wings (left top / bottom, mirrored right) and corner decos.
    for (fx, fy) in [(false, false), (false, true), (true, false), (true, true)] {
        let (sx, sy) = (if fx { -1.0 } else { 1.0 }, if fy { -1.0 } else { 1.0 });
        piece(c, "ret_heavycannon_scope_deco_02_top", [cen[0] - 802.0 * s * sx, cen[1] - 367.0 * s * sy], [0.0, 0.0], 0.45 * s, 0.0, (fx, fy), deco);
        piece(c, "ret_heavycannon_scope_deco_01_top", [cen[0] - 835.0 * s * sx, cen[1] - 540.0 * s * sy], [0.0, 0.0], 0.66 * s, 0.0, (fx, fy), [0.9, 0.95, 0.85, 0.7 * a]);
    }
    // Ammo on the right wing (Doom: bullets icon + count), the rest of the HUD is hidden.
    let wd = &WEAPONS[1];
    let ammo = st.ammo[wd.ammo.index()];
    let ac = alpha(ammo_color(wd.ammo), a);
    // just above the lower-right wing's bar (Doom capture)
    c.icon_ammo(wd.ammo, [cen[0] + 535.0 * s, cen[1] + 130.0 * s], 15.0 * s, ac);
    c.display(cen[0] + 560.0 * s, cen[1] + 130.0 * s, 34.0 * s, ac, &ammo.to_string(), 3);
}

/// The scope used before Doom's textures (kept, user): dark ring, cross and ticks.
#[allow(dead_code)]
fn hc_scope_classic(c: &Ctx, st: &State) {
    let s = c.s;
    let cen = [c.w / 2.0, c.h / 2.0];
    let a = st.zoom;
    let r0 = c.h * (0.62 - 0.2 * a);
    let far = (c.w * c.w + c.h * c.h).sqrt();
    let t = far - r0;
    c.dl.add_circle(cen, r0 + t * 0.5, col([0.0, 0.0, 0.0, 0.92 * a])).thickness(t).num_segments(96).build();
    c.dl.add_circle(cen, r0, col([0.75, 0.85, 0.3, 0.5 * a])).thickness(2.0 * s).num_segments(96).build();
    let l = r0 * 0.9;
    let lc = col([0.85, 0.95, 0.35, 0.55 * a]);
    c.dl.add_line([cen[0] - l, cen[1]], [cen[0] - 14.0 * s, cen[1]], lc).thickness(1.5 * s).build();
    c.dl.add_line([cen[0] + 14.0 * s, cen[1]], [cen[0] + l, cen[1]], lc).thickness(1.5 * s).build();
    c.dl.add_line([cen[0], cen[1] + 14.0 * s], [cen[0], cen[1] + l], lc).thickness(1.5 * s).build();
    for i in 1..5 {
        let y = cen[1] + i as f32 * r0 * 0.15;
        c.dl.add_line([cen[0] - 8.0 * s, y], [cen[0] + 8.0 * s, y], lc).thickness(1.5 * s).build();
    }
    c.dl.add_circle(cen, 3.0 * s, col([0.95, 0.3, 0.2, a])).filled(true).build();
    let k = ((st.time - st.bolt_at) / crate::slayer::BOLT_RECOVERY).clamp(0.0, 1.0);
    arc(c, cen, 30.0 * s, k, [0.85, 0.95, 0.35, 0.7 * a], 3.0 * s);
}

static HOOK_ICON: std::sync::Mutex<Option<([f32; 2], f32)>> = std::sync::Mutex::new(None);
/// Plasma reticle: (box slide 0..1, last time, last heat, Heat Blast time).
static PLASMA: std::sync::Mutex<Option<([f32; 3], f32, f32, f32)>> = std::sync::Mutex::new(None);

/// Super Shotgun bracket radius at 1080p (Doom footage: 53 px).
pub const SSG_BRACKET_R: f32 = 53.0;

pub fn reticle(c: &Ctx, st: &State) {
    let s = c.s;
    let cen = [c.w / 2.0, c.h / 2.0];
    let k = RET_K * s;
    let kick = (-(st.time - st.last_shot_at) * 10.0).exp();
    match st.weapon {
        4 => {
            // Super Shotgun: Doom's bracket "(" twice, mirrored, spread to a 53 px radius, and the
            // meathook icon (Doom's claw, mirrored) under them; it points at a demon inside the
            // brackets that can be hooked, otherwise sits at home.
            let spread = (SSG_BRACKET_R - 49.5 * RET_K + 4.0 * kick) * s;
            piece(c, "ret_supershotgun_base_top", [cen[0] - spread, cen[1]], [84.5, 78.5], k, 0.0, (false, false), RET);
            piece(c, "ret_supershotgun_base_top", [cen[0] + spread, cen[1]], [84.5, 78.5], k, 0.0, (true, false), RET);
            let home = [cen[0], cen[1] + 60.0 * s];
            let goal = st.hook_icon.unwrap_or(home);
            // Dim while the hook recharges, a quick fade back up when it is ready.
            let icon_a = if st.hook_ready_for < 0.0 { 0.3 } else { 0.3 + 0.7 * (st.hook_ready_for / 0.15).min(1.0) };
            let at = {
                let mut g = HOOK_ICON.lock().unwrap_or_else(|e| e.into_inner());
                let (mut p, t_prev) = g.unwrap_or((home, st.time));
                let dt = (st.time - t_prev).clamp(0.0, 0.1);
                let rate = if st.hook_icon.is_some() { 22.0 } else { 9.0 };
                let kk = 1.0 - (-dt * rate).exp();
                p = [p[0] + (goal[0] - p[0]) * kk, p[1] + (goal[1] - p[1]) * kk];
                *g = Some((p, st.time));
                p
            };
            // Claw texture: content x 26..40, y 16..44; 17 px tall like Doom's.
            let kh = 0.59 * s;
            piece(c, "ret_supershotgun_hook_top", [at[0] - 0.5 * s, at[1]], [40.5, 30.0], kh, 0.0, (false, false), alpha(RET, RET[3] * icon_a));
            piece(c, "ret_supershotgun_hook_top", [at[0] + 0.5 * s, at[1]], [40.5, 30.0], kh, 0.0, (true, false), alpha(RET, RET[3] * icon_a));
        }
        5 => {
            // Ballista: two small brackets; holding right click closes them in until they form
            // Doom's small circle (12 px) - the shot is ready.
            let t = st.arb.clamp(0.0, 1.0);
            let e = t * t * (3.0 - 2.0 * t);
            // open: arcs at 20 px; closed: the arcs sit on the 12 px circle (bracket R 18.4 px).
            let spread = ((2.0 * (1.0 - e) - 6.4 * e) + 3.0 * kick * (1.0 - e)) * s;
            let half_a = if t >= 1.0 { 0.0 } else { 1.0 - ((t - 0.85) / 0.15).clamp(0.0, 1.0) };
            if half_a > 0.0 {
                piece(c, "ret_ballista_base_top", [cen[0] - spread, cen[1]], [25.0, 22.0], k, 0.0, (false, false), alpha(RET, RET[3] * half_a));
                piece(c, "ret_ballista_base_top", [cen[0] + spread, cen[1]], [25.0, 22.0], k, 0.0, (true, false), alpha(RET, RET[3] * half_a));
            }
            if t > 0.0 {
                // Side bars (Doom's arc progress bars) fill bottom to top while the Arbalest charges.
                for fx in [false, true] {
                    let at = [cen[0] + if fx { 220.0 * s } else { -220.0 * s }, cen[1]];
                    // texture "(" 27x227; anchor at its inner middle
                    piece(c, "ret_ballista_scope_progbar_top", at, [4.0, 113.5], 0.8 * s, 0.0, (fx, false), [0.55, 0.6, 0.55, 0.6]);
                    piece_part(c, "ret_ballista_scope_progbar_glow", at, [12.5, 122.5], 0.8 * s, 0.0, (fx, false), alpha(RET, 0.5), [0.0, 1.0 - t, 1.0, 1.0]);
                    piece_part(c, "ret_ballista_scope_progbar_top", at, [4.0, 113.5], 0.8 * s, 0.0, (fx, false), RET, [0.0, 1.0 - t, 1.0, 1.0]);
                }
            }
            if t > 0.85 {
                let ca = ((t - 0.85) / 0.15).clamp(0.0, 1.0);
                piece(c, "ret_ballista_base_circle_top", cen, [49.5, 50.0], 0.62 * s, 0.0, (false, false), alpha([0.95, 1.0, 0.5, 1.0], ca));
            }
        }
        6 => {
            // Chaingun: Doom's circle.
            piece(c, "ret_chaingun_base_top", cen, [59.5, 59.5], k * (1.0 + 0.06 * kick), 0.0, (false, false), RET);
        }
        3 => {
            // Rocket Launcher: Doom's chevron x4 in a diamond, pointing out.
            let d = (19.0 + 4.0 * kick) * s;
            for i in 0..4 {
                let a = i as f32 * PI / 2.0;
                let (dx, dy) = ((a - PI / 2.0).cos(), (a - PI / 2.0).sin());
                piece(c, "ret_rocket_base_top", [cen[0] + dx * d, cen[1] + dy * d], [19.5, 15.0], k, a, (false, false), RET);
            }
        }
        2 => {
            // Plasma Rifle: Doom's four corners and three heat boxes that fill as the gun heats.
            let (hw, hh) = (25.0 * s, 36.0 * s);
            for (fx, fy) in [(false, false), (true, false), (false, true), (true, true)] {
                let at = [cen[0] + if fx { hw } else { -hw }, cen[1] + if fy { hh } else { -hh }];
                piece(c, "ret_plasma__corner_base_top", at, [20.0, 15.0], k, 0.0, (fx, fy), RET);
            }
            // Heat boxes fill like progress bars, in Doom's order: right, bottom, left (each from
            // its bottom / left end). Fill texture rows 37..53 of 90.
            // Full heat: the boxes slide into the gaps of the frame; after a Heat Blast they slide
            // back out and the frame flashes white (Doom).
            let slide = {
                let mut g = PLASMA.lock().unwrap_or_else(|e| e.into_inner());
                let (mut sl, t_prev, prev_heat, mut flash_t) = g.unwrap_or(([0.0; 3], st.time, st.heat, -10.0));
                let dt = (st.time - t_prev).clamp(0.0, 0.1);
                if prev_heat >= 0.3 && st.heat < prev_heat - 0.2 {
                    flash_t = st.time;
                }
                // each box slides in as soon as it is full
                for (i, v) in sl.iter_mut().enumerate() {
                    let goal = if st.heat * 3.0 - i as f32 >= 0.995 { 1.0 } else { 0.0 };
                    *v = (*v + (goal - *v).signum() * dt / 0.12).clamp(0.0, 1.0);
                }
                *g = Some((sl, st.time, st.heat, flash_t));
                (sl, st.time - flash_t)
            };
            let (sl, since_blast) = slide;
            if since_blast < 0.6 {
                let f = 1.0 - since_blast / 0.6;
                for (fx, fy) in [(false, false), (true, false), (false, true), (true, true)] {
                    let at = [cen[0] + if fx { hw } else { -hw }, cen[1] + if fy { hh } else { -hh }];
                    piece(c, "ret_plasma__corner_base_top", at, [20.0, 15.0], k, 0.0, (fx, fy), [1.0, 1.0, 1.0, 0.95 * f]);
                }
            }
            // in: centred on the frame's lines (sides x 25, bottom y 36)
            let e: [f32; 3] = std::array::from_fn(|i| sl[i] * sl[i] * (3.0 - 2.0 * sl[i]));
            let pips: [([f32; 2], f32); 3] = [
                ([cen[0] + (34.0 - 9.0 * e[0]) * s, cen[1]], 0.0),
                ([cen[0], cen[1] + (44.0 - 8.0 * e[1]) * s], -PI / 2.0),
                ([cen[0] - (34.0 - 9.0 * e[2]) * s, cen[1]], 0.0),
            ];
            for (i, (at, rot)) in pips.iter().enumerate() {
                piece(c, "ret_plasma_heat_pip_border_top", *at, [30.0, 45.0], k, *rot, (false, false), RET);
                let fill = ((st.heat * 3.0) - i as f32).clamp(0.0, 1.0);
                if fill > 0.0 {
                    let (v0, v1) = (37.0 / 90.0, 54.0 / 90.0);
                    // the first (right) box fills top to bottom, the others from their bottom end
                    let uv = if i == 0 { [0.0, v0, 1.0, v0 + (v1 - v0) * fill] } else { [0.0, v1 - (v1 - v0) * fill, 1.0, v1] };
                    piece_part(c, "ret_plasma_heat_pip_fill_top", *at, [30.0, 45.0], k, *rot, (false, false), RET, uv);
                }
            }
        }
        7 => {
            // BFG: Doom's four corners, the centre diamond and the side bars (light up while it
            // charges).
            let (hw, hh) = (42.0 * s, 37.0 * s);
            for (fx, fy) in [(false, false), (true, false), (false, true), (true, true)] {
                let at = [cen[0] + if fx { hw } else { -hw }, cen[1] + if fy { hh } else { -hh }];
                piece(c, "ret_corner", at, [11.0, 11.0], k, 0.0, (fx, fy), alpha(RET, 0.55));
            }
            piece(c, "ret_center_diamond", cen, [39.5, 39.5], k, 0.0, (false, false), RET);
            // Side bars fill like progress bars while the BFG winds up, from the outside in (left
            // bar left to right, right bar right to left); the shot leaves when both are full.
            let fill = st.bfg_charge.clamp(0.0, 1.0);
            for fx in [false, true] {
                let at = [cen[0] + if fx { 70.0 * s } else { -70.0 * s }, cen[1]];
                piece(c, "ret_prog_bar_empty", at, [39.5, 16.5], k, 0.0, (fx, false), alpha(RET, 0.3));
                if fill > 0.0 {
                    // bar content u 15..65 of 80; flipped for the right bar, so it fills from its outer end
                    let (u0, u1) = (15.0 / 80.0, 65.0 / 80.0);
                    let uv = [0.0, 0.0, u0 + (u1 - u0) * fill, 1.0];
                    piece_part(c, "ret_prog_bar_glow", at, [39.5, 16.5], k, 0.0, (fx, false), alpha(RET, 0.9), uv);
                    piece_part(c, "ret_prog_bar_empty", at, [39.5, 16.5], k, 0.0, (fx, false), RET, uv);
                }
            }
        }
        0 => {
            // Combat Shotgun / Sticky Bombs: five of Doom's arc pieces in a circle and five double
            // arrows on it (one per bomb: green ready, grey spent).
            // Bombs are used clockwise from the upper-right arrow (Doom): arrow j sits at -54 + 72 j
            // degrees and is spent once 5 - n bombs have been fired.
            let spent = 5 - st.sticky_ready as i32;
            let orange = [1.0, 0.55, 0.15, 0.95];
            let red = [1.0, 0.2, 0.12, 0.95];
            let (since, all) = st.sticky_flash;
            for i in 0..5 {
                let arrow = -0.3 * PI + i as f32 * 2.0 * PI / 5.0;
                let arc_a = arrow + PI / 5.0;
                // arc texture = top of its circle (screen angle -90 deg)
                // the short arc (46 deg) leaves an open spot for each arrow (Doom)
                piece(c, "ret_combatshotgun_sticky_circle_mastered_top", cen, [18.5, 44.1], 0.86 * s * (1.0 + 0.05 * kick), arc_a + PI / 2.0, (false, false), RET);
                let r = 38.0 * s * (1.0 + 0.05 * kick);
                let at = [cen[0] + arrow.cos() * r, cen[1] + arrow.sin() * r];
                let rot = arrow + PI / 2.0;
                if st.sticky_reload >= 0.0 {
                    // Empty magazine: every arrow red, fading out as it reloads (Doom).
                    let a = (1.0 - st.sticky_reload).max(0.15);
                    piece(c, "ret_combatshotgun_sticky_ammo_top", at, [20.5, 24.5], k, rot, (false, false), alpha(red, red[3] * a));
                    continue;
                }
                if i >= spent {
                    // Ready; the one just recharged (or all, after a reload) flashes bright.
                    let fresh = (all || i == spent) && since < 0.35;
                    let tint = if fresh { mix([1.0, 1.0, 0.9, 1.0], RET, since / 0.35) } else { RET };
                    piece(c, "ret_combatshotgun_sticky_ammo_top", at, [20.5, 24.5], k * if fresh { 1.0 + 0.3 * (1.0 - since / 0.35) } else { 1.0 }, rot, (false, false), tint);
                } else {
                    piece(c, "ret_combatshotgun_sticky_ammo_top", at, [20.5, 24.5], k, rot, (false, false), RET_DIM);
                    if i == spent - 1 && st.sticky_charge > 0.0 {
                        // The next bomb: the arrow fills in orange from its bottom (rows 18..31).
                        let (v0, v1) = (18.0 / 50.0, 32.0 / 50.0);
                        piece_part(c, "ret_combatshotgun_sticky_ammo_top", at, [20.5, 24.5], k, rot, (false, false), orange, [0.0, v1 - (v1 - v0) * st.sticky_charge, 1.0, v1]);
                    }
                }
            }
        }
        1 => {
            // Heavy Cannon: Doom's four-part circle (two arcs a side) with its soft glow.
            for fx in [false, true] {
                let at = [cen[0] + if fx { 2.0 * s } else { -2.0 * s }, cen[1]];
                piece(c, "ret_heavycannon_base_top", at, [18.2, 19.0], k * 1.3 * (1.0 + 0.08 * kick), 0.0, (fx, false), RET);
            }
            // Precision Bolt charge: four marks around the circle (almost transparent white).
            // After a bolt all turn orange and go back one by one - left, bottom, right, top -
            // as it recharges; when it is ready they all flash green and fade back (Doom).
            let since = st.time - st.bolt_at;
            let rec = crate::slayer::BOLT_RECOVERY;
            let orange = [1.0, 0.45, 0.1, 0.95];
            let faint = [1.0, 1.0, 1.0, 0.22];
            // left, bottom, right, top: angle of the mark's outward direction
            for (j, ang) in [PI, PI / 2.0, 0.0, -PI / 2.0].iter().enumerate() {
                let tint = if since < rec {
                    if since < rec * (j as f32 + 1.0) / 4.0 { orange } else { faint }
                } else if since < rec + 0.5 {
                    let f = (since - rec) / 0.5;
                    mix([0.7, 1.0, 0.3, 1.0], faint, f * f)
                } else {
                    faint
                };
                let r = 29.0 * s;
                let at = [cen[0] + ang.cos() * r, cen[1] + ang.sin() * r];
                // texture: narrow end up (content rows 20..39) -> turned to point outward
                piece(c, "ret_heavycannon_scope_pip_top", at, [29.5, 29.5], 0.62 * s, ang + PI / 2.0, (false, false), tint);
            }
        }
        _ => {
            c.dl.add_circle(cen, 34.0 * s, col([0.45, 1.0, 0.45, 0.85])).thickness(3.0 * s).num_segments(6).build();
        }
    }
}

// ---------------------------------------------------------------------- weapon wheel

/// Wedge textures share one layout: wheel centre 700 px below the texture's top edge,
/// outer radius 700, inner radius 315 (measured from weaponwheel_wedge_8pc_stroke).
const TEX_R: f32 = 700.0;
const TEX_INNER: f32 = 315.0;

fn wedge_quad(c: &Ctx, name: &str, cen: [f32; 2], scale: f32, ang: f32, top: f32, tint: Rgba) {
    let Some(t) = c.tex.get(name) else { return };
    // Texture rect relative to the wheel centre (y up = towards the outer rim).
    let x0 = -t.w * 0.5;
    let x1 = t.w * 0.5;
    let y0 = top; // distance of the texture's top edge from the centre
    let y1 = top - t.h;
    let rot = |x: f32, y: f32| {
        let (sn, cs) = ang.sin_cos();
        // ang = 0 points up (screen -y), clockwise positive.
        let rx = x * cs + y * sn;
        let ry = -x * sn + y * cs;
        [cen[0] + rx * scale, cen[1] - ry * scale]
    };
    c.dl
        .add_image_quad(t.id, rot(x0, y0), rot(x1, y0), rot(x1, y1), rot(x0, y1))
        .col(col(tint))
        .build();
}

pub fn wheel(c: &Ctx, st: &State) {
    let s = c.s;
    let open = (st.wheel_t / 0.12).clamp(0.0, 1.0);
    if open <= 0.0 {
        return;
    }
    let ease = 1.0 - (1.0 - open).powi(3);
    let cen = [c.w / 2.0, c.h / 2.0];
    let radius = 430.0 * s * (0.9 + 0.1 * ease);
    let scale = radius / TEX_R;
    let fade = ease;

    // Darken the world behind the wheel.
    c.dl.add_rect([0.0, 0.0], [c.w, c.h], col([0.0, 0.0, 0.0, 0.35 * fade])).filled(true).build();
    if let Some(t) = c.tex.get("weaponwheel_backer") {
        let r = radius * 1.12;
        c.dl.add_image(t.id, [cen[0] - r, cen[1] - r], [cen[0] + r, cen[1] + r]).col(col([1.0, 1.0, 1.0, 0.9 * fade])).build();
    }
    if let Some(t) = c.tex.get("weaponwheel_backer_center") {
        let r = TEX_INNER * scale * (219.0 / 157.5);
        c.dl.add_image(t.id, [cen[0] - r, cen[1] - r], [cen[0] + r, cen[1] + r]).col(col([0.75, 0.85, 0.75, 0.85 * fade])).build();
    }
    // Outer frame ring.
    c.dl.add_circle(cen, radius * 1.035, col([0.75, 0.78, 0.7, 0.35 * fade])).thickness(3.0 * s).num_segments(96).build();

    let current = st.weapon;
    let pick = st.wheel_pick.unwrap_or(current);
    for (pos, &slot) in WHEEL_ORDER.iter().enumerate() {
        let wd = &WEAPONS[slot];
        let ang = pos as f32 * PI / 4.0;
        let tint = ammo_color(wd.ammo);
        let sel = slot == pick;
        let has_ammo = st.ammo[wd.ammo.index()] >= wd.ammo_per_shot;
        let a = fade * if has_ammo { 1.0 } else { 0.45 };

        wedge_quad(c, "weaponwheel_wedge_8pc", cen, scale, ang, TEX_R, mix([0.3, 0.32, 0.3, 0.75 * a], alpha(tint, 0.55 * a), if sel { 0.6 } else { 0.25 }));
        // the selected wedge itself pulses (its own shape, so it always lines up), live:
        // [hud] wheel_pulse = [min glow, max glow, speed, -]
        // the stroke / no-ammo textures have their own padding: each is placed so its sides line up
        // with the wedge's (where the side lines of each texture meet, measured from the art)
        if sel {
            let wp = crate::config::get_cached().hud.get("wheel_pulse").copied().unwrap_or([0.15, 0.5, 1.0, 1.0]);
            let pulse = 0.5 + 0.5 * (st.time * 6.0 * wp[2]).sin();
            wedge_quad(c, "weaponwheel_wedge_8pc", cen, scale, ang, TEX_R, alpha(tint, (wp[0] + (wp[1] - wp[0]) * pulse) * fade));
        } else if !has_ammo {
            wedge_quad(c, "weaponwheel_wedge_8pc_glow_noammo", cen, scale, ang, TEX_R - 3.1, [0.9, 0.2, 0.15, 0.35 * fade]);
        }
        wedge_quad(c, "weaponwheel_wedge_8pc_stroke", cen, scale, ang, TEX_R + 9.0, alpha(tint, (if sel { 1.0 } else { 0.55 }) * a));
        wedge_quad(c, if sel { "weaponwheel_wedge_8pc_lighttop_on" } else { "weaponwheel_wedge_8pc_lighttop_off" }, cen, scale, ang, TEX_R + 30.0, alpha(tint, a));
        if slot == current {
            wedge_quad(c, "weaponwheel_wedge_8pc_lightbottom_on", cen, scale, ang, TEX_INNER + 40.0, alpha(tint, fade));
        }

        // Contents: weapon icon, ammo glyph + count, mod hex. Upright, at the wedge centroid.
        let mid = (TEX_R + TEX_INNER) * 0.5 * scale;
        let (sn, cs) = ang.sin_cos();
        let pc = [cen[0] + sn * mid, cen[1] - cs * mid];
        let ic = alpha(tint, a * if sel { 1.0 } else { 0.85 });
        c.image(WEAPON_ICONS[slot], [pc[0], pc[1] - 30.0 * s], 205.0 * s * if sel { 1.08 } else { 1.0 }, ic);
        let ammo = st.ammo[wd.ammo.index()];
        let t = ammo.to_string();
        let tsz = c.text_size(c.fonts.label, &t);
        let row_y = pc[1] + 28.0 * s;
        c.icon_ammo(wd.ammo, [pc[0] - 38.0 * s, row_y], 10.0 * s, ic);
        match c.doom_font(false) {
            Some(f) => {
                c.doom_text(f, pc[0] - 22.0 * s, row_y, 15.0 * s, ic, &t, 0.0, 1.0, true);
            }
            None => c.text(c.fonts.label, [pc[0] - 22.0 * s, row_y - tsz[1] * 0.5], ic, &t),
        }
        // The weapon's mod (Doom's own icon; it was a placeholder hex).
        let hx = [pc[0] + 44.0 * s, row_y];
        if c.tex.contains_key(MOD_ICONS[slot]) {
            c.image(MOD_ICONS[slot], hx, 34.0 * s, alpha(ic, 0.95));
        }
    }

    // Mouse pointer: the same motion that picks the wedge (clamped +-720 x, +-400 y in remap),
    // spread over the screen (400 = half the screen height), drawn as a dot.
    {
        use std::sync::atomic::Ordering::Relaxed;
        let (mx, my) = (crate::remap::WHEEL_DX.load(Relaxed) as f32, crate::remap::WHEEL_DY.load(Relaxed) as f32);
        let k = c.h * 0.5 / 400.0;
        let p = [(cen[0] + mx * k).clamp(0.0, c.w), (cen[1] + my * k).clamp(0.0, c.h)];
        let pt = ammo_color(WEAPONS[pick].ammo);
        c.dl.add_circle(p, 5.0 * s, col([0.0, 0.0, 0.0, 0.6 * fade])).filled(true).build();
        c.dl.add_circle(p, 3.5 * s, col(alpha(mix(pt, [1.0, 1.0, 1.0, 1.0], 0.5), fade))).filled(true).build();
    }

    // Selected weapon name in the centre.
    let wd = &WEAPONS[pick];
    let tint = ammo_color(wd.ammo);
    match c.doom_font(false) {
        Some(f) => {
            let cap = 17.0 * c.s;
            let (l, r) = c.doom_text_span(f, cap, wd.name, 0.0, 1.0);
            c.doom_text(f, cen[0] - (r - l) * 0.5, cen[1], cap, alpha(tint, fade), wd.name, 0.0, 1.0, true);
        }
        None => {
            let tsz = c.text_size(c.fonts.label, wd.name);
            c.text(c.fonts.label, [cen[0] - tsz[0] * 0.5, cen[1] - tsz[1] * 0.5], alpha(tint, fade), wd.name);
        }
    }
}

// ---------------------------------------------------------------------- pickups

/// World pickups as glowing, bobbing billboards (health blue cross, armor green shield, ammo by type).
/// Boss health in Elden Ring's own style (ER's bar is hidden with the rest of its HUD):
/// name above the left end, long thin red bar with a dark gold frame, bottom centre.
pub fn boss_bars(c: &Ctx, bosses: &[(String, i32, i32)]) {
    for (i, (name, hp, max)) in bosses.iter().enumerate().take(3) {
        let w = 1000.0 * c.s;
        let h = 9.0 * c.s;
        let x0 = (c.w - w) * 0.5;
        // Top centre (Elden Ring's own bar can't be shown without its whole HUD).
        let y0 = (78.0 + i as f32 * 58.0) * c.s;
        let frac = (*hp as f32 / (*max).max(1) as f32).clamp(0.0, 1.0);
        let pad = 2.0 * c.s;
        c.dl.add_rect([x0 - pad, y0 - pad], [x0 + w + pad, y0 + h + pad], col([0.02, 0.02, 0.02, 0.75])).filled(true).build();
        c.dl.add_rect([x0, y0], [x0 + w * frac, y0 + h], col([0.62, 0.09, 0.07, 1.0])).filled(true).build();
        c.dl.add_rect([x0, y0], [x0 + w * frac, y0 + h * 0.35], col([0.85, 0.25, 0.2, 0.55])).filled(true).build();
        c.dl.add_rect([x0 - pad, y0 - pad], [x0 + w + pad, y0 + h + pad], col([0.62, 0.52, 0.33, 0.9])).thickness(1.2 * c.s).build();
        if !name.is_empty() {
            let p = [x0 + 1.0 * c.s, y0 - 30.0 * c.s];
            c.text(c.fonts.label, [p[0] + 1.5 * c.s, p[1] + 1.5 * c.s], [0.0, 0.0, 0.0, 0.8], name);
            c.text(c.fonts.label, p, [0.93, 0.9, 0.82, 1.0], name);
        }
    }
}

/// Enemy health: slim slanted bars over heads (Doom HUD style), boss bar across the top.
pub fn enemy_bars(c: &Ctx, bars: &[crate::slayer::EnemyBar], time: f32) {
    let Some(cam) = crate::game::camera_full() else { return };
    let red = [0.92, 0.16, 0.12, 1.0];
    let back = [0.02, 0.02, 0.03, 0.65];
    for b in bars.iter().filter(|b| b.on_screen) {
        let Some(sp) = cam.project(b.pos, c.w, c.h) else { continue };
        let w = (95.0 * (10.0 / b.dist.max(2.0)).sqrt()).clamp(44.0, 120.0) * c.s;
        let h = (w * 0.07).max(4.0 * c.s);
        let (x0, y0) = (sp[0] - w * 0.5, sp[1] - h * 0.5);
        let sk = h * 0.8; // Doom-style slant
        let quad = |x: f32, ww: f32, col_: Rgba| {
            c.dl.add_polyline(
                vec![[x + sk, y0], [x + ww + sk, y0], [x + ww, y0 + h], [x, y0 + h]],
                col(col_),
            )
            .filled(true)
            .build();
        };
        quad(x0 - 2.0 * c.s, w + 4.0 * c.s, back);
        let fill = if b.staggered {
            // Glory kill ready: Doom flashes orange/blue.
            if (time * 8.0).sin() > 0.0 { [1.0, 0.55, 0.1, 1.0] } else { [0.3, 0.6, 1.0, 1.0] }
        } else {
            red
        };
        quad(x0, w * b.frac.clamp(0.0, 1.0), fill);
    }
    // Bosses use Elden Ring's own boss bar (name + style); ours is never drawn.
    if let Some(b) = bars.iter().filter(|b| b.boss && false).min_by(|a, b| a.dist.total_cmp(&b.dist)) {
        let w = 760.0 * c.s;
        let h = 14.0 * c.s;
        let (x0, y0) = ((c.w - w) * 0.5, 54.0 * c.s);
        c.dl.add_rect([x0 - 3.0 * c.s, y0 - 3.0 * c.s], [x0 + w + 3.0 * c.s, y0 + h + 3.0 * c.s], col(back)).filled(true).build();
        c.dl.add_rect([x0, y0], [x0 + w * b.frac.clamp(0.0, 1.0), y0 + h], col(red)).filled(true).build();
        c.dl.add_rect([x0 - 3.0 * c.s, y0 - 3.0 * c.s], [x0 + w + 3.0 * c.s, y0 + h + 3.0 * c.s], col([0.9, 0.8, 0.5, 0.8])).build();
    }
}

pub fn pickups(c: &Ctx, list: &[crate::pickups::Visual], time: f32) {
    use crate::pickups::Kind;
    let Some(cam) = crate::game::camera_full() else { return };
    for p in list {
        let bob = (time * 3.0 + p.pos.x).sin() * 0.06;
        let world = p.pos + glam::Vec3::Y * (0.15 + bob);
        let Some(sp) = cam.project(world, c.w, c.h) else { continue };
        let dist = (world - cam.pos).length().max(0.5);
        // Capped and faded out up close: magnet-pulled pickups fly through the lens.
        let r = ((0.22 / dist) * c.h * 0.9).min(c.h * 0.035);
        let fade = ((dist - 0.9) / 1.2).clamp(0.0, 1.0);
        if r < 2.0 || fade <= 0.02 {
            continue;
        }
        // Pops in when dropped; shrinks and fades away when despawning.
        let pop = (p.age / 0.25).clamp(0.0, 1.0) * p.life;
        let fade = fade * p.life;
        // Doom's own icons (HUD textures: icon_health_new / icon_armor with their glow pieces,
        // and the converted ammo icons), lit in the pickup's colour over a soft halo - they were
        // vector crosses / shields on a disc (user). Vector glyphs only if a texture is missing.
        let (color, icon, glow_tex, size) = match p.kind {
            Kind::Health => (HEALTH, "dh_icon_health_new", Some("dh_icon_health_new_glow"), 2.8),
            Kind::Armor => (ARMOR, "dh_icon_armor", Some("dh_icon_armor_glow"), 2.8),
            Kind::Ammo(a) => (ammo_color(a), ammo_tex(a), None, 2.3),
        };
        for (k, a) in [(1.9, 0.06), (1.3, 0.12)] {
            c.dl.add_circle(sp, r * k * pop, col(alpha(color, a * fade))).filled(true).num_segments(20).build();
        }
        if c.tex.contains_key(icon) {
            if let Some(g) = glow_tex {
                c.image(g, sp, r * size * 1.15 * pop, alpha(color, 0.7 * fade));
            }
            c.image(icon, sp, r * size * pop, alpha(mix(color, [1.0, 1.0, 1.0, 1.0], 0.3), fade));
            continue;
        }
        c.dl.add_circle(sp, r * pop, col(alpha(mix(color, [1.0, 1.0, 1.0, 1.0], 0.35), 0.95 * fade))).filled(true).num_segments(20).build();
        let ic = [0.04, 0.06, 0.08, 0.95 * fade];
        match p.kind {
            Kind::Health => c.icon_cross(sp, r * 0.6 * pop, ic),
            Kind::Armor => c.icon_shield(sp, r * 0.6 * pop, ic),
            Kind::Ammo(a) => c.icon_ammo(a, sp, r * 0.65 * pop, ic),
        }
    }
}

/// Doom's ammo icon texture for an ammo type (tools/convert_ammo_icons.py).
fn ammo_tex(a: Ammo) -> &'static str {
    match a {
        Ammo::Shells => "ammo_shells",
        Ammo::Bullets => "ammo_bullets",
        Ammo::Cells => "ammo_cells",
        Ammo::Rockets => "ammo_rockets",
        Ammo::Bfg => "ammo_bfg",
    }
}

/// Doom's "[E] USE" call-to-action prompt (its own frame texture and letters), saying INTERACT,
/// under the crosshair while Elden Ring offers an interaction (user's Doom screenshot: frame
/// 288 px wide at 1080p, its top line at y 730, the text centred just below).
/// Live: [hud] interact = [dx, dy, scale, alpha].
pub fn interact_prompt(c: &Ctx, key: &str) {
    let t = tune();
    let e = Tf::el(&t, "interact", 960.0, 744.0);
    let at = |x: f32, y: f32| {
        let (x, y) = e.pt(x, y);
        [c.w * 0.5 + (x - 960.0) * c.s, c.h - (1080.0 - y) * c.s]
    };
    let color: Rgba = [0.86, 0.95, 0.45, 0.95 * e.a];
    if let Some(tx) = c.tex.get("dh_cta_decor") {
        // visible frame 764 px of the 822 px texture -> 288 px; its top line sits 25 px down
        let k = 288.0 / 764.0;
        let (fw, fh) = (tx.w * k, tx.h * k);
        let p0 = at(960.0 - fw * 0.5, 730.0 - 25.0 * k);
        let p1 = at(960.0 + fw * 0.5, 730.0 - 25.0 * k + fh);
        c.dl.add_image(tx.id, p0, p1).col(col(color)).build();
    }
    // a controller button: its icon left of the word
    let pad_icon = key.strip_prefix('@');
    let label = if key.is_empty() || pad_icon.is_some() { "INTERACT".to_string() } else { format!("[{key}] INTERACT") };
    if let (Some(tex), Some(f)) = (pad_icon, c.doom_font(false)) {
        let (l, r) = c.doom_text_span(f, 15.0 * c.s * e.sc, &label, 0.6 * c.s * e.sc, 1.0);
        let mid = at(960.0, 744.0);
        let sz = 34.0 * c.s * e.sc;
        c.image(tex, [mid[0] - (r - l) * 0.5 - sz * 0.75, mid[1]], sz, [1.0, 1.0, 1.0, e.a]);
    }
    let cap = 15.0 * c.s * e.sc;
    let mid = at(960.0, 744.0);
    match c.doom_font(false) {
        Some(f) => {
            let tr = 0.6 * c.s * e.sc;
            let (l, r) = c.doom_text_span(f, cap, &label, tr, 1.0);
            c.doom_text(f, mid[0] - (r - l) * 0.5, mid[1], cap, color, &label, tr, 1.0, true);
        }
        None => {
            let sz = c.text_size(c.fonts.small, &label);
            c.text(c.fonts.small, [mid[0] - sz[0] * 0.5, mid[1] - sz[1] * 0.5], color, &label);
        }
    }
}

/// A Doom-lettered line under the crosshair (out of ammo at a boss: "CHAINSAW TO GET AMMO").
/// Live: [hud] saw_hint = [dx, dy, scale, alpha].
pub fn center_hint(c: &Ctx, text: &str) {
    let t = tune();
    let e = Tf::el(&t, "saw_hint", 960.0, 640.0);
    let (x, y) = e.pt(960.0, 640.0);
    let p = [c.w * 0.5 + (x - 960.0) * c.s, c.h - (1080.0 - y) * c.s];
    // Doom's warning orange, gently pulsing
    let pulse = 0.8 + 0.2 * (c.ui.time() as f32 * 4.0).sin();
    let color: Rgba = [1.0, 0.62, 0.18, pulse * e.a];
    let cap = 17.0 * c.s * e.sc;
    match c.doom_font(true) {
        Some(f) => {
            let tr = 0.8 * c.s * e.sc;
            let (l, r) = c.doom_text_span(f, cap, text, tr, 1.0);
            c.doom_text(f, p[0] - (r - l) * 0.5, p[1], cap, color, text, tr, 1.0, true);
        }
        None => {
            let sz = c.text_size(c.fonts.small, text);
            c.text(c.fonts.small, [p[0] - sz[0] * 0.5, p[1] - sz[1] * 0.5], color, text);
        }
    }
}

/// The Crucible's reticle (Doom's own pieces from Taras Nabad, placed like the user's Doom
/// video): the small 4-pin circle on the crosshair, and under it the three-part meter whose pips
/// (left arrow, middle block, right arrow) light one per charge. Live: [hud] crucible_ret =
/// [dx, dy, scale, alpha] (the meter; dy = its centre below the crosshair, 1080p px).
pub fn crucible_reticle(c: &Ctx, charges: u32) {
    let t = tune();
    let e = t.hud.get("crucible_ret").copied().unwrap_or([0.0, 82.0, 1.0, 1.0]);
    let s = c.s * e[2].clamp(0.3, 3.0);
    let red: Rgba = [1.0, 0.18, 0.12, e[3].clamp(0.0, 1.0)];
    let cen = [c.w * 0.5, c.h * 0.5];
    // circle: 44 px of ink in its 100 px texture -> ~30 px
    let ck = 30.0 / 44.0 * s;
    for (name, a) in [("st_ret_center_circle_4pins_glow", 0.5), ("st_ret_center_circle_4pins", 1.0)] {
        if let Some(tx) = c.tex.get(name) {
            let (w, h) = (tx.w * ck, tx.h * ck);
            c.dl.add_image(tx.id, [cen[0] - w * 0.5, cen[1] - h * 0.5], [cen[0] + w * 0.5, cen[1] + h * 0.5]).col(col(alpha(red, red[3] * a))).build();
        }
    }
    // meter: the border is 220 px wide -> 160 px, centred e[1] px under the crosshair
    let mk = 160.0 / 220.0 * s;
    let mc = [cen[0] + e[0] * c.s, cen[1] + e[1] * c.s];
    let draw = |name: &str, tint: Rgba, dy: f32| {
        if let Some(tx) = c.tex.get(name) {
            let (w, h) = (tx.w * mk, tx.h * mk);
            c.dl.add_image(tx.id, [mc[0] - w * 0.5, mc[1] - h * 0.5 + dy * mk], [mc[0] + w * 0.5, mc[1] + h * 0.5 + dy * mk]).col(col(tint)).build();
        }
    };
    draw("st_ret_meter_border_glow", alpha(red, red[3] * 0.45), 0.0);
    draw("st_ret_meter_border", red, 0.0);
    // pips: x ranges 0-69 / 70-125 / 126-195 of the 195 px texture; lit ones full red with Doom's
    // glare, empty ones dim. (pips 7.5 px under the border's centre: centred top and bottom in its frame - live: crucible_pips dy)
    let pdy = t.hud.get("crucible_pips").map_or(7.5, |v| v[1]);
    if let Some(tx) = c.tex.get("st_ret_meter_pips") {
        let ranges = [(0.0f32, 69.0f32), (70.0, 125.0), (126.0, 195.0)];
        let (w, h) = (tx.w * mk, tx.h * mk);
        let x0 = mc[0] - w * 0.5;
        let y0 = mc[1] - h * 0.5 + pdy * mk;
        for (i, (a, b)) in ranges.iter().enumerate() {
            let lit = (i as u32) >= 3 - charges; // empties from the left (user)
            let tint = if lit { red } else { alpha(red, red[3] * 0.18) };
            let (u0, u1) = (a / tx.w, b / tx.w);
            c.dl.add_image(tx.id, [x0 + a * mk, y0], [x0 + b * mk, y0 + h]).uv_min([u0, 0.0]).uv_max([u1, 1.0]).col(col(tint)).build();
            if lit {
                if let Some(g) = c.tex.get("st_ret_meter_pips_glare_add") {
                    c.dl.add_image(g.id, [x0 + a * mk, y0], [x0 + b * mk, y0 + h]).uv_min([u0, 0.0]).uv_max([u1, 1.0]).col(col(alpha([1.0, 0.7, 0.5, 1.0], 0.5 * red[3]))).build();
                }
            }
        }
    }
}

