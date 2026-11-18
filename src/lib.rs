//! Successor Stars: a real-time fleet battle with hundreds of ships.
//!
//! * [`sim`]: the headless fleet-battle simulation (ECS), its AI and rules.
//! * [`campaign`]: the turn-based strategic layer (galaxy, fleets, supply, AI).
//! * [`game`]: the Bevy app (fleet view, camera, orders UI, input) on top.

// Bevy system signatures are long by nature.
#![allow(clippy::type_complexity)]

pub mod campaign;
pub mod game;
pub mod sim;

