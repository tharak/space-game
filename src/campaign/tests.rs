use super::battle::*;
use super::map::*;
use super::state::*;
use super::turn::*;
use super::*;
use crate::sim::Report;
use crate::sim::battle::EndReason;

fn report(winner: Option<usize>, start: [u32; 2], survivors: [u32; 2], damaged: [u32; 2]) -> Report {
    Report {
        winner,
        reason: EndReason::Broke,
        time: 120.0,
        first_contact: Some(40.0),
        first_breakthrough: [None, None],
        lost: [start[0] - survivors[0], start[1] - survivors[1]],
        destroyed: [0, 0],
        start,
        survivors,
        damaged,
    }
}

/// A campaign with no fleets, for hand-built situations.
fn empty() -> Campaign {
    let mut c = Campaign::new(1);
    c.fleets.clear();
    c
}

#[test]
fn map_is_connected_and_only_the_corridors_cross_the_nebula() {
    let g = galaxy();
    for s in 0..g.systems.len() {
        assert!(g.hops(0, s) < u32::MAX, "{} unreachable", g.systems[s].name);
    }
    for l in &g.lanes {
        let (xa, xb) = (g.systems[l.a].pos.0, g.systems[l.b].pos.0);
        if xa.signum() != xb.signum() && xa != 0.0 && xb != 0.0 {
            panic!("lane {}–{} crosses the nebula", g.systems[l.a].name, g.systems[l.b].name);
        }
        if xa.abs() < NEBULA_HALF_WIDTH || xb.abs() < NEBULA_HALF_WIDTH {
            assert!([FORTRESS, HUB].contains(&l.a) || [FORTRESS, HUB].contains(&l.b));
        }
    }
    let path = g.path(CAPITALS[0], CAPITALS[1], |_| true).unwrap();
    assert!(path.contains(&FORTRESS) || path.contains(&HUB));
}

#[test]
fn cutting_the_supply_line_escalates() {
    let mut c = empty();
    // A Blue fleet deep in Red space, at Red's corridor mouth.
    let id = c.add_fleet(0, MOUTHS[1][0], 100);
    c.systems[MOUTHS[1][0]].garrison = 0;
    c.systems[MOUTHS[1][0]].owner = Some(0);
    finish_turn(&mut c, [false, false]);
    // Owned but cut off from the capital: no supply.
    assert!(!supplied(&c, 0)[MOUTHS[1][0]]);
    let mut levels = Vec::new();
    for _ in 0..3 {
        levels.push(c.fleet(id).unwrap().supply_level());
        finish_turn(&mut c, [false, false]);
    }
    assert_eq!(levels, vec![1, 1, 2]);
    // Back home, supply is restored.
    c.fleet_mut(id).unwrap().loc = Location::At(CAPITALS[0]);
    finish_turn(&mut c, [false, false]);
    assert_eq!(c.fleet(id).unwrap().supply_level(), 0);
}

#[test]
fn long_lanes_take_two_turns_and_fleets_meet_inside() {
    let mut c = empty();
    let a = c.add_fleet(0, MOUTHS[0][0], 80);
    let b = c.add_fleet(1, MOUTHS[1][0], 80);
    c.fleet_mut(a).unwrap().path = vec![FORTRESS];
    end_turn(&mut c);
    assert!(matches!(c.fleet(a).unwrap().loc, Location::Lane { left: 1, .. }));
    assert!(c.pending.is_empty());
    end_turn(&mut c);
    assert_eq!(c.fleet(a).unwrap().at(), Some(FORTRESS));
    // The fortress garrison fights.
    assert_eq!(c.pending.len(), 1);
    assert_eq!(c.pending[0].garrison[1], c.systems[FORTRESS].garrison);
    let _ = b;

    // Two enemies crossing the same one-turn lane meet halfway.
    let mut c = empty();
    let a = c.add_fleet(0, 4, 80);
    let b = c.add_fleet(1, 7, 80);
    c.systems[7].owner = Some(1);
    c.fleet_mut(a).unwrap().path = vec![7];
    c.fleet_mut(b).unwrap().path = vec![4];
    end_turn(&mut c);
    assert_eq!(c.pending.len(), 1);
    assert_eq!(c.pending[0].place, Place::Lane(4, 7));
}

#[test]
fn income_fills_fleets_then_garrisons_and_needs_supply() {
    let mut c = empty();
    let id = c.add_fleet(0, CAPITALS[0], FLEET_CAP - 1);
    let g0 = c.systems[CAPITALS[0]].garrison;
    finish_turn(&mut c, [false, false]);
    assert_eq!(c.fleet(id).unwrap().ships, FLEET_CAP);
    assert_eq!(c.systems[CAPITALS[0]].garrison, g0 + 2);
    // A system cut off from its capital pays nothing.
    let lowen = 2;
    c.systems[0].owner = Some(1);
    let before = c.systems[lowen].garrison;
    finish_turn(&mut c, [false, false]);
    assert_eq!(c.systems[lowen].garrison, before);
}

#[test]
fn damaged_ships_repair_at_home() {
    let mut c = empty();
    let id = c.add_fleet(0, CAPITALS[0], 100);
    c.fleet_mut(id).unwrap().damaged = 40;
    finish_turn(&mut c, [false, false]);
    assert_eq!(c.fleet(id).unwrap().damaged, 30);
}

#[test]
fn capture_needs_the_garrison_gone() {
    let mut c = empty();
    let target = 16; // Ashgrave, Red
    let id = c.add_fleet(0, target, 100);
    let _ = id;
    finish_turn(&mut c, [false, false]);
    assert_eq!(c.systems[target].owner, Some(1), "a garrisoned system holds");
    c.systems[target].garrison = 0;
    finish_turn(&mut c, [false, false]);
    assert_eq!(c.systems[target].owner, Some(0));
}

#[test]
fn battle_results_spread_over_fleets_and_send_the_loser_home() {
    let mut c = empty();
    let a = c.add_fleet(0, 7, 100);
    let a2 = c.add_fleet(0, 7, 50);
    let b = c.add_fleet(1, 7, 120);
    for id in [a, a2] {
        c.fleet_mut(id).unwrap().prev = 4;
    }
    c.fleet_mut(b).unwrap().prev = 16;
    c.systems[7].garrison = 30;
    end_turn(&mut c);
    assert_eq!(c.pending.len(), 1);
    let bid = c.pending[0].id;
    assert_eq!(c.pending[0].garrison, [30, 0]);
    // Blue (150 + 30 garrison) wins with 90 left, 20 damaged; Red keeps 50.
    apply_report(&mut c, bid, &report(Some(0), [180, 120], [90, 50], [20, 10]));
    let (fa, fa2) = (c.fleet(a).unwrap(), c.fleet(a2).unwrap());
    assert_eq!(fa.ships + fa2.ships + c.systems[7].garrison, 90);
    assert!(fa.ships > fa2.ships);
    assert_eq!(fa.damaged + fa2.damaged, 20);
    assert_eq!(fa.admiral.xp, 1);
    // Red falls back to where it came from... which is Red's Ashgrave.
    assert_eq!(c.fleet(b).unwrap().at(), Some(16));
    assert_eq!(c.fleet(b).unwrap().ships, 50);
}

#[test]
fn a_cut_off_loser_is_lost_and_replaced() {
    let mut c = empty();
    let a = c.add_fleet(0, 16, 100);
    let b = c.add_fleet(1, 16, 60);
    c.fleet_mut(a).unwrap().prev = 16;
    c.systems[16].garrison = 0;
    end_turn(&mut c);
    let bid = c.pending[0].id;
    // Blue loses deep in Red territory with no friendly system adjacent.
    apply_report(&mut c, bid, &report(Some(1), [100, 60], [40, 50], [0, 0]));
    assert!(c.fleet(a).is_none());
    assert_eq!(c.replacements.len(), 1);
    for _ in 0..3 {
        finish_turn(&mut c, [false, false]);
    }
    assert!(c.fleets.iter().any(|f| f.side == 0 && f.at() == Some(CAPITALS[0]) && f.ships == 40));
    let _ = b;
}

#[test]
fn admirals_are_promoted_with_traits() {
    let mut c = empty();
    let id = c.add_fleet(0, 4, 100);
    for _ in 0..5 {
        let b = PendingBattle { id: 7, place: Place::System(4), fleets: [vec![id], vec![]], garrison: [0, 10], garrison_owner: Some(None) };
        c.pending.push(b);
        apply_report(&mut c, 7, &report(Some(0), [100, 10], [100, 0], [0, 0]));
    }
    let adm = &c.fleet(id).unwrap().admiral;
    assert_eq!((adm.xp, adm.rank, adm.traits.len()), (5, 2, 2));
}

#[test]
fn save_round_trip() {
    let mut c = Campaign::new(5);
    ai::orders(&mut c, 0);
    end_turn(&mut c);
    let back = save::from_json(&save::to_json(&c)).unwrap();
    assert_eq!(back, c);
}

#[test]
fn ai_campaigns_stay_consistent() {
    for seed in 1..=2 {
        let mut c = Campaign::new(seed);
        ai::orders(&mut c, 0);
        ai::orders(&mut c, 1);
        while c.winner.is_none() && c.turn <= 30 {
            turn::end_turn(&mut c);
            for b in &c.pending {
                assert!(b.fleets[0].len() + b.fleets[1].len() > 0);
            }
            auto_resolve_all(&mut c);
            assert!(c.pending.is_empty());
            turn::finish_turn(&mut c, [true, true]);
            for side in 0..2 {
                assert!(c.fleets.iter().filter(|f| f.side == side).count() <= MAX_FLEETS);
            }
            for f in &c.fleets {
                assert!(f.ships > 0 && f.ships <= FLEET_CAP && f.damaged <= f.ships, "{f:?}");
            }
            for s in &c.systems {
                assert!(s.garrison <= GARRISON_CAP);
            }
        }
        assert!(c.battles_fought > 3, "seed {seed}: the AIs never fought");
    }
}
