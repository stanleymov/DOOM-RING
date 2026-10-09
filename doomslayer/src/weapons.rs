//! Doom Eternal arsenal. Numbers follow Doom Eternal's feel (fire rate, pellets, ammo pools),
//! scaled for Elden Ring HP pools (see `enemy_hp_mult`).

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Ammo {
    Shells,
    Bullets,
    Cells,
    Rockets,
    Bfg,
}

impl Ammo {
    pub const ALL: [Ammo; 5] = [Ammo::Shells, Ammo::Bullets, Ammo::Cells, Ammo::Rockets, Ammo::Bfg];

    pub fn max(self) -> i32 {
        match self {
            Ammo::Shells => 24,
            Ammo::Bullets => 180,
            Ammo::Cells => 250,
            Ammo::Rockets => 13,
            Ammo::Bfg => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Ammo::Shells => "SHELLS",
            Ammo::Bullets => "BULLETS",
            Ammo::Cells => "CELLS",
            Ammo::Rockets => "ROCKETS",
            Ammo::Bfg => "BFG",
        }
    }

    pub fn index(self) -> usize {
        self as usize
    }
}

#[derive(Debug)]
pub struct Weapon {
    pub name: &'static str,
    pub ammo: Ammo,
    pub ammo_per_shot: i32,
    /// Seconds between shots.
    /// Seconds between shots: Doom's firingInterval (weapon decls; the SSG with its reload upgrade).
    pub interval: f32,
    pub pellets: u32,
    /// Cone half-angle in degrees.
    pub spread: f32,
    /// Per-pellet power: AtkParam physical correction in % (before `weapon_damage_mult`).
    /// Verified in game: the game ignores atk_phys for player bullets and scales a fixed base by
    /// this %. Troll (4338 HP scaled): 100% -> 26, 200% -> 78, 500% -> 277, 1000% -> 644 per hit.
    pub damage: f32,
    pub stagger: u16,
    pub poise_damage: f32,
    pub speed: f32,
    pub range: f32,
    pub hit_radius: f32,
    pub explosive: bool,
    pub automatic: bool,
    /// Trail effect override (-1 = invisible pellet). None keeps the template's.
    pub sfx_bullet: Option<i32>,
    /// Impact effect override. None keeps the template's.
    pub sfx_hit: Option<i32>,
    /// Weapon wheel / number key slot (0-based).
    pub slot: usize,
    /// doom_audio event played on every shot.
    pub fire_sound: &'static str,
    /// Follow-up sound (delay s, event), e.g. SSG shells out / pump.
    pub reload_sound: Option<(f32, &'static str)>,
}

pub const WEAPONS: [Weapon; 8] = [
    Weapon {
        name: "COMBAT SHOTGUN",
        ammo: Ammo::Shells,
        ammo_per_shot: 1,
        interval: 0.65,
        pellets: 8,
        spread: 3.2,
        damage: 160.0,
        stagger: 20,
        poise_damage: 12.0,
        speed: 140.0,
        range: 40.0,
        hit_radius: 0.12,
        explosive: false,
        automatic: false,
        sfx_bullet: Some(-1),
        sfx_hit: Some(5001002),
        slot: 0,
        fire_sound: "shotgun_fire",
        // Doom's combat shotgun has no audible pump per shot (checked against a game capture).
        reload_sound: None,
    },
    Weapon {
        name: "HEAVY CANNON",
        ammo: Ammo::Bullets,
        ammo_per_shot: 1,
        interval: 0.11,
        pellets: 1,
        spread: 0.8,
        damage: 260.0,
        stagger: 6,
        poise_damage: 4.0,
        speed: 220.0,
        range: 120.0,
        hit_radius: 0.08,
        explosive: false,
        automatic: true,
        sfx_bullet: Some(-1),
        sfx_hit: Some(300123),
        slot: 1,
        fire_sound: "heavy_cannon_fire",
        reload_sound: None,
    },
    Weapon {
        name: "PLASMA RIFLE",
        ammo: Ammo::Cells,
        ammo_per_shot: 1,
        interval: 0.1,
        pellets: 1,
        spread: 0.6,
        damage: 150.0,
        stagger: 4,
        poise_damage: 3.0,
        speed: 90.0,
        range: 80.0,
        hit_radius: 0.15,
        explosive: false,
        automatic: true,
        sfx_bullet: None,
        sfx_hit: None,
        slot: 2,
        fire_sound: "plasma_fire",
        reload_sound: None,
    },
    Weapon {
        name: "ROCKET LAUNCHER",
        ammo: Ammo::Rockets,
        ammo_per_shot: 1,
        interval: 1.25,
        pellets: 1,
        spread: 0.0,
        damage: 900.0,
        stagger: 60,
        poise_damage: 40.0,
        speed: 55.0,
        range: 120.0,
        hit_radius: 0.3,
        explosive: true,
        automatic: false,
        sfx_bullet: Some(524022),
        sfx_hit: None,
        slot: 3,
        fire_sound: "rocket_fire",
        reload_sound: None,
    },
    Weapon {
        name: "SUPER SHOTGUN",
        ammo: Ammo::Shells,
        ammo_per_shot: 2,
        interval: 1.3,
        pellets: 16,
        spread: 4.5,
        damage: 200.0,
        stagger: 40,
        poise_damage: 25.0,
        speed: 140.0,
        range: 30.0,
        hit_radius: 0.14,
        explosive: false,
        automatic: false,
        sfx_bullet: Some(-1),
        sfx_hit: Some(5001002),
        slot: 4,
        fire_sound: "ssg_fire",
        reload_sound: Some((0.35, "ssg_reload")),
    },
    Weapon {
        name: "BALLISTA",
        ammo: Ammo::Cells,
        ammo_per_shot: 25,
        interval: 1.1,
        pellets: 1,
        spread: 0.0,
        damage: 1400.0,
        stagger: 80,
        poise_damage: 60.0,
        speed: 300.0,
        range: 200.0,
        hit_radius: 0.2,
        explosive: false,
        automatic: false,
        sfx_bullet: Some(525502),
        sfx_hit: Some(525503),
        slot: 5,
        fire_sound: "ballista_fire",
        reload_sound: None,
    },
    Weapon {
        name: "CHAINGUN",
        ammo: Ammo::Bullets,
        ammo_per_shot: 1,
        interval: 0.08,
        pellets: 1,
        spread: 2.0,
        damage: 170.0,
        stagger: 3,
        poise_damage: 2.0,
        speed: 220.0,
        range: 100.0,
        hit_radius: 0.08,
        explosive: false,
        automatic: true,
        sfx_bullet: Some(-1),
        sfx_hit: Some(300123),
        slot: 6,
        fire_sound: "chaingun_fire",
        reload_sound: None,
    },
    Weapon {
        name: "BFG 9000",
        ammo: Ammo::Bfg,
        ammo_per_shot: 1,
        interval: 2.5,
        pellets: 1,
        spread: 0.0,
        damage: 6000.0,
        stagger: 200,
        poise_damage: 200.0,
        speed: 25.0,
        range: 120.0,
        hit_radius: 1.5,
        explosive: true,
        automatic: false,
        sfx_bullet: Some(524027),
        sfx_hit: None,
        slot: 7,
        fire_sound: "bfg_fire",
        reload_sound: None,
    },
];

pub const SUPER_SHOTGUN: usize = 4;
