use bevy::ecs::message::Messages;
use bevy::prelude::*;

use super::battle::*;
use super::commands::{BattleCommand, apply};
use super::components::*;
use super::config::*;
use super::dice::Dice;
use crate::hex::Hex;

fn world_with(dice: Dice) -> World {
    let mut world = World::new();
    init_world(&mut world, MatchSettings { control: ControlMode::Hotseat, ..default() }, dice);
    *world.resource_mut::<Phase>() = Phase::Combat;
    world
}

fn events(world: &mut World) -> Vec<BattleEvent> {
    world.resource_mut::<Messages<BattleEvent>>().drain().collect()
}

#[test]
fn front_flank_rear_need_5_4_3() {
    // Blue at (5,10) facing east; Red targets placed so Blue sits in their
    // front / flank / rear arc.
    for (red_facing, need) in [(3u8, 5u8), (1, 4), (0, 3)] {
        let mut w = world_with(Dice::scripted([6, 6, 6, 6]));
        let b = spawn_unit(&mut w, 0, Hex::new(5, 10), 0, true);
        let r = spawn_unit(&mut w, 1, Hex::new(7, 10), red_facing, true);
        let shot = fire(&mut w, b, r);
        assert_eq!(shot.need, need, "red facing {red_facing}");
    }
}

#[test]
fn shaken_rolls_half_dice_and_critical_supply_worsens_to_hit() {
    let mut w = world_with(Dice::scripted([1, 1, 6, 6, 6]));
    let b = spawn_unit(&mut w, 0, Hex::new(5, 10), 0, true);
    let r = spawn_unit(&mut w, 1, Hex::new(7, 10), 3, true);
    w.entity_mut(b).insert(Morale::Shaken);
    w.resource_mut::<Battle>().fleets[0].supply = Supply::Critical;
    let shot = fire(&mut w, b, r);
    assert_eq!(shot.need, 6);
    let rolls = events(&mut w)
        .into_iter()
        .find_map(|e| if let BattleEvent::Shot { rolls, .. } = e { Some(rolls) } else { None })
        .unwrap();
    assert_eq!(rolls.len(), 2);
}

#[test]
fn hits_reduce_strength_then_morale_check_can_shake() {
    // 4 dice: two hits on 5+, then morale roll 1 (+1 command) fails.
    let mut w = world_with(Dice::scripted([5, 6, 1, 1, 1]));
    let b = spawn_unit(&mut w, 0, Hex::new(5, 10), 0, true);
    let r = spawn_unit(&mut w, 1, Hex::new(7, 10), 3, true);
    fire(&mut w, b, r);
    assert_eq!(w.get::<Strength>(r).unwrap().0, 2);
    assert_eq!(*w.get::<Morale>(r).unwrap(), Morale::Shaken);
    assert!(w.entity(r).contains::<HitSinceAct>());
}

#[test]
fn losing_the_flagship_sets_flag_lost_and_checks_every_survivor() {
    let mut w = world_with(Dice::scripted([6, 6, 6, 6, 6, 6, 6, 6]));
    let b = spawn_unit(&mut w, 0, Hex::new(5, 10), 0, true);
    let r = spawn_unit(&mut w, 1, Hex::new(7, 10), 3, true);
    let _r2 = spawn_unit(&mut w, 1, Hex::new(20, 4), 3, false);
    fire(&mut w, b, r);
    assert!(!w.entity(r).contains::<Alive>());
    assert!(w.resource::<Battle>().fleets[1].flag_lost);
    let checks = events(&mut w).iter().filter(|e| matches!(e, BattleEvent::MoraleChecked { .. })).count();
    assert_eq!(checks, 1, "only the distant survivor checks (fleet-wide)");
}

#[test]
fn line_of_sight_is_blocked_by_any_squadron() {
    let mut w = world_with(Dice::default());
    let b = spawn_unit(&mut w, 0, Hex::new(5, 10), 0, true);
    let _screen = spawn_unit(&mut w, 0, Hex::new(6, 10), 0, false);
    let _r = spawn_unit(&mut w, 1, Hex::new(7, 10), 3, true);
    let ships = snapshot(&mut w);
    let me = *find(&ships, b).unwrap();
    assert!(legal_targets(&ships, &me).is_empty());
}

#[test]
fn shaken_unit_refuses_to_close() {
    let mut w = world_with(Dice::default());
    let b = spawn_unit(&mut w, 0, Hex::new(5, 10), 0, true);
    let _r = spawn_unit(&mut w, 1, Hex::new(9, 10), 3, true);
    w.entity_mut(b).insert(Morale::Shaken);
    assert_eq!(step_into(&mut w, b, 0), Err(StepError::Shaken));
    assert_eq!(step_into(&mut w, b, 3), Ok(()));
}

#[test]
fn routed_unit_flees_off_its_own_edge() {
    let mut w = world_with(Dice::scripted([1]));
    let b = spawn_unit(&mut w, 0, Hex::new(1, 13), 3, false);
    let _r = spawn_unit(&mut w, 1, Hex::new(20, 13), 3, true);
    w.entity_mut(b).insert((Morale::Routed, HitSinceAct));
    routed_activation(&mut w, b);
    assert!(!w.entity(b).contains::<Alive>());
}

#[test]
fn human_activation_out_of_command_is_move_or_fire() {
    let mut w = world_with(Dice::scripted([1, 1, 1, 1]));
    let flag = spawn_unit(&mut w, 0, Hex::new(2, 13), 0, true);
    let u = spawn_unit(&mut w, 0, Hex::new(10, 13), 0, false);
    let _r = spawn_unit(&mut w, 1, Hex::new(12, 13), 3, true);
    let _ = flag;
    w.resource_mut::<Battle>().turn = 1;
    w.insert_resource(Activation(Some(Act {
        side: 0,
        unit: None,
        mp: 0,
        moved: false,
        fired: false,
        fire_mode: false,
        cmd: false,
    })));
    apply(&mut w, BattleCommand::SelectHex(Hex::new(10, 13)));
    let a = w.resource::<Activation>().0.unwrap();
    assert_eq!(a.unit, Some(u));
    assert!(!a.cmd);
    apply(&mut w, BattleCommand::SelectHex(Hex::new(12, 13)));
    assert!(w.resource::<Activation>().0.is_none(), "firing out of command ends the activation");
}

#[test]
fn manual_deployment_then_ai_deploys_and_combat_starts() {
    let mut w = World::new();
    init_world(&mut w, MatchSettings { control: ControlMode::PlayBlue, size: 5, ..default() }, Dice::seeded(7));
    new_battle(&mut w);
    assert_eq!(*w.resource::<Phase>(), Phase::Deployment);
    for r in 10..15 {
        apply(&mut w, BattleCommand::SelectHex(Hex::new(5, r)));
    }
    apply(&mut w, BattleCommand::ConfirmDeployment);
    assert_eq!(*w.resource::<Phase>(), Phase::Combat);
    assert_eq!(w.resource::<Battle>().fleets[0].roster.len(), 5);
    assert_eq!(w.resource::<Battle>().fleets[1].roster.len(), 5);
}

#[test]
fn every_scenario_terminates_for_both_ais_and_sizes() {
    for scenario in 0..SCENARIOS.len() {
        for size in [5, 9, 12] {
            for ai in [AiKind::Baseline, AiKind::Doctrine] {
                let settings = MatchSettings { scenario, size, ai: [ai; 2], ..default() };
                let (_, turns) = crate::simulate(settings, scenario as u32 * 31 + size as u32);
                assert!(turns >= 1 && turns <= MAX_TURNS, "{scenario} {size} {ai:?}");
            }
        }
    }
}
