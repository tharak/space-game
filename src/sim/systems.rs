//! The fixed-step simulation, one ordinary ECS system per concern, chained
//! in the `SimTick` schedule. Nothing here knows about rendering or input.

use bevy::ecs::schedule::ScheduleLabel;
use bevy::prelude::*;

use super::battle::*;
use super::components::*;
use super::rng::Rng;
use super::spatial::{Entry, SpatialHash};

#[derive(ScheduleLabel, Debug, Clone, PartialEq, Eq, Hash)]
pub struct SimTick;

pub fn build_schedule() -> Schedule {
    let mut schedule = Schedule::new(SimTick);
    schedule.add_systems(
        (
            tick_clock,
            rebuild_hash,
            super::ai::commander,
            steer_divisions,
            steer_ships,
            integrate,
            fire,
            remove_fled,
            close_ranks,
            morale,
            check_end,
        )
            .chain()
            .run_if(sim_running),
    );
    schedule
}

pub fn sim_running(battle: Res<Battle>) -> bool {
    !battle.over()
}

pub fn tick_clock(mut battle: ResMut<Battle>) {
    battle.time += DT;
}

pub fn rebuild_hash(mut hash: ResMut<SpatialHash>, ships: Query<(Entity, &Pos, &Heading, &Side, &InDivision)>) {
    hash.rebuild(ships.iter().map(|(entity, pos, heading, side, division)| Entry {
        entity,
        pos: pos.0,
        heading: heading.0,
        side: side.0,
        division: division.0,
    }));
}

/// Direction a side runs to when routed.
pub fn home_dir(side: usize) -> Vec2 {
    if side == 0 { Vec2::NEG_X } else { Vec2::X }
}

/// Moves each division's anchor and wheels its facing according to its
/// order and morale.
pub fn steer_divisions(mut divs: Query<(Entity, &mut Division)>, hash: Res<SpatialHash>, tuning: Res<Tuning>, battle: Res<Battle>) {
    let mut targets: Vec<(Entity, Option<Vec2>)> = Vec::new();
    let mut fighting: Vec<(Entity, usize, Vec2)> = Vec::new();
    for (e, d) in &divs {
        if let Order::Engage(t) = d.order {
            targets.push((e, hash.centroid(t)));
        }
        if d.morale != Morale::Routed
            && let Some(c) = hash.centroid(e)
        {
            fighting.push((e, d.side, c));
        }
    }
    for (e, mut d) in &mut divs {
        d.since_contact += DT;
        d.closest += 10.0 * DT;
        let centroid = hash.centroid(e).unwrap_or(d.anchor);
        let shaken = d.morale == Morale::Shaken;
        // A charge that has gone clear through the enemy wheels about and
        // engages the nearest enemy division, usually from behind.
        if d.order == Order::Charge && d.since_contact > 4.0 && d.since_contact.is_finite() && d.closest > tuning.range * 2.0 {
            let nearest = fighting
                .iter()
                .filter(|(_, s, _)| *s != d.side)
                .min_by(|a, b| a.2.distance_squared(centroid).total_cmp(&b.2.distance_squared(centroid)));
            if let Some((t, _, _)) = nearest {
                d.order = Order::Engage(*t);
            }
        }
        let m = battle.mods_of(d.side);
        let base = tuning.anchor_speed * m.speed * d.pace;
        let ahead = |d: &Division| d.anchor + d.forward() * 100.0;
        let (goal, facing, speed) = if d.morale == Morale::Routed {
            let home = home_dir(d.side);
            (Some(d.anchor + home * 200.0), home.to_angle(), tuning.rout_speed * 0.9)
        } else {
            match d.order {
                Order::Hold => (None, d.facing, 0.0),
                Order::Advance => {
                    // Stop at firing range, or once level with the nearest
                    // enemy division (rather than sailing past it).
                    let passed = fighting
                        .iter()
                        .filter(|(_, s, _)| *s != d.side)
                        .min_by(|a, b| a.2.distance_squared(centroid).total_cmp(&b.2.distance_squared(centroid)))
                        .is_some_and(|(_, _, c)| (*c - centroid).dot(d.forward()) < tuning.range * 0.5);
                    let stop = shaken || passed || d.closest < tuning.range * tuning.standoff;
                    ((!stop).then(|| ahead(&d)), d.facing, base)
                }
                Order::Charge => ((!shaken).then(|| ahead(&d)), d.facing, base),
                Order::MoveTo { to, facing } => {
                    let travel = to - d.anchor;
                    // Face the way you are going on a long move, unless that
                    // would mean turning your back to reach it.
                    let face = if travel.length() > 150.0 && angle_diff(facing, travel.to_angle()).abs() < 1.7 {
                        travel.to_angle()
                    } else {
                        facing
                    };
                    let closer = shaken && travel.dot(d.forward()) > 0.0;
                    ((!closer).then_some(to), face, base)
                }
                Order::Engage(_) => match targets.iter().find(|(x, _)| *x == e).and_then(|(_, c)| *c) {
                    Some(tc) => {
                        let dir = (tc - centroid).normalize_or(d.forward());
                        let want = tc - dir * (tuning.range * tuning.standoff - d.depth * 0.5);
                        // Close to firing range, but never back away from an
                        // enemy that comes closer: hold the position instead.
                        let backing = want.distance(tc) > centroid.distance(tc);
                        let stay = backing || (shaken && want.distance(tc) < centroid.distance(tc));
                        ((!stay).then_some(want), dir.to_angle(), base)
                    }
                    None => {
                        d.order = Order::Hold;
                        (None, d.facing, 0.0)
                    }
                },
                Order::Withdraw => (Some(d.anchor + home_dir(d.side) * 200.0), d.facing, base * 0.7),
            }
        };
        d.facing = turn_toward(d.facing, facing, tuning.div_turn_rate * m.turn * DT);
        if let Some(goal) = goal {
            let to = goal - d.anchor;
            let dist = to.length();
            if dist > 0.5 {
                let dir = to / dist;
                // Never run more than a little ahead of the ships themselves
                // (2.5 s of anchor travel: 30 units in the battle).
                let lead = (d.anchor - centroid).dot(dir);
                if lead < tuning.anchor_speed * 2.5 || d.morale == Morale::Routed {
                    d.anchor += dir * (speed * DT).min(dist);
                }
            }
        }
    }
}

/// Each ship steers for its slot and turns toward what it is shooting at.
pub fn steer_ships(
    mut ships: Query<(&mut Vel, &mut Heading, &Pos, &Slot, &InDivision, &Weapon)>,
    divs: Query<&Division>,
    tuning: Res<Tuning>,
    battle: Res<Battle>,
) {
    for (mut vel, mut heading, pos, slot, div, weapon) in &mut ships {
        let Ok(d) = divs.get(div.0) else { continue };
        let speed = battle.mods_of(d.side).speed * d.pace;
        let target = d.to_world(slot.0);
        let to = target - pos.0;
        let max = match (d.morale, d.order) {
            (Morale::Routed, _) => tuning.rout_speed,
            (_, Order::Charge) => tuning.ship_speed * tuning.charge_mult * speed,
            _ => tuning.ship_speed * speed,
        };
        let len = to.length();
        let desired = if len > 0.5 { to / len * (len * 0.8).min(max) } else { Vec2::ZERO };
        vel.0 = vel.0.lerp(desired, 0.15);
        let want = match (d.morale, weapon.aim) {
            (Morale::Routed, _) => d.facing,
            (_, Some(aim)) => d.facing + angle_diff(d.facing, aim.to_angle()).clamp(-tuning.aim_limit, tuning.aim_limit),
            _ => d.facing,
        };
        heading.0 = turn_toward(heading.0, want, tuning.ship_turn_rate * DT);
    }
}

/// Moves ships and keeps them apart: friends at most a little closer than
/// formation spacing, enemies further, so an intact line physically blocks.
pub fn integrate(
    mut ships: Query<(Entity, &mut Pos, &Vel, &Side, &InDivision)>,
    divs: Query<&Division>,
    hash: Res<SpatialHash>,
    tuning: Res<Tuning>,
    battle: Res<Battle>,
) {
    let s = tuning.spacing;
    let (friend, block) = (s * 0.8, tuning.stop_distance.max(s * 1.4));
    for (e, mut pos, vel, side, div) in &mut ships {
        let p = pos.0;
        // Friends keep a little apart.
        let mut push = Vec2::ZERO;
        let mut blockers: Vec<Vec2> = Vec::new();
        hash.for_each_within(p, block, |o, d2| {
            if o.entity == e {
                return;
            }
            let d = d2.sqrt();
            let away = if d > 1e-4 { (p - o.pos) / d } else if e < o.entity { Vec2::Y } else { Vec2::NEG_Y };
            if o.side == side.0 {
                if d < friend {
                    push += away * (friend - d) * 0.5;
                }
            } else {
                blockers.push(away);
            }
        });
        // Enemies never shove each other: a ship simply stops rather than
        // move into a hostile ship's space (a defending line holds its
        // ground; the way through it is to destroy the ships in the way).
        let mut step = vel.0 * DT + push * 0.5;
        for away in blockers {
            let into = -step.dot(away);
            if into > 0.0 {
                step += away * into;
            }
        }
        pos.0 = p + step;
        let routed = divs.get(div.0).is_ok_and(|d| d.morale == Morale::Routed);
        if !routed {
            let b = battle.bounds;
            pos.0 = pos.0.clamp(b.min, b.max);
        }
    }
}

/// No friendly ship (other than the shooter) sits within `clearance` of the
/// line of fire between `from` and `to`.
pub fn line_clear(hash: &SpatialHash, from: Vec2, to: Vec2, side: usize, me: Entity, clearance: f32) -> bool {
    let v = to - from;
    let dist = v.length();
    if dist < 1e-3 {
        return true;
    }
    let dir = v / dist;
    let mut clear = true;
    hash.for_each_within(from + v * 0.5, dist * 0.5 + clearance, |o, _| {
        if !clear || o.side != side || o.entity == me {
            return;
        }
        let rel = o.pos - from;
        let t = rel.dot(dir);
        if t > 0.0 && t < dist && rel.perp_dot(dir).abs() < clearance {
            clear = false;
        }
    });
    clear
}

/// Formations that concentrate fire on the enemy nearest the fleet's centre.
pub fn focus_fire(shape: super::scenario::FleetShape) -> bool {
    matches!(shape, super::scenario::FleetShape::Arrow | super::scenario::FleetShape::Crescent)
}

/// Which arc of a target (facing `heading`) a shot from `from` lands in:
/// 0 front, 1 flank, 2 rear.
pub fn incoming_arc(target: Vec2, heading: f32, from: Vec2) -> usize {
    let a = angle_diff(heading, (from - target).to_angle()).abs();
    if a <= std::f32::consts::FRAC_PI_3 {
        0
    } else if a <= 2.0 * std::f32::consts::FRAC_PI_3 {
        1
    } else {
        2
    }
}

pub fn hit_chance(tuning: &Tuning, arc: usize, dist: f32) -> f32 {
    tuning.hit[arc] * (1.0 - (1.0 - tuning.range_falloff) * (dist / tuning.range).clamp(0.0, 1.0))
}

/// Every ship whose guns are ready shoots the nearest enemy inside its gun
/// cone. Damage lands after everyone has fired.
#[allow(clippy::too_many_arguments)]
pub fn fire(
    mut commands: Commands,
    mut shooters: Query<(&Pos, &Heading, &Side, &InDivision, &mut Weapon), Without<Supply>>,
    mut hulls: Query<(&mut Ship, Has<FleetFlagship>)>,
    mut divs: Query<&mut Division>,
    hash: Res<SpatialHash>,
    tuning: Res<Tuning>,
    mut battle: ResMut<Battle>,
    mut rng: ResMut<Rng>,
    mut events: MessageWriter<SimEvent>,
) {
    let mut hits: Vec<(Entry, f32)> = Vec::new();
    let mut contact: Vec<Entity> = Vec::new();
    let mut shock: Vec<(Entity, f32)> = Vec::new();
    let mut closest: Vec<(Entity, f32)> = Vec::new();
    let mut flag_down: Vec<usize> = Vec::new();
    // Arrow and Crescent fleets concentrate: each ship shoots, of the enemies
    // it can reach, the one closest to its whole fleet's centre.
    let focus: [Option<Vec2>; 2] = std::array::from_fn(|side| {
        if !focus_fire(battle.shapes[side]) {
            return None;
        }
        let (mut sum, mut n) = (Vec2::ZERO, 0.0);
        let mut ids: Vec<Entity> = hash.divisions.keys().copied().collect();
        ids.sort();
        for (e, d) in ids.into_iter().filter_map(|e| divs.get(e).ok().map(|d| (e, d))) {
            if d.side == side && d.morale != Morale::Routed && d.alive > 0 {
                let c = hash.centroid(e).unwrap_or(d.anchor);
                sum += c * d.alive as f32;
                n += d.alive as f32;
            }
        }
        (n > 0.0).then(|| sum / n)
    });
    // Iterate in the hash's stable order so a seed always replays the same.
    for i in 0..hash.entries.len() {
        let me = hash.entries[i];
        let Ok((pos, heading, side, div, mut weapon)) = shooters.get_mut(me.entity) else { continue };
        weapon.cooldown -= DT;
        let Ok(d) = divs.get(div.0) else { continue };
        if d.morale == Morale::Routed {
            weapon.aim = None;
            continue;
        }
        if weapon.cooldown > 0.0 {
            continue;
        }
        let (p, h, s) = (pos.0, heading.0, side.0);
        // Concentrate only if this division is near its fleet's centre.
        let centre = focus.get(s).copied().flatten().filter(|c| hash.centroid(div.0).is_some_and(|dc| dc.distance(*c) <= tuning.focus_radius));
        let mut nearest: Option<(Entry, f32)> = None;
        let mut candidates: Vec<(Entry, f32, f32)> = Vec::new();
        hash.for_each_within(p, tuning.range, |o, d2| {
            if o.side == s {
                return;
            }
            if nearest.is_none_or(|n| d2 < n.1) {
                nearest = Some((*o, d2));
            }
            let bearing = (o.pos - p).to_angle();
            if angle_diff(h, bearing).abs() <= tuning.fire_arc {
                let score = centre.map_or(d2, |c| o.pos.distance_squared(c));
                candidates.push((*o, d2, score));
            }
        });
        if let Some((_, n2)) = nearest {
            closest.push((div.0, n2.sqrt()));
        }
        if candidates.is_empty() {
            weapon.aim = nearest.map(|(n, _)| n.pos - p);
            continue;
        }
        // Best target first; take the first one no friendly ship is in front of.
        candidates.sort_by(|a, b| a.2.total_cmp(&b.2).then(a.0.entity.cmp(&b.0.entity)));
        let clear = candidates.iter().take(6).find(|(t, _, _)| line_clear(&hash, p, t.pos, s, me.entity, tuning.los_clearance)).copied();
        let Some((target, d2, _)) = clear else {
            // Every shot is masked by friends: turn toward the best target
            // and look again shortly.
            weapon.aim = Some(candidates[0].0.pos - p);
            weapon.cooldown = tuning.blocked_retry;
            continue;
        };
        let dist = d2.sqrt();
        let arc = incoming_arc(target.pos, target.heading, p);
        let hit = rng.chance(hit_chance(&tuning, arc, dist));
        let slow = if d.morale == Morale::Shaken { tuning.shaken_cooldown } else { 1.0 };
        weapon.cooldown = tuning.cooldown * slow / battle.mods_of(s).fire_rate.max(0.1) * rng.range(0.85, 1.15);
        weapon.aim = Some(target.pos - p);
        events.write(SimEvent::Shot { from: p, to: target.pos, side: s, hit, shooter: me.entity });
        contact.push(div.0);
        contact.push(target.division);
        if hit {
            hits.push((target, 1.0));
            shock.push((target.division, tuning.hit_shock[usize::from(arc != 0)]));
        }
        battle.last_shot = battle.time;
        if battle.first_contact.is_none() {
            battle.first_contact = Some(battle.time);
            events.write(SimEvent::Contact);
        }
    }
    for e in contact {
        if let Ok(mut d) = divs.get_mut(e) {
            d.since_contact = 0.0;
        }
    }
    for (e, dist) in closest {
        if let Ok(mut d) = divs.get_mut(e) {
            d.closest = d.closest.min(dist);
        }
    }
    for (e, amount) in shock {
        if let Ok(mut d) = divs.get_mut(e) {
            d.shock += amount / d.start.max(1) as f32;
        }
    }
    for (target, damage) in hits {
        let Ok((mut ship, flagship)) = hulls.get_mut(target.entity) else { continue };
        if ship.hull <= 0.0 {
            continue;
        }
        ship.hull -= damage;
        if ship.hull > 0.0 {
            continue;
        }
        commands.entity(target.entity).despawn();
        if let Some(n) = battle.destroyed.get_mut(target.side) {
            *n += 1;
        }
        let mut label = String::new();
        if let Ok(mut d) = divs.get_mut(target.division) {
            d.alive = d.alive.saturating_sub(1);
            d.shock += 1.0 / d.start.max(1) as f32;
            label = d.label();
        }
        events.write(SimEvent::Destroyed { pos: target.pos, side: target.side, flagship, division: label });
        if flagship {
            if let Some(f) = battle.flag_lost.get_mut(target.side) {
                *f = true;
            }
            events.write(SimEvent::FlagshipLost { side: target.side });
            flag_down.push(target.side);
        }
    }
    // Losing the flagship stuns the whole fleet at once.
    if !flag_down.is_empty() {
        for mut d in &mut divs {
            if flag_down.contains(&d.side) {
                d.shock += tuning.flag_shock;
            }
        }
    }
}

/// Deep formations (wedge, column, ring) close up every few seconds: rear
/// ships step into the gaps at the front. Lines are too thin to do that,
/// so a line that loses ships keeps the hole.
pub fn close_ranks(world: &mut World) {
    let due = {
        let b = world.resource::<Battle>();
        (b.time / 3.0).floor() != ((b.time - DT) / 3.0).floor()
    };
    if !due {
        return;
    }
    let deep: Vec<(Entity, super::formation::Formation)> = world
        .query::<(Entity, &Division)>()
        .iter(world)
        .filter(|(_, d)| d.morale != Morale::Routed && d.alive > 0 && d.formation.closes_ranks() && d.alive < d.slots)
        .map(|(e, d)| (e, d.formation))
        .collect();
    for (e, f) in deep {
        super::set_formation(world, e, f);
    }
}

/// Routed ships that leave the battle area are gone for good.
pub fn remove_fled(
    mut commands: Commands,
    ships: Query<(Entity, &Pos, &InDivision)>,
    mut divs: Query<&mut Division>,
    mut battle: ResMut<Battle>,
) {
    let b = battle.bounds;
    let mut fled = Vec::new();
    for (e, pos, div) in &ships {
        if b.contains(pos.0) {
            continue;
        }
        if let Ok(mut d) = divs.get_mut(div.0)
            && d.morale == Morale::Routed
        {
            d.alive = d.alive.saturating_sub(1);
            fled.push((e, d.side));
        }
    }
    for (e, side) in fled {
        commands.entity(e).despawn();
        if let Some(n) = battle.fled.get_mut(side) {
            *n += 1;
        }
    }
}

/// Once a second: breakthroughs, then each division's morale. Stress grows
/// with recent losses (shock), total losses, a lost flagship, poor supply
/// and being broken through; command presence steadies it.
#[allow(clippy::too_many_arguments)]
pub fn morale(
    mut divs: Query<(Entity, &mut Division)>,
    flagships: Query<(&Pos, &Side), With<FleetFlagship>>,
    hash: Res<SpatialHash>,
    tuning: Res<Tuning>,
    mut battle: ResMut<Battle>,
    mut rng: ResMut<Rng>,
    mut events: MessageWriter<SimEvent>,
) {
    let decay = 0.5f32.powf(DT / tuning.shock_half_life);
    for (_, mut d) in &mut divs {
        d.shock *= decay;
    }
    battle.morale_timer += DT;
    if battle.morale_timer < 1.0 {
        return;
    }
    battle.morale_timer -= 1.0;

    // Breakthroughs: enough of a division's ships are behind an enemy
    // division's rear rank, inside its width.
    let snapshot: Vec<(Entity, Division)> =
        divs.iter().filter(|(_, d)| d.morale != Morale::Routed && d.alive > 0).map(|(e, d)| (e, d.clone())).collect();
    let mut broken: Vec<(Entity, Entity)> = Vec::new();
    for (a, da) in &snapshot {
        let Some(ships) = hash.divisions.get(a) else { continue };
        for (b, db) in &snapshot {
            if da.side == db.side || battle.breakthroughs.contains(&(*a, *b)) {
                continue;
            }
            let Some(cb) = hash.centroid(*b) else { continue };
            let frame = Division { anchor: cb, ..db.clone() };
            let behind = ships
                .members
                .iter()
                .filter(|i| {
                    let local = frame.to_local(hash.entries[**i as usize].pos);
                    local.x < db.depth - tuning.spacing && local.x > db.depth - 250.0 && local.y.abs() < db.half_width
                })
                .count() as u32;
            if behind >= (da.alive / 4).max(6) {
                broken.push((*a, *b));
            }
        }
    }
    // Two divisions that have passed into each other are a melee, not a
    // breakthrough.
    let mutual: Vec<(Entity, Entity)> = broken.iter().filter(|(a, b)| broken.contains(&(*b, *a))).copied().collect();
    broken.retain(|p| !mutual.contains(p));
    for (a, b) in broken {
        let (da, db) = (&snapshot.iter().find(|(e, _)| *e == a).unwrap().1, &snapshot.iter().find(|(e, _)| *e == b).unwrap().1);
        battle.breakthroughs.push((a, b));
        let side = da.side;
        let now = battle.time;
        if let Some(first) = battle.first_breakthrough.get_mut(side)
            && first.is_none()
        {
            *first = Some(now);
        }
        events.write(SimEvent::Breakthrough { side, division: da.label(), through: db.label() });
        if let Ok((_, mut d)) = divs.get_mut(b) {
            d.shock += tuning.k_breakthrough;
        }
    }

    let flag_pos: Vec<(Vec2, usize)> = flagships.iter().map(|(p, s)| (p.0, s.0)).collect();
    let flag_lost = battle.flag_lost;
    let mods = battle.mods;
    let fleet_left = [battle.remaining(0), battle.remaining(1)];
    for (e, mut d) in &mut divs {
        if d.morale == Morale::Routed || d.alive == 0 {
            continue;
        }
        let centroid = hash.centroid(e).unwrap_or(d.anchor);
        let in_command = d.flagship
            || flag_pos.iter().any(|(p, s)| *s == d.side && p.distance(centroid) <= tuning.command_radius);
        let lost = 1.0 - d.alive as f32 / d.start.max(1) as f32;
        let stress = d.shock * tuning.k_shock
            + lost * tuning.k_lost
            + (1.0 - fleet_left.get(d.side).copied().unwrap_or(1.0)) * tuning.k_fleet_lost
            + if flag_lost.get(d.side).copied().unwrap_or(false) { tuning.k_flag_lost } else { 0.0 }
            + mods.get(d.side).map_or(0.0, |m| m.morale)
            - if in_command { tuning.k_command } else { 0.0 }
            + rng.range(-tuning.noise, tuning.noise);
        let next = match d.morale {
            Morale::Steady if stress > tuning.shake_at => Morale::Shaken,
            Morale::Shaken if stress > tuning.rout_at => Morale::Routed,
            Morale::Shaken if stress < tuning.rally_at => Morale::Steady,
            m => m,
        };
        if next != d.morale {
            events.write(SimEvent::MoraleChanged { side: d.side, division: d.label(), from: d.morale, to: next });
            d.morale = next;
        }
    }
}

/// There is no break point: a battle ends when one side has no fighting
/// divisions left (all destroyed, fled or routed — morale does the rest).
/// At the time limit the side with more hull left wins.
pub fn check_end(
    divs: Query<&Division>,
    ships: Query<(&Ship, &Side, &InDivision)>,
    tuning: Res<Tuning>,
    mut battle: ResMut<Battle>,
    mut events: MessageWriter<SimEvent>,
) {
    if battle.endless {
        return;
    }
    let mut fighting = [0u32; 2];
    for d in &divs {
        if d.morale != Morale::Routed
            && let Some(n) = fighting.get_mut(d.side)
        {
            *n += d.alive;
        }
    }
    let lost: [u32; 2] = std::array::from_fn(|s| battle.start[s].saturating_sub(fighting[s]));
    battle.lost = lost;
    let broke: [bool; 2] = std::array::from_fn(|s| fighting[s] == 0);
    let (winner, reason) = if broke[0] || broke[1] {
        let frac = |s: usize| lost[s] as f32 / battle.start[s].max(1) as f32;
        let winner = match (broke[0], broke[1]) {
            (true, false) => Some(1),
            (false, true) => Some(0),
            _ if frac(0) > frac(1) => Some(1),
            _ if frac(1) > frac(0) => Some(0),
            _ => None,
        };
        (winner, EndReason::Broke)
    } else if battle.first_contact.is_some() && battle.time - battle.last_shot > tuning.disengage_after {
        // Worn-out fleets drift apart. Whoever kept clearly more of its
        // fleet in the fight holds the field.
        let left = [battle.remaining(0), battle.remaining(1)];
        let winner = if (left[0] - left[1]).abs() < 0.1 {
            None
        } else if left[0] > left[1] {
            Some(0)
        } else {
            Some(1)
        };
        (winner, EndReason::Disengaged)
    } else if battle.time >= tuning.time_limit {
        let mut hull = [0.0f32; 2];
        for (ship, side, div) in &ships {
            if divs.get(div.0).is_ok_and(|d| d.morale != Morale::Routed)
                && let Some(h) = hull.get_mut(side.0)
            {
                *h += ship.hull;
            }
        }
        let winner = if (hull[0] - hull[1]).abs() < 1.0 {
            None
        } else if hull[0] > hull[1] {
            Some(0)
        } else {
            Some(1)
        };
        (winner, EndReason::TimeLimit)
    } else {
        return;
    };
    battle.result = Some(Outcome { winner, reason, time: battle.time });
    events.write(SimEvent::Over { winner, reason });
}
