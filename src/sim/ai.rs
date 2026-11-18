//! The division-level "commander" AI. Every two seconds it re-reads the
//! battle and gives each of its divisions one of the same orders a player
//! can give: pin an enemy division, hit it in the flank, drive a wedge
//! through it, or keep the flagship reserve back in command range.

use bevy::prelude::*;

use super::battle::*;
use super::components::*;
use super::formation::Formation;
use super::spatial::SpatialHash;

const THINK_EVERY: f32 = 2.0;

#[derive(Clone, Copy, Debug)]
struct View {
    e: Entity,
    side: usize,
    centroid: Vec2,
    facing: f32,
    alive: u32,
    morale: Morale,
    flagship: bool,
    formation: Formation,
    order: Order,
}

pub fn commander(
    mut divs: Query<(Entity, &mut Division)>,
    hash: Res<SpatialHash>,
    settings: Res<SimSettings>,
    tuning: Res<Tuning>,
    mut battle: ResMut<Battle>,
) {
    battle.ai_timer += DT;
    if battle.ai_timer < THINK_EVERY {
        return;
    }
    battle.ai_timer -= THINK_EVERY;
    let views: Vec<View> = divs
        .iter()
        .filter(|(_, d)| d.alive > 0 && d.morale != Morale::Routed)
        .map(|(e, d)| View {
            e,
            side: d.side,
            centroid: hash.centroid(e).unwrap_or(d.anchor),
            facing: d.facing,
            alive: d.alive,
            morale: d.morale,
            flagship: d.flagship,
            formation: d.formation,
            order: d.order,
        })
        .collect();
    let mut orders: Vec<(Entity, Order)> = Vec::new();
    for side in 0..2 {
        if settings.control.human_controls(side) || settings.ai[side] != AiKind::Commander {
            continue;
        }
        orders.extend(plan(side, &views, &tuning));
    }
    for (e, order) in orders {
        if let Ok((_, mut d)) = divs.get_mut(e) {
            d.order = order;
        }
    }
}

fn weighted_centroid<'a>(views: impl Iterator<Item = &'a View>) -> Option<Vec2> {
    let (sum, n) = views.fold((Vec2::ZERO, 0.0), |(s, n), v| (s + v.centroid * v.alive as f32, n + v.alive as f32));
    (n > 0.0).then(|| sum / n)
}

fn plan(side: usize, views: &[View], tuning: &Tuning) -> Vec<(Entity, Order)> {
    let enemies: Vec<&View> = views.iter().filter(|v| v.side != side).collect();
    let mine: Vec<&View> = views.iter().filter(|v| v.side == side).collect();
    let (Some(enemy_c), Some(my_c)) = (weighted_centroid(enemies.iter().copied()), weighted_centroid(mine.iter().copied()))
    else {
        return Vec::new();
    };
    let toward = (enemy_c - my_c).normalize_or(Vec2::X);
    let nearest_to = |p: Vec2| enemies.iter().copied().min_by(|a, b| a.centroid.distance(p).total_cmp(&b.centroid.distance(p)));
    let line: Vec<&View> = mine.iter().copied().filter(|v| !v.flagship).collect();
    // Enemy divisions already pinned by someone in front of them.
    let mut pinned: Vec<Entity> = Vec::new();
    let mut orders = Vec::new();

    for d in &line {
        // Stick with the current opponent while it is still close, so
        // divisions do not wheel back and forth between targets.
        let current = match d.order {
            Order::Engage(e) => enemies.iter().copied().find(|v| v.e == e),
            _ => None,
        };
        let nearest = nearest_to(d.centroid);
        let t = match (current, nearest) {
            (Some(c), Some(n)) if c.centroid.distance(d.centroid) < n.centroid.distance(d.centroid) + tuning.range * 1.5 => c,
            (_, Some(n)) => n,
            _ => continue,
        };
        let dist = d.centroid.distance(t.centroid);
        let bearing = (t.centroid - d.centroid).normalize_or(toward);
        let order = if d.formation == Formation::Wedge && d.morale == Morale::Steady {
            // The spearhead drives straight through whatever is nearest and
            // comes about on the far side.
            Order::MoveTo { to: t.centroid + bearing * 350.0, facing: bearing.to_angle() }
        } else if d.morale == Morale::Shaken {
            // Hold, face the threat, let it rally.
            Order::MoveTo { to: d.centroid, facing: bearing.to_angle() }
        } else if dist > tuning.range * 2.0 {
            // Still closing: keep the fleet's line and advance together.
            let to = d.centroid + toward * 200.0;
            Order::MoveTo { to, facing: toward.to_angle() }
        } else if pinned.contains(&t.e) && t.morale != Morale::Shaken {
            // Its opponent is already pinned: wrap around the near flank.
            let fwd = Vec2::from_angle(t.facing);
            let left = fwd.perp();
            let flank = if (d.centroid - t.centroid).dot(left) >= 0.0 { left } else { -left };
            let to = t.centroid + flank * (tuning.range * tuning.standoff + 120.0) - fwd * 60.0;
            Order::MoveTo { to, facing: (t.centroid - to).to_angle() }
        } else {
            pinned.push(t.e);
            Order::Engage(t.e)
        };
        orders.push((d.e, order));
    }

    // The flagship reserve keeps station behind the line, inside command
    // range. It plugs the line when a division routs or the enemy gets
    // close, and otherwise waits.
    if let Some(reserve) = mine.iter().find(|v| v.flagship) {
        let line_c = weighted_centroid(line.iter().copied()).unwrap_or(my_c);
        let station = line_c - toward * (tuning.command_radius * 0.45);
        let threat = nearest_to(reserve.centroid).filter(|t| {
            t.centroid.distance(reserve.centroid) < tuning.range * 2.5 || t.centroid.distance(station) < 300.0
        });
        let order = match threat {
            Some(t) => Order::Engage(t.e),
            None => Order::MoveTo { to: station, facing: toward.to_angle() },
        };
        orders.push((reserve.e, order));
    }
    orders
}
