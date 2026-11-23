use super::dominion::*;
use crate::sim::components::Morale;
use crate::voronoi::Vec2;
use crate::voronoi::sites::VoronoiConfig;

fn cfg() -> VoronoiConfig {
    VoronoiConfig { sites: 30, w: 720, h: 400, ..Default::default() }
}

fn world() -> Dominion {
    Dominion::new(&cfg(), 12, 3, Tuning::default())
}

/// The map with no production and only these two fleets.
fn duel(a: (Owner, Vec2, f32, u32), b: (Owner, Vec2, f32, u32)) -> (Dominion, u32, u32) {
    let mut d = world();
    d.clear_fleets();
    for p in &mut d.planets {
        p.value = 0;
    }
    let ia = d.add_fleet(a.0, a.1, a.2, a.3, None);
    let ib = d.add_fleet(b.0, b.1, b.2, b.3, None);
    (d, ia, ib)
}

const BLUE: Owner = Owner::Side(0);
const RED: Owner = Owner::Side(1);
const WEST: f32 = std::f32::consts::PI;

fn ships(d: &Dominion, id: u32) -> u32 {
    d.fleet(id).map_or(0, |f| f.ships)
}

#[test]
fn the_world_starts_with_home_planets_fleets_and_garrisons() {
    let d = world();
    let (planets, fleets, ships, free) = d.tally();
    assert_eq!(planets, [1, 1]);
    assert_eq!(fleets, [3, 3]);
    assert_eq!(ships, [3 * START_SHIPS, 3 * START_SHIPS]);
    assert_eq!(free, 10);
    for (i, p) in d.planets.iter().enumerate() {
        if let Owner::Independent(_) = p.owner {
            let g = d.fleets.iter().find(|f| f.home == Some(i)).expect("garrison");
            assert_eq!((g.owner, g.ships), (p.owner, INDEPENDENT_START));
        }
    }
    // Every fleet's ships are ECS entities.
    let total: usize = d.fleets.iter().map(|f| f.ship_px.len()).sum();
    assert_eq!(total as u32, d.fleets.iter().map(|f| f.ships).sum::<u32>());
}

#[test]
fn planets_build_ships_into_their_garrison() {
    let mut d = world();
    d.tuning.secs_per_ship = 6.0;
    // Build mechanics only: upkeep would stop Blue's lone planet building.
    d.tuning.upkeep = 0.0;
    let home = d.planets.iter().position(|p| p.owner == BLUE).unwrap();
    d.planets[home].value = 2;
    // Blue's home has no garrison yet: the first ship starts one.
    d.step(3.05);
    let g = d.fleets.iter().find(|f| f.home == Some(home)).expect("a new garrison");
    let id = g.id;
    assert_eq!(g.ships, 1);
    d.step(21.0);
    let g = d.fleet(id).unwrap();
    assert_eq!(g.ships, 8);
    assert_eq!(g.ship_px.len(), 8);
    // Ordered away, it stops garrisoning and the planet starts a new one.
    d.set_ships(id, 12);
    assert!(d.order_move(id, Vec2::new(360.0, 200.0)));
    d.step(3.1);
    assert!(d.fleets.iter().any(|f| f.home == Some(home) && f.id != id));
}

#[test]
fn independents_never_take_orders_and_cap_their_garrison() {
    let mut d = world();
    d.tuning.secs_per_ship = 2.0;
    let g = d.fleets.iter().find(|f| matches!(f.owner, Owner::Independent(_))).unwrap().clone();
    assert!(!d.controllable_id(g.id));
    assert!(!d.order_move(g.id, Vec2::ZERO));
    d.step(60.0);
    assert!(d.fleets.iter().filter(|f| matches!(f.owner, Owner::Independent(_))).all(|f| f.ships <= INDEPENDENT_CAP));
}

#[test]
fn fleets_move_at_the_set_speed_and_stop_there() {
    let (mut d, a, _) = duel((BLUE, Vec2::new(100.0, 100.0), 0.0, 20), (RED, Vec2::new(650.0, 350.0), WEST, 10));
    assert!(d.order_move(a, Vec2::new(250.0, 100.0)));
    d.step(2.0);
    let x = d.fleet(a).unwrap().pos.x;
    assert!(x > 120.0 && x < 100.0 + 2.0 * d.tuning.speed + 5.0, "{x}");
    d.step(15.0);
    let f = d.fleet(a).unwrap();
    assert!(f.dest.is_none());
    assert!(f.pos.distance(Vec2::new(250.0, 100.0)) < 6.0, "{:?}", f.pos);
}

#[test]
fn fleets_out_of_range_dont_fight() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(100.0, 200.0), 0.0, 30), (RED, Vec2::new(400.0, 200.0), WEST, 30));
    d.step(5.0);
    assert_eq!((ships(&d, a), ships(&d, b)), (30, 30));
    assert!(d.shots.is_empty());
}

#[test]
fn fleets_in_range_fight_with_the_battle_systems() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 30), (RED, Vec2::new(325.0, 200.0), WEST, 30));
    d.step(1.0);
    assert!(d.fleet(a).unwrap().engaged, "shots fired");
    d.step(40.0);
    assert!(ships(&d, a) < 30 && ships(&d, b) < 30, "{} {}", ships(&d, a), ships(&d, b));
}

#[test]
fn a_beaten_fleet_routs_and_flees() {
    let (mut d, a, _) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 12), (RED, Vec2::new(325.0, 200.0), WEST, 80));
    let mut routed = false;
    for _ in 0..120 {
        d.step(1.0);
        match d.fleet(a) {
            Some(f) if f.morale == Morale::Routed => {
                routed = true;
                assert!(!f.controllable());
                assert!(!d.order_move(a, Vec2::new(500.0, 200.0)));
            }
            Some(_) => {}
            None => break,
        }
    }
    assert!(routed, "the small fleet broke before it was wiped out");
}

#[test]
fn formations_are_set_on_controllable_fleets() {
    let (mut d, a, _) = duel((BLUE, Vec2::new(100.0, 200.0), 0.0, 40), (RED, Vec2::new(600.0, 200.0), WEST, 40));
    assert!(d.set_formation(a, Formation::Wedge));
    assert_eq!(d.fleet(a).unwrap().formation, Formation::Wedge);
    d.set_ships(a, 5);
    assert_eq!(ships(&d, a), 5);
    assert!(d.set_formation(a, Formation::Line), "under 10 ships, but away from home: still commandable");
}

#[test]
fn a_planet_falls_only_after_its_garrison_is_gone_and_it_is_held() {
    let mut d = world();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| matches!(p.owner, Owner::Independent(_))).map(|(i, p)| (i, *p)).unwrap();
    d.planets[i].value = 1;
    // A big Blue fleet next to it, facing it: the garrison fights first.
    d.add_fleet(BLUE, p.pos + Vec2::new(-22.0, 0.0), 0.0, 150, None);
    d.step(1.0);
    assert_eq!(d.planets[i].owner, p.owner, "garrison still there");
    d.step(120.0);
    assert!(d.fleets.iter().all(|f| f.home != Some(i) || f.owner == BLUE), "garrison destroyed");
    assert_eq!(d.planets[i].owner, BLUE);
}

#[test]
fn both_sides_present_stops_the_capture() {
    let mut d = world();
    d.clear_fleets();
    let p = d.planets.iter().position(|p| matches!(p.owner, Owner::Independent(_))).unwrap();
    for q in &mut d.planets {
        q.value = 0;
    }
    let at = d.planets[p].pos;
    d.add_fleet(BLUE, at + Vec2::new(-10.0, 0.0), 0.0, 10, None);
    d.add_fleet(RED, at + Vec2::new(10.0, 0.0), WEST, 10, None);
    d.step(HOLD_SECS * 0.9);
    assert!(matches!(d.planets[p].owner, Owner::Independent(_)));
    assert!(d.planets[p].hold.is_none());
}

#[test]
fn deterministic_and_renders() {
    let mut a = world();
    let mut b = world();
    a.step(20.0);
    b.step(20.0);
    assert_eq!(a.fleets, b.fleets);
    // A fixed-size picture, whatever the map size or zoom.
    let mut view = View::fit(&a.cfg, VIEW_W, VIEW_H);
    let bg = render_background(&a, &view, true, 1);
    let c = render(&a, &bg, &view, &DrawOptions::default());
    assert_eq!(render_background(&a, &view, true, 2).px.len(), bg.px.len());
    assert_eq!((c.w, c.h), (VIEW_W, VIEW_H));
    view.zoom_at(Vec2::new(700.0, 400.0), 4.0, &a.cfg);
    let bg = render_background(&a, &view, true, 1);
    assert_eq!(render(&a, &bg, &view, &DrawOptions::default()).px.len(), c.px.len());
}

#[test]
fn the_view_zooms_around_the_cursor_and_stays_on_the_map() {
    let c = cfg();
    let mut v = View::fit(&c, VIEW_W, VIEW_H);
    // Fitted: the whole 720×400 world fills the 1440×800 picture.
    assert_eq!(v.zoom, 2.0);
    assert_eq!(v.to_screen(Vec2::ZERO), Vec2::ZERO);
    let p = Vec2::new(1000.0, 300.0);
    let under = v.to_world(p);
    v.zoom_at(p, 3.0, &c);
    assert_eq!(v.zoom, 6.0);
    assert!((v.to_world(p) - under).length() < 1e-3, "the point under the cursor stays put");
    // Can't zoom out past the whole map or pan off it.
    v.zoom_at(p, 0.01, &c);
    assert_eq!(v.zoom, 2.0);
    v.pan(Vec2::new(-5000.0, 0.0), &c);
    assert_eq!(v.center, Vec2::new(360.0, 200.0));
    // Round trip.
    let w = Vec2::new(123.0, 45.0);
    assert!((v.to_world(v.to_screen(w)) - w).length() < 1e-3);
}

#[test]
fn a_moving_fleet_never_rams_an_enemy() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(100.0, 200.0), 0.0, 20), (RED, Vec2::new(300.0, 200.0), WEST, 30));
    assert!(d.order_move(a, Vec2::new(500.0, 200.0)));
    d.step(12.0);
    let (fa, fb) = (d.fleet(a).unwrap(), d.fleet(b).unwrap());
    let gap = fa.ship_px.iter().flat_map(|s| fb.ship_px.iter().map(move |t| s.0.distance(t.0))).fold(f32::INFINITY, f32::min);
    let stop = d.battle.stop_distance * d.unit();
    assert!(gap > stop * 0.8, "never closer than the stop distance: {gap} vs {stop}");
    assert!(fa.engaged && fb.engaged, "and they fight");
}

#[test]
fn a_fleet_sent_at_a_planet_takes_it() {
    let mut d = world();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| matches!(p.owner, Owner::Independent(_))).map(|(i, p)| (i, *p)).unwrap();
    d.planets[i].value = 1;
    let blue = d.add_fleet(BLUE, p.pos + Vec2::new(-80.0, 0.0), 0.0, 80, None);
    assert!(d.order_move(blue, p.pos));
    d.step(90.0);
    assert_eq!(d.planets[i].owner, BLUE);
}

/// Blue's home planet of size 2 and nothing else on the map.
fn economy() -> (Dominion, usize) {
    let mut d = world();
    d.clear_fleets();
    let home = d.planets.iter().position(|p| p.owner == BLUE).unwrap();
    for (i, p) in d.planets.iter_mut().enumerate() {
        p.value = if i == home { 2 } else { 0 };
    }
    d.tuning.secs_per_ship = 0.5;
    (d, home)
}

#[test]
fn ships_cost_credits_and_wait_when_short() {
    let (mut d, _) = economy();
    d.tuning.upkeep = 0.0;
    d.tuning.ship_cost = 100.0;
    d.credits[0] = 250.0;
    d.tuning.income = 0.0;
    d.tuning.population = 0.0;
    d.step(10.0);
    // Two ships bought, then the planet waits for credits.
    assert_eq!(d.ships_of(BLUE), 2);
    assert!((d.credits_of(BLUE) - 50.0).abs() < 1e-3);
}

#[test]
fn planets_earn_by_size_minus_population() {
    let (mut d, _) = economy();
    d.tuning.ship_cost = 1e6;
    d.credits[0] = 0.0;
    d.step(10.0);
    // Size 2 × (10 − 2) a second for 10 s.
    assert!((d.credits_of(BLUE) - 160.0).abs() < 1.0, "{}", d.credits_of(BLUE));
}

#[test]
fn upkeep_caps_the_fleet_at_what_planets_can_pay() {
    let (mut d, _) = economy();
    d.tuning.ship_cost = 1.0;
    d.credits[0] = 10_000.0;
    // 16 credits a second pay for 160 ships at 0.1 each.
    d.step(120.0);
    assert_eq!(d.ships_of(BLUE), 160);
    assert!(d.net_income(BLUE) >= 0.0);
}

#[test]
fn unpaid_upkeep_makes_ships_desert() {
    let (mut d, _) = economy();
    d.tuning.ship_cost = 1e6;
    let id = d.add_fleet(BLUE, Vec2::new(360.0, 200.0), 0.0, 300, None);
    d.credits[0] = 0.0;
    // 300 ships cost 30 a second against an income of 16.
    d.step(MAINTENANCE_SECS * 6.0 + 0.05);
    let left = ships(&d, id);
    assert!((150..300).contains(&left), "{left}");
    assert!(d.deserted[0] > 0);
    // It settles near what the planet pays for.
    d.step(120.0);
    assert!((155..=165).contains(&ships(&d, id)), "{}", ships(&d, id));
}

#[test]
fn shots_cost_credits_and_a_broke_side_cannot_fire() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 30), (RED, Vec2::new(325.0, 200.0), WEST, 30));
    d.tuning.upkeep = 0.0;
    d.credits[1] = 0.0;
    d.step(20.0);
    assert!(d.credits_of(BLUE) < START_CREDITS);
    assert!(d.shots.iter().all(|s| s.2 == 0), "Red fired with no credits");
    assert!(ships(&d, b) < 30);
    assert_eq!(ships(&d, a), 30);
}

#[test]
fn the_maintenance_phase_repairs_fleets_out_of_combat() {
    use crate::sim::components::Ship;
    let (mut d, _) = economy();
    d.tuning.ship_cost = 1e6;
    d.tuning.upkeep = 0.0;
    d.add_fleet(BLUE, Vec2::new(360.0, 200.0), 0.0, 10, None);
    for mut s in d.world.query::<&mut Ship>().iter_mut(&mut d.world) {
        s.hull = 1.0;
    }
    let before = d.credits_of(BLUE);
    d.step(MAINTENANCE_SECS + 0.05);
    assert!(d.world.query::<&Ship>().iter(&d.world).all(|s| s.hull == s.max_hull));
    // 10 ships × 4 hull points × 2 credits, less the income meanwhile.
    let income = 16.0 * (MAINTENANCE_SECS + 0.05);
    assert!((before + income - d.credits_of(BLUE) - 80.0).abs() < 2.0, "{}", d.credits_of(BLUE));
}
