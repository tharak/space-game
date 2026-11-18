//! Battle-wide resources: settings, tuning constants, score-keeping and the
//! events the simulation publishes.

use bevy::prelude::*;

use super::components::Morale;

/// Fixed simulation step, in seconds of battle time.
pub const DT: f32 = 0.05;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Control {
    #[default]
    PlayBlue,
    PlayRed,
    Spectate,
}

impl Control {
    pub fn human_controls(self, side: usize) -> bool {
        matches!((self, side), (Control::PlayBlue, 0) | (Control::PlayRed, 1))
    }

    pub fn tag(self) -> &'static str {
        match self {
            Control::PlayBlue => "blue",
            Control::PlayRed => "red",
            Control::Spectate => "spectate",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum AiKind {
    /// Reads the battle every two seconds: pins with some divisions, flanks
    /// with others, keeps the flagship back and commits the reserve.
    #[default]
    Commander,
    /// Never changes the opening orders: the formation fights on its own.
    Advance,
}

impl AiKind {
    pub fn tag(self) -> &'static str {
        match self {
            AiKind::Commander => "commander",
            AiKind::Advance => "advance",
        }
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct SimSettings {
    pub scenario: usize,
    /// Ships per side.
    pub ships: usize,
    pub control: Control,
    pub ai: [AiKind; 2],
    /// Swap which fleet each side gets (for seat-balanced sims).
    pub mirror: bool,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self { scenario: 0, ships: 240, control: Control::PlayBlue, ai: [AiKind::Commander; 2], mirror: false }
    }
}

/// Every number that shapes how a battle plays, in one place for tuning.
#[derive(Resource, Clone, Debug)]
pub struct Tuning {
    pub spacing: f32,
    pub ship_speed: f32,
    pub anchor_speed: f32,
    pub charge_mult: f32,
    pub rout_speed: f32,
    /// Radians per second a division's formation can wheel.
    pub div_turn_rate: f32,
    pub ship_turn_rate: f32,
    /// How far a ship may turn away from its division's facing to aim.
    pub aim_limit: f32,
    pub range: f32,
    /// Half-angle of the gun cone around the ship's heading.
    pub fire_arc: f32,
    pub cooldown: f32,
    /// Hit chance into the target's front, flank and rear.
    pub hit: [f32; 3],
    /// Hit chance multiplier at maximum range (1.0 at point blank).
    pub range_falloff: f32,
    pub hull: f32,
    pub flagship_hull: f32,
    pub shaken_cooldown: f32,
    pub low_supply_fire: f32,
    /// Morale: shock half-life (s) and the stress weights and thresholds.
    pub shock_half_life: f32,
    pub k_shock: f32,
    pub k_lost: f32,
    pub k_flag_lost: f32,
    pub k_supply: f32,
    pub k_command: f32,
    pub k_breakthrough: f32,
    pub noise: f32,
    pub shake_at: f32,
    pub rout_at: f32,
    pub rally_at: f32,
    pub command_radius: f32,
    /// Engage stops at this fraction of weapon range.
    pub standoff: f32,
    pub time_limit: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        Self {
            spacing: 14.0,
            ship_speed: 20.0,
            anchor_speed: 12.0,
            charge_mult: 1.35,
            rout_speed: 24.0,
            div_turn_rate: 0.22,
            ship_turn_rate: 1.6,
            aim_limit: 0.6,
            range: 110.0,
            fire_arc: 1.05,
            cooldown: 4.0,
            hit: [0.12, 0.25, 0.4],
            range_falloff: 0.5,
            hull: 5.0,
            flagship_hull: 10.0,
            shaken_cooldown: 1.5,
            low_supply_fire: 0.92,
            shock_half_life: 15.0,
            k_shock: 1.5,
            k_lost: 0.6,
            k_flag_lost: 0.15,
            k_supply: 0.0,
            k_command: 0.1,
            k_breakthrough: 0.15,
            noise: 0.08,
            shake_at: 0.55,
            rout_at: 0.9,
            rally_at: 0.3,
            command_radius: 600.0,
            standoff: 0.8,
            time_limit: 600.0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndReason {
    Broke,
    TimeLimit,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub winner: Option<usize>,
    pub reason: EndReason,
    pub time: f32,
}

#[derive(Resource, Clone, Debug, Default)]
pub struct Battle {
    pub time: f32,
    pub start: [u32; 2],
    pub destroyed: [u32; 2],
    pub fled: [u32; 2],
    pub flag_lost: [bool; 2],
    /// Fire-rate multiplier per side (supply).
    pub fire_rate: [f32; 2],
    pub bounds: Rect,
    pub result: Option<Outcome>,
    pub first_contact: Option<f32>,
    /// (breaking division, broken division) pairs already announced.
    pub breakthroughs: Vec<(Entity, Entity)>,
    pub first_breakthrough: [Option<f32>; 2],
    pub morale_timer: f32,
    pub ai_timer: f32,
    /// Ships lost (destroyed, fled, or in a routed division) at the end.
    pub lost: [u32; 2],
}

impl Battle {
    pub fn over(&self) -> bool {
        self.result.is_some()
    }
}

#[derive(Message, Clone, Debug)]
pub enum SimEvent {
    Shot { from: Vec2, to: Vec2, side: usize, hit: bool },
    Destroyed { pos: Vec2, side: usize, flagship: bool, division: String },
    MoraleChanged { side: usize, division: String, from: Morale, to: Morale },
    FlagshipLost { side: usize },
    Breakthrough { side: usize, division: String, through: String },
    Contact,
    Over { winner: Option<usize>, reason: EndReason },
}
