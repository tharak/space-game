//! The galaxy: two powers on either side of an impassable nebula, joined
//! only by two corridors — one held by a fortress, one by a neutral trade
//! world. The map is hand-made and mirror-symmetric so neither side starts
//! ahead.

use serde::{Deserialize, Serialize};

pub type SysId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Normal,
    /// Taking it ends the campaign.
    Capital,
    /// A fortress: big garrison that regrows every turn.
    Fortress,
    /// A neutral trade world: rich, lightly held.
    Hub,
}

#[derive(Clone, Copy, Debug)]
pub struct SystemDef {
    pub name: &'static str,
    pub pos: (f32, f32),
    /// Ships it adds to its owner every turn.
    pub value: u32,
    pub kind: Kind,
    pub owner: Option<usize>,
    pub garrison: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lane {
    pub a: SysId,
    pub b: SysId,
    /// Turns to cross.
    pub turns: u32,
}

pub const POWERS: [&str; 2] = ["Meridian Compact", "Hesper Throne"];
pub const NEBULA_HALF_WIDTH: f32 = 170.0;

/// One side's nine systems, in the frame of the Blue (west) side; Red's are
/// the mirror image.
const HALF: [(&str, &str, (f32, f32), u32, Kind); 9] = [
    ("Meridian", "Hesper", (-950.0, 0.0), 3, Kind::Capital),
    ("Caldera", "Vantor", (-760.0, 270.0), 2, Kind::Normal),
    ("Lowen", "Ilmar", (-760.0, -260.0), 2, Kind::Normal),
    ("Orrin", "Corve", (-560.0, 440.0), 1, Kind::Normal),
    ("Pale Harbor", "Duskfall", (-560.0, 70.0), 2, Kind::Normal),
    ("Sabre Drift", "Mereth", (-560.0, -400.0), 1, Kind::Normal),
    ("Aster Reach", "Kestrel", (-330.0, 450.0), 2, Kind::Normal),
    ("Hollis", "Ashgrave", (-330.0, 40.0), 1, Kind::Normal),
    ("Brine", "Lantern", (-330.0, -440.0), 2, Kind::Normal),
];

const HALF_LANES: [(usize, usize); 12] =
    [(0, 1), (0, 2), (0, 4), (1, 3), (1, 4), (2, 4), (2, 5), (3, 6), (4, 7), (6, 7), (7, 8), (5, 8)];

pub const FORTRESS: SysId = 18;
pub const HUB: SysId = 19;
/// Each side's systems at the mouths of the two corridors: (north, south).
pub const MOUTHS: [[SysId; 2]; 2] = [[6, 8], [15, 17]];
pub const CAPITALS: [SysId; 2] = [0, 9];

pub struct Galaxy {
    pub systems: Vec<SystemDef>,
    pub lanes: Vec<Lane>,
}

impl Galaxy {
    pub fn new() -> Self {
        let mut systems = Vec::new();
        for side in 0..2 {
            for (blue, red, (x, y), value, kind) in HALF {
                let pos = if side == 0 { (x, y) } else { (-x, y) };
                let garrison = if kind == Kind::Capital { 60 } else { 10 + 5 * value };
                systems.push(SystemDef { name: if side == 0 { blue } else { red }, pos, value, kind, owner: Some(side), garrison });
            }
        }
        systems.push(SystemDef { name: "Vigil Gate", pos: (0.0, 470.0), value: 1, kind: Kind::Fortress, owner: None, garrison: 150 });
        systems.push(SystemDef { name: "Tollmarch", pos: (0.0, -460.0), value: 3, kind: Kind::Hub, owner: None, garrison: 40 });
        let mut lanes = Vec::new();
        for side in 0..2 {
            for (a, b) in HALF_LANES {
                lanes.push(Lane { a: a + 9 * side, b: b + 9 * side, turns: 1 });
            }
        }
        // Crossing the nebula takes two turns each way.
        for side in 0..2 {
            lanes.push(Lane { a: MOUTHS[side][0], b: FORTRESS, turns: 2 });
            lanes.push(Lane { a: MOUTHS[side][1], b: HUB, turns: 2 });
        }
        Self { systems, lanes }
    }

    pub fn lane(&self, a: SysId, b: SysId) -> Option<Lane> {
        self.lanes.iter().copied().find(|l| (l.a == a && l.b == b) || (l.a == b && l.b == a))
    }

    pub fn neighbours(&self, s: SysId) -> impl Iterator<Item = SysId> + '_ {
        self.lanes.iter().filter_map(move |l| {
            if l.a == s {
                Some(l.b)
            } else if l.b == s {
                Some(l.a)
            } else {
                None
            }
        })
    }

    /// Shortest path in turns from `from` to `to` (excluding `from`), only
    /// through systems `passable` allows (the destination always is).
    pub fn path(&self, from: SysId, to: SysId, passable: impl Fn(SysId) -> bool) -> Option<Vec<SysId>> {
        let n = self.systems.len();
        let mut dist = vec![u32::MAX; n];
        let mut prev = vec![usize::MAX; n];
        let mut done = vec![false; n];
        dist[from] = 0;
        while let Some(u) = (0..n).filter(|i| !done[*i] && dist[*i] < u32::MAX).min_by_key(|i| (dist[*i], *i)) {
            done[u] = true;
            if u == to {
                break;
            }
            if u != from && !passable(u) {
                continue;
            }
            for l in &self.lanes {
                let v = if l.a == u {
                    l.b
                } else if l.b == u {
                    l.a
                } else {
                    continue;
                };
                if dist[u] + l.turns < dist[v] {
                    dist[v] = dist[u] + l.turns;
                    prev[v] = u;
                }
            }
        }
        if dist[to] == u32::MAX || from == to {
            return None;
        }
        let mut path = vec![to];
        let mut at = to;
        while prev[at] != from {
            at = prev[at];
            path.push(at);
        }
        path.reverse();
        Some(path)
    }

    /// Lane-hops between two systems ignoring ownership.
    pub fn hops(&self, from: SysId, to: SysId) -> u32 {
        if from == to {
            return 0;
        }
        self.path(from, to, |_| true).map_or(u32::MAX, |p| p.len() as u32)
    }
}

impl Default for Galaxy {
    fn default() -> Self {
        Self::new()
    }
}

/// The one galaxy, built once.
pub fn galaxy() -> &'static Galaxy {
    static G: std::sync::OnceLock<Galaxy> = std::sync::OnceLock::new();
    G.get_or_init(Galaxy::new)
}
