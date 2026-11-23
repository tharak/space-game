//! Successor Stars: a real-time fleet battle with hundreds of ships.
//!
//! * [`board`]: the hex-board rules prototype (BattleStars v0.2.1), headless.
//! * [`sim`]: the headless fleet-battle simulation (ECS), its AI and rules.
//! * [`campaign`]: the turn-based strategic layer (galaxy, fleets, supply, AI).
//! * [`game`]: the Bevy app (fleet view, camera, orders UI, input) on top.

// Bevy system signatures are long by nature.
#![allow(clippy::type_complexity)]

pub mod board;
pub mod campaign;
pub mod game;
pub mod prototypes;
pub mod sim;
pub mod voronoi;

