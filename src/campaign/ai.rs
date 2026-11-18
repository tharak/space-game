//! The rival power's strategy, one decision per fleet per turn: repair
//! when hurt, defend what is threatened, attack what it can beat (jointly
//! if need be), and otherwise mass at the corridor mouths.

use super::map::{CAPITALS, FORTRESS, HUB, Kind, MOUTHS, SysId, galaxy};
use super::state::*;
use super::turn::supplied;

/// Ships an attacker must bring against what is visibly defending `s`.
fn defence(c: &Campaign, s: SysId, side: usize) -> f32 {
    let enemy: f32 = c.fleets.iter().filter(|f| f.side != side && f.at() == Some(s)).map(|f| f.strength()).sum();
    let garrison = if c.systems[s].owner != Some(side) { c.systems[s].garrison as f32 } else { 0.0 };
    let fortress = if s == FORTRESS && garrison > 0.0 { 1.4 } else { 1.0 };
    enemy + garrison * fortress
}

fn target_value(s: SysId) -> f32 {
    let g = galaxy();
    let sys = &g.systems[s];
    sys.value as f32
        + match sys.kind {
            Kind::Capital => 10.0,
            Kind::Fortress | Kind::Hub => 3.0,
            Kind::Normal => 0.0,
        }
}

fn go(c: &mut Campaign, id: u32, to: SysId) {
    let g = galaxy();
    let Some(f) = c.fleet(id) else { return };
    let Some(at) = f.at() else { return };
    if at == to {
        c.fleet_mut(id).unwrap().path.clear();
        return;
    }
    let path = g.path(at, to, |_| true);
    c.fleet_mut(id).unwrap().path = path.unwrap_or_default();
}

pub fn orders(c: &mut Campaign, side: usize) {
    let g = galaxy();
    let ok = supplied(c, side);
    let ids: Vec<u32> = c.fleets.iter().filter(|f| f.side == side && f.at().is_some()).map(|f| f.id).collect();
    let mut decided: Vec<u32> = Vec::new();

    // Absorb garrisons into under-strength fleets standing on them.
    for id in &ids {
        let f = c.fleet(*id).unwrap().clone();
        let s = f.at().unwrap();
        if c.systems[s].owner == Some(side) && !matches!(g.systems[s].kind, Kind::Capital | Kind::Fortress) {
            let take = c.systems[s].garrison.min(FLEET_CAP.saturating_sub(f.ships));
            c.systems[s].garrison -= take;
            c.fleet_mut(*id).unwrap().ships += take;
        }
    }

    // Merge small fleets standing together; raise new fleets from big
    // garrisons, so income turns into mobile power.
    super::state::merge_at_will(c, side);
    raise_fleets(c, side, &ok);
    let ids: Vec<u32> = c.fleets.iter().filter(|f| f.side == side && f.at().is_some()).map(|f| f.id).collect();

    // 1. Repair: badly damaged or cut-off fleets go home.
    for id in &ids {
        let f = c.fleet(*id).unwrap().clone();
        let s = f.at().unwrap();
        let hurt = f.damaged * 100 > f.ships * 35 || f.ships < 30;
        let cut_off = f.unsupplied > 0;
        if hurt || cut_off {
            if c.systems[s].owner == Some(side) && ok[s] {
                c.fleet_mut(*id).unwrap().path.clear();
            } else if let Some(home) = (0..c.systems.len()).filter(|x| ok[*x]).min_by_key(|x| (g.hops(s, *x), *x)) {
                go(c, *id, home);
            }
            decided.push(*id);
        }
    }

    // 2. Defend: an owned system next to an enemy fleet stronger than what
    // holds it gets the nearest free fleet.
    let mut threats: Vec<(SysId, f32)> = Vec::new();
    for s in c.owned(side).collect::<Vec<_>>() {
        let danger: f32 = g
            .neighbours(s)
            .map(|n| c.fleets.iter().filter(|f| f.side != side && f.at() == Some(n)).map(|f| f.strength()).sum::<f32>())
            .fold(0.0, f32::max);
        let held: f32 = c.fleets_at(s, side).map(|f| f.strength()).sum::<f32>() + c.systems[s].garrison as f32;
        if danger > held * 1.1 {
            threats.push((s, danger - held + if g.systems[s].kind == Kind::Capital { 1000.0 } else { 0.0 }));
        }
    }
    threats.sort_by(|a, b| b.1.total_cmp(&a.1));
    for (s, _) in threats {
        let free = ids
            .iter()
            .filter(|id| !decided.contains(id))
            .min_by_key(|id| (g.hops(c.fleet(**id).unwrap().at().unwrap(), s), **id))
            .copied();
        if let Some(id) = free
            && g.hops(c.fleet(id).unwrap().at().unwrap(), s) <= 2
        {
            go(c, id, s);
            decided.push(id);
        }
    }

    // 3. Attack: adjacent targets the free fleets nearby can beat together.
    let mut targets: Vec<(SysId, f32)> = Vec::new();
    for s in 0..c.systems.len() {
        let hostile = c.systems[s].owner != Some(side) || c.fleets.iter().any(|f| f.side != side && f.at() == Some(s));
        if hostile {
            targets.push((s, target_value(s)));
        }
    }
    targets.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    for (s, _) in targets {
        let near: Vec<u32> = ids
            .iter()
            .copied()
            .filter(|id| !decided.contains(id))
            .filter(|id| {
                let at = c.fleet(*id).unwrap().at().unwrap();
                g.lane(at, s).is_some()
            })
            .collect();
        let power: f32 = near.iter().map(|id| c.fleet(*id).unwrap().strength()).sum();
        if !near.is_empty() && power > 1.3 * defence(c, s, side) {
            for id in near.iter().take(STACK_LIMIT) {
                go(c, *id, s);
                decided.push(*id);
            }
        }
    }

    // 4. Otherwise mass on one front: the corridor where the enemy is
    // weaker (re-chosen every few turns), pushing through once it is ours.
    let enemy_near = |m: SysId| -> f32 {
        c.fleets.iter().filter(|f| f.side != side).filter_map(|f| f.at()).filter(|s| g.hops(*s, m) <= 2).count() as f32
    };
    let north = MOUTHS[side][0];
    let south = MOUTHS[side][1];
    let main = if (c.turn / 8 + side as u32).is_multiple_of(3) {
        // Now and then, switch to keep the enemy guessing.
        if enemy_near(north) <= enemy_near(south) { south } else { north }
    } else if enemy_near(north) + c.systems[FORTRESS].garrison as f32 / 100.0 <= enemy_near(south) {
        north
    } else {
        south
    };
    let corridor = if main == north { FORTRESS } else { HUB };
    let front = if c.systems[corridor].owner == Some(side) { corridor } else { main };
    let free: Vec<u32> = ids.iter().copied().filter(|id| !decided.contains(id)).collect();
    for id in free {
        let at = c.fleet(id).unwrap().at().unwrap();
        if at == front || c.fleets_at(front, side).count() < STACK_LIMIT {
            go(c, id, front);
        } else {
            // Front full: wait one step behind it.
            if let Some(step) = g.path(at, front, |_| true).and_then(|p| p.len().checked_sub(2).map(|i| p[i])) {
                go(c, id, step);
            }
        }
    }
    let _ = (CAPITALS, HUB);
}

/// Turns a big garrison into a new fleet under a fresh admiral.
fn raise_fleets(c: &mut Campaign, side: usize, ok: &[bool]) {
    let g = galaxy();
    for s in 0..c.systems.len() {
        if c.systems[s].owner != Some(side) || !ok[s] || s == FORTRESS {
            continue;
        }
        let keep = if g.systems[s].kind == Kind::Capital { 40 } else { 0 };
        let spare = c.systems[s].garrison.saturating_sub(keep);
        if spare >= 60 && c.fleets.iter().filter(|f| f.side == side).count() < MAX_FLEETS {
            let ships = spare.min(FLEET_CAP);
            c.systems[s].garrison -= ships;
            c.add_fleet(side, s, ships);
        }
    }
}
