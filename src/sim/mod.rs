//! The headless fleet-battle simulation: hundreds of ships as ECS entities,
//! stepped at a fixed rate by the [`systems::SimTick`] schedule. The same
//! code runs in the game, the tests and the balance runner.

pub mod ai;
pub mod battle;
pub mod components;
pub mod formation;
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
    let mut ships: Vec<(Entity, Vec2)> = world
        .query::<(Entity, &Pos, &InDivision)>()
        .iter(world)
        .filter(|(_, _, div)| div.0 == division)
        .map(|(e, p, _)| (e, d.to_local(p.0)))
        .collect();
    ships.sort_by_key(|s| s.0);
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
    if let Some(mut d) = world.get_mut::<Division>(division) {
        d.formation = formation;
        d.slots = ships.len() as u32;
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
