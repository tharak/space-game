use super::dominion::*;
use crate::voronoi::Vec2;
use crate::voronoi::sites::VoronoiConfig;

fn cfg() -> VoronoiConfig {
    VoronoiConfig { sites: 30, w: 300, h: 180, ..Default::default() }
}

/// One planet, one Blue fleet sitting on it.
fn duel(defence: u32, ships: u32) -> Dominion {
    let mut d = Dominion::new(&cfg(), 1, 1);
    d.planets[0].defence = defence;
    let pos = d.planets[0].pos;
    d.fleets = vec![FleetToken { side: 0, pos, ships }];
    d
}

#[test]
fn planets_are_a_subset_of_sites_and_start_independent() {
    let d = Dominion::new(&cfg(), 12, 3);
    assert_eq!(d.planets.len(), 12);
    assert_eq!(d.planet_at.iter().flatten().count(), 12);
    for (i, p) in d.planets.iter().enumerate() {
        assert_eq!(p.owner, Owner::Independent(i), "each planet is its own faction");
        assert_eq!(d.planet_at[p.site], Some(i));
        assert!((MIN_DEFENCE..=MAX_DEFENCE).contains(&p.defence));
    }
    // Inland regions are used before edge ones.
    let edge = d.regions.edge_owners();
    assert!(d.planets.iter().all(|p| !edge.contains(&(p.site as u32))));
    // Never more planets than sites.
    assert_eq!(Dominion::new(&cfg(), 999, 3).planets.len(), 30);
    assert_eq!(d.holdings(), ([0, 0], 12));
    assert_eq!(d.income(), [0, 0]);
}

#[test]
fn a_stronger_fleet_takes_the_planet() {
    let mut d = duel(50, 75);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(0));
    // Captured planets pay at once, and are held from then on.
    assert_eq!(d.stock[0], d.planets[0].value);
    assert_eq!(d.pressure()[0], Pressure::Held(0));
}

#[test]
fn a_weaker_fleet_is_resisted() {
    let mut d = duel(100, 75);
    assert_eq!(d.pressure()[0], Pressure::Resists(0));
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Independent(0));
    assert_eq!(d.stock, [0, 0]);
}

#[test]
fn contested_planets_dont_change_hands() {
    let mut d = duel(25, 50);
    let p = d.planets[0].pos;
    d.fleets = vec![
        FleetToken { side: 0, pos: p - Vec2::new(20.0, 0.0), ships: 50 },
        FleetToken { side: 1, pos: p + Vec2::new(20.0, 0.0), ships: 50 },
    ];
    assert_eq!(d.pressure()[0], Pressure::Contested);
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Independent(0));
}

#[test]
fn a_captured_planet_defends_against_the_other_side() {
    let mut d = duel(50, 75);
    d.end_turn();
    let p = d.planets[0].pos;
    d.fleets = vec![FleetToken { side: 1, pos: p, ships: 40 }];
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(0), "Red has fewer ships than the defence");
    d.fleets[0].ships = 60;
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(1));
}

#[test]
fn a_faction_can_hold_many_planets() {
    let mut d = Dominion::new(&cfg(), 10, 3);
    for p in &mut d.planets {
        p.defence = MIN_DEFENCE;
    }
    // One huge Blue fleet in the middle reaches everything.
    d.fleets = vec![FleetToken { side: 0, pos: Vec2::new(150.0, 90.0), ships: MAX_SHIPS }];
    d.end_turn();
    assert_eq!(d.holdings(), ([10, 0], 0));
    assert_eq!(d.income()[0], d.planets.iter().map(|p| p.value).sum::<u32>());
}

#[test]
fn reinforcements_cost_and_cap() {
    let mut d = Dominion::new(&cfg(), 5, 3);
    let blue = d.fleets.iter().position(|f| f.side == 0).unwrap();
    d.stock[0] = REINFORCE_COST;
    assert!(d.reinforce(blue));
    assert_eq!((d.stock[0], d.fleets[blue].ships), (0, START_SHIPS + REINFORCE_SHIPS));
    assert!(!d.reinforce(blue), "no money left");
    d.stock[0] = 1000;
    while d.reinforce(blue) {}
    assert_eq!(d.fleets[blue].ships, MAX_SHIPS);
}

#[test]
fn renders_deterministically() {
    let d = Dominion::new(&cfg(), 12, 3);
    let a = render(&d, DrawOptions::default());
    assert_eq!((a.w, a.h, a.px.len()), (300, 180, 300 * 180 * 3));
    assert_eq!(render(&d, DrawOptions::default()), a);
    assert_eq!(render(&d, DrawOptions { step: 3, ..Default::default() }).px.len(), a.px.len());
}

#[test]
fn fleets_only_reach_as_far_as_their_ships_allow() {
    let mut d = duel(25, 50);
    let p = d.planets[0].pos;
    // 50 ships reach REACH px: just inside takes it, just outside doesn't.
    d.fleets[0].pos = p + Vec2::new(REACH - 2.0, 0.0);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    d.fleets[0].pos = p + Vec2::new(REACH + 2.0, 0.0);
    assert_eq!(d.pressure()[0], Pressure::None);
    // Twice the ships, twice the reach.
    d.fleets[0].ships = 100;
    d.fleets[0].pos = p + Vec2::new(2.0 * REACH - 2.0, 0.0);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
}

#[test]
fn fleet_slider_sets_fleets_per_side_and_tones_differ() {
    for n in [1, 4, MAX_FLEETS_PER_SIDE] {
        let d = Dominion::new(&cfg(), 5, n);
        assert_eq!(d.fleets.iter().filter(|f| f.side == 0).count(), n);
        assert_eq!(d.fleets.iter().filter(|f| f.side == 1).count(), n);
    }
    for side in 0..2 {
        let tones: Vec<_> = (0..MAX_FLEETS_PER_SIDE).map(|k| fleet_color(side, k)).collect();
        for i in 0..tones.len() {
            for j in i + 1..tones.len() {
                assert_ne!(tones[i], tones[j], "side {side} fleets {i} and {j}");
            }
        }
    }
}
