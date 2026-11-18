//! The strategic campaign: a galaxy of star systems joined by lanes, two
//! powers, fleets under admirals whose ship counts persist from battle to
//! battle, supply lines, ship income from held systems, and a rival AI.
//! Turn-based and headless; the battles themselves are the `sim`.

// Systems are indexed by id across several parallel vectors.
#![allow(clippy::needless_range_loop)]

pub mod ai;
pub mod battle;
pub mod map;
pub mod save;
pub mod state;
pub mod turn;
#[cfg(test)]
mod tests;

pub use state::Campaign;

use crate::sim::battle::{Control, SimSettings};
use crate::sim::simulate;

/// The sim settings for a pending battle (AI sides pick their counters).
pub fn battle_settings(c: &Campaign, b: &state::PendingBattle, shapes: [Option<crate::sim::scenario::FleetShape>; 2], control: Control) -> SimSettings {
    SimSettings { forces: Some(battle::forces_for(c, b, shapes)), control, ..Default::default() }
}

/// Resolves every pending battle with the headless sim.
pub fn auto_resolve_all(c: &mut Campaign) {
    while let Some(b) = c.pending.first().cloned() {
        let settings = battle_settings(c, &b, [None, None], Control::Spectate);
        let report = simulate(settings, battle::battle_seed(c, &b), None);
        battle::apply_report(c, b.id, &report);
    }
}

/// Summary of an AI-vs-AI campaign.
#[derive(Clone, Debug)]
pub struct CampaignResult {
    pub winner: Option<usize>,
    pub turns: u32,
    pub battles: u32,
}

/// Plays a whole campaign with the AI on both sides.
pub fn run_ai_campaign(seed: u32, max_turns: u32) -> CampaignResult {
    let mut c = Campaign::new(seed);
    ai::orders(&mut c, 0);
    ai::orders(&mut c, 1);
    while c.winner.is_none() && c.turn <= max_turns {
        turn::end_turn(&mut c);
        auto_resolve_all(&mut c);
        turn::finish_turn(&mut c, [true, true]);
    }
    CampaignResult { winner: c.winner, turns: c.turn - 1, battles: c.battles_fought }
}
