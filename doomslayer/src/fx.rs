//! Doom weapon visuals drawn by our own renderer (viewmodel pass), never by Elden Ring: tracers,
//! plasma bolts, rockets, the BFG ball, the Ballista beam and impact bursts, all with Doom's own
//! particle textures (doom_fx/, tools/convert_fx.py). Elden Ring's bullet/effect pools are fixed
//! size and ran out (spawns failed with "construction failed"), so nothing here uses them.
//!
//! Shots live in world space; the renderer turns them into camera-facing quads every frame.

use std::{sync::Mutex, time::Instant};

use glam::Vec3;

#[derive(Clone, Copy, PartialEq)]
pub enum Style {
    /// Shotgun pellets: short hot streaks.
    Pellet,
    /// Heavy cannon / chaingun: tracer streak.
    Tracer,
    /// Plasma: blue bolt.
    Bolt,
    /// Ballista: lingering beam.
    Beam,
    /// Rocket: fire ball with a trail.
    Rocket,
    /// BFG: big green ball.
    Ball,
}

#[derive(Clone)]
pub struct Shot {
    pub t0: Instant,
    pub slot: usize,
    pub style: Style,
    /// Barrel point in the world (where the visuals start).
    pub origin: Vec3,
    /// Where each pellet ended and whether it hit a character.
    pub ends: Vec<(Vec3, bool)>,
    pub seed: u32,
}

pub static SHOTS: Mutex<Vec<Shot>> = Mutex::new(Vec::new());

pub struct Look {
    pub style: Style,
    /// "fire" / "blue" / "green" texture ramp.
    pub ramp: &'static str,
    /// Visual speed (m/s).
    pub speed: f32,
    /// Projectile / streak size (m).
    pub size: f32,
    /// Impact burst size (m).
    pub impact: f32,
}

/// Extra looks for weapon mods (index into LOOKS).
pub const STICKY: usize = 8;
pub const ARBALEST: usize = 9;
pub const HEAT_BLAST: usize = 10;
pub const EXPLOSION: usize = 11;

pub const LOOKS: [Look; 12] = [
    Look { style: Style::Pellet, ramp: "fire", speed: 260.0, size: 0.05, impact: 0.35 },  // combat shotgun
    Look { style: Style::Tracer, ramp: "fire", speed: 300.0, size: 0.06, impact: 0.3 },   // heavy cannon
    Look { style: Style::Bolt, ramp: "blue", speed: 90.0, size: 0.35, impact: 0.6 },      // plasma rifle
    Look { style: Style::Rocket, ramp: "fire", speed: 55.0, size: 0.45, impact: 3.5 },    // rocket launcher
    Look { style: Style::Pellet, ramp: "fire", speed: 260.0, size: 0.06, impact: 0.4 },   // super shotgun
    Look { style: Style::Beam, ramp: "fire", speed: 600.0, size: 0.2, impact: 0.9 },      // ballista (red-orange like Doom)
    Look { style: Style::Tracer, ramp: "fire", speed: 300.0, size: 0.05, impact: 0.25 },  // chaingun
    Look { style: Style::Ball, ramp: "green", speed: 25.0, size: 1.4, impact: 8.0 * crate::damage::BFG_RADIUS / 9.0 }, // BFG (visual scales with the blast)
    Look { style: Style::Bolt, ramp: "fire", speed: 40.0, size: 0.22, impact: 0.3 },      // sticky bomb
    Look { style: Style::Beam, ramp: "fire", speed: 600.0, size: 0.32, impact: 0.8 },     // arbalest bolt
    Look { style: Style::Ball, ramp: "blue", speed: 60.0, size: 1.8, impact: 6.0 },       // heat blast
    Look { style: Style::Rocket, ramp: "fire", speed: 1000.0, size: 0.1, impact: 3.5 },   // explosion
];

/// Bombs / bolts stuck in the world or in demons (drawn as blinking glows): (pos, ramp, blink).
pub static STUCK: Mutex<Vec<(Vec3, &'static str, bool)>> = Mutex::new(Vec::new());

/// Meathook target while the hook is out (chain drawn from the Super Shotgun to it).
pub static HOOK: Mutex<Option<(Vec3, Instant)>> = Mutex::new(None);

/// Explosion visual at a point (no travel).
pub fn explosion(at: Vec3) {
    shot(EXPLOSION, at, vec![(at, true)]);
}

/// Remote detonation: the newest rocket in flight ends where it is now.
pub fn cut_rocket(at: Vec3) {
    if let Ok(mut v) = SHOTS.lock() {
        if let Some(s) = v.iter_mut().rev().find(|s| s.slot == 3) {
            if let Some(e) = s.ends.first_mut() {
                e.0 = at;
            }
            let travel = at.distance(s.origin) / LOOKS[3].speed;
            s.t0 = Instant::now() - std::time::Duration::from_secs_f32(travel.max(0.0));
        }
    }
}

pub fn shot(slot: usize, origin: Vec3, ends: Vec<(Vec3, bool)>) {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    let seed = N.fetch_add(0x9e37_79b9, std::sync::atomic::Ordering::Relaxed);
    if let Ok(mut v) = SHOTS.lock() {
        let slot = slot.min(LOOKS.len() - 1);
        v.push(Shot { t0: Instant::now(), slot, style: LOOKS[slot].style, origin, ends, seed });
        let n = v.len();
        if n > 64 {
            v.drain(..n - 64);
        }
    }
}

pub fn look_of(style: Style) -> &'static Look {
    LOOKS.iter().find(|l| l.style == style).unwrap_or(&LOOKS[0])
}

/// Seconds a shot's visuals live: travel to the furthest end + the impact.
pub fn life(s: &Shot, look: &Look) -> f32 {
    let far = s.ends.iter().map(|(e, _)| e.distance(s.origin)).fold(0.0f32, f32::max);
    let impact = if matches!(s.style, Style::Rocket | Style::Ball) { 0.45 } else { 0.18 };
    let linger = if s.style == Style::Beam { 0.3 } else { 0.0 };
    (far / look.speed).min(3.0) + impact + linger
}

pub fn hash(mut x: u32) -> f32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    (x % 10_000) as f32 / 10_000.0
}
