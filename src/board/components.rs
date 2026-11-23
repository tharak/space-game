//! ECS components for a squadron. Every squadron is one entity carrying
//! these; the rules and the renderer both read them directly.

use bevy::prelude::*;

use crate::board::hex::Hex;

/// Marks an entity as a squadron. `order` is the global spawn order, which
/// the rules use for deterministic iteration and tie-breaks.
#[derive(Component, Clone, Copy, Debug)]
pub struct Squadron {
    pub order: u32,
}

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct HexPos(pub Hex);

/// Facing 0..5, see `hex::DIR_ANGLE`.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Facing(pub u8);

/// 0 = Blue, 1 = Red.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Side(pub usize);

/// Remaining strength 0..4 (dice thrown and hits absorbed).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Strength(pub u8);

#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Morale {
    #[default]
    Steady,
    Shaken,
    Routed,
}

impl Morale {
    pub fn name(self) -> &'static str {
        match self {
            Morale::Steady => "Steady",
            Morale::Shaken => "Shaken",
            Morale::Routed => "ROUTED",
        }
    }
}

/// Display id such as `B3`.
#[derive(Component, Clone, Debug)]
pub struct Callsign(pub String);

#[derive(Component, Clone, Copy, Debug)]
pub struct Flagship;

/// Present while the squadron is on the board. Removed when it is destroyed
/// or flees off the map (the entity stays so losses can be counted).
#[derive(Component, Clone, Copy, Debug)]
pub struct Alive;

/// Has activated this turn.
#[derive(Component, Clone, Copy, Debug)]
pub struct Activated;

/// Took a hit since its last activation (blocks a routed unit's rally).
#[derive(Component, Clone, Copy, Debug)]
pub struct HitSinceAct;
