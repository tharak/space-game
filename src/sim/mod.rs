//! The headless fleet-battle simulation: hundreds of ships as ECS entities,
//! stepped at a fixed rate by the [`systems::SimTick`] schedule. The same
//! code runs in the game, the tests and the balance runner.

pub mod ai;
pub mod battle;
pub mod components;
pub mod formation;
pub mod odds;
pub mod rng;
pub mod scenario;
pub mod spatial;
pub mod systems;
#[cfg(test)]
mod tests;

use bevy::ecs::message::Messages;
use bevy::prelude::*;

use battle::*;
use components::*;
use formation::Formation;
use rng::Rng;
use spatial::SpatialHash;
use systems::SimTick;

/// What a player (or script) can tell a division to do.
#[derive(Message, Clone, Copy, Debug, PartialEq)]
pub enum Command {
    Order { division: Entity, order: Order },
    Formation { division: Entity, formation: Formation },
}

/// Inserts the simulation's resources and schedule (once) and starts a
/// battle for `settings`.
pub fn init_world(world: &mut World, settings: SimSettings, seed: u32) {
    world.insert_resource(settings);
    if !world.contains_resource::<Tuning>() {
        world.init_resource::<Tuning>();
    }
    world.insert_resource(Rng::seeded(seed));
    world.init_resource::<Battle>();
    world.init_resource::<SpatialHash>();
    world.init_resource::<Messages<SimEvent>>();
    if world.get_resource::<Schedules>().is_none_or(|s| !s.contains(SimTick)) {
        world.add_schedule(systems::build_schedule());
    }
    scenario::spawn_battle(world);
    // Index the new ships now so picking and centroids work before the
    // first tick (a player starts paused).
    use bevy::ecs::system::RunSystemOnce;
    let _ = world.run_system_once(systems::rebuild_hash);
}

/// A world with the sim's resources and schedule but no ships: for maps
/// and demos that spawn their own divisions (prototype01, the ECS page).
/// Nobody is AI-commanded.
pub fn open_world(tuning: Tuning, seed: u32, bounds: Rect) -> World {
    let mut world = World::new();
    world.insert_resource(SimSettings { control: Control::Spectate, ai: [AiKind::Advance; 2], ..default() });
    world.insert_resource(tuning);
    world.insert_resource(Rng::seeded(seed));
    world.insert_resource(Battle { bounds, ..default() });
    world.init_resource::<SpatialHash>();
    world.init_resource::<Messages<SimEvent>>();
    world.add_schedule(systems::build_schedule());
    world
}

/// Spawns a division of `ships` in `form` at `anchor`, like a battle does.
pub fn spawn_division(world: &mut World, side: usize, form: Formation, anchor: Vec2, facing: f32, ships: usize, order: Order) -> Entity {
    let spacing = world.resource::<Tuning>().spacing;
    let slots = formation::layout(form, ships, spacing);
    let (depth, half_width) = formation::extent(&slots);
    let number = world.query::<&Division>().iter(world).filter(|d| d.side == side).count() + 1;
    let division = world
        .spawn(Division {
            side,
            number,
            formation: form,
            anchor,
            facing,
            order,
            morale: Morale::Steady,
            start: 0,
            alive: 0,
            slots: ships as u32,
            shock: 0.0,
            flagship: false,
            depth,
            half_width,
            since_contact: f32::INFINITY,
            closest: f32::INFINITY,
        })
        .id();
    let rot = Vec2::from_angle(facing);
    let points: Vec<Vec2> = slots.iter().map(|s| anchor + rot.rotate(*s)).collect();
    spawn_ships(world, division, &points);
    set_formation(world, division, form);
    division
}

/// New ships for `division` at `points` (they take slots on the next re-form).
fn spawn_ships(world: &mut World, division: Entity, points: &[Vec2]) {
    let Some(d) = world.get::<Division>(division).cloned() else { return };
    let tuning = world.resource::<Tuning>().clone();
    for p in points {
        let cd = world.resource_mut::<Rng>().range(0.0, tuning.cooldown);
        world.spawn((
            Ship { hull: tuning.hull, max_hull: tuning.hull },
            Side(d.side),
            Pos(*p),
            Vel::default(),
            Heading(d.facing),
            InDivision(division),
            Slot(Vec2::ZERO),
            Weapon { cooldown: cd, aim: None },
        ));
    }
    let n = points.len() as u32;
    if let Some(mut d) = world.get_mut::<Division>(division) {
        d.start += n;
        d.alive += n;
    }
    if let Some(s) = world.resource_mut::<Battle>().start.get_mut(d.side) {
        *s += n;
    }
}

/// How far behind a division's rear rank its supply ships trail, in gun ranges.
pub const SUPPLY_TRAIL: f32 = 1.5;

/// A supply ship carrying `cargo` joins `division` at `at`; returns it.
pub fn add_supply(world: &mut World, division: Entity, at: Vec2, cargo: [f32; 3]) -> Option<Entity> {
    let before: Vec<Entity> = world.query_filtered::<Entity, With<Ship>>().iter(world).collect();
    spawn_ships(world, division, &[at]);
    let e = world.query_filtered::<Entity, With<Ship>>().iter(world).find(|e| !before.contains(e))?;
    world.entity_mut(e).insert(Supply(cargo));
    reform(world, division);
    Some(e)
}

/// Despawns these ships of a division (not counted as losses) and re-forms it.
pub fn despawn_ships(world: &mut World, division: Entity, ships: &[Entity]) {
    let mut n = 0u32;
    for &e in ships {
        if world.get::<InDivision>(e).is_some_and(|d| d.0 == division) {
            world.despawn(e);
            n += 1;
        }
    }
    if let Some(mut d) = world.get_mut::<Division>(division) {
        d.alive = d.alive.saturating_sub(n);
        d.start = d.start.saturating_sub(n);
    }
    reform(world, division);
}

/// Adds `n` fresh ships to a division at its anchor and re-forms it, as if
/// they had always been there (no change to its losses or morale).
pub fn add_ships(world: &mut World, division: Entity, n: usize) {
    let Some(anchor) = world.get::<Division>(division).map(|d| d.anchor) else { return };
    spawn_ships(world, division, &vec![anchor; n]);
    reform(world, division);
}

/// Takes `n` ships out of a division (the rear-most first) without counting
/// them as losses, and re-forms it.
pub fn remove_ships(world: &mut World, division: Entity, n: usize) {
    let Some(d) = world.get::<Division>(division).cloned() else { return };
    let mut ships: Vec<(Entity, f32)> = world
        .query::<(Entity, &Pos, &InDivision)>()
        .iter(world)
        .filter(|(_, _, div)| div.0 == division)
        .map(|(e, p, _)| (e, d.to_local(p.0).x))
        .collect();
    ships.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let n = n.min(ships.len());
    for (e, _) in &ships[..n] {
        world.despawn(*e);
    }
    if let Some(mut d) = world.get_mut::<Division>(division) {
        d.alive = d.alive.saturating_sub(n as u32);
        d.start = d.start.saturating_sub(n as u32);
    }
    reform(world, division);
}

/// Moves `ships` (same side as `to`) into division `to`, moving their
/// counts with them, and re-forms every division touched. Divisions left
/// empty stay (the caller despawns them).
pub fn transfer_ships(world: &mut World, ships: &[Entity], to: Entity) {
    let mut touched = vec![to];
    let mut moved = 0u32;
    for &e in ships {
        let Some(from) = world.get::<InDivision>(e).map(|d| d.0) else { continue };
        if from == to {
            continue;
        }
        world.entity_mut(e).insert(InDivision(to));
        if let Some(mut d) = world.get_mut::<Division>(from) {
            d.alive = d.alive.saturating_sub(1);
            d.start = d.start.saturating_sub(1);
        }
        if !touched.contains(&from) {
            touched.push(from);
        }
        moved += 1;
    }
    if let Some(mut d) = world.get_mut::<Division>(to) {
        d.alive += moved;
        d.start += moved;
    }
    for d in touched {
        reform(world, d);
    }
}

/// Re-forms a division in its current formation (even a routed one).
fn reform(world: &mut World, division: Entity) {
    let Some(d) = world.get::<Division>(division).cloned() else { return };
    if d.morale == Morale::Routed {
        world.get_mut::<Division>(division).unwrap().morale = Morale::Shaken;
        set_formation(world, division, d.formation);
        world.get_mut::<Division>(division).unwrap().morale = Morale::Routed;
    } else {
        set_formation(world, division, d.formation);
    }
}

pub fn step(world: &mut World) {
    world.run_schedule(SimTick);
}

pub fn apply_command(world: &mut World, command: Command) {
    match command {
        Command::Order { division, order } => {
            if let Some(mut d) = world.get_mut::<Division>(division)
                && d.morale != Morale::Routed
            {
                d.order = order;
            }
        }
        Command::Formation { division, formation } => set_formation(world, division, formation),
    }
}

/// Re-forms a division: new slots for its surviving ships, each slot given
/// to the nearest unassigned ship (front slots first), which also closes up
/// any holes from losses.
pub fn set_formation(world: &mut World, division: Entity, formation: Formation) {
    let spacing = world.resource::<Tuning>().spacing;
    let Some(d) = world.get::<Division>(division).cloned() else { return };
    if d.morale == Morale::Routed {
        return;
    }
    let mut ships: Vec<(Entity, Vec2)> = Vec::new();
    let mut supply: Vec<(Entity, Vec2)> = Vec::new();
    for (e, p, div, s) in world.query::<(Entity, &Pos, &InDivision, Has<Supply>)>().iter(world) {
        if div.0 == division {
            if s { &mut supply } else { &mut ships }.push((e, d.to_local(p.0)));
        }
    }
    ships.sort_by_key(|s| s.0);
    supply.sort_by(|a, b| a.1.y.total_cmp(&b.1.y).then(a.0.cmp(&b.0)));
    let mut slots = formation::layout(formation, ships.len(), spacing);
    let (depth, half_width) = formation::extent(&slots);
    slots.sort_by(|a, b| b.x.total_cmp(&a.x).then(a.y.total_cmp(&b.y)));
    let mut taken = vec![false; ships.len()];
    for slot in slots {
        let pick = (0..ships.len())
            .filter(|i| !taken[*i])
            .min_by(|a, b| ships[*a].1.distance_squared(slot).total_cmp(&ships[*b].1.distance_squared(slot)));
        if let Some(i) = pick {
            taken[i] = true;
            world.entity_mut(ships[i].0).insert(Slot(slot));
        }
    }
    // Supply ships trail on a line behind the rear rank, out of the fight.
    // (A convoy of supply ships alone keeps them at its anchor.)
    let gap = if ships.is_empty() { 0.0 } else { world.resource::<Tuning>().range * SUPPLY_TRAIL };
    let n = supply.len() as f32;
    for (k, (e, _)) in supply.iter().enumerate() {
        let y = (k as f32 - (n - 1.0) / 2.0) * spacing * 1.5;
        world.entity_mut(*e).insert(Slot(Vec2::new(depth - gap, y)));
    }
    if let Some(mut d) = world.get_mut::<Division>(division) {
        d.formation = formation;
        d.slots = (ships.len() + supply.len()) as u32;
        d.depth = depth;
        d.half_width = half_width;
    }
}

/// Summary of one headless battle.
#[derive(Clone, Debug)]
pub struct Report {
    pub winner: Option<usize>,
    pub reason: EndReason,
    pub time: f32,
    pub first_contact: Option<f32>,
    pub first_breakthrough: [Option<f32>; 2],
    pub lost: [u32; 2],
    pub destroyed: [u32; 2],
    pub start: [u32; 2],
    /// Ships still alive at the end, routed ones included.
    pub survivors: [u32; 2],
    /// Of the survivors, how many are below 60% hull.
    pub damaged: [u32; 2],
}

/// Summarises a finished battle in `world` (works for a live game too).
pub fn report(world: &mut World) -> Option<Report> {
    let mut survivors = [0u32; 2];
    let mut damaged = [0u32; 2];
    for (ship, side) in world.query::<(&Ship, &Side)>().iter(world) {
        survivors[side.0] += 1;
        if ship.hull < ship.max_hull * 0.6 {
            damaged[side.0] += 1;
        }
    }
    let b = world.resource::<Battle>();
    let outcome = b.result.clone()?;
    Some(Report {
        winner: outcome.winner,
        reason: outcome.reason,
        time: outcome.time,
        first_contact: b.first_contact,
        first_breakthrough: b.first_breakthrough,
        lost: b.lost,
        destroyed: b.destroyed,
        start: b.start,
        survivors,
        damaged,
    })
}

/// A battle simulated a slice at a time in its own `World`, so the game can
/// auto-resolve a big battle without freezing a frame.
pub struct AutoResolve {
    world: World,
}

impl AutoResolve {
    pub fn new(settings: SimSettings, seed: u32) -> Self {
        let mut world = World::new();
        let settings = SimSettings { control: Control::Spectate, ..settings };
        init_world(&mut world, settings, seed);
        Self { world }
    }

    /// Runs up to `ticks` steps; returns the report once the battle is over.
    pub fn advance(&mut self, ticks: u32) -> Option<Report> {
        for _ in 0..ticks {
            if self.world.resource::<Battle>().over() {
                break;
            }
            step(&mut self.world);
            self.world.resource_mut::<Messages<SimEvent>>().update();
        }
        report(&mut self.world)
    }

    /// Battle time so far, against the time limit (for a progress bar).
    pub fn progress(&self) -> f32 {
        let t = self.world.resource::<Tuning>().time_limit;
        (self.world.resource::<Battle>().time / t).clamp(0.0, 1.0)
    }
}

/// Runs a whole AI-vs-AI battle with no rendering.
pub fn simulate(settings: SimSettings, seed: u32, tuning: Option<Tuning>) -> Report {
    let mut world = World::new();
    if let Some(t) = tuning {
        world.insert_resource(t);
    }
    let settings = SimSettings { control: Control::Spectate, ..settings };
    init_world(&mut world, settings, seed);
    while !world.resource::<Battle>().over() {
        step(&mut world);
        world.resource_mut::<Messages<SimEvent>>().update();
    }
    report(&mut world).expect("battle over")
}
