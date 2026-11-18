//! Successor Stars: a hex fleet-battle rules prototype.
//!
//! * [`hex`]: pure hex geometry.
//! * [`rules`]: the headless ECS battle rules, commands and AI.
//! * [`game`]: the Bevy app (board rendering, UI, input) built on the rules.

pub mod game;
pub mod hex;
pub mod rules;

/// Runs a whole AI-vs-AI battle headlessly and returns the winner
/// (`None` = draw). Used by the `sim` binary and tests.
pub fn simulate(settings: rules::battle::MatchSettings, seed: u32) -> (Option<usize>, u32) {
    use bevy::prelude::World;
    use rules::battle::*;
    let mut world = World::new();
    let settings = MatchSettings { control: rules::config::ControlMode::Spectate, ..settings };
    init_world(&mut world, settings, rules::dice::Dice::seeded(seed));
    new_battle(&mut world);
    loop {
        match advance(&mut world) {
            Next::Over => break,
            Next::Ai(side) => rules::ai::activate_next(&mut world, side),
            Next::Human(_) => unreachable!("spectate has no human sides"),
        }
        world.resource_mut::<bevy::ecs::message::Messages<BattleEvent>>().clear();
    }
    let battle = world.resource::<Battle>();
    (battle.result.as_ref().and_then(|r| r.winner), battle.turn)
}
