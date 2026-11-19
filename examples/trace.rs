//! Prints a battle timeline every 10 s: per-division ships, morale, order
//! and centroid. `cargo run --release --example trace -- <scenario> <ai: a|c> [seed] [ships]`

use bevy::ecs::message::Messages;
use bevy::prelude::*;
use space_game::sim::battle::*;
use space_game::sim::components::*;
use space_game::sim::spatial::SpatialHash;
use space_game::sim::{init_world, step};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let scenario = args.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let ai = if args.get(1).map(|s| s.as_str()) == Some("c") { AiKind::Commander } else { AiKind::Advance };
    let seed = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let ships = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(240);
    let mut world = World::new();
    let settings = SimSettings { scenario, ships, ai: [ai; 2], control: Control::Spectate, mirror: false, forces: None, skirmish: None };
    init_world(&mut world, settings, seed);
    let shots = std::env::var("TRACE_PPM").ok();
    let mut next_shot = 0.0;
    let mut next = 0.0;
    loop {
        let t = world.resource::<Battle>().time;
        if t >= next || world.resource::<Battle>().over() {
            next += 10.0;
            let b = world.resource::<Battle>().clone();
            println!("t={:>5.0}s destroyed {:?} fled {:?} lost {:?}", t, b.destroyed, b.fled, b.lost);
            let mut divs: Vec<(Entity, Division)> = world.query::<(Entity, &Division)>().iter(&world).map(|(e, d)| (e, d.clone())).collect();
            divs.sort_by_key(|d| (d.1.side, d.1.number));
            let hash = world.resource::<SpatialHash>();
            for (e, d) in &divs {
                let c = hash.centroid(*e).unwrap_or(d.anchor);
                print!("  {}:{:>3} {:<6} {:<8} ({:>5.0},{:>5.0}) sh{:.2}|", d.label(), d.alive, d.morale.name(), d.order.name(), c.x, c.y, d.shock);
                if d.number == 6 { println!(); }
            }
            if b.over() { println!("{:?}", b.result); break; }
        }
        if let Some(dir) = &shots
            && t >= next_shot
        {
            next_shot += 15.0;
            snapshot(&mut world, &format!("{dir}/t{:03}.ppm", t as u32));
        }
        step(&mut world);
        let now = world.resource::<Battle>().time;
        for ev in world.resource_mut::<Messages<SimEvent>>().drain() {
            match ev {
                SimEvent::Shot { .. } | SimEvent::Destroyed { .. } => {}
                e => println!("    {:>5.1}s {:?}", now, e),
            }
        }
    }
}

/// Writes a top-down picture of the battle: Blue ships blue, Red ships
/// orange, shaken yellow, routed grey; flagships white.
fn snapshot(world: &mut World, path: &str) {
    let (w, h) = (900usize, 700usize);
    let scale = 0.45;
    let mut img = vec![[11u8, 14, 20]; w * h];
    let divs: Vec<(Entity, Morale)> = world.query::<(Entity, &Division)>().iter(world).map(|(e, d)| (e, d.morale)).collect();
    let ships: Vec<(Vec2, f32, usize, Entity, bool)> = world
        .query::<(&Pos, &Heading, &Side, &InDivision, Has<FleetFlagship>)>()
        .iter(world)
        .map(|(p, hd, s, d, f)| (p.0, hd.0, s.0, d.0, f))
        .collect();
    for (p, heading, side, div, flag) in ships {
        let morale = divs.iter().find(|d| d.0 == div).map_or(Morale::Steady, |d| d.1);
        let c = match (flag, morale, side) {
            (true, _, _) => [255, 255, 255],
            (_, Morale::Routed, _) => [110, 110, 110],
            (_, Morale::Shaken, _) => [255, 210, 100],
            (_, _, 0) => [74, 158, 255],
            _ => [255, 120, 70],
        };
        let cx = w as f32 / 2.0 + p.x * scale;
        let cy = h as f32 / 2.0 - p.y * scale;
        let nose = Vec2::from_angle(heading);
        for k in -2..=3 {
            for off in [-1.0f32, 0.0, 1.0] {
                let q = Vec2::new(cx, cy) + Vec2::new(nose.x, -nose.y) * k as f32 + Vec2::new(-nose.y, -nose.x) * off * if k > 1 { 0.0 } else { 1.0 };
                let (x, y) = (q.x as i32, q.y as i32);
                if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                    img[y as usize * w + x as usize] = c;
                }
            }
        }
    }
    let mut out = format!("P6 {w} {h} 255\n").into_bytes();
    for px in img {
        out.extend_from_slice(&px);
    }
    std::fs::write(path, out).unwrap();
}
