//! The bridge between campaign and battle: build the two `Force`s for a
//! pending battle, and apply the battle's `Report` back to the fleets.

use super::map::{FORTRESS, SysId, galaxy};
use super::state::*;
use crate::sim::Report;
use crate::sim::battle::{Force, SideMods};
use crate::sim::scenario::FleetShape;

/// The senior admiral's fleet on one side of a battle.
pub fn senior<'a>(c: &'a Campaign, b: &PendingBattle, side: usize) -> Option<&'a Fleet> {
    b.fleets[side].iter().filter_map(|id| c.fleet(*id)).max_by_key(|f| (f.admiral.rank, f.admiral.xp, f.ships, u32::MAX - f.id))
}

/// The formation that answers `enemy` (what the AI picks).
pub fn counter(enemy: FleetShape) -> FleetShape {
    match enemy {
        FleetShape::Line | FleetShape::Columns => FleetShape::Arrow,
        FleetShape::Arrow => FleetShape::Crescent,
        _ => FleetShape::Line,
    }
}

fn mods_for(f: Option<&Fleet>) -> SideMods {
    let mut m = SideMods::default();
    let Some(f) = f else { return m };
    for t in &f.admiral.traits {
        match t {
            Trait::Gunnery => m.fire_rate *= 1.08,
            Trait::Steadfast => m.morale -= 0.08,
            Trait::Maneuver => {
                m.speed *= 1.2;
                m.turn *= 1.2;
            }
            Trait::Logistician => {}
        }
    }
    match f.supply_level() {
        1 => {
            m.fire_rate *= 0.92;
            m.morale += 0.05;
        }
        2 => {
            m.fire_rate *= 0.8;
            m.morale += 0.15;
        }
        _ => {}
    }
    m
}

/// The two sides of a battle as sim forces. `shapes` overrides a side's
/// formation (the player's choice); AI sides counter the other's shape.
pub fn forces_for(c: &Campaign, b: &PendingBattle, shapes: [Option<FleetShape>; 2]) -> [Force; 2] {
    let lead = [senior(c, b, 0), senior(c, b, 1)];
    let base: [FleetShape; 2] = std::array::from_fn(|k| shapes[k].or(lead[k].map(|f| f.shape)).unwrap_or(FleetShape::Line));
    let shape: [FleetShape; 2] = std::array::from_fn(|k| match shapes[k] {
        Some(s) => s,
        None if lead[k].is_none() => FleetShape::Line,
        None => counter(base[1 - k]),
    });
    std::array::from_fn(|k| {
        let fleets: Vec<&Fleet> = b.fleets[k].iter().filter_map(|id| c.fleet(*id)).collect();
        let ships: u32 = fleets.iter().map(|f| f.ships).sum::<u32>() + b.garrison[k];
        let damaged: u32 = fleets.iter().map(|f| f.damaged).sum();
        let mut mods = mods_for(lead[k]);
        // Fortress guns behind the garrison.
        if b.place == Place::System(FORTRESS) && b.garrison[k] > 0 {
            mods.fire_rate *= 1.25;
            mods.morale -= 0.1;
        }
        Force { ships, damaged, shape: shape[k], mods }
    })
}

/// Splits `total` over `weights` by largest remainder.
fn split(total: u32, weights: &[u32]) -> Vec<u32> {
    let sum: u64 = weights.iter().map(|w| *w as u64).sum();
    if sum == 0 {
        return vec![0; weights.len()];
    }
    let exact: Vec<f64> = weights.iter().map(|w| total as f64 * *w as f64 / sum as f64).collect();
    let mut out: Vec<u32> = exact.iter().map(|e| e.floor() as u32).collect();
    let mut left = total - out.iter().sum::<u32>();
    let mut order: Vec<usize> = (0..weights.len()).collect();
    order.sort_by(|a, b| (exact[*b] - exact[*b].floor()).total_cmp(&(exact[*a] - exact[*a].floor())).then(a.cmp(b)));
    for i in order {
        if left == 0 {
            break;
        }
        out[i] += 1;
        left -= 1;
    }
    out
}

fn place_name(p: Place) -> String {
    let g = galaxy();
    match p {
        Place::System(s) => g.systems[s].name.to_string(),
        Place::Lane(a, b) => format!("the {}–{} lane", g.systems[a].name, g.systems[b].name),
    }
}

/// Where a beaten fleet falls back to, if anywhere.
fn retreat_to(c: &Campaign, f: &Fleet, place: Place) -> Option<SysId> {
    let g = galaxy();
    match place {
        Place::Lane(..) => Some(f.prev),
        Place::System(s) => {
            let friendly = |x: SysId| c.systems[x].owner == Some(f.side);
            if f.prev != s && friendly(f.prev) {
                return Some(f.prev);
            }
            g.neighbours(s).filter(|n| friendly(*n)).min_by_key(|n| *n)
        }
    }
}

fn lose_fleet(c: &mut Campaign, id: u32, why: &str) {
    if let Some(f) = c.fleet(id).cloned() {
        c.note(format!("{} and the {} {} ships of the fleet are lost — {why}.", f.admiral.title(), f.ships, super::map::POWERS[f.side]));
        c.replacements.push(Replacement { side: f.side, due: c.turn + 3 });
    }
    c.fleets.retain(|f| f.id != id);
}

fn fall_back(c: &mut Campaign, ids: &[u32], place: Place) {
    for id in ids {
        let Some(f) = c.fleet(*id).cloned() else { continue };
        if f.ships == 0 {
            lose_fleet(c, *id, "destroyed to the last ship");
            continue;
        }
        match retreat_to(c, &f, place) {
            Some(to) => {
                let f = c.fleet_mut(*id).unwrap();
                f.loc = Location::At(to);
                f.path.clear();
            }
            None => lose_fleet(c, *id, "cut off with nowhere to retreat"),
        }
    }
}

fn award(c: &mut Campaign, ids: &[u32]) {
    for id in ids {
        let Some(f) = c.fleet_mut(*id) else { continue };
        f.admiral.xp += 1;
        let rank = RANK_XP.iter().filter(|x| f.admiral.xp >= **x).count() as u32;
        if rank > f.admiral.rank {
            f.admiral.rank = rank;
            // A new trait each promotion, in an order that depends on the
            // admiral so they differ.
            let start = (f.id as usize + f.admiral.name.len()) % 4;
            if let Some(t) = (0..4).map(|k| Trait::ALL[(start + k) % 4]).find(|t| !f.admiral.has(*t)) {
                f.admiral.traits.push(t);
            }
            let msg = format!("{} is promoted ({}).", f.admiral.title(), f.admiral.traits.iter().map(|t| t.name()).collect::<Vec<_>>().join(", "));
            c.note(msg);
        }
    }
}

/// Applies a battle's result: losses and damage spread over the fleets in
/// proportion to their size, garrisons reduced, the loser falls back,
/// winners gain experience.
pub fn apply_report(c: &mut Campaign, battle_id: u32, r: &Report) {
    let Some(pos) = c.pending.iter().position(|b| b.id == battle_id) else { return };
    let b = c.pending.remove(pos);
    c.battles_fought += 1;
    for k in 0..2 {
        let fleets: Vec<u32> = b.fleets[k].iter().copied().filter(|id| c.fleet(*id).is_some()).collect();
        let mut weights: Vec<u32> = fleets.iter().map(|id| c.fleet(*id).unwrap().ships).collect();
        weights.push(b.garrison[k]);
        let alive = split(r.survivors[k].min(weights.iter().sum()), &weights);
        let fleet_alive: Vec<u32> = alive[..fleets.len()].to_vec();
        let damaged = split(r.damaged[k].min(fleet_alive.iter().sum()), &fleet_alive);
        for (i, id) in fleets.iter().enumerate() {
            let f = c.fleet_mut(*id).unwrap();
            f.ships = fleet_alive[i];
            f.damaged = damaged[i].min(f.ships);
        }
        if b.garrison[k] > 0
            && let Place::System(s) = b.place
        {
            c.systems[s].garrison = alive[fleets.len()];
        }
    }
    let lost = [r.start[0].saturating_sub(r.survivors[0]), r.start[1].saturating_sub(r.survivors[1])];
    let name = place_name(b.place);
    let side_label = |k: usize| {
        if b.fleets[k].is_empty() {
            if b.garrison_owner == Some(None) { "the neutral garrison".to_string() } else { "the garrison".to_string() }
        } else {
            super::map::POWERS[k].to_string()
        }
    };
    match r.winner {
        Some(w) => {
            c.note(format!(
                "Battle of {name}: {} defeats {} ({} ships lost to {}).",
                side_label(w),
                side_label(1 - w),
                lost[w],
                lost[1 - w]
            ));
            // A beaten garrison is gone.
            if let Place::System(s) = b.place
                && b.garrison[1 - w] > 0
            {
                c.systems[s].garrison = 0;
            }
            award(c, &b.fleets[w]);
            fall_back(c, &b.fleets[1 - w], b.place);
        }
        None => {
            c.note(format!("Battle of {name} is a draw; both fleets disengage."));
            // The side that doesn't own the place pulls back.
            let owner = match b.place {
                Place::System(s) => c.systems[s].owner,
                Place::Lane(..) => None,
            };
            for k in 0..2 {
                if owner != Some(k) {
                    fall_back(c, &b.fleets[k], b.place);
                }
            }
        }
    }
    c.fleets.retain(|f| f.ships > 0);
}

/// One side declines the battle: its fleets fall back, losing stragglers,
/// and its garrison there (if any) surrenders.
pub fn withdraw(c: &mut Campaign, battle_id: u32, side: usize) {
    let Some(pos) = c.pending.iter().position(|b| b.id == battle_id) else { return };
    let b = c.pending.remove(pos);
    for id in &b.fleets[side] {
        if let Some(f) = c.fleet_mut(*id) {
            let stragglers = f.ships.div_ceil(10);
            f.ships -= stragglers;
            f.damaged = f.damaged.min(f.ships);
        }
    }
    if b.garrison[side] > 0
        && let Place::System(s) = b.place
    {
        c.systems[s].garrison = 0;
    }
    c.note(format!("{} withdraws from {}, losing stragglers.", super::map::POWERS[side], place_name(b.place)));
    fall_back(c, &b.fleets[side], b.place);
    c.fleets.retain(|f| f.ships > 0);
}

/// Seed for a battle, so a campaign replays the same.
pub fn battle_seed(c: &Campaign, b: &PendingBattle) -> u32 {
    c.seed ^ b.id.wrapping_mul(2_654_435_761) ^ c.turn.wrapping_mul(40_503)
}

/// Does the player (side `player`) take part?
pub fn involves(b: &PendingBattle, side: usize) -> bool {
    !b.fleets[side].is_empty()
}
