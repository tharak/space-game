use super::dominion::*;
use crate::voronoi::Vec2;
use crate::voronoi::sites::VoronoiConfig;

fn cfg() -> VoronoiConfig {
    VoronoiConfig { sites: 30, w: 300, h: 180, ..Default::default() }
}

/// One planet, one fleet of `side` sitting on it.
fn duel(defence: f32, strength: f32) -> Dominion {
    let mut d = Dominion::new(&cfg(), 1);
    d.planets[0].defence = defence;
    let pos = d.planets[0].pos;
    d.fleets = vec![FleetToken { side: 0, pos, strength }];
    d
}

#[test]
fn planets_are_a_subset_of_sites_and_start_independent() {
    let d = Dominion::new(&cfg(), 12);
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
    assert_eq!(Dominion::new(&cfg(), 999).planets.len(), 30);
    assert_eq!(d.holdings(), ([0, 0], 12));
    assert_eq!(d.income(), [0, 0]);
}

#[test]
fn a_stronger_fleet_takes_the_planet() {
    let mut d = duel(1.0, 1.5);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(0));
    // Captured planets pay at once, and are held from then on.
    assert_eq!(d.stock[0], d.planets[0].value);
    assert_eq!(d.pressure()[0], Pressure::Held(0));
}

#[test]
fn a_weaker_fleet_is_resisted() {
    let mut d = duel(2.0, 1.5);
    assert_eq!(d.pressure()[0], Pressure::Resists(0));
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Independent(0));
    assert_eq!(d.stock, [0, 0]);
}

#[test]
fn contested_planets_dont_change_hands() {
    let mut d = duel(0.5, 1.0);
    let p = d.planets[0].pos;
    d.fleets = vec![
        FleetToken { side: 0, pos: p - Vec2::new(20.0, 0.0), strength: 1.0 },
        FleetToken { side: 1, pos: p + Vec2::new(20.0, 0.0), strength: 1.0 },
    ];
    assert_eq!(d.pressure()[0], Pressure::Contested);
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Independent(0));
}

#[test]
fn a_captured_planet_defends_against_the_other_side() {
    let mut d = duel(1.0, 1.5);
    d.end_turn();
    let p = d.planets[0].pos;
    d.fleets = vec![FleetToken { side: 1, pos: p, strength: 0.8 }];
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(0), "Red is weaker than the defence");
    d.fleets[0].strength = 1.2;
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(1));
}

#[test]
fn a_faction_can_hold_many_planets() {
    let mut d = Dominion::new(&cfg(), 10);
    for p in &mut d.planets {
        p.defence = MIN_DEFENCE;
    }
    // One huge Blue fleet in the middle reaches everything.
    d.fleets = vec![FleetToken { side: 0, pos: Vec2::new(150.0, 90.0), strength: MAX_STRENGTH }];
    d.end_turn();
    assert_eq!(d.holdings(), ([10, 0], 0));
    assert_eq!(d.income()[0], d.planets.iter().map(|p| p.value).sum::<u32>());
}

#[test]
fn reinforcements_cost_and_cap() {
    let mut d = Dominion::new(&cfg(), 5);
    let blue = d.fleets.iter().position(|f| f.side == 0).unwrap();
    d.stock[0] = REINFORCE_COST;
    assert!(d.reinforce(blue));
    assert_eq!((d.stock[0], d.fleets[blue].strength), (0, 1.0 + REINFORCE_STEP));
    assert!(!d.reinforce(blue), "no money left");
    d.stock[0] = 1000;
    while d.reinforce(blue) {}
    assert_eq!(d.fleets[blue].strength, MAX_STRENGTH);
}

#[test]
fn renders_deterministically() {
    let d = Dominion::new(&cfg(), 12);
    let a = render(&d, DrawOptions::default());
    assert_eq!((a.w, a.h, a.px.len()), (300, 180, 300 * 180 * 3));
    assert_eq!(render(&d, DrawOptions::default()), a);
    assert_eq!(render(&d, DrawOptions { step: 3, ..Default::default() }).px.len(), a.px.len());
}
