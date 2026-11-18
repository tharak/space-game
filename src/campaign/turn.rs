//! Turn resolution. `end_turn` moves every fleet and finds the battles;
//! once they are all resolved (fought, auto-resolved or avoided),
//! `finish_turn` handles capture, supply, income, repair, reinforcements and
//! the AI's next orders.

use super::ai;
use super::map::{FORTRESS, Kind, SysId, galaxy};
use super::state::*;

/// Moves fleets and queues this turn's battles in `c.pending`.
pub fn end_turn(c: &mut Campaign) {
    c.log.clear();
    let g = galaxy();
    let mut moves: Vec<(u32, SysId, SysId)> = Vec::new();
    let ids: Vec<u32> = c.fleets.iter().map(|f| f.id).collect();
    for id in ids {
        let f = c.fleet(id).unwrap().clone();
        match f.loc {
            Location::Lane { from, to, left } => {
                let f = c.fleet_mut(id).unwrap();
                f.loc = if left <= 1 { Location::At(to) } else { Location::Lane { from, to, left: left - 1 } };
                let _ = from;
            }
            Location::At(s) => {
                let Some(&next) = f.path.first() else { continue };
                let Some(lane) = g.lane(s, next) else {
                    c.fleet_mut(id).unwrap().path.clear();
                    continue;
                };
                if c.fleets_at(next, f.side).count() >= STACK_LIMIT {
                    continue; // no room yet: wait a turn
                }
                let f = c.fleet_mut(id).unwrap();
                f.path.remove(0);
                f.prev = s;
                if lane.turns <= 1 {
                    f.loc = Location::At(next);
                    moves.push((id, s, next));
                } else {
                    f.loc = Location::Lane { from: s, to: next, left: lane.turns - 1 };
                }
            }
        }
    }

    let mut pending: Vec<PendingBattle> = Vec::new();
    let mut busy: Vec<u32> = Vec::new();
    let mut next_battle = c.turn * 100;
    let mut push = |pending: &mut Vec<PendingBattle>, place, fleets: [Vec<u32>; 2], garrison: [u32; 2], owner| {
        next_battle += 1;
        pending.push(PendingBattle { id: next_battle, place, fleets, garrison, garrison_owner: owner });
    };

    // Fleets that swapped places along a one-turn lane met halfway: put
    // them back where they started and fight in the lane.
    for (i, &(a, from_a, to_a)) in moves.iter().enumerate() {
        for &(b, from_b, to_b) in &moves[i + 1..] {
            let (fa, fb) = (c.fleet(a).unwrap(), c.fleet(b).unwrap());
            if fa.side != fb.side && from_a == to_b && to_a == from_b && !busy.contains(&a) && !busy.contains(&b) {
                let sides = [fa.side, fb.side];
                for (id, from, to) in [(a, from_a, to_a), (b, from_b, to_b)] {
                    let f = c.fleet_mut(id).unwrap();
                    f.loc = Location::At(from);
                    f.path.insert(0, to);
                }
                let mut fleets: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
                fleets[sides[0]].push(a);
                fleets[sides[1]].push(b);
                busy.extend([a, b]);
                push(&mut pending, Place::Lane(from_a, to_a), fleets, [0, 0], None);
            }
        }
    }
    // Enemies inside the same long lane.
    let in_lanes: Vec<(u32, usize, SysId, SysId)> = c
        .fleets
        .iter()
        .filter_map(|f| match f.loc {
            Location::Lane { from, to, .. } => Some((f.id, f.side, from.min(to), from.max(to))),
            _ => None,
        })
        .collect();
    for &(_, _, a, b) in &in_lanes {
        let here: Vec<&(u32, usize, SysId, SysId)> = in_lanes.iter().filter(|x| x.2 == a && x.3 == b && !busy.contains(&x.0)).collect();
        if here.iter().any(|x| x.1 == 0) && here.iter().any(|x| x.1 == 1) {
            let mut fleets: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
            for x in &here {
                fleets[x.1].push(x.0);
                busy.push(x.0);
            }
            push(&mut pending, Place::Lane(a, b), fleets, [0, 0], None);
        }
    }
    // Systems: both sides present, or one side facing a hostile garrison.
    for s in 0..c.systems.len() {
        let mut fleets: [Vec<u32>; 2] = [Vec::new(), Vec::new()];
        for f in c.fleets.iter().filter(|f| f.at() == Some(s) && !busy.contains(&f.id)) {
            fleets[f.side].push(f.id);
        }
        let sys = &c.systems[s];
        let mut garrison = [0, 0];
        if !fleets[0].is_empty() && !fleets[1].is_empty() {
            if let Some(o) = sys.owner {
                garrison[o] = sys.garrison;
            }
            let owner = sys.owner.filter(|_| sys.garrison > 0).map(Some);
            push(&mut pending, Place::System(s), fleets, garrison, owner);
        } else {
            for k in 0..2 {
                if !fleets[k].is_empty() && sys.owner != Some(k) && sys.garrison > 0 {
                    garrison[1 - k] = sys.garrison;
                    push(&mut pending, Place::System(s), fleets.clone(), garrison, Some(sys.owner));
                }
            }
        }
    }
    c.pending = pending;
}

/// Systems each side can supply: connected to its capital through its own
/// systems.
pub fn supplied(c: &Campaign, side: usize) -> Vec<bool> {
    let g = galaxy();
    let mut ok = vec![false; c.systems.len()];
    let cap = super::map::CAPITALS[side];
    if c.systems[cap].owner != Some(side) {
        return ok;
    }
    let mut queue = vec![cap];
    ok[cap] = true;
    while let Some(u) = queue.pop() {
        for v in g.neighbours(u) {
            if !ok[v] && c.systems[v].owner == Some(side) {
                ok[v] = true;
                queue.push(v);
            }
        }
    }
    ok
}

/// Whether a fleet is in supply: in or next to a supplied friendly system.
pub fn fleet_in_supply(f: &Fleet, ok: &[bool]) -> bool {
    let g = galaxy();
    match f.loc {
        Location::At(s) => ok[s] || g.neighbours(s).any(|n| ok[n]),
        Location::Lane { from, to, .. } => ok[from] || ok[to],
    }
}

/// Capture, supply, income, repair, reinforcements, AI orders, next turn.
pub fn finish_turn(c: &mut Campaign, ai_sides: [bool; 2]) {
    let g = galaxy();
    // Capture: one side alone in a system with no garrison left takes it.
    for s in 0..c.systems.len() {
        let present: Vec<usize> = (0..2).filter(|k| c.fleets_at(s, *k).next().is_some()).collect();
        if let [k] = present[..]
            && c.systems[s].owner != Some(k)
            && c.systems[s].garrison == 0
        {
            c.systems[s].owner = Some(k);
            let name = g.systems[s].name;
            c.note(format!("{} takes {name}.", super::map::POWERS[k]));
            if g.systems[s].kind == Kind::Capital {
                c.winner = Some(k);
                c.note(format!("{name} has fallen. The {} wins the war.", super::map::POWERS[k]));
            }
        }
    }

    // Supply.
    let ok = [supplied(c, 0), supplied(c, 1)];
    for f in &mut c.fleets {
        if fleet_in_supply(f, &ok[f.side]) {
            f.unsupplied = 0;
        } else {
            f.unsupplied += 1;
        }
    }

    // Income: new ships join a friendly fleet in the system, else the garrison.
    for s in 0..c.systems.len() {
        let Some(owner) = c.systems[s].owner else { continue };
        if !ok[owner][s] {
            continue;
        }
        let mut income = g.systems[s].value;
        let mut here: Vec<usize> =
            (0..c.fleets.len()).filter(|i| c.fleets[*i].side == owner && c.fleets[*i].at() == Some(s)).collect();
        here.sort_by_key(|i| (c.fleets[*i].ships, c.fleets[*i].id));
        for i in here {
            let room = FLEET_CAP.saturating_sub(c.fleets[i].ships);
            let add = room.min(income);
            c.fleets[i].ships += add;
            income -= add;
        }
        c.systems[s].garrison = (c.systems[s].garrison + income).min(GARRISON_CAP);
    }
    // The fortress rebuilds its garrison while held (or unclaimed).
    let fort = &mut c.systems[FORTRESS];
    if fort.owner.is_none_or(|o| ok[o][FORTRESS]) {
        fort.garrison = (fort.garrison + 10).min(if fort.owner.is_none() { 150 } else { GARRISON_CAP });
    }

    // Repair in supplied friendly systems.
    for f in &mut c.fleets {
        if let Some(s) = f.at()
            && c.systems[s].owner == Some(f.side)
            && ok[f.side][s]
            && f.damaged > 0
        {
            f.damaged -= f.damaged.div_ceil(4);
        }
    }

    // Replacement fleets for lost admirals.
    let due: Vec<Replacement> = c.replacements.iter().copied().filter(|r| r.due <= c.turn + 1).collect();
    c.replacements.retain(|r| r.due > c.turn + 1);
    for r in due {
        let cap = super::map::CAPITALS[r.side];
        if c.systems[cap].owner == Some(r.side) && c.fleets.iter().filter(|f| f.side == r.side).count() < MAX_FLEETS {
            let id = c.add_fleet(r.side, cap, 40);
            let name = c.fleet(id).unwrap().admiral.title();
            c.note(format!("{} takes command of a new fleet at {}.", name, g.systems[cap].name));
        }
    }

    // Fleets that ended up with nothing.
    c.fleets.retain(|f| f.ships > 0);

    if c.winner.is_none() && c.turn >= 120 {
        let n = [c.owned(0).count(), c.owned(1).count()];
        c.winner = Some(if n[0] >= n[1] { 0 } else { 1 });
        c.note("The war ends in exhaustion; the side holding more systems claims victory.");
    }
    c.turn += 1;
    for side in 0..2 {
        if ai_sides[side] && c.winner.is_none() {
            ai::orders(c, side);
        }
    }
}
