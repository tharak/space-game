//! Opponent AI.
//!
//! * `Baseline` is a faithful port of the original advance-and-shoot logic
//!   (the same one the Monte-Carlo sim used), kept for comparability.
//! * `Doctrine` answers the playtest findings: it keeps the flagship behind
//!   the line and inside command range of its squadrons, refuses to show
//!   flanks, prefers flank/rear shots and kills, can shoot-and-scoot when in
//!   command, and activates squadrons in a sensible order.

use std::collections::{HashSet, VecDeque};

use bevy::prelude::*;

use super::battle::*;
use super::components::*;
use super::config::*;
use crate::hex::{self, Arc, Hex};

/// Activates one squadron of `side` with the configured AI.
pub fn activate_next(world: &mut World, side: usize) {
    let kind = world.resource::<MatchSettings>().ai[side];
    let unit = match kind {
        AiKind::Baseline => unactivated_of_side(world, side).first().copied(),
        AiKind::Doctrine => choose_unit(world, side),
    };
    let Some(e) = unit else { return };
    world.entity_mut(e).insert(Activated);
    world.resource_mut::<Battle>().last_acted = side;
    match kind {
        AiKind::Baseline => baseline_activate(world, e),
        AiKind::Doctrine => doctrine_activate(world, e),
    }
}

// ---------------------------------------------------------------------------
// Baseline (port of the original prototype)
// ---------------------------------------------------------------------------

/// Nearest legal target, ties broken by lowest strength.
fn pick_target(ships: &[Ship], me: &Ship) -> Option<Ship> {
    let mut best: Option<Ship> = None;
    for t in legal_targets(ships, me) {
        let better = match best {
            None => true,
            Some(b) => {
                let (dt, db) = (hex::dist(me.hex, t.hex), hex::dist(me.hex, b.hex));
                dt < db || (dt == db && t.strength < b.strength)
            }
        };
        if better {
            best = Some(t);
        }
    }
    best
}

fn me(world: &mut World, e: Entity) -> (Vec<Ship>, Ship) {
    let ships = snapshot(world);
    let s = *find(&ships, e).expect("ship exists");
    (ships, s)
}

/// One MP toward the nearest enemy; false if no useful step exists.
fn baseline_step(world: &mut World, e: Entity) -> bool {
    let (ships, s) = me(world, e);
    let Some(ne) = nearest_enemy(&ships, &s) else { return false };
    let d = hex::desired_dir(s.hex, ne.hex);
    if s.facing != d {
        set_facing(world, e, hex::turn_toward(s.facing, d));
        return true;
    }
    let next = hex::neighbor(s.hex, d);
    if in_bounds(next) && !occupied(&ships).contains(&next) && hex::dist(next, ne.hex) < hex::dist(s.hex, ne.hex) {
        set_hex(world, e, next);
        return true;
    }
    false
}

fn fire_best(world: &mut World, e: Entity) -> bool {
    let (ships, s) = me(world, e);
    match pick_target(&ships, &s) {
        Some(t) => {
            fire(world, e, t.e);
            true
        }
        None => false,
    }
}

pub fn baseline_activate(world: &mut World, e: Entity) {
    if !world.entity(e).contains::<Alive>() {
        return;
    }
    let (ships, s) = me(world, e);
    if s.morale == Morale::Routed {
        routed_activation(world, e);
        return;
    }
    world.entity_mut(e).remove::<HitSinceAct>();
    let cmd = in_command(&ships, &s);
    let hold = world.resource::<Battle>().fleets[s.side].formation.holds();
    let tgt = pick_target(&ships, &s);
    if s.morale == Morale::Shaken {
        if tgt.is_some() {
            fire(world, e, tgt.unwrap().e);
            return;
        }
        if !hold && let Some(ne) = nearest_enemy(&ships, &s) {
            let d = hex::desired_dir(s.hex, ne.hex);
            if s.facing != d {
                set_facing(world, e, hex::turn_toward(s.facing, d));
            }
        }
        if cmd {
            fire_best(world, e);
        }
        return;
    }
    if let Some(t) = tgt {
        fire(world, e, t.e);
        return;
    }
    if hold {
        if let Some(ne) = nearest_enemy(&ships, &s)
            && hex::dist(s.hex, ne.hex) <= RANGE + 1
        {
            let d = hex::desired_dir(s.hex, ne.hex);
            if s.facing != d {
                set_facing(world, e, hex::turn_toward(s.facing, d));
            }
        }
        if cmd {
            fire_best(world, e);
        }
        return;
    }
    for _ in 0..MP_MAX {
        if !baseline_step(world, e) {
            break;
        }
    }
    if cmd {
        fire_best(world, e);
    }
}

// ---------------------------------------------------------------------------
// Doctrine AI
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    TurnLeft,
    TurnRight,
    Forward,
    Backward,
}

#[derive(Clone, Debug)]
struct Pose {
    hex: Hex,
    facing: u8,
    path: Vec<Step>,
}

#[derive(Clone, Debug)]
pub struct Plan {
    path: Vec<Step>,
    target: Option<Entity>,
    fire_first: bool,
    pub score: f32,
    pub attack: f32,
}

const PRESSURE_BASE: f32 = 0.25;
const PRESSURE_PER_TURN: f32 = 0.08;

fn hit_chance(arc: Arc, critical: bool) -> f32 {
    let need = match arc {
        Arc::Front => 5,
        Arc::Flank => 4,
        Arc::Rear => 3,
    } + critical as i32;
    ((7 - need).max(0) as f32) / 6.0
}

fn dice_of(s: &Ship) -> f32 {
    match s.morale {
        Morale::Steady => s.strength as f32,
        Morale::Shaken => s.strength.div_ceil(2) as f32,
        Morale::Routed => 0.0,
    }
}

/// Every pose reachable this activation, with the shortest action path.
fn reachable(ships: &[Ship], me: &Ship) -> Vec<Pose> {
    let mut occ = occupied(ships);
    occ.remove(&me.hex);
    let nearest = |at: Hex| {
        alive(ships).filter(|s| s.side != me.side).map(|s| hex::dist(at, s.hex)).min().unwrap_or(i32::MAX)
    };
    let mut seen: HashSet<(Hex, u8)> = HashSet::new();
    let mut out = Vec::new();
    let mut queue = VecDeque::from([(Pose { hex: me.hex, facing: me.facing, path: Vec::new() }, MP_MAX)]);
    seen.insert((me.hex, me.facing));
    let can_enter = |from: Hex, to: Hex| {
        in_bounds(to) && !occ.contains(&to) && !(me.morale == Morale::Shaken && nearest(to) < nearest(from))
    };
    while let Some((pose, mp)) = queue.pop_front() {
        if mp > 0 {
            let mut options: Vec<(Step, Hex, u8, u8)> = vec![
                (Step::TurnLeft, pose.hex, (pose.facing + 1) % 6, mp - 1),
                (Step::TurnRight, pose.hex, (pose.facing + 5) % 6, mp - 1),
            ];
            let fwd = hex::neighbor(pose.hex, pose.facing);
            if can_enter(pose.hex, fwd) {
                options.push((Step::Forward, fwd, pose.facing, mp - 1));
            }
            let back = hex::neighbor(pose.hex, (pose.facing + 3) % 6);
            if mp == MP_MAX && can_enter(pose.hex, back) {
                options.push((Step::Backward, back, pose.facing, 0));
            }
            for (step, h, f, left) in options {
                if seen.insert((h, f)) {
                    let mut path = pose.path.clone();
                    path.push(step);
                    queue.push_back((Pose { hex: h, facing: f, path }, left));
                }
            }
        }
        out.push(pose);
    }
    out
}

struct Ctx<'a> {
    ships: &'a [Ship],
    me: Ship,
    critical: bool,
    hold: bool,
    station: Option<Hex>,
    friend_centroid: Option<(f32, f32)>,
    /// Will to close the distance; grows every turn so mirrored doctrine
    /// fleets cannot both wait forever.
    pressure: f32,
}

fn cart(h: Hex) -> (f32, f32) {
    (h.c as f32 + 0.5 * (h.r & 1) as f32, h.r as f32 * 0.866)
}

fn centroid(hexes: impl Iterator<Item = Hex>) -> Option<(f32, f32)> {
    let (mut x, mut y, mut n) = (0.0, 0.0, 0.0);
    for h in hexes {
        let (cx, cy) = cart(h);
        x += cx;
        y += cy;
        n += 1.0;
    }
    (n > 0.0).then(|| (x / n, y / n))
}

fn nearest_hex_to(p: (f32, f32)) -> Hex {
    board_hexes()
        .min_by(|a, b| {
            let (ax, ay) = cart(*a);
            let (bx, by) = cart(*b);
            let da = (ax - p.0).powi(2) + (ay - p.1).powi(2);
            let db = (bx - p.0).powi(2) + (by - p.1).powi(2);
            da.total_cmp(&db)
        })
        .unwrap()
}

/// Where the flagship wants to sit: behind the friendly centre of mass,
/// away from the enemy, so the line screens it but stays in command.
fn flagship_station(ships: &[Ship], me: &Ship) -> Option<Hex> {
    let friends = centroid(alive(ships).filter(|s| s.side == me.side && s.e != me.e).map(|s| s.hex))?;
    let enemies = centroid(alive(ships).filter(|s| s.side != me.side).map(|s| s.hex))?;
    let (dx, dy) = (friends.0 - enemies.0, friends.1 - enemies.1);
    let len = (dx * dx + dy * dy).sqrt().max(0.001);
    Some(nearest_hex_to((friends.0 + dx / len * 2.0, friends.1 + dy / len * 2.0)))
}

/// Expected value of the best shot from a pose.
fn attack_value(ctx: &Ctx, at: Hex, facing: u8) -> (f32, Option<Entity>) {
    let mut occ = occupied(ctx.ships);
    occ.remove(&ctx.me.hex);
    occ.insert(at);
    let dice = dice_of(&ctx.me);
    let mut best = (0.0, None);
    for t in alive(ctx.ships).filter(|t| t.side != ctx.me.side) {
        if !can_shoot(at, facing, t.hex, &occ) {
            continue;
        }
        let arc = hex::incoming_arc(t.hex, t.facing, at);
        let expected = dice * hit_chance(arc, ctx.critical);
        let mut v = expected;
        if expected + 0.5 >= t.strength as f32 {
            v += 1.0; // a likely kill also triggers contagion
        }
        if arc != Arc::Front {
            v *= 1.15; // flank/rear hits also cost the target morale
        }
        if t.morale == Morale::Shaken {
            v *= 1.15;
        }
        if t.morale == Morale::Routed {
            v *= 0.6; // already out of the fight
        }
        if t.flagship {
            v *= 1.8; // decapitation wins battles
        }
        if v > best.0 {
            best = (v, Some(t.e));
        }
    }
    best
}

/// How much fire this pose invites next: every enemy that could plausibly
/// shoot here, weighted by which of our arcs it would hit.
fn exposure(ctx: &Ctx, at: Hex, facing: u8) -> f32 {
    let mut total = 0.0;
    for e in alive(ctx.ships).filter(|e| e.side != ctx.me.side && e.morale != Morale::Routed) {
        let d = hex::dist(at, e.hex);
        let reach = match d {
            d if d <= RANGE => 1.0,
            d if d == RANGE + 1 => 0.55,
            d if d == RANGE + 2 => 0.25,
            _ => continue,
        };
        let arc = hex::incoming_arc(at, facing, e.hex);
        let morale_cost = if arc == Arc::Front { 1.0 } else { 1.35 };
        let tempo = if e.activated { 0.75 } else { 1.0 };
        total += dice_of(e) * hit_chance(arc, false) * reach * morale_cost * tempo;
    }
    total
}

fn evaluate(ctx: &Ctx, pose: &Pose, attack: f32) -> f32 {
    let me = &ctx.me;
    let exp = exposure(ctx, pose.hex, pose.facing);
    let friends = || alive(ctx.ships).filter(|s| s.side == me.side && s.e != me.e);
    let support =
        friends().filter(|s| s.morale == Morale::Steady && hex::dist(s.hex, pose.hex) == 1).count().min(2) as f32;
    let nearest = alive(ctx.ships)
        .filter(|s| s.side != me.side)
        .map(|s| hex::dist(pose.hex, s.hex))
        .min()
        .unwrap_or(0) as f32;
    if me.flagship {
        let covered = friends().filter(|s| hex::dist(s.hex, pose.hex) <= CMD_R).count() as f32;
        let station = ctx.station.map_or(0.0, |s| hex::dist(s, pose.hex) as f32);
        return attack * 0.5 - exp * 1.6 + covered * 0.3 - station * 0.35 + support * 0.1;
    }
    let cmd = in_command_at(ctx.ships, me.side, pose.hex) as u8 as f32;
    let exposure_weight = if me.morale == Morale::Shaken { 0.8 } else { 0.42 };
    let advance = if ctx.hold { 0.0 } else { -ctx.pressure * (nearest - RANGE as f32).max(0.0) };
    let cohesion = ctx.friend_centroid.map_or(0.0, |c| {
        let (x, y) = cart(pose.hex);
        let d = ((x - c.0).powi(2) + (y - c.1).powi(2)).sqrt();
        -0.06 * (d - 3.0).max(0.0)
    });
    attack - exp * exposure_weight + cmd * 0.6 + support * 0.15 + advance + cohesion
}

/// The best move-and/or-fire plan for one squadron.
pub fn best_plan(world: &mut World, e: Entity) -> Plan {
    let (ships, me) = me(world, e);
    let battle = world.resource::<Battle>();
    let ctx = Ctx {
        ships: &ships,
        me,
        critical: battle.fleets[me.side].supply == Supply::Critical,
        hold: battle.fleets[me.side].formation.holds(),
        station: if me.flagship { flagship_station(&ships, &me) } else { None },
        friend_centroid: centroid(alive(&ships).filter(|s| s.side == me.side && s.e != me.e).map(|s| s.hex)),
        pressure: PRESSURE_BASE + PRESSURE_PER_TURN * battle.turn as f32,
    };
    let cmd = in_command(&ships, &me);
    let (start_attack, start_target) = attack_value(&ctx, me.hex, me.facing);
    let mut best: Option<Plan> = None;
    let mut consider = |plan: Plan| {
        if best.as_ref().is_none_or(|b| plan.score > b.score + 1e-6) {
            best = Some(plan);
        }
    };
    for pose in reachable(&ships, &me) {
        let moved = !pose.path.is_empty();
        let (end_attack, end_target) = attack_value(&ctx, pose.hex, pose.facing);
        // Fire at the end of the move (in command, or not moving at all).
        if cmd || !moved {
            consider(Plan {
                score: evaluate(&ctx, &pose, end_attack),
                attack: end_attack,
                target: end_target,
                fire_first: false,
                path: pose.path.clone(),
            });
        }
        // Shoot, then move (in command only): shoot-and-scoot.
        if cmd && moved && start_target.is_some() {
            consider(Plan {
                score: evaluate(&ctx, &pose, start_attack),
                attack: start_attack,
                target: start_target,
                fire_first: true,
                path: pose.path.clone(),
            });
        }
        // Move without firing (out of command).
        if !cmd && moved {
            consider(Plan {
                score: evaluate(&ctx, &pose, 0.0),
                attack: 0.0,
                target: None,
                fire_first: false,
                path: pose.path,
            });
        }
    }
    best.expect("standing still is always a plan")
}

/// Activation order: routed squadrons first (forced), then the best shots;
/// the flagship moves last unless it is already under threat.
fn choose_unit(world: &mut World, side: usize) -> Option<Entity> {
    let candidates = unactivated_of_side(world, side);
    let mut best: Option<(f32, Entity)> = None;
    for e in candidates {
        let (ships, s) = me(world, e);
        let priority = if s.morale == Morale::Routed {
            1000.0
        } else if s.flagship {
            let ctx = Ctx {
                ships: &ships,
                me: s,
                critical: false,
                hold: false,
                station: None,
                friend_centroid: None,
                pressure: 0.0,
            };
            if exposure(&ctx, s.hex, s.facing) > 1.0 { 500.0 } else { -1000.0 }
        } else {
            best_plan(world, e).attack
        };
        if best.is_none_or(|(p, _)| priority > p) {
            best = Some((priority, e));
        }
    }
    best.map(|(_, e)| e)
}

fn fire_if_legal(world: &mut World, e: Entity, target: Entity) {
    if !world.entity(e).contains::<Alive>() || !world.entity(target).contains::<Alive>() {
        return;
    }
    let (ships, s) = me(world, e);
    if legal_targets(&ships, &s).iter().any(|t| t.e == target) {
        fire(world, e, target);
    }
}

pub fn doctrine_activate(world: &mut World, e: Entity) {
    if !world.entity(e).contains::<Alive>() {
        return;
    }
    if world.get::<Morale>(e) == Some(&Morale::Routed) {
        routed_activation(world, e);
        return;
    }
    world.entity_mut(e).remove::<HitSinceAct>();
    let plan = best_plan(world, e);
    if plan.fire_first && let Some(t) = plan.target {
        fire_if_legal(world, e, t);
    }
    for step in &plan.path {
        if !world.entity(e).contains::<Alive>() {
            return;
        }
        let f = world.get::<Facing>(e).unwrap().0;
        let ok = match step {
            Step::TurnLeft => {
                set_facing(world, e, f + 1);
                true
            }
            Step::TurnRight => {
                set_facing(world, e, f + 5);
                true
            }
            Step::Forward => step_into(world, e, f).is_ok(),
            Step::Backward => step_into(world, e, (f + 3) % 6).is_ok(),
        };
        if !ok {
            break;
        }
    }
    if !plan.fire_first && let Some(t) = plan.target {
        fire_if_legal(world, e, t);
    }
}
