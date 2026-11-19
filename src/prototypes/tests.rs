use super::dominion::*;
use crate::voronoi::Vec2;
use crate::voronoi::sites::VoronoiConfig;

fn small() -> Dominion {
    Dominion::new(&VoronoiConfig { sites: 30, w: 300, h: 180, ..Default::default() })
}

#[test]
fn starts_with_three_fleets_a_side_and_edge_planets_worth_nothing() {
    let d = small();
    assert_eq!(d.fleets.iter().filter(|f| f.side == 0).count(), FLEETS_PER_SIDE);
    assert_eq!(d.fleets.iter().filter(|f| f.side == 1).count(), FLEETS_PER_SIDE);
    for o in d.regions.edge_owners() {
        assert_eq!(d.planets[o as usize].value, 0);
    }
    assert!(d.planets.iter().any(|p| p.value > 0));
}

#[test]
fn income_is_the_value_of_held_planets() {
    let d = small();
    let control = d.control();
    for side in 0..2 {
        let expect: u32 = d.planets.iter().zip(&control).filter(|(_, c)| **c == Control::Side(side)).map(|(p, _)| p.value).sum();
        assert_eq!(d.income()[side], expect);
    }
}

#[test]
fn contested_planets_pay_nobody() {
    let mut d = small();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| p.value > 0).map(|(i, p)| (i, *p)).unwrap();
    // One fleet a side, equally strong and equally far from the planet.
    d.fleets = vec![
        FleetToken { side: 0, pos: p.pos - Vec2::new(20.0, 0.0), strength: 1.0 },
        FleetToken { side: 1, pos: p.pos + Vec2::new(20.0, 0.0), strength: 1.0 },
    ];
    assert_eq!(d.control()[i], Control::Contested);
    let before = d.income();
    d.fleets[0].strength = 2.0;
    assert_eq!(d.control()[i], Control::Side(0));
    assert!(d.income()[0] >= before[0] + p.value);
}

#[test]
fn moving_a_fleet_onto_a_planet_takes_it() {
    let mut d = small();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| p.value > 0).map(|(i, p)| (i, *p)).unwrap();
    let red = d.fleets.iter().position(|f| f.side == 1).unwrap();
    d.fleets[red].pos = p.pos;
    assert_eq!(d.control()[i], Control::Side(1));
}

#[test]
fn a_stronger_fleet_never_holds_less() {
    let mut d = small();
    let before = d.holdings().0[0];
    for f in d.fleets.iter_mut().filter(|f| f.side == 0) {
        f.strength *= 2.0;
    }
    assert!(d.holdings().0[0] >= before);
}

#[test]
fn turns_pay_and_reinforcements_cost() {
    let mut d = small();
    let income = d.income();
    d.end_turn();
    assert_eq!((d.stock, d.turn), (income, 2));
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
    let d = small();
    let a = render(&d, DrawOptions::default());
    assert_eq!((a.w, a.h, a.px.len()), (300, 180, 300 * 180 * 3));
    assert_eq!(render(&d, DrawOptions::default()), a);
    let coarse = render(&d, DrawOptions { step: 2, ..Default::default() });
    assert_eq!(coarse.px.len(), a.px.len());
}
