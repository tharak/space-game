//! A per-frame read model of the battle for the UI: status lines, the
//! division list, the selected division and the result.

use bevy::prelude::*;

use super::flow::{Phase, Speed, clock};
use super::input::{PointerMode, Selection};
use super::theme::*;
use crate::sim::battle::{Battle, Control, EndReason, SimSettings, Tuning};
use crate::sim::components::*;
use crate::sim::scenario::SCENARIOS;
use crate::sim::spatial::SpatialHash;

pub type Spans = Vec<(String, Color)>;

#[derive(Clone, Debug, PartialEq)]
pub struct DivisionRow {
    pub entity: Entity,
    pub side: usize,
    pub number: usize,
    pub text: String,
    pub color: Color,
    pub selected: bool,
    pub gone: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResultView {
    pub title: String,
    pub body: String,
    pub line: String,
}

#[derive(Resource, Default, Clone, Debug)]
pub struct UiModel {
    pub phase: Phase,
    pub spectate: bool,
    pub paused: bool,
    pub mult: u32,
    pub status: Vec<Spans>,
    pub divisions: Vec<DivisionRow>,
    pub info: Vec<Spans>,
    /// The selected division is the player's and can take orders.
    pub can_order: bool,
    pub move_mode: bool,
    pub formation: Option<crate::sim::formation::Formation>,
    pub result: Option<ResultView>,
    pub campaign: super::campaign_ui::CampaignModel,
    /// The battle on screen belongs to the campaign.
    pub campaign_battle: bool,
    pub has_save: bool,
}

fn span(s: impl Into<String>, c: Color) -> (String, Color) {
    (s.into(), c)
}

pub fn build_model(world: &mut World) {
    let phase = *world.resource::<Phase>();
    let settings = *world.resource::<SimSettings>();
    let speed = *world.resource::<Speed>();
    let sel = *world.resource::<Selection>();
    let battle = world.resource::<Battle>().clone();
    let tuning = world.resource::<Tuning>().clone();
    let command_radius = tuning.command_radius;
    let mut m = UiModel {
        phase,
        spectate: settings.control == Control::Spectate,
        paused: speed.paused,
        mult: speed.mult,
        ..default()
    };
    m.has_save = world.resource::<super::ui::SaveExists>().0;
    if matches!(phase, Phase::Menu | Phase::Start | Phase::Labs | Phase::Ecs) {
        *world.resource_mut::<UiModel>() = m;
        return;
    }
    let campaign_title = {
        let st = world.resource::<super::campaign_ui::CampaignState>();
        st.battle.is_some().then(|| st.resolving_name.clone())
    };
    {
        let st = world.resource::<super::campaign_ui::CampaignState>();
        m.campaign_battle = st.battle.is_some();
        if super::campaign_ui::in_campaign(phase) {
            m.campaign = super::campaign_ui::build_campaign_model(st, phase);
            *world.resource_mut::<UiModel>() = m;
            return;
        }
    }
    let mut divs: Vec<(Entity, Division)> = world.query::<(Entity, &Division)>().iter(world).map(|(e, d)| (e, d.clone())).collect();
    divs.sort_by_key(|(_, d)| (d.side, d.number));
    let flags: Vec<(Vec2, usize)> =
        world.query_filtered::<(&Pos, &Side), With<FleetFlagship>>().iter(world).map(|(p, s)| (p.0, s.0)).collect();
    let flag_hull: Vec<(usize, f32, f32)> = world
        .query_filtered::<(&Side, &Ship), With<FleetFlagship>>()
        .iter(world)
        .map(|(s, ship)| (s.0, ship.hull, ship.max_hull))
        .collect();
    let hash = world.resource::<SpatialHash>();

    // Status: the clock and each fleet's strength.
    let scenario = SCENARIOS[settings.scenario];
    let title: String = campaign_title
        .clone()
        .or_else(|| settings.skirmish.map(|k| k.title()))
        .unwrap_or_else(|| scenario.title.to_string());
    let state = if phase == Phase::Over {
        "ended"
    } else if speed.paused {
        "paused"
    } else {
        ""
    };
    m.status.push(vec![
        span(format!("{title}  "), GOLD),
        span(clock(battle.time), TEXT),
        span(format!("  ×{} {state}", speed.mult), DIM),
    ]);
    for side in 0..2 {
        let fighting = battle.start[side].saturating_sub(battle.lost[side]);
        let you = if settings.control.human_controls(side) { " (you)" } else { "" };
        let pct = battle.remaining(side) * 100.0;
        let mut line = vec![
            span(format!("{}{you}: ", side_name(side)), side_color(side)),
            span(format!("{fighting}/{} fighting", battle.start[side]), TEXT),
            span(format!(" ({pct:.0}%)"), if pct < 50.0 { BAD } else { DIM }),
        ];
        match flag_hull.iter().find(|(s, _, _)| *s == side) {
            _ if battle.flag_lost[side] => line.push(span(" · flagship LOST", BAD)),
            Some((_, hull, max)) => {
                let c = if *hull <= max * 0.4 { BAD } else if *hull < *max { GOLD } else { DIM };
                line.push(span(format!(" · flagship {hull:.0}/{max:.0}"), c));
            }
            None => {}
        }
        m.status.push(line);
    }

    // Division list: the player's fleet, or both when spectating.
    for (e, d) in &divs {
        if !(m.spectate || settings.control.human_controls(d.side)) {
            continue;
        }
        let gone = d.alive == 0 || d.morale == Morale::Routed;
        let flag = if d.flagship { " ◆" } else { "" };
        let text = if gone {
            format!("{}{flag}  {}  — {}", d.label(), d.formation.name(), if d.alive == 0 { "destroyed" } else { "routed" })
        } else {
            format!("{}{flag}  {:<9} {:>3}/{:<3} {:<6} {}", d.label(), d.formation.name(), d.alive, d.start, d.morale.name(), d.order.name())
        };
        let color = if gone {
            DIM
        } else if d.morale == Morale::Steady {
            side_color(d.side)
        } else {
            morale_color(d.morale)
        };
        m.divisions.push(DivisionRow { entity: *e, side: d.side, number: d.number, text, color, selected: sel.division == Some(*e), gone });
    }

    // Selected division detail.
    if let Some((e, d)) = sel.division.and_then(|s| divs.iter().find(|(e, _)| *e == s)) {
        let centre = hash.centroid(*e).unwrap_or(d.anchor);
        let in_command = d.flagship || flags.iter().any(|(p, s)| *s == d.side && p.distance(centre) <= command_radius);
        m.info.push(vec![
            span(format!("{} ", d.label()), side_color(d.side)),
            span(format!("{} division · {}", side_name(d.side), d.formation.name()), TEXT),
            span(if d.flagship { " · flagship" } else { "" }, GOLD),
        ]);
        m.info.push(vec![
            span(format!("{} of {} ships · ", d.alive, d.start), TEXT),
            span(d.morale.name(), morale_color(d.morale)),
            span(if in_command { " · in command" } else { " · OUT of command" }, if in_command { DIM } else { BAD }),
        ]);
        let order = match d.order {
            Order::Engage(t) => {
                let name = divs.iter().find(|(x, _)| *x == t).map_or("?".to_string(), |(_, d)| d.label());
                format!("Order: engage {name}")
            }
            o => format!("Order: {}", o.name()),
        };
        m.info.push(vec![span(order, DIM)]);
        m.can_order = settings.control.human_controls(d.side) && d.alive > 0 && d.morale != Morale::Routed && phase == Phase::Battle;
        m.formation = Some(d.formation);
        m.move_mode = sel.mode == PointerMode::Move;
    } else if !m.spectate {
        m.info.push(vec![span("Click one of your ships, a division in the list, or press Tab.", DIM)]);
    } else {
        m.info.push(vec![span("Click any ship to inspect its division.", DIM)]);
    }

    if phase == Phase::Over
        && let Some(outcome) = &battle.result
    {
        let title = match outcome.winner {
            Some(w) if settings.control.human_controls(w) => "Victory".to_string(),
            Some(w) if settings.control != Control::Spectate => format!("Defeat — {} wins", side_name(w)),
            Some(w) => format!("{} wins", side_name(w)),
            None => "Draw".to_string(),
        };
        let body = match (outcome.winner, outcome.reason) {
            (Some(w), EndReason::Broke) => format!(
                "{}'s fleet broke after {}: {} of {} ships destroyed, fled or routed.",
                side_name(1 - w),
                clock(outcome.time),
                battle.lost[1 - w],
                battle.start[1 - w]
            ),
            (Some(w), EndReason::TimeLimit) => format!("Time ran out; {} kept more of its fleet in the fight.", side_name(w)),
            (Some(w), EndReason::Disengaged) => format!(
                "Both fleets were too shaken to press on. {} kept more of its fleet in the fight ({:.0}% to {:.0}%) and holds the field.",
                side_name(w),
                battle.remaining(w) * 100.0,
                battle.remaining(1 - w) * 100.0
            ),
            (None, EndReason::Disengaged) => "Both fleets were too shaken to press on, and drifted apart: draw.".to_string(),
            (None, _) => "Time ran out with both fleets still fighting.".to_string(),
        };
        let line = format!(
            "{} | ctrl={} | ships={} | ai={} | winner={} | time={} | lost {}-{} | destroyed {}-{}",
            title,
            settings.control.tag(),
            settings.ships,
            settings.ai[0].tag(),
            outcome.winner.map_or("draw", side_name),
            clock(outcome.time),
            battle.lost[0],
            battle.lost[1],
            battle.destroyed[0],
            battle.destroyed[1],
        );
        m.result = Some(ResultView { title, body, line });
    }
    *world.resource_mut::<UiModel>() = m;
}
