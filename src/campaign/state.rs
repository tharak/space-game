//! Everything that persists across the campaign: who owns what, every fleet
//! with its admiral and ship count, and the battles waiting to be fought.

use serde::{Deserialize, Serialize};

use super::map::{Galaxy, SysId};
use crate::sim::scenario::FleetShape;

pub const FLEET_CAP: u32 = 160;
pub const STACK_LIMIT: usize = 3;
pub const GARRISON_CAP: u32 = 200;
pub const MAX_FLEETS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trait {
    /// +8% fire rate.
    Gunnery,
    /// Steadier morale.
    Steadfast,
    /// Faster movement and wheeling.
    Maneuver,
    /// Supply penalties start a step later.
    Logistician,
}

impl Trait {
    pub const ALL: [Trait; 4] = [Trait::Gunnery, Trait::Steadfast, Trait::Maneuver, Trait::Logistician];

    pub fn name(self) -> &'static str {
        match self {
            Trait::Gunnery => "Gunnery",
            Trait::Steadfast => "Steadfast",
            Trait::Maneuver => "Maneuver",
            Trait::Logistician => "Logistician",
        }
    }
}

pub const RANKS: [&str; 4] = ["Commodore", "Rear Admiral", "Vice Admiral", "Admiral"];
/// Experience needed for ranks 1, 2 and 3.
pub const RANK_XP: [u32; 3] = [2, 5, 9];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Admiral {
    pub name: String,
    pub rank: u32,
    pub xp: u32,
    pub traits: Vec<Trait>,
}

impl Admiral {
    pub fn title(&self) -> String {
        format!("{} {}", RANKS[self.rank.min(3) as usize], self.name)
    }

    pub fn has(&self, t: Trait) -> bool {
        self.traits.contains(&t)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Location {
    At(SysId),
    /// Between two systems, `left` turns from `to`.
    Lane { from: SysId, to: SysId, left: u32 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fleet {
    pub id: u32,
    pub side: usize,
    pub admiral: Admiral,
    pub ships: u32,
    /// Of `ships`, how many are damaged (fight at half hull until repaired).
    pub damaged: u32,
    pub loc: Location,
    /// Systems still to visit, in order.
    pub path: Vec<SysId>,
    /// The last system it stood in (where it retreats to).
    pub prev: SysId,
    pub shape: FleetShape,
    /// Consecutive turns out of supply.
    pub unsupplied: u32,
}

impl Fleet {
    pub fn at(&self) -> Option<SysId> {
        match self.loc {
            Location::At(s) => Some(s),
            Location::Lane { .. } => None,
        }
    }

    /// Supply state with the Logistician trait applied: 0 ok, 1 low, 2 critical.
    pub fn supply_level(&self) -> u32 {
        let turns = self.unsupplied.saturating_sub(u32::from(self.admiral.has(Trait::Logistician)));
        match turns {
            0 => 0,
            1 | 2 => 1,
            _ => 2,
        }
    }

    /// Fighting value: damaged ships count half.
    pub fn strength(&self) -> f32 {
        self.ships as f32 - self.damaged as f32 * 0.5
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct System {
    pub owner: Option<usize>,
    pub garrison: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Place {
    System(SysId),
    /// Fleets met in the lane between two systems.
    Lane(SysId, SysId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingBattle {
    pub id: u32,
    pub place: Place,
    /// Fleet ids on each side.
    pub fleets: [Vec<u32>; 2],
    /// Garrison ships fighting on each side (a neutral garrison takes the
    /// side opposite the attacker).
    pub garrison: [u32; 2],
    /// Whose garrison it is (`None` = neutral), if any fights.
    pub garrison_owner: Option<Option<usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Replacement {
    pub side: usize,
    pub due: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, bevy::prelude::Resource)]
pub struct Campaign {
    pub turn: u32,
    pub seed: u32,
    pub systems: Vec<System>,
    pub fleets: Vec<Fleet>,
    pub next_id: u32,
    pub pending: Vec<PendingBattle>,
    pub replacements: Vec<Replacement>,
    /// Name pools already used, per side.
    pub names_used: [u32; 2],
    pub winner: Option<usize>,
    /// Messages from the last turn, newest last.
    pub log: Vec<String>,
    pub battles_fought: u32,
}

pub const ADMIRALS: [[&str; 10]; 2] = [
    ["Ines Marrow", "Tobias Vane", "Caro Lindqvist", "Eamon Reyes", "Sefa Okoro", "Mira Talbot", "Jonah Strand", "Ada Quill", "Pell Navarro", "Rhea Castell"],
    ["Varic Hale", "Sorrel Ashby", "Ivo Drach", "Tamsin Kell", "Oren Vasque", "Lisette Mor", "Corin Dahl", "Ysolde Brann", "Fenwick Ord", "Hesta Vael"],
];

impl Campaign {
    pub fn new(seed: u32) -> Self {
        let galaxy = Galaxy::new();
        let systems = galaxy.systems.iter().map(|s| System { owner: s.owner, garrison: s.garrison }).collect();
        let mut c = Campaign {
            turn: 1,
            seed,
            systems,
            fleets: Vec::new(),
            next_id: 1,
            pending: Vec::new(),
            replacements: Vec::new(),
            names_used: [0; 2],
            winner: None,
            log: Vec::new(),
            battles_fought: 0,
        };
        // Four fleets each: two at the capital, one at each corridor mouth.
        for side in 0..2 {
            let base = 9 * side;
            for (at, ships) in [(base, 120), (base + 4, 100), (base + 6, 100), (base + 8, 100)] {
                c.add_fleet(side, at, ships);
            }
        }
        c.log.push("The war begins. Two corridors cross the Shroud nebula: Vigil Gate's fortress in the north, neutral Tollmarch in the south.".into());
        c
    }

    pub fn add_fleet(&mut self, side: usize, at: SysId, ships: u32) -> u32 {
        let n = self.names_used[side] as usize;
        let roman = ["", " II", " III", " IV", " V"];
        let name = format!("{}{}", ADMIRALS[side][n % 10], roman[(n / 10).min(4)]);
        self.names_used[side] += 1;
        let id = self.next_id;
        self.next_id += 1;
        self.fleets.push(Fleet {
            id,
            side,
            admiral: Admiral { name, rank: 0, xp: 0, traits: Vec::new() },
            ships,
            damaged: 0,
            loc: Location::At(at),
            path: Vec::new(),
            prev: at,
            shape: FleetShape::Line,
            unsupplied: 0,
        });
        id
    }

    pub fn fleet(&self, id: u32) -> Option<&Fleet> {
        self.fleets.iter().find(|f| f.id == id)
    }

    pub fn fleet_mut(&mut self, id: u32) -> Option<&mut Fleet> {
        self.fleets.iter_mut().find(|f| f.id == id)
    }

    pub fn fleets_at(&self, s: SysId, side: usize) -> impl Iterator<Item = &Fleet> {
        self.fleets.iter().filter(move |f| f.side == side && f.at() == Some(s))
    }

    /// Ships each side has in fleets.
    pub fn ships(&self, side: usize) -> u32 {
        self.fleets.iter().filter(|f| f.side == side).map(|f| f.ships).sum()
    }

    pub fn owned(&self, side: usize) -> impl Iterator<Item = SysId> + '_ {
        self.systems.iter().enumerate().filter(move |(_, s)| s.owner == Some(side)).map(|(i, _)| i)
    }

    pub fn note(&mut self, text: impl Into<String>) {
        self.log.push(text.into());
    }
}

/// Merges a side's fleets standing in the same system while they fit under
/// the cap: the smaller joins the larger, and its admiral returns home to
/// await a new command.
pub fn merge_at_will(c: &mut Campaign, side: usize) {
    loop {
        let mut pair = None;
        'outer: for a in c.fleets.iter().filter(|f| f.side == side) {
            for b in c.fleets.iter().filter(|f| f.side == side && f.id != a.id) {
                if a.at().is_some() && a.at() == b.at() && a.ships >= b.ships && a.ships + b.ships <= FLEET_CAP && b.ships < 80 {
                    pair = Some((a.id, b.id));
                    break 'outer;
                }
            }
        }
        let Some((a, b)) = pair else { break };
        merge(c, a, b);
    }
}

/// Folds fleet `b` into fleet `a` (same system).
pub fn merge(c: &mut Campaign, a: u32, b: u32) {
    let Some(fb) = c.fleet(b).cloned() else { return };
    if let Some(fa) = c.fleet_mut(a) {
        let room = FLEET_CAP.saturating_sub(fa.ships);
        let moved = fb.ships.min(room);
        fa.ships += moved;
        fa.damaged += fb.damaged.min(moved);
        let rest = fb.ships - moved;
        if rest > 0 {
            if let Some(f) = c.fleet_mut(b) {
                f.ships = rest;
                f.damaged = f.damaged.min(rest);
            }
            return;
        }
    }
    c.fleets.retain(|f| f.id != b);
}

/// A new fleet from the garrison of system `s` (player action).
pub fn raise_fleet(c: &mut Campaign, side: usize, s: SysId) -> Option<u32> {
    let keep = if super::map::galaxy().systems[s].kind == super::map::Kind::Capital { 40 } else { 0 };
    let spare = c.systems[s].garrison.saturating_sub(keep).min(FLEET_CAP);
    if c.systems[s].owner != Some(side) || spare < 20 || c.fleets.iter().filter(|f| f.side == side).count() >= MAX_FLEETS {
        return None;
    }
    c.systems[s].garrison -= spare;
    Some(c.add_fleet(side, s, spare))
}

/// A fleet takes on garrison ships where it stands, up to the cap.
pub fn absorb_garrison(c: &mut Campaign, id: u32) {
    let Some(f) = c.fleet(id).cloned() else { return };
    let Some(s) = f.at() else { return };
    if c.systems[s].owner != Some(f.side) {
        return;
    }
    let keep = if super::map::galaxy().systems[s].kind == super::map::Kind::Capital { 40 } else { 0 };
    let take = c.systems[s].garrison.saturating_sub(keep).min(FLEET_CAP.saturating_sub(f.ships));
    c.systems[s].garrison -= take;
    c.fleet_mut(id).unwrap().ships += take;
}
