use super::dominion::*;
use crate::voronoi::Vec2;
use crate::voronoi::sites::VoronoiConfig;

fn cfg() -> VoronoiConfig {
    VoronoiConfig { sites: 30, w: 300, h: 180, ..Default::default() }
}

/// One planet, one Blue fleet sitting on it (default tuning: 0.1 a ship).
fn duel(planet_weight: f32, ships: u32) -> Dominion {
    let mut d = Dominion::new(&cfg(), 1, 1, Tuning::default());
    d.planets[0].weight = planet_weight;
    let pos = d.planets[0].pos;
    d.fleets = vec![FleetToken { side: 0, pos, ships }];
    d
}

#[test]
fn planets_are_a_subset_of_sites_and_start_independent() {
    let d = Dominion::new(&cfg(), 12, 3, Tuning::default());
    assert_eq!(d.planets.len(), 12);
    assert_eq!(d.planet_at.iter().flatten().count(), 12);
    for (i, p) in d.planets.iter().enumerate() {
        assert_eq!(p.owner, Owner::Independent(i), "each planet is its own faction");
        assert_eq!(d.planet_at[p.site], Some(i));
        assert_eq!(p.weight, Tuning::default().planet_weight, "all planets weigh the same");
    }
    // Inland regions are used before edge ones.
    let edge = d.regions.edge_owners();
    assert!(d.planets.iter().all(|p| !edge.contains(&(p.site as u32))));
    // Never more planets than sites.
    assert_eq!(Dominion::new(&cfg(), 999, 3, Tuning::default()).planets.len(), 30);
    assert_eq!(d.holdings(), ([0, 0], 12));
    assert_eq!(d.income(), [0, 0]);
}

#[test]
fn a_stronger_fleet_takes_the_planet() {
    let mut d = duel(5.0, 75);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(0));
    // Captured planets pay at once, and are held from then on.
    assert_eq!(d.stock[0], d.planets[0].value);
    assert_eq!(d.pressure()[0], Pressure::Held(0));
}

#[test]
fn a_weaker_fleet_is_resisted() {
    let mut d = duel(10.0, 75);
    assert_eq!(d.pressure()[0], Pressure::Resists(0));
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Independent(0));
    assert_eq!(d.stock, [0, 0]);
}

#[test]
fn contested_planets_dont_change_hands() {
    let mut d = duel(2.5, 50);
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
    let mut d = duel(5.0, 75);
    d.end_turn();
    let p = d.planets[0].pos;
    d.fleets = vec![FleetToken { side: 1, pos: p, ships: 40 }];
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(0), "Red (4.0) is lighter than the planet (5.0)");
    d.fleets[0].ships = 60;
    d.end_turn();
    assert_eq!(d.planets[0].owner, Owner::Side(1));
}

#[test]
fn a_faction_can_hold_many_planets() {
    let mut d = Dominion::new(&cfg(), 10, 3, Tuning::default());
    // Light planets, and enough reach to cover this small map.
    d.set_tuning(Tuning { planet_weight: 1.0, reach_per_weight: 40.0, ..Tuning::default() });
    // One huge Blue fleet in the middle reaches everything.
    d.fleets = vec![FleetToken { side: 0, pos: Vec2::new(150.0, 90.0), ships: MAX_SHIPS }];
    d.end_turn();
    assert_eq!(d.holdings(), ([10, 0], 0));
    assert_eq!(d.income()[0], d.planets.iter().map(|p| p.value).sum::<u32>());
}

#[test]
fn reinforcements_cost_and_cap() {
    let mut d = Dominion::new(&cfg(), 5, 3, Tuning::default());
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
    let d = Dominion::new(&cfg(), 12, 3, Tuning::default());
    let a = render(&d, DrawOptions::default());
    assert_eq!((a.w, a.h, a.px.len()), (300, 180, 300 * 180 * 3));
    assert_eq!(render(&d, DrawOptions::default()), a);
    assert_eq!(render(&d, DrawOptions { step: 3, ..Default::default() }).px.len(), a.px.len());
}

#[test]
fn fleets_only_reach_as_far_as_their_ships_allow() {
    let mut d = duel(2.5, 50);
    let p = d.planets[0].pos;
    // 50 ships × 0.1 = weight 5, × 14 px = 70 px on a 720-wide map,
    // scaled to this 300-wide one.
    let reach = d.fleets[0].reach(&d.tuning, map_scale(&d.cfg));
    assert!((reach - 70.0 * 300.0 / 720.0).abs() < 1e-3, "{reach}");
    d.fleets[0].pos = p + Vec2::new(reach - 2.0, 0.0);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    d.fleets[0].pos = p + Vec2::new(reach + 2.0, 0.0);
    assert_eq!(d.pressure()[0], Pressure::None);
    // Twice the ships, twice the reach.
    d.fleets[0].ships = 100;
    d.fleets[0].pos = p + Vec2::new(2.0 * reach - 2.0, 0.0);
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    // Reach scales with the reach-per-weight slider.
    d.set_tuning(Tuning { reach_per_weight: 7.0, ..d.tuning });
    assert_eq!(d.pressure()[0], Pressure::None);
}

#[test]
fn fleet_slider_sets_fleets_per_side_and_tones_differ() {
    for n in [1, 4, MAX_FLEETS_PER_SIDE] {
        let d = Dominion::new(&cfg(), 5, n, Tuning::default());
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

#[test]
fn tuning_sets_weights() {
    let mut d = duel(5.0, 50);
    // 50 ships at 0.1 = 5.0: not more than the planet's 5.0.
    assert_eq!(d.pressure()[0], Pressure::Resists(0));
    d.set_tuning(Tuning { weight_per_ship: 0.11, ..d.tuning });
    assert_eq!(d.pressure()[0], Pressure::Falls(0));
    d.set_tuning(Tuning { planet_weight: 6.0, ..d.tuning });
    assert_eq!(d.planets[0].weight, 6.0);
    assert_eq!(d.pressure()[0], Pressure::Resists(0));
}

#[test]
fn fleets_are_drawn_as_battle_ships() {
    assert_eq!(ship_icons(1), 1);
    assert_eq!(ship_icons(50), 10);
    assert_eq!(ship_icons(51), 11);
    assert_eq!(ship_icons(10_000), 40);
    let slots = fleet_slots(10);
    assert_eq!(slots.len(), 10);
    // Centred on the fleet, flagship in the middle.
    let centre = slots.iter().copied().sum::<Vec2>() / 10.0;
    assert!(centre.length() < 1e-3);
    assert!(slots.iter().all(|s| s.length() >= slots[0].length()));
}

#[test]
fn reach_scales_with_the_map_not_the_sites() {
    let t = Tuning::default();
    let fleet = FleetToken { side: 0, pos: Vec2::ZERO, ships: 50 };
    let small = VoronoiConfig { w: 720, h: 400, ..Default::default() };
    let big = VoronoiConfig { w: 1440, h: 800, sites: 200, ..Default::default() };
    assert_eq!(fleet.reach(&t, map_scale(&small)), 70.0);
    // Twice the map, twice the reach: the same share of the map.
    assert_eq!(fleet.reach(&t, map_scale(&big)), 140.0);
}
