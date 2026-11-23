//! The hex board: the BattleStars v0.2.1 battle rules (the one design that
//! passed a playtest), headless on a Bevy `World`. Squadrons are entities,
//! the battle lives in resources, and every rule is a plain function on the
//! world, so the board page, the tests and [`simulate`] share the code.
//!
//! * [`hex`]: odd-r offset hex geometry, arcs and line of sight.
//! * [`battle`]: movement, fire, morale, rout, flagship loss, turn order.
//! * [`commands`]: player intent (turn, move, fire, select a hex).
//! * [`ai`]: the doctrine and baseline AIs.

pub mod ai;
pub mod battle;
pub mod commands;
pub mod components;
pub mod config;
pub mod dice;
pub mod formations;
pub mod hex;

#[cfg(test)]
mod tests;

use bevy::ecs::message::Messages;
use bevy::prelude::World;

use battle::*;

/// Runs a whole AI-vs-AI battle headlessly and returns the winner
/// (`None` = draw) and the turn it ended on.
pub fn simulate(settings: MatchSettings, seed: u32) -> (Option<usize>, u32) {
    let mut world = World::new();
    let settings = MatchSettings { control: config::ControlMode::Spectate, ..settings };
    init_world(&mut world, settings, dice::Dice::seeded(seed));
    new_battle(&mut world);
    loop {
        match advance(&mut world) {
            Next::Over => break,
            Next::Ai(side) => ai::activate_next(&mut world, side),
            Next::Human(_) => unreachable!("spectate has no human sides"),
        }
        world.resource_mut::<Messages<BattleEvent>>().clear();
    }
    let battle = world.resource::<Battle>();
    (battle.result.as_ref().and_then(|r| r.winner), battle.turn)
}
