//! The small vocabulary of player intent. Keyboard, buttons and board clicks
//! all become a `BattleCommand`; `apply` executes it against the world.

use bevy::prelude::*;

use super::battle::*;
use super::components::*;
use super::config::*;
use super::dice::Dice;
use crate::hex::{self, Hex};

#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattleCommand {
    /// Counter-clockwise 60 degrees (+1 facing index).
    TurnLeft,
    /// Clockwise 60 degrees (-1 facing index).
    TurnRight,
    Forward,
    Backward,
    FireMode,
    EndActivation,
    SetFlagship,
    RemoveDeployed,
    ConfirmDeployment,
    /// Start (or restart) the battle described by `MatchSettings`.
    Restart,
    ToMenu,
    /// Click/tap on a board hex.
    SelectHex(Hex),
}

pub fn apply(world: &mut World, cmd: BattleCommand) {
    let phase = *world.resource::<Phase>();
    match cmd {
        BattleCommand::Restart => new_battle(world),
        BattleCommand::ToMenu => {
            world.insert_resource(Activation(None));
            world.resource_mut::<Deployment>().current = None;
            *world.resource_mut::<Phase>() = Phase::Menu;
        }
        _ if phase == Phase::Deployment => apply_setup(world, cmd),
        _ if phase == Phase::Combat => apply_combat(world, cmd),
        _ => {}
    }
}

// --- deployment ------------------------------------------------------------

fn apply_setup(world: &mut World, cmd: BattleCommand) {
    let size = world.resource::<MatchSettings>().size;
    let mut dep = world.resource_mut::<Deployment>();
    let Some(setup) = dep.current.as_mut() else { return };
    match cmd {
        BattleCommand::SelectHex(h) => {
            if let Some(i) = setup.placed.iter().position(|p| p.hex == h) {
                setup.selected = Some(i);
            } else if setup.placed.len() < size && in_setup_zone(setup.side, h) {
                let facing = if setup.side == 0 { 0 } else { 3 };
                setup.placed.push(Placed { hex: h, facing });
                setup.selected = Some(setup.placed.len() - 1);
                if setup.flagship.is_none() {
                    setup.flagship = setup.selected;
                }
            }
        }
        BattleCommand::TurnLeft | BattleCommand::TurnRight => {
            if let Some(i) = setup.selected {
                let delta = if cmd == BattleCommand::TurnLeft { 1 } else { 5 };
                setup.placed[i].facing = (setup.placed[i].facing + delta) % 6;
            }
        }
        BattleCommand::SetFlagship => {
            if setup.selected.is_some() {
                setup.flagship = setup.selected;
            }
        }
        BattleCommand::RemoveDeployed => {
            if let Some(i) = setup.selected {
                setup.placed.remove(i);
                setup.flagship = match setup.flagship {
                    Some(f) if f == i => (!setup.placed.is_empty()).then_some(0),
                    Some(f) if f > i => Some(f - 1),
                    other => other,
                };
                setup.selected = if setup.placed.is_empty() { None } else { Some(i.min(setup.placed.len() - 1)) };
            }
        }
        BattleCommand::ConfirmDeployment => confirm_setup(world),
        _ => {}
    }
}

fn confirm_setup(world: &mut World) {
    let size = world.resource::<MatchSettings>().size;
    let Some(setup) = world.resource::<Deployment>().current.clone() else { return };
    if setup.placed.len() != size {
        return;
    }
    let side = setup.side;
    world.resource_mut::<Battle>().fleets[side].formation = Formation::Custom;
    for (i, p) in setup.placed.iter().enumerate() {
        spawn_unit(world, side, p.hex, p.facing, Some(i) == setup.flagship);
    }
    note(world, format!("{} deployment confirmed — {size} squadrons.", side_name(side)), Tone::Title);
    world.resource_mut::<Deployment>().current = None;
    let next = {
        let mut dep = world.resource_mut::<Deployment>();
        (!dep.queue.is_empty()).then(|| dep.queue.remove(0))
    };
    if let Some(side) = next {
        *world.resource_mut::<Phase>() = Phase::Deployment;
        world.resource_mut::<Deployment>().current =
            Some(SetupState { side, placed: Vec::new(), selected: None, flagship: None });
        note(world, format!("{}: deploy your squadrons — click your shaded zone.", side_name(side)), Tone::Title);
        return;
    }
    let control = world.resource::<MatchSettings>().control;
    for s in 0..2 {
        if !control.human_controls(s) {
            let formation = world.resource_mut::<Dice>().pick(&Formation::STANDARD);
            deploy_formation(world, formation, s);
            note(world, format!("{} (AI) deploys in {} formation.", side_name(s), formation.name()), Tone::Title);
        }
    }
    start_combat(world);
}

// --- combat ------------------------------------------------------------------

fn act(world: &World) -> Option<Act> {
    world.resource::<Activation>().0
}

fn set_act(world: &mut World, act: Option<Act>) {
    world.insert_resource(Activation(act));
}

fn apply_combat(world: &mut World, cmd: BattleCommand) {
    let Some(mut a) = act(world) else { return };
    match cmd {
        BattleCommand::SelectHex(h) => select_hex(world, a, h),
        BattleCommand::TurnLeft | BattleCommand::TurnRight => {
            if !can_move(&a) {
                return;
            }
            let u = a.unit.unwrap();
            let f = world.get::<Facing>(u).unwrap().0;
            set_facing(world, u, f + if cmd == BattleCommand::TurnLeft { 1 } else { 5 });
            a.mp -= 1;
            a.moved = true;
            a.fire_mode = false;
            set_act(world, Some(a));
        }
        BattleCommand::Forward | BattleCommand::Backward => {
            let back = cmd == BattleCommand::Backward;
            if if back { !can_back(&a) } else { !can_move(&a) } {
                return;
            }
            let u = a.unit.unwrap();
            let f = world.get::<Facing>(u).unwrap().0;
            let dir = if back { (f + 3) % 6 } else { f };
            match step_into(world, u, dir) {
                Ok(()) => {
                    a.mp = if back { 0 } else { a.mp - 1 };
                    a.moved = true;
                    a.fire_mode = false;
                    set_act(world, Some(a));
                }
                Err(StepError::Shaken) => {
                    let label = label_of(world, u);
                    emit(world, BattleEvent::ShakenRefused { label });
                }
                Err(StepError::Blocked) => {}
            }
        }
        BattleCommand::FireMode => {
            if can_fire(world, &a) {
                a.fire_mode = true;
                set_act(world, Some(a));
            }
        }
        BattleCommand::EndActivation if a.unit.is_some() => set_act(world, None),
        _ => {}
    }
}

fn select_hex(world: &mut World, a: Act, h: Hex) {
    let ships = snapshot(world);
    let Some(clicked) = alive(&ships).find(|s| s.hex == h).copied() else { return };
    match a.unit {
        None => select_unit(world, a, clicked),
        Some(u) if clicked.side == a.side && clicked.e != u && !clicked.activated => {
            if can_switch_selection(&a) {
                world.entity_mut(u).remove::<Activated>();
                select_unit(world, Act { unit: None, ..a }, clicked);
            }
        }
        Some(_) if clicked.side != a.side => fire_at(world, a, clicked.e),
        _ => {}
    }
}

fn select_unit(world: &mut World, mut a: Act, ship: Ship) {
    if ship.side != a.side || ship.activated || !ship.alive {
        return;
    }
    world.entity_mut(ship.e).insert(Activated);
    world.resource_mut::<Battle>().last_acted = ship.side;
    if ship.morale == Morale::Routed {
        routed_activation(world, ship.e);
        set_act(world, None);
        return;
    }
    world.entity_mut(ship.e).remove::<HitSinceAct>();
    let ships = snapshot(world);
    a.unit = Some(ship.e);
    a.mp = MP_MAX;
    a.moved = false;
    a.fired = false;
    a.fire_mode = false;
    a.cmd = in_command(&ships, &ship);
    set_act(world, Some(a));
}

fn fire_at(world: &mut World, mut a: Act, tgt: Entity) {
    if !can_fire(world, &a) {
        return;
    }
    let u = a.unit.unwrap();
    let ships = snapshot(world);
    let me = *find(&ships, u).unwrap();
    if !legal_targets(&ships, &me).iter().any(|t| t.e == tgt) {
        return;
    }
    fire(world, u, tgt);
    a.fired = true;
    a.fire_mode = false;
    set_act(world, Some(a));
    if check_break(world) {
        return;
    }
    // Out of command, firing was the whole activation.
    if !a.cmd {
        set_act(world, None);
    }
}

/// Hexes the selected squadron could fire into right now (for shading).
pub fn fire_zone(world: &mut World) -> Vec<Hex> {
    let Some(a) = act(world) else { return Vec::new() };
    let Some(u) = a.unit else { return Vec::new() };
    let ships = snapshot(world);
    let Some(me) = find(&ships, u).copied() else { return Vec::new() };
    let occ = occupied(&ships);
    board_hexes()
        .filter(|h| *h != me.hex && can_shoot(me.hex, me.facing, *h, &occ))
        .filter(|h| hex::dist(me.hex, *h) > 0)
        .collect()
}
