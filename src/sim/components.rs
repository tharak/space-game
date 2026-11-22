//! ECS data for the fleet battle. Every ship is an entity; every division is
//! an entity its ships point at. Positions are in battle units (one ship
//! spacing is about 14), angles in radians with 0 = +x and positive = left.

use bevy::prelude::*;

use super::formation::Formation;

/// Blue (0) or Red (1). Ships of different sides fight; a third side (2)
/// fights both but not itself (prototype01's independents).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Side(pub usize);

#[derive(Component, Clone, Copy, Debug)]
pub struct Ship {
    pub hull: f32,
    pub max_hull: f32,
}

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Pos(pub Vec2);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Vel(pub Vec2);

/// Where the ship points. Its guns fire inside a cone around this.
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Heading(pub f32);

#[derive(Component, Clone, Copy, Debug)]
pub struct InDivision(pub Entity);

/// The ship's place in its division's formation, in the division's frame
/// (+x forward, +y left).
#[derive(Component, Clone, Copy, Debug)]
pub struct Slot(pub Vec2);

#[derive(Component, Clone, Copy, Debug, Default)]
pub struct Weapon {
    /// Seconds until it can fire again.
    pub cooldown: f32,
    /// Direction to the target it last engaged, for turning toward it.
    pub aim: Option<Vec2>,
}

/// The fleet commander's ship. Losing it shakes the whole fleet.
#[derive(Component, Clone, Copy, Debug)]
pub struct FleetFlagship;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Morale {
    #[default]
    Steady,
    /// Fires slower and will not advance.
    Shaken,
    /// Runs for its home edge and no longer fights; counts as lost.
    Routed,
}

impl Morale {
    pub fn name(self) -> &'static str {
        match self {
            Morale::Steady => "Steady",
            Morale::Shaken => "Shaken",
            Morale::Routed => "Routed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Order {
    /// Stay where you are, turn to the given facing.
    Hold,
    /// Keep going straight ahead.
    Advance,
    /// Go to a point and face a direction.
    MoveTo { to: Vec2, facing: f32 },
    /// Close to firing range of an enemy division and face it.
    Engage(Entity),
    /// Advance at the charge, ignoring the urge to stop at firing range.
    Charge,
    /// Fall back toward the home edge, still facing the enemy.
    Withdraw,
}

impl Order {
    pub fn name(&self) -> &'static str {
        match self {
            Order::Hold => "Hold",
            Order::Advance => "Advance",
            Order::MoveTo { .. } => "Move",
            Order::Engage(_) => "Engage",
            Order::Charge => "Charge",
            Order::Withdraw => "Withdraw",
        }
    }
}

#[derive(Component, Clone, Debug)]
pub struct Division {
    pub side: usize,
    /// 1-based number within its fleet, for labels.
    pub number: usize,
    pub formation: Formation,
    /// Formation centre the ships steer around.
    pub anchor: Vec2,
    pub facing: f32,
    pub order: Order,
    pub morale: Morale,
    /// Ships at the start of the battle and still alive now.
    pub start: u32,
    pub alive: u32,
    /// Slots in the current layout (alive can be lower after losses).
    pub slots: u32,
    /// Recent losses as a fraction of `start`; decays over time.
    pub shock: f32,
    /// Holds the fleet flagship.
    pub flagship: bool,
    /// Rear-most slot x and half the formation's width, for breakthroughs.
    pub depth: f32,
    pub half_width: f32,
    /// Seconds since one of its ships last fired or was hit.
    pub since_contact: f32,
    /// Roughly how close its nearest ship is to an enemy (from gun scans;
    /// drifts upward when nobody is in range).
    pub closest: f32,
}

impl Division {
    pub fn label(&self) -> String {
        format!("{}{}", if self.side == 0 { 'B' } else { 'R' }, self.number)
    }

    pub fn forward(&self) -> Vec2 {
        Vec2::from_angle(self.facing)
    }

    /// Division frame -> world.
    pub fn to_world(&self, local: Vec2) -> Vec2 {
        self.anchor + Vec2::from_angle(self.facing).rotate(local)
    }

    /// World -> division frame.
    pub fn to_local(&self, world: Vec2) -> Vec2 {
        Vec2::from_angle(-self.facing).rotate(world - self.anchor)
    }
}

pub fn side_name(side: usize) -> &'static str {
    if side == 0 { "Blue" } else { "Red" }
}

/// Smallest signed difference `to - from`, in (-PI, PI].
pub fn angle_diff(from: f32, to: f32) -> f32 {
    let d = (to - from).rem_euclid(std::f32::consts::TAU);
    if d > std::f32::consts::PI { d - std::f32::consts::TAU } else { d }
}

/// Turns `from` toward `to` by at most `max`.
pub fn turn_toward(from: f32, to: f32, max: f32) -> f32 {
    let d = angle_diff(from, to);
    from + d.clamp(-max, max)
}
