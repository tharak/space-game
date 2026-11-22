//! Drives the battle: starts and restarts it, applies orders, steps the
//! simulation at the chosen speed, and turns sim events into log lines.

use bevy::ecs::message::Messages;
use bevy::prelude::*;

use super::camera::CameraRig;
use super::campaign_ui::{self, CampaignState};
use super::input::Selection;
use crate::sim::battle::{Battle, Control, EndReason, SimEvent, SimSettings};
use crate::sim::components::{Morale, side_name};
use crate::sim::scenario::SCENARIOS;
use crate::sim::{Command, apply_command, init_world, step};

#[derive(Resource, Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Phase {
    /// The start page: shared config and the prototypes.
    #[default]
    Start,
    /// The Voronoi labs page.
    Labs,
    /// The ECS systems page.
    Ecs,
    /// The fleet battle and campaign menu.
    Menu,
    Battle,
    Over,
    /// The galaxy map.
    Campaign,
    /// The card before a campaign battle: fight, auto-resolve or withdraw.
    PreBattle,
    /// A campaign battle being auto-resolved.
    Resolving,
}

impl Phase {
    pub fn in_battle(self) -> bool {
        matches!(self, Phase::Battle | Phase::Over)
    }
}

/// Simulation speed: ticks per fixed step (1 = real time), or paused.
#[derive(Resource, Clone, Copy, Debug)]
pub struct Speed {
    pub paused: bool,
    pub mult: u32,
}

impl Default for Speed {
    fn default() -> Self {
        Self { paused: true, mult: 1 }
    }
}

#[derive(Message, Clone, Copy, PartialEq, Eq, Debug)]
pub enum AppRequest {
    Start,
    /// Back to the start page.
    Home,
    /// The Voronoi labs page.
    LabsPage,
    /// The ECS systems page.
    EcsPage,
    Menu,
    NewCampaign,
    ContinueCampaign,
    EndTurn,
    ReturnToCampaign,
    Fight,
    AutoResolve,
    Withdraw,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    Plain,
    Title,
    Bad,
    Good,
    Side(usize),
}

#[derive(Resource, Default)]
pub struct OverlayHidden(pub bool);

#[derive(Resource, Default)]
pub struct LogBook {
    pub lines: Vec<(String, Tone)>,
    /// Bumped when the log is cleared, so the view can reset.
    pub generation: u64,
}

impl LogBook {
    pub fn push(&mut self, text: impl Into<String>, tone: Tone) {
        self.lines.push((text.into(), tone));
    }
}

/// Seed source for new battles; replaced by a fixed seed in tests/autoplay.
#[derive(Resource)]
pub struct SeedSource(pub u32);

pub fn apply_requests(world: &mut World) {
    let requests: Vec<AppRequest> = world.resource_mut::<Messages<AppRequest>>().drain().collect();
    for r in requests {
        match r {
            AppRequest::Start => {
                world.resource_mut::<CampaignState>().battle = None;
                world.resource_mut::<SimSettings>().forces = None;
                start_battle(world)
            }
            AppRequest::Home => *world.resource_mut::<Phase>() = Phase::Start,
            AppRequest::LabsPage => *world.resource_mut::<Phase>() = Phase::Labs,
            AppRequest::EcsPage => *world.resource_mut::<Phase>() = Phase::Ecs,
            AppRequest::Menu => {
                *world.resource_mut::<Phase>() = Phase::Menu;
                world.resource_mut::<super::ui::SaveExists>().0 = crate::campaign::save::load().is_some();
            }
            AppRequest::NewCampaign => {
                campaign_ui::new_campaign(world);
                world.resource_mut::<super::ui::SaveExists>().0 = true;
            }
            AppRequest::ContinueCampaign => campaign_ui::continue_campaign(world),
            AppRequest::EndTurn => campaign_ui::end_player_turn(world),
            AppRequest::ReturnToCampaign => campaign_ui::return_to_campaign(world),
            AppRequest::Fight => campaign_ui::prebattle_fight(world),
            AppRequest::AutoResolve => campaign_ui::prebattle_auto(world),
            AppRequest::Withdraw => campaign_ui::prebattle_withdraw(world),
        }
    }
    let commands: Vec<Command> = world.resource_mut::<Messages<Command>>().drain().collect();
    if *world.resource::<Phase>() == Phase::Battle {
        for c in commands {
            apply_command(world, c);
        }
    }
}

pub fn start_battle(world: &mut World) {
    start_battle_with(world, None);
}

/// Starts a battle from the current `SimSettings`; `intro` replaces the
/// scenario's log lines (campaign battles).
pub fn start_battle_with(world: &mut World, intro: Option<Vec<String>>) {
    let seed = {
        let mut s = world.resource_mut::<SeedSource>();
        s.0 = s.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        s.0
    };
    let settings = *world.resource::<SimSettings>();
    init_world(world, settings, seed);
    *world.resource_mut::<Phase>() = Phase::Battle;
    world.resource_mut::<OverlayHidden>().0 = false;
    world.resource_mut::<CameraRig>().fit_pending = true;
    *world.resource_mut::<Selection>() = Selection::default();
    // Players start paused so they can look and give orders first.
    *world.resource_mut::<Speed>() = Speed { paused: settings.control != Control::Spectate, mult: 1 };
    let scenario = SCENARIOS[settings.scenario];
    let start = world.resource::<Battle>().start;
    let mut log = world.resource_mut::<LogBook>();
    log.lines.clear();
    log.generation += 1;
    if let Some(lines) = intro {
        let mut lines = lines.into_iter();
        if let Some(first) = lines.next() {
            log.push(first, Tone::Title);
        }
        for l in lines {
            log.push(l, Tone::Plain);
        }
    } else if let Some(k) = settings.skirmish {
        log.push(format!("{} — {} ships a side", k.title(), start[0]), Tone::Title);
        log.push(
            format!("{} squadrons of {} ships each, plus the flagship's squadron in reserve.", k.squadrons, k.per_squadron),
            Tone::Plain,
        );
        log.push(format!("Blue: {} · Red: {}", k.shapes[0].name(), k.shapes[1].name()), Tone::Plain);
    } else {
        log.push(format!("{} — {} ships a side", scenario.title, start[0]), Tone::Title);
        log.push(scenario.blurb, Tone::Plain);
        let (a, b) = if settings.mirror { (scenario.fleets[1], scenario.fleets[0]) } else { (scenario.fleets[0], scenario.fleets[1]) };
        log.push(format!("Blue: {} · Red: {}", a.name(), b.name()), Tone::Plain);
    }
    log.push("No break point: as a fleet wears down its divisions waver and rout; the battle ends when one side has nothing left fighting.", Tone::Plain);
    if settings.control != Control::Spectate {
        log.push("Paused — give orders, then press Space or Resume.", Tone::Good);
    }
}

/// Runs in `FixedUpdate` (one step = `DT` of battle time at 1×).
pub fn run_sim(world: &mut World) {
    if *world.resource::<Phase>() != Phase::Battle {
        return;
    }
    let speed = *world.resource::<Speed>();
    if speed.paused {
        return;
    }
    for _ in 0..speed.mult {
        step(world);
        if world.resource::<Battle>().over() {
            *world.resource_mut::<Phase>() = Phase::Over;
            break;
        }
    }
}

pub fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn log_events(mut events: MessageReader<SimEvent>, mut log: ResMut<LogBook>, battle: Res<Battle>) {
    let t = clock(battle.time);
    for event in events.read() {
        let (text, tone) = match event {
            SimEvent::Shot { .. } => continue,
            SimEvent::Destroyed { flagship: false, .. } => continue,
            SimEvent::Destroyed { flagship: true, side, .. } => (format!("{t} {} flagship destroyed!", side_name(*side)), Tone::Bad),
            SimEvent::Contact => (format!("{t} First shots exchanged."), Tone::Title),
            SimEvent::MoraleChanged { side, division, to, .. } => match to {
                Morale::Shaken => (format!("{t} {division} is shaken"), Tone::Bad),
                Morale::Routed => (format!("{t} {division} ROUTS"), Tone::Bad),
                Morale::Steady => (format!("{t} {division} rallies"), Tone::Good),
            }
            .into_side(*side),
            SimEvent::FlagshipLost { side } => {
                (format!("{t} {} has lost its flagship — the whole fleet wavers", side_name(*side)), Tone::Bad)
            }
            SimEvent::Breakthrough { side, division, through } => {
                (format!("{t} {division} breaks through {through}!"), Tone::Side(*side))
            }
            SimEvent::Over { winner, reason } => {
                let text = match (winner, reason) {
                    (Some(w), EndReason::Broke) => format!("{t} {} breaks. {} wins.", side_name(1 - w), side_name(*w)),
                    (Some(w), EndReason::TimeLimit) => format!("{t} Time. {} holds more of the field.", side_name(*w)),
                    (Some(w), EndReason::Disengaged) => {
                        format!("{t} The fleets drift apart, exhausted. {} holds the field.", side_name(*w))
                    }
                    (None, EndReason::Disengaged) => format!("{t} The fleets drift apart, exhausted: draw."),
                    (None, _) => format!("{t} Time. Neither side gave way: draw."),
                };
                (text, Tone::Title)
            }
        };
        log.push(text, tone);
    }
}

trait IntoSide {
    fn into_side(self, side: usize) -> (String, Tone);
}

impl IntoSide for (String, Tone) {
    /// Good news for a side is told in its colour, bad news stays orange.
    fn into_side(self, side: usize) -> (String, Tone) {
        match self.1 {
            Tone::Good => (self.0, Tone::Side(side)),
            _ => self,
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod clipboard {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = "export function copy_text(t) { if (navigator.clipboard) { navigator.clipboard.writeText(t).catch(() => {}); } }")]
    extern "C" {
        pub fn copy_text(text: &str);
    }
}

pub fn copy_to_clipboard(text: &str) {
    #[cfg(target_arch = "wasm32")]
    clipboard::copy_text(text);
    #[cfg(not(target_arch = "wasm32"))]
    println!("{text}");
}
