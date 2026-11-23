//! Rule constants and scenario data for Fleet Battle Rules v0.2.1
//! (see `docs/battle-rules.md`).

use crate::board::hex::{self, Hex};

pub const RANGE: i32 = 3;
pub const CMD_R: i32 = 4;
pub const MP_MAX: u8 = 3;
pub const MAX_TURNS: u32 = 15;
pub const MAX_STRENGTH: u8 = 4;

/// The board is a hexagon: every hex within `BOARD_RADIUS` of `BOARD_CENTER`.
/// COLS/ROWS are only the bounding box.
pub const COLS: i32 = 27;
pub const ROWS: i32 = 27;
pub const BOARD_CENTER: Hex = Hex::new(13, 13);
pub const BOARD_RADIUS: i32 = 13;

pub fn in_bounds(h: Hex) -> bool {
    hex::dist(BOARD_CENTER, h) <= BOARD_RADIUS
}

pub fn board_hexes() -> impl Iterator<Item = Hex> {
    (0..ROWS).flat_map(|r| (0..COLS).map(move |c| Hex::new(c, r))).filter(|h| in_bounds(*h))
}

/// Anchor columns formation offsets deploy around (Blue, Red) and the row
/// lateral offsets are centred on.
pub const DEPLOY_ANCHOR: [i32; 2] = [7, 19];
pub const DEPLOY_ROW_CENTER: i32 = 13;

/// Manual deployment zones (inclusive column ranges), one per side.
pub const SETUP_ZONE: [(i32, i32); 2] = [(0, 9), (17, COLS - 1)];

pub fn in_setup_zone(side: usize, h: Hex) -> bool {
    let (lo, hi) = SETUP_ZONE[side];
    h.c >= lo && h.c <= hi && in_bounds(h)
}

pub fn side_name(side: usize) -> &'static str {
    if side == 0 { "Blue" } else { "Red" }
}

/// Facing a routed squadron is forced into: toward its own map edge.
pub fn home_edge_dir(side: usize) -> u8 {
    if side == 0 { 3 } else { 0 }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Supply {
    #[default]
    Ok,
    Low,
    Critical,
}

impl Supply {
    pub fn name(self) -> &'static str {
        match self {
            Supply::Ok => "ok",
            Supply::Low => "low",
            Supply::Critical => "critical",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Formation {
    Line,
    Spindle,
    Crescent,
    Echelon,
    Sphere,
    Column,
    Custom,
}

impl Formation {
    pub const STANDARD: [Formation; 6] = [
        Formation::Line,
        Formation::Spindle,
        Formation::Crescent,
        Formation::Echelon,
        Formation::Sphere,
        Formation::Column,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Formation::Line => "line",
            Formation::Spindle => "spindle",
            Formation::Crescent => "crescent",
            Formation::Echelon => "echelon",
            Formation::Sphere => "sphere",
            Formation::Column => "column",
            Formation::Custom => "custom",
        }
    }

    /// Formations whose AI holds position instead of advancing.
    pub fn holds(self) -> bool {
        self == Formation::Sphere
    }
}

pub struct Scenario {
    pub title: &'static str,
    pub a: Formation,
    pub b: Formation,
    pub supply_a: Supply,
    pub supply_b: Supply,
    pub note: &'static str,
}

const fn scen(title: &'static str, a: Formation, b: Formation, supply_b: Supply, note: &'static str) -> Scenario {
    Scenario { title, a, b, supply_a: Supply::Ok, supply_b, note }
}

use Formation as F;
pub const SCENARIOS: [Scenario; 9] = [
    scen("Spindle vs Wide Line", F::Spindle, F::Line, Supply::Ok,
        "THE key test. The baseline AI loses this 26/74 — it can't exploit a breakthrough. Can you pierce the line and win with maneuver?"),
    scen("Wide Line vs Crescent", F::Line, F::Crescent, Supply::Ok,
        "The wheel predicts the line's massed arcs win; the sim says crescent 64/36. Settle it."),
    scen("Crescent vs Spindle", F::Crescent, F::Spindle, Supply::Ok,
        "Wrap the wedge, rake its flanks. Sim: crescent 55/45 with dumb hands."),
    scen("Echelon vs Spindle", F::Echelon, F::Spindle, Supply::Ok,
        "Refuse a flank, deflect the punch into a flank trade. Sim: echelon 64/36."),
    scen("Sphere vs Crescent", F::Sphere, F::Crescent, Supply::Ok,
        "Survival formation in the wrong context on purpose. Score = turns survived, not wins."),
    scen("Wide Line vs Column (sanity)", F::Line, F::Column, Supply::Ok,
        "Travel order should be slaughtered. If Column ever wins, something is broken."),
    scen("Low-supply mirror", F::Line, F::Line, Supply::Low,
        "Red is low on supply (−1 morale only, guns unaffected). Sim: 66/34 — a clear but playable handicap. Can you win from behind?"),
    scen("Flagship hunt (mirror)", F::Line, F::Line, Supply::Ok,
        "Mirror match. Try decapitation: kill the enemy flagship — fleet-wide morale check, permanent −1, command radius gone."),
    scen("Critical-supply mirror", F::Line, F::Line, Supply::Critical,
        "Red is at critical supply (−1 morale AND worse to-hit). Sim: 96/3. Near-hopeless by design: never fight like this."),
];

/// Who drives each side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ControlMode {
    #[default]
    PlayBlue,
    PlayRed,
    Hotseat,
    Spectate,
}

impl ControlMode {
    pub fn human_controls(self, side: usize) -> bool {
        match self {
            ControlMode::PlayBlue => side == 0,
            ControlMode::PlayRed => side == 1,
            ControlMode::Hotseat => true,
            ControlMode::Spectate => false,
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            ControlMode::PlayBlue => "Blue",
            ControlMode::PlayRed => "Red",
            ControlMode::Hotseat => "hotseat",
            ControlMode::Spectate => "spectate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MoveMode {
    /// One squadron at a time, alternating sides.
    #[default]
    Interleaved,
    /// Each side activates every squadron before the other side moves.
    AllAtOnce,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeployMode {
    #[default]
    Manual,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AiKind {
    /// Positional doctrine AI: keeps its flagship back, faces threats,
    /// prefers flank shots and kills.
    #[default]
    Doctrine,
    /// The original advance-and-shoot logic used by the Monte-Carlo sim.
    Baseline,
}

impl AiKind {
    pub fn name(self) -> &'static str {
        match self {
            AiKind::Doctrine => "doctrine",
            AiKind::Baseline => "baseline",
        }
    }
}
