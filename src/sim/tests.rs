use bevy::ecs::message::Messages;
use bevy::prelude::*;

use super::battle::*;
use super::components::*;
use super::formation::{self, Formation};
use super::scenario::SCENARIOS;
use super::spatial::{Entry, SpatialHash};
use super::systems::{hit_chance, incoming_arc};
use super::*;

fn world_for(scenario: usize, ships: usize, ai: AiKind, seed: u32) -> World {
    let mut world = World::new();
    let settings = SimSettings { scenario, ships, ai: [ai; 2], control: Control::Spectate, mirror: false, forces: None };
    init_world(&mut world, settings, seed);
    world
}

fn run_for(world: &mut World, seconds: f32) {
    let until = world.resource::<Battle>().time + seconds;
    while world.resource::<Battle>().time < until && !world.resource::<Battle>().over() {
        step(world);
        world.resource_mut::<Messages<SimEvent>>().update();
    }
}

#[test]
fn layouts_have_the_right_count_and_no_overlaps() {
    for f in [
        Formation::Line,
        Formation::Wedge,
        Formation::EchelonLeft,
        Formation::EchelonRight,
        Formation::Crescent,
        Formation::Column,
        Formation::Ring,
    ] {
        for n in [1, 7, 20, 40, 80] {
            let slots = formation::layout(f, n, 14.0);
            assert_eq!(slots.len(), n, "{f:?} {n}");
            let c = slots.iter().copied().sum::<Vec2>() / n as f32;
            assert!(c.length() < 1e-3, "{f:?} not centred");
            for i in 0..n {
                for j in i + 1..n {
                    assert!(slots[i].distance(slots[j]) > 10.0, "{f:?} {n}: slots {i} and {j} overlap");
                }
            }
        }
    }
}

#[test]
fn fleets_start_inside_the_battle_area() {
    for (i, _) in SCENARIOS.iter().enumerate() {
        for ships in [120, 240, 480] {
            let mut world = world_for(i, ships, AiKind::Commander, 1);
            let bounds = world.resource::<Battle>().bounds;
            let count = world.query::<&Pos>().iter(&world).inspect(|p| assert!(bounds.contains(p.0))).count();
            assert_eq!(count, ships * 2);
        }
    }
}

#[test]
fn spatial_hash_matches_brute_force() {
    let mut rng = rng::Rng::seeded(9);
    let entries: Vec<Entry> = (0..400)
        .map(|i| Entry {
            entity: Entity::from_raw_u32(i + 1).unwrap(),
            pos: Vec2::new(rng.range(-900.0, 900.0), rng.range(-600.0, 600.0)),
            heading: 0.0,
            side: (i % 2) as usize,
            division: Entity::from_raw_u32(1000).unwrap(),
        })
        .collect();
    let mut hash = SpatialHash::new(110.0);
    hash.rebuild(entries.iter().copied());
    for probe in [Vec2::ZERO, Vec2::new(300.0, -200.0), Vec2::new(-880.0, 590.0)] {
        for r in [20.0, 110.0, 250.0] {
            let mut got = Vec::new();
            hash.for_each_within(probe, r, |e, _| got.push(e.entity));
            got.sort();
            let mut want: Vec<Entity> = entries.iter().filter(|e| e.pos.distance(probe) <= r).map(|e| e.entity).collect();
            want.sort();
            assert_eq!(got, want);
        }
    }
}

#[test]
fn arcs_and_hit_chances() {
    let t = Tuning::default();
    // Target at the origin facing +x.
    assert_eq!(incoming_arc(Vec2::ZERO, 0.0, Vec2::new(100.0, 10.0)), 0);
    assert_eq!(incoming_arc(Vec2::ZERO, 0.0, Vec2::new(0.0, 100.0)), 1);
    assert_eq!(incoming_arc(Vec2::ZERO, 0.0, Vec2::new(-100.0, -5.0)), 2);
    assert!(hit_chance(&t, 2, 50.0) > hit_chance(&t, 1, 50.0));
    assert!(hit_chance(&t, 1, 50.0) > hit_chance(&t, 0, 50.0));
    assert!(hit_chance(&t, 0, 10.0) > hit_chance(&t, 0, 100.0));
}

#[test]
fn same_seed_same_battle() {
    let a = simulate(SimSettings { scenario: 0, ships: 120, ..default() }, 42, None);
    let b = simulate(SimSettings { scenario: 0, ships: 120, ..default() }, 42, None);
    assert_eq!((a.winner, a.time, a.lost, a.destroyed), (b.winner, b.time, b.lost, b.destroyed));
}

#[test]
fn heavy_losses_rout_a_division() {
    let mut world = world_for(1, 120, AiKind::Advance, 3);
    let div = world.query::<(Entity, &Division)>().iter(&world).find(|(_, d)| d.side == 0 && d.number == 3).unwrap().0;
    world.get_mut::<Division>(div).unwrap().shock = 1.0;
    run_for(&mut world, 3.0);
    let morale = world.get::<Division>(div).unwrap().morale;
    assert_ne!(morale, Morale::Steady, "a division with heavy recent losses should waver");
    world.get_mut::<Division>(div).unwrap().shock = 2.0;
    run_for(&mut world, 3.0);
    assert_eq!(world.get::<Division>(div).unwrap().morale, Morale::Routed);
}

#[test]
fn enemies_block_instead_of_shoving() {
    // Two lines, Blue charging into a Red line that holds: Red ships must
    // not be pushed backwards.
    let mut world = world_for(1, 120, AiKind::Advance, 5);
    let reds: Vec<Entity> = world.query::<(Entity, &Division)>().iter(&world).filter(|(_, d)| d.side == 1).map(|(e, _)| e).collect();
    for r in &reds {
        world.get_mut::<Division>(*r).unwrap().order = Order::Hold;
    }
    let blues: Vec<Entity> = world.query::<(Entity, &Division)>().iter(&world).filter(|(_, d)| d.side == 0).map(|(e, _)| e).collect();
    for b in &blues {
        world.get_mut::<Division>(*b).unwrap().order = Order::Charge;
    }
    let start_x: f32 = world.query::<(&Pos, &Side)>().iter(&world).filter(|(_, s)| s.0 == 1).map(|(p, _)| p.0.x).fold(f32::MAX, f32::min);
    run_for(&mut world, 75.0);
    let min_x = world
        .query::<(&Pos, &Side, &InDivision)>()
        .iter(&world)
        .filter(|(_, s, d)| s.0 == 1 && world.get::<Division>(d.0).is_some_and(|d| d.morale != Morale::Routed))
        .map(|(p, _, _)| p.0.x)
        .fold(f32::MAX, f32::min);
    assert!(min_x > start_x - 30.0, "holding Red ships were pushed back from {start_x} to {min_x}");
}

#[test]
fn every_scenario_ends_before_the_time_limit() {
    for (i, s) in SCENARIOS.iter().enumerate() {
        let r = simulate(SimSettings { scenario: i, ships: 120, ..default() }, 11, None);
        assert_ne!(r.reason, EndReason::TimeLimit, "{} ran to the time limit", s.title);
        assert!(r.first_contact.is_some_and(|t| t < 90.0), "{}: no contact", s.title);
    }
}

#[test]
fn the_arrow_breaks_through_the_line() {
    let r = simulate(SimSettings { scenario: 0, ships: 240, ..default() }, 3, None);
    assert!(r.first_breakthrough[0].is_some(), "the arrow (Blue) never broke through: {r:?}");
}

#[test]
fn campaign_forces_set_sizes_damage_and_modifiers() {
    use super::scenario::FleetShape;
    let fast = SideMods { fire_rate: 1.5, ..default() };
    let forces = [
        Force { ships: 60, damaged: 30, shape: FleetShape::Arrow, mods: fast },
        Force { ships: 150, damaged: 0, shape: FleetShape::Line, mods: SideMods::default() },
    ];
    let mut world = World::new();
    init_world(&mut world, SimSettings { forces: Some(forces), control: Control::Spectate, ..default() }, 4);
    let mut count = [0u32; 2];
    let mut half = [0u32; 2];
    for (ship, side) in world.query::<(&Ship, &Side)>().iter(&world) {
        count[side.0] += 1;
        if ship.hull < ship.max_hull {
            half[side.0] += 1;
        }
    }
    assert_eq!(count, [60, 150]);
    assert_eq!(half, [30, 0]);
    assert_eq!(world.resource::<Battle>().mods[0].fire_rate, 1.5);
    let r = simulate(SimSettings { forces: Some(forces), ..default() }, 4, None);
    assert!(r.survivors[0] <= 60 && r.survivors[1] <= 150);
    assert!(r.damaged[0] <= r.survivors[0] && r.damaged[1] <= r.survivors[1]);
    assert_eq!(r.winner, Some(1), "150 fresh ships should beat 60 half-damaged ones");
}

#[test]
fn auto_resolve_matches_simulate() {
    let settings = SimSettings { scenario: 1, ships: 120, ..default() };
    let full = simulate(settings, 21, None);
    let mut auto = AutoResolve::new(settings, 21);
    let r = loop {
        if let Some(r) = auto.advance(97) {
            break r;
        }
    };
    assert_eq!((r.winner, r.time, r.survivors), (full.winner, full.time, full.survivors));
}

#[test]
fn arrow_concentrates_fire_on_the_enemy_nearest_its_centre() {
    use super::scenario::FleetShape;
    let forces = [
        Force { ships: 120, damaged: 0, shape: FleetShape::Arrow, mods: SideMods::default() },
        Force { ships: 120, damaged: 0, shape: FleetShape::Line, mods: SideMods::default() },
    ];
    let mut world = World::new();
    init_world(&mut world, SimSettings { forces: Some(forces), control: Control::Spectate, ai: [AiKind::Advance; 2], ..default() }, 9);
    assert!(super::systems::focus_fire(world.resource::<Battle>().shapes[0]));
    assert!(!super::systems::focus_fire(world.resource::<Battle>().shapes[1]));
    // Run into contact, then compare where each side's shots land.
    let (mut arrow, mut line) = (Vec::new(), Vec::new());
    while world.resource::<Battle>().time < 90.0 && !world.resource::<Battle>().over() {
        step(&mut world);
        for ev in world.resource_mut::<Messages<SimEvent>>().drain() {
            if let SimEvent::Shot { from, to, side, .. } = ev {
                if side == 0 { arrow.push((from, to)) } else { line.push((from, to)) }
            }
        }
    }
    assert!(!arrow.is_empty() && !line.is_empty());
    // Arrow fire converges: its shots land in a much tighter spread across
    // the front than the Line's nearest-target fire does.
    let spread = |v: &[(Vec2, Vec2)]| {
        let m = v.iter().map(|(_, t)| t.y).sum::<f32>() / v.len() as f32;
        (v.iter().map(|(_, t)| (t.y - m).powi(2)).sum::<f32>() / v.len() as f32).sqrt()
    };
    assert!(spread(&arrow) < spread(&line), "arrow spread {} vs line {}", spread(&arrow), spread(&line));
}

#[test]
fn friendly_ships_block_the_line_of_fire() {
    let mut hash = SpatialHash::new(110.0);
    let e = |i| Entity::from_raw_u32(i).unwrap();
    let d = e(999);
    let me = Entry { entity: e(1), pos: Vec2::ZERO, heading: 0.0, side: 0, division: d };
    let friend = Entry { entity: e(2), pos: Vec2::new(50.0, 2.0), heading: 0.0, side: 0, division: d };
    let enemy = Entry { entity: e(3), pos: Vec2::new(50.0, -2.0), heading: 0.0, side: 1, division: d };
    hash.rebuild([me, friend, enemy].into_iter());
    let t = Tuning::default();
    // A friend 2 units off the line, halfway: blocked.
    assert!(!super::systems::line_clear(&hash, Vec2::ZERO, Vec2::new(100.0, 0.0), 0, e(1), t.los_clearance));
    // Enemies never block, and a friend well off the line doesn't either.
    assert!(super::systems::line_clear(&hash, Vec2::ZERO, Vec2::new(100.0, 0.0), 1, e(3), t.los_clearance));
    assert!(super::systems::line_clear(&hash, Vec2::ZERO, Vec2::new(100.0, 40.0), 0, e(1), t.los_clearance));
    // A staggered second rank fires through the gap in the first.
    let line = formation::layout(Formation::Line, 20, 14.0);
    let front: Vec<Vec2> = line.iter().copied().filter(|p| p.x > -1.0).collect();
    let back = line.iter().copied().find(|p| p.x < -1.0).unwrap();
    let gap_clear = front.iter().all(|f| {
        let v = Vec2::X;
        let rel = *f - back;
        rel.perp_dot(v).abs() >= t.los_clearance || rel.dot(v) <= 0.0
    });
    assert!(gap_clear, "second rank should be staggered between first-rank ships");
}
