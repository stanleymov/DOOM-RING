//! Doom pickups: enemies burst into health / armor / ammo that lands on the ground, bobs, and is
//! pulled into the Slayer when he gets close (glory kills -> health, chainsaw -> ammo shower,
//! flame belch burns -> armor shards, ordinary kills -> a little ammo, more when you're low).

use glam::Vec3;

use crate::{game, raycast, weapons::Ammo};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    Health,
    Armor,
    Ammo(Ammo),
}

#[derive(Clone, Debug)]
pub struct Pickup {
    pub kind: Kind,
    pub amount: i32,
    pub pos: Vec3,
    vel: Vec3,
    ground: f32,
    pub age: f32,
    /// Seconds before it may be collected (lets the burst fly out first).
    delay: f32,
    /// Strong magnet (glory/chainsaw drops fly straight to you like in Doom).
    homing: bool,
    pub visible: bool,
    /// Seconds continuously out of sight (armor shards despawn after 2 s of it).
    hidden_t: f32,
    /// Despawning: seconds into the shrink-and-fade-out (0 = not despawning).
    dying: f32,
}

/// Despawn shrink-and-fade time (s).
const FADE_OUT: f32 = 0.5;

pub struct Pickups {
    pub list: Vec<Pickup>,
    seed: u32,
}

/// What was collected this frame.
#[derive(Default)]
pub struct Collected {
    pub health: i32,
    pub armor: i32,
    pub ammo: [i32; 5],
}

impl Pickups {
    pub fn new() -> Self {
        Self { list: Vec::new(), seed: 0x9e37_79b9 }
    }

    fn rnd(&mut self) -> f32 {
        self.seed ^= self.seed << 13;
        self.seed ^= self.seed >> 17;
        self.seed ^= self.seed << 5;
        (self.seed % 10_000) as f32 / 10_000.0
    }

    /// Burst `count` pickups out of `at` (a dying demon's chest).
    pub fn burst(&mut self, at: Vec3, kind: Kind, count: usize, amount: i32, homing: bool) {
        // Ground under the spawn point (fallback: 1.2 m below the chest).
        let ground = raycast::cast_sphere(at + Vec3::Y * 0.5, Vec3::NEG_Y * 6.0, 0.1, 0x2000058, |h| !raycast::is_chr_hit(h))
            .map(|h| h.pos.y)
            .unwrap_or(at.y - 1.2);
        for _ in 0..count {
            let a = self.rnd() * std::f32::consts::TAU;
            let sp = 1.5 + self.rnd() * 2.5;
            let up = 3.0 + self.rnd() * 2.5;
            self.list.push(Pickup {
                kind,
                amount,
                pos: at,
                vel: Vec3::new(a.cos() * sp, up, a.sin() * sp),
                ground: ground + 0.25,
                age: 0.0,
                delay: if homing { 0.25 } else { 0.5 },
                homing,
                visible: true,
                hidden_t: 0.0,
                dying: 0.0,
            });
        }
        if self.list.len() > 120 {
            let n = self.list.len() - 120;
            self.list.drain(..n);
        }
    }

    pub fn update(&mut self, dt: f32, player: Vec3, cam: Option<(Vec3, Vec3)>, magnet: f32) -> Collected {
        let mut got = Collected::default();
        let chest = player + Vec3::Y * 1.0;
        self.list.retain_mut(|p| {
            p.age += dt;
            let to = chest - p.pos;
            let d = to.length();
            let pull = p.age > p.delay && (p.homing || d < magnet);
            if pull {
                let speed = if p.homing { 14.0 + p.age * 20.0 } else { 9.0 + (magnet - d).max(0.0) * 4.0 };
                p.vel = to.normalize_or_zero() * speed;
                p.pos += p.vel * dt;
            } else {
                p.vel.y -= 18.0 * dt;
                p.vel.x *= (1.0 - 2.5 * dt).max(0.0);
                p.vel.z *= (1.0 - 2.5 * dt).max(0.0);
                p.pos += p.vel * dt;
                if p.pos.y < p.ground {
                    p.pos.y = p.ground;
                    p.vel = Vec3::ZERO;
                }
            }
            if p.age > p.delay && d < 1.1 {
                match p.kind {
                    Kind::Health => got.health += p.amount,
                    Kind::Armor => got.armor += p.amount,
                    Kind::Ammo(a) => got.ammo[a.index()] += p.amount,
                }
                return false;
            }
            // Armor shards (shield icons) vanish after 10 s, or after 2 s straight out of sight
            // (walls/terrain only - enemies and bosses don't block): they drifted to odd places
            // when you moved away (the map re-bases its local coordinates).
            // They shrink and fade out over half a second instead of popping away.
            p.hidden_t = if p.visible { 0.0 } else { p.hidden_t + dt };
            if p.kind == Kind::Armor && p.dying == 0.0 && (p.age > 10.0 || p.hidden_t > 2.0) {
                p.dying = 1e-4;
            }
            if p.dying > 0.0 {
                p.dying += dt;
                if p.dying > FADE_OUT {
                    return false;
                }
            }
            p.age < 40.0
        });
        // Occlusion: hide pickups behind walls (cheap, staggered over frames).
        if let Some((cpos, _)) = cam {
            let n = self.list.len();
            for (i, p) in self.list.iter_mut().enumerate() {
                if (i + (p.age * 10.0) as usize) % 4 != 0 && n > 8 {
                    continue;
                }
                let dir = p.pos - cpos;
                p.visible = dir.length() < 45.0
                    && raycast::cast_sphere(cpos, dir * 0.97, 0.05, 0x2000058, |h| !raycast::is_chr_hit(h)).is_none();
            }
        }
        got
    }
}

/// Render-side copy.
#[derive(Clone)]
pub struct Visual {
    pub kind: Kind,
    pub pos: Vec3,
    pub age: f32,
    /// 1 = normal, falls to 0 while despawning (size and opacity).
    pub life: f32,
}

pub fn visuals(p: &Pickups) -> Vec<Visual> {
    p.list
        .iter()
        .filter(|p| p.visible)
        .map(|p| Visual { kind: p.kind, pos: p.pos, age: p.age, life: if p.dying > 0.0 { (1.0 - p.dying / FADE_OUT).clamp(0.0, 1.0) } else { 1.0 } })
        .collect()
}

#[allow(dead_code)]
pub fn player_pos() -> Option<Vec3> {
    game::player().map(|p| game::chr_pos(&p.chr_ins))
}
