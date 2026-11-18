//! A per-frame read model of the battle for the UI and board overlays.
//! Built by one exclusive system so every other presentation system is a
//! plain, read-only system over `UiModel`.

use std::collections::HashSet;

use bevy::prelude::*;

use super::theme::*;
use crate::hex::Hex;
use crate::rules::battle::*;
use crate::rules::commands::fire_zone;
use crate::rules::config::*;

pub type Spans = Vec<(String, Color)>;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub struct Can {
    pub turn: bool,
    pub forward: bool,
    pub back: bool,
    pub fire: bool,
    pub end: bool,
    pub setup_turn: bool,
    pub setup_flag: bool,
    pub setup_remove: bool,
    pub setup_confirm: bool,
}

#[derive(Resource, Default)]
pub struct UiModel {
    pub phase: Phase,
    pub status: Vec<Spans>,
    /// One entry per line.
    pub info: Vec<Spans>,
    pub can: Can,
    pub spectate: bool,
    pub fire_zone: HashSet<Hex>,
    pub targets: Vec<Entity>,
    pub selected: Option<Entity>,
    /// Flagship hex per side, for the command-reach tint.
    pub flagships: [Option<Hex>; 2],
    pub setup: Option<SetupState>,
    pub result: Option<BattleResult>,
}

pub fn build_model(world: &mut World) {
    let phase = *world.resource::<Phase>();
    let settings = *world.resource::<MatchSettings>();
    let mut m = UiModel { phase, spectate: settings.control == ControlMode::Spectate, ..default() };
    if phase == Phase::Menu {
        world.insert_resource(m);
        return;
    }
    let battle = world.resource::<Battle>().clone();
    let ships = snapshot(world);
    for side in 0..2 {
        m.flagships[side] = flagship_of(&ships, side).map(|s| s.hex);
    }
    m.result = battle.result.clone();

    if phase == Phase::Deployment {
        let setup = world.resource::<Deployment>().current.clone();
        if let Some(setup) = &setup {
            let size = settings.size;
            m.status = vec![vec![
                (format!("{} deployment", side_name(setup.side)), side_text(setup.side)),
                (format!(" — {}/{} placed", setup.placed.len(), size), TEXT),
            ]];
            let sel = setup.selected;
            m.info = vec![vec![(
                match sel {
                    Some(i) => format!(
                        "Selected squadron{}. Turn it, set flagship, remove it, or click an empty zone hex to place another.",
                        if setup.flagship == Some(i) { " — flagship" } else { "" }
                    ),
                    None if setup.placed.len() < size => format!(
                        "Click an empty hex in your shaded zone to place your next squadron ({} left).",
                        size - setup.placed.len()
                    ),
                    None => format!("All {size} squadrons placed. Click one to adjust, or confirm."),
                },
                TEXT,
            )]];
            m.can.setup_turn = sel.is_some();
            m.can.setup_flag = sel.is_some() && sel != setup.flagship;
            m.can.setup_remove = sel.is_some();
            m.can.setup_confirm = setup.placed.len() == size;
        }
        m.setup = setup;
        world.insert_resource(m);
        return;
    }

    let losses_of = |side: usize| battle.fleets[side].roster.iter().filter(|e| find(&ships, **e).is_some_and(|s| !s.alive)).count();
    let fleet_tag = |side: usize| {
        let f = &battle.fleets[side];
        let sup = if f.supply == Supply::Ok { String::new() } else { format!(" ({} supply)", f.supply.name()) };
        format!("{} {}{}: {}/{} losses", side_name(side), f.formation.name(), sup, losses_of(side), battle.break_at)
    };
    m.status = vec![
        vec![(format!("Turn {}/{}", battle.turn, MAX_TURNS), GOLD)],
        vec![(fleet_tag(0), BLUE)],
        vec![(fleet_tag(1), RED)],
    ];

    let act = world.resource::<Activation>().0;
    if battle.over() {
        m.info = vec![vec![("Battle over.".into(), TEXT)]];
    } else if let Some(a) = act {
        match a.unit {
            None => {
                m.info = vec![vec![
                    (side_name(a.side).into(), side_text(a.side)),
                    (": click one of your un-activated squadrons.".into(), TEXT),
                ]];
            }
            Some(u) => {
                let me = *find(&ships, u).expect("selected ship");
                let label = label_of(world, u);
                let can_fire_now = can_fire(world, &a);
                let mut info: Vec<Spans> = vec![
                    vec![
                        (format!("{}{}", label, if me.flagship { " (flagship)" } else { "" }), GOLD),
                        (format!(" — str {}, {}", me.strength, me.morale.name()), TEXT),
                    ],
                    vec![(
                        if a.cmd { "In command: move AND fire".into() } else { "OUT of command: move OR fire".to_string() },
                        if a.cmd { TEXT } else { BAD },
                    )],
                    vec![(format!("MP {}/{}{}", a.mp, MP_MAX, if a.fired { " · has fired" } else { "" }), TEXT)],
                ];
                if can_fire_now {
                    info.push(vec![("Click a red-outlined enemy to fire.".into(), RED)]);
                }
                if can_switch_selection(&a) {
                    info.push(vec![(
                        "Changed your mind? Click another un-activated squadron — nothing's committed yet.".into(),
                        DIM,
                    )]);
                }
                m.info = info;
                m.can.turn = can_move(&a);
                m.can.forward = can_move(&a);
                m.can.back = can_back(&a);
                m.can.fire = can_fire_now;
                m.can.end = true;
                m.selected = Some(u);
                if can_fire_now {
                    m.targets = legal_targets(&ships, &me).iter().map(|t| t.e).collect();
                }
                m.fire_zone = fire_zone(world).into_iter().collect();
            }
        }
    } else {
        m.info = vec![vec![(if m.spectate { "Spectating — press Step or Auto." } else { "AI is acting…" }.into(), DIM)]];
    }
    world.insert_resource(m);
}
