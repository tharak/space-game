//! Captains: who leads prototype01's fleets. Each has a name, a skill and a
//! flagship; only fleets with a captain take orders.

use bevy::prelude::Entity;

use crate::voronoi::rng::ParkMiller;

/// One bonus per captain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Skill {
    /// +50% command capacity.
    Commander,
    /// Repairs need half the parts; damaged ships break up half as often.
    Engineer,
    /// Ships eat 30% less food and desert half as often.
    Quartermaster,
    /// +25% rate of fire.
    Gunner,
    /// Shots use half the ammo.
    Sharpshooter,
    /// +25% speed.
    Navigator,
    /// Shortage, over-capacity and flagship-loss shock halved.
    Inspiring,
    /// The fleet's ships count double as cops at planets.
    Governor,
    /// Supply ships for this fleet carry 50% more, and it is served first.
    Logistician,
    /// Half the chance to die when the flagship is lost.
    Survivor,
    /// Captures in half the time; conquered planets resent it half as much.
    Conqueror,
}

impl Skill {
    pub const ALL: [Skill; 11] = [
        Skill::Commander,
        Skill::Engineer,
        Skill::Quartermaster,
        Skill::Gunner,
        Skill::Sharpshooter,
        Skill::Navigator,
        Skill::Inspiring,
        Skill::Governor,
        Skill::Logistician,
        Skill::Survivor,
        Skill::Conqueror,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Skill::Commander => "Commander",
            Skill::Engineer => "Engineer",
            Skill::Quartermaster => "Quartermaster",
            Skill::Gunner => "Gunner",
            Skill::Sharpshooter => "Sharpshooter",
            Skill::Navigator => "Navigator",
            Skill::Inspiring => "Inspiring",
            Skill::Governor => "Governor",
            Skill::Logistician => "Logistician",
            Skill::Survivor => "Survivor",
            Skill::Conqueror => "Conqueror",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            Skill::Commander => "+50% ships commanded",
            Skill::Engineer => "repairs with half the parts, fewer wrecks",
            Skill::Quartermaster => "30% less food, half the desertion",
            Skill::Gunner => "+25% rate of fire",
            Skill::Sharpshooter => "shots use half the ammo",
            Skill::Navigator => "+25% speed",
            Skill::Inspiring => "half the shock from shortages, crowding and losing the flagship",
            Skill::Governor => "ships count double as cops",
            Skill::Logistician => "+50% supply loads, served first",
            Skill::Survivor => "half the chance to die with the flagship",
            Skill::Conqueror => "captures twice as fast, half the resentment",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Captain {
    pub id: u32,
    pub name: String,
    pub side: usize,
    pub skill: Skill,
    /// The flagship they command from; `None` while they wait for a new one.
    pub flagship: Option<Entity>,
    pub alive: bool,
    /// Seconds until a new flagship, after losing one.
    pub back_in: f32,
}

impl Captain {
    pub fn title(&self) -> String {
        format!("Capt. {} ({})", self.name, self.skill.name())
    }

    /// The first letter of the family name, for the map.
    pub fn initial(&self) -> char {
        self.name.split_whitespace().last().and_then(|w| w.chars().next()).unwrap_or('?')
    }
}

const GIVEN: [&str; 24] = [
    "Mara", "Tobin", "Ilse", "Corvin", "Ysolde", "Bram", "Oriel", "Dace", "Sefa", "Halden", "Nyra", "Pell", "Wren", "Ansel", "Liora", "Garrick",
    "Tamsin", "Euric", "Shiel", "Rook", "Vela", "Odric", "Kesh", "Fenna",
];
const FAMILY: [&str; 24] = [
    "Venn", "Ashgrove", "Kestrel", "Morrow", "Thale", "Quill", "Darrow", "Ostrander", "Pike", "Halloway", "Sable", "Varga", "Lune", "Brask",
    "Corwen", "Ivers", "Marlow", "Tey", "Rusk", "Selwyn", "Orme", "Hask", "Wyle", "Ebon",
];

/// A name not in `taken`, drawn from the lists.
pub fn draw_name(rng: &mut ParkMiller, taken: &[String]) -> String {
    for _ in 0..200 {
        let g = GIVEN[((rng.next_float() * GIVEN.len() as f32) as usize).min(GIVEN.len() - 1)];
        let f = FAMILY[((rng.next_float() * FAMILY.len() as f32) as usize).min(FAMILY.len() - 1)];
        let name = format!("{g} {f}");
        if !taken.contains(&name) {
            return name;
        }
    }
    format!("Captain {}", taken.len() + 1)
}

/// `n` skills for a side, all different while they last (seeded).
pub fn draw_skills(rng: &mut ParkMiller, n: usize) -> Vec<Skill> {
    let mut pool = Skill::ALL.to_vec();
    for i in (1..pool.len()).rev() {
        let j = ((rng.next_float() * (i + 1) as f32) as usize).min(i);
        pool.swap(i, j);
    }
    (0..n).map(|k| pool[k % pool.len()]).collect()
}
