use super::dominion::*;
use super::captains::Skill;
use super::highways::closest_on_segment;
use crate::sim::components::{Morale, Ship, Stores, Supply};
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
        p.stock = [0.0; 3];
    }
    // Combat only: supplies never run out, captains command any number.
    d.tuning.ammo_per_shot = 0.0;
    d.tuning.food_per_phase = 0.0;
    d.tuning.command = 1000.0;
    let ia = captained(&mut d, a.0, a.1, a.2, a.3);
    let ib = captained(&mut d, b.0, b.1, b.2, b.3);
    (d, ia, ib)
}

/// A fleet of `n` ships at `pos`: for Blue and Red, `n − 1` escorts and a
/// captain's flagship (Survivor: no effect on the fight).
fn captained(d: &mut Dominion, owner: Owner, pos: Vec2, facing: f32, n: u32) -> u32 {
    captained_with(d, owner, pos, facing, n, Skill::Survivor)
}

fn captained_with(d: &mut Dominion, owner: Owner, pos: Vec2, facing: f32, n: u32, skill: Skill) -> u32 {
    if matches!(owner, Owner::Independent(_)) {
        return d.add_fleet(owner, pos, facing, n, None);
    }
    let id = d.add_fleet(owner, pos, facing, n.max(2) - 1, None);
    d.add_captain(id, skill).unwrap();
    id
}

const BLUE: Owner = Owner::Side(0);
const RED: Owner = Owner::Side(1);
const WEST: f32 = std::f32::consts::PI;

/// Sets what every combat ship of a fleet carries.
fn set_stores(d: &mut Dominion, id: u32, per_ship: Stock) {
    for e in d.fleet(id).unwrap().ship_ids.clone() {
        if let Some(mut s) = d.world.get_mut::<Stores>(e) {
            s.0 = per_ship;
        }
    }
}

/// The ends of the longest highway.
fn long_road(d: &Dominion) -> (Vec2, Vec2) {
    let &(a, b) = d
        .highways
        .edges
        .iter()
        .max_by(|x, y| {
            let l = |e: &(usize, usize)| d.highways.nodes[e.0].distance(d.highways.nodes[e.1]);
            l(x).total_cmp(&l(y))
        })
        .unwrap();
    (d.highways.nodes[a], d.highways.nodes[b])
}

fn ships(d: &Dominion, id: u32) -> u32 {
    d.fleet(id).map_or(0, |f| f.ships)
}

#[test]
fn the_world_starts_with_home_planets_fleets_and_garrisons() {
    let d = world();
    let (planets, fleets, ships, free) = d.tally();
    assert_eq!(planets, [1, 1]);
    // Three start fleets each, plus the home planet's garrison.
    assert_eq!(fleets, [4, 4]);
    for (side, n) in ships.iter().enumerate() {
        let home = d.planets.iter().find(|p| p.owner == Owner::Side(side)).unwrap();
        assert_eq!(home.value, MAX_PLANET_SIZE);
        assert_eq!(home.produces, [AMMO, FOOD, PARTS]);
        // Each start fleet: escorts plus the captain's flagship.
        assert_eq!(*n, 3 * (START_SHIPS + 1) + home.value);
    }
    // Three captains a side, all different skills, each leading a fleet.
    for side in 0..2 {
        let caps: Vec<_> = d.captains.iter().filter(|c| c.side == side).collect();
        assert_eq!(caps.len(), 3);
        assert!(caps[0].skill != caps[1].skill && caps[1].skill != caps[2].skill && caps[0].skill != caps[2].skill);
        for c in caps {
            assert!(d.fleets.iter().any(|f| f.captain == Some(c.id) && f.controllable()));
        }
    }
    assert_eq!(START_SHIPS, 0, "flagships start alone");
    assert_eq!(free, 10);
    // Every planet starts with as many ships as its size.
    for (i, p) in d.planets.iter().enumerate() {
        let g = d.fleets.iter().find(|f| f.home == Some(i)).expect("garrison");
        assert_eq!((g.owner, g.ships, g.supply), (p.owner, p.value, 0));
    }
    // Start fleets: lone flagships at the home planet, on its highways.
    for f in d.fleets.iter().filter(|f| f.home.is_none()) {
        assert_eq!((f.ships, f.supply), (1, 0));
        assert!(f.captain.is_some());
        let home = d.planets.iter().find(|p| p.owner == f.owner).unwrap();
        assert!(f.pos.distance(home.pos) <= d.capture_radius());
        let s = d.highways.snap(f.pos).unwrap();
        assert!(s.point.distance(f.pos) < 3.0, "{:?} is off the highways", f.pos);
    }
    // Every fleet's ships are ECS entities.
    let total: usize = d.fleets.iter().map(|f| f.ship_px.len()).sum();
    assert_eq!(total as u32, d.fleets.iter().map(|f| f.ships + f.supply).sum::<u32>());
}

#[test]
fn planets_build_ships_into_their_garrison() {
    let mut d = world();
    d.tuning.secs_per_ship = 6.0;
    // Build mechanics only: no costs, no strikes, no captain with room.
    d.rebellion.threshold = 2.0;
    d.tuning.ship_cost = 0.0;
    d.tuning.command = 0.0;
    let home = d.planets.iter().position(|p| p.owner == BLUE).unwrap();
    d.planets[home].value = 2;
    let g = d.fleets.iter().find(|f| f.home == Some(home)).expect("a garrison");
    let (id, start) = (g.id, g.ships);
    d.step(3.05);
    assert_eq!(ships(&d, id), start + 1);
    d.step(21.0);
    let g = d.fleet(id).unwrap();
    assert_eq!(g.ships, start + 8);
    assert_eq!(g.ship_px.len() as u32, start + 8);
    // Without a captain it can still be ordered away.
    assert!(d.order_move(id, Vec2::new(360.0, 200.0)));
}

#[test]
fn a_new_ship_goes_to_the_nearest_captain_with_room() {
    let (mut d, home) = economy();
    d.tuning.ship_cost = 0.0;
    d.tuning.food_per_phase = 0.0;
    d.tuning.command = 2.0;
    let near_p = d.highways.neighbors[home][0];
    let (a, b) = (d.planets[home].pos, d.planets[near_p].pos);
    // A full captain close by, one with room further away.
    let full = captained(&mut d, BLUE, a + (b - a) * 0.3, 0.0, 3);
    let roomy = captained(&mut d, BLUE, a + (b - a) * 0.7, 0.0, 1);
    d.step(0.6);
    assert_eq!(ships(&d, full), 3);
    let coming: Vec<&Fleet> = d.fleets.iter().filter(|f| f.joining == Some(roomy)).collect();
    assert!(!coming.is_empty(), "a reinforcement is on its way");
    // It travels there and joins; once both have no room, ships stay home.
    d.step(20.0);
    assert_eq!(ships(&d, roomy), 3);
    assert!(!d.fleets.iter().any(|f| f.joining.is_some()));
    let garrison = d.fleets.iter().find(|f| f.home == Some(home)).expect("the rest garrison the planet");
    assert!(garrison.ships > 0);
}

#[test]
fn a_new_ship_joins_the_fleet_resting_at_its_planet() {
    let (mut d, home) = economy();
    d.tuning.ship_cost = 0.0;
    d.tuning.food_per_phase = 0.0;
    let at = d.planets[home].pos;
    let id = captained(&mut d, BLUE, at, 0.0, 4);
    let before = ships(&d, id);
    d.step(1.0);
    assert!(ships(&d, id) > before, "the new ships joined the captain's fleet");
    assert_eq!(d.fleets.iter().filter(|f| f.owner == BLUE).count(), 1, "no new garrison");
}

#[test]
fn independents_never_take_orders_and_cap_their_garrison() {
    let mut d = world();
    d.tuning.secs_per_ship = 2.0;
    let g = d.fleets.iter().find(|f| matches!(f.owner, Owner::Independent(_))).unwrap().clone();
    assert!(!d.controllable_id(g.id));
    assert!(!d.order_move(g.id, Vec2::ZERO));
    d.step(60.0);
    assert!(d.fleets.iter().filter(|f| matches!(f.owner, Owner::Independent(_))).all(|f| {
        let size = d.planets[f.home.unwrap()].value;
        f.ships <= INDEPENDENT_CAP_PER_SIZE * size
    }));
}

#[test]
fn fleets_move_at_the_set_speed_along_a_highway_and_stop_there() {
    let mut d = world();
    d.clear_fleets();
    d.tuning.food_per_phase = 0.0;
    let (from, to) = long_road(&d);
    let a = captained(&mut d, BLUE, from, (to - from).to_angle(), 10);
    assert!(d.order_move(a, to));
    d.step(2.0);
    let gone = d.fleet(a).unwrap().pos.distance(from);
    assert!(gone > 20.0 && gone < 2.0 * d.tuning.speed + 5.0, "{gone}");
    d.step(from.distance(to) / d.tuning.speed + 10.0);
    let f = d.fleet(a).unwrap();
    assert!(f.dest.is_none());
    assert!(f.pos.distance(to) < 6.0, "{:?}", f.pos);
}

#[test]
fn fleets_only_travel_on_highways() {
    let mut d = world();
    d.clear_fleets();
    d.tuning.food_per_phase = 0.0;
    let home = d.planets.iter().position(|p| p.owner == BLUE).unwrap();
    let far = d.planets.iter().position(|p| p.owner == RED).unwrap();
    d.tuning.secs_per_ship = 1e6;
    let at = d.planets[home].pos;
    let a = captained(&mut d, BLUE, at, 0.0, 6);
    // Ordered to empty space: it goes to the nearest point on a highway.
    let goal = d.planets[far].pos + Vec2::new(-30.0, 25.0);
    assert!(d.order_move(a, goal));
    let end = d.highways.snap(goal).unwrap().point;
    assert_eq!(d.fleet(a).unwrap().dest, Some(end));
    // Every waypoint after the first is a planet or the end.
    let route = d.fleet(a).unwrap().route.clone();
    for w in &route[..route.len() - 1] {
        assert!(d.planets.iter().any(|p| p.pos.distance(*w) < 0.01), "{w:?}");
    }
    for _ in 0..200 {
        d.step(0.5);
        let f = d.fleet(a).unwrap();
        let off = d.highways.edges.iter().map(|&(x, y)| closest_on_segment(d.highways.nodes[x], d.highways.nodes[y], f.pos).distance(f.pos)).fold(f32::INFINITY, f32::min);
        assert!(off < 8.0, "{off} px off the highways at {:?}", f.pos);
        if f.dest.is_none() {
            break;
        }
    }
    // Arrived: the ships close up on the anchor.
    d.step(5.0);
    let f = d.fleet(a).unwrap();
    assert!(f.pos.distance(end) < 6.0, "{:?} {:?} {:?} {:?}", f.pos, end, f.dest, f.route);
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
fn planets_and_fleets_share_one_voronoi_map() {
    let d = world();
    let inf = Influence::new(&d);
    for y in (0..400).step_by(17) {
        for x in (0..720).step_by(23) {
            let site = inf.at(Vec2::new(x as f32, y as f32)).expect("every point is in a cell");
            match site {
                Site::Planet(i) => assert!(i < d.planets.len()),
                Site::Fleet(i) => assert!(d.fleets[i].armed()),
            }
        }
    }
    // At a fleet's centre (away from planets), the fleet's own cell.
    let f = d.fleets.iter().position(|f| f.home.is_none()).unwrap();
    assert_eq!(inf.at(d.fleets[f].pos), Some(Site::Fleet(f)));
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
    let bg = render_background(&a, &view, 1);
    let c = render(&a, &bg, &view, &DrawOptions::default());
    // With a selection (its count is drawn above it).
    let sel = DrawOptions { selected: a.fleets[0].ship_ids.clone(), ..Default::default() };
    assert_eq!(render(&a, &bg, &view, &sel).px.len(), c.px.len());
    assert_eq!(render_background(&a, &view, 2).px.len(), bg.px.len());
    assert_eq!((c.w, c.h), (VIEW_W, VIEW_H));
    view.zoom_at(Vec2::new(700.0, 400.0), 4.0, &a.cfg);
    let bg = render_background(&a, &view, 1);
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
    let (mut d, _, _) = duel((BLUE, Vec2::ZERO, 0.0, 1), (RED, Vec2::ZERO, 0.0, 1));
    d.clear_fleets();
    let (from, to) = long_road(&d);
    let dir = (to - from).normalize();
    let a = captained(&mut d, BLUE, from, dir.to_angle(), 20);
    let b = captained(&mut d, RED, from + dir * 200.0_f32.min(from.distance(to) * 0.6), WEST + dir.to_angle(), 30);
    assert!(d.order_move(a, to));
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
    d.tuning.command = 100.0;
    let blue = captained(&mut d, BLUE, p.pos + Vec2::new(-80.0, 0.0), 0.0, 80);
    assert!(d.order_move(blue, p.pos));
    let mut t = 0.0;
    while d.planets[i].owner != BLUE && t < 90.0 {
        d.step(0.5);
        t += 0.5;
    }
    assert_eq!(d.planets[i].owner, BLUE);
    // A fresh conquest is resented for a minute.
    assert!(d.planets[i].occupation > 0.25);
    d.step(OCCUPATION_SECS + 1.0);
    assert_eq!(d.planets[i].occupation, 0.0);
    assert_eq!(d.planets[i].owner, BLUE, "80 ships keep the peace");
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
    // Content people: nobody ever rebels.
    d.rebellion.threshold = 2.0;
    (d, home)
}

#[test]
fn ships_are_built_of_parts_and_wait_when_short() {
    let (mut d, home) = economy();
    d.tuning.ship_cost = 5.0;
    d.tuning.income = 0.0;
    d.planets[home].stock = [50.0, 50.0, 12.0];
    // (Before the maintenance phase, when the garrison would draw parts.)
    d.step(3.0);
    // Two ships built, then the planet waits for parts.
    assert_eq!(d.ships_of(BLUE), 2);
    assert!((d.planets[home].stock[PARTS] - 2.0).abs() < 1e-3);
}

#[test]
fn planets_make_their_kinds_at_their_tax_share() {
    let (mut d, home) = economy();
    d.tuning.ship_cost = 1e6;
    d.planets[home].produces = [AMMO, FOOD, PARTS];
    d.planets[home].stock = [0.0; 3];
    d.step(10.0);
    // Size 2: one point of ammo, one of food; income 4 × tax 0.2 a second.
    let s = d.planets[home].stock;
    assert!((s[AMMO] - 8.0).abs() < 0.1 && (s[FOOD] - 8.0).abs() < 0.1 && s[PARTS] == 0.0, "{s:?}");
    // Strikes take their share.
    d.planets[home].unrest = 0.5;
    assert!((d.planet_income(BLUE)[AMMO] - 0.4).abs() < 1e-4);
    d.set_tax(home, 2.0);
    assert_eq!(d.planets[home].tax, MAX_TAX);
}

#[test]
fn an_unguarded_overtaxed_planet_revolts() {
    let (mut d, home) = economy();
    d.rebellion = Default::default();
    d.tuning.ship_cost = 1e6;
    // Not a home planet any more (those stay loyal).
    d.planets[home].home_of = None;
    d.set_tax(home, 0.6);
    d.step(60.0);
    let p = d.planets[home];
    assert_eq!(p.owner, Owner::Independent(home));
    assert_eq!(d.revolts[0], 1);
    let rebels = d.fleets.iter().find(|f| f.home == Some(home)).expect("a rebel garrison");
    assert_eq!(rebels.owner, Owner::Independent(home));
    assert!(rebels.ships >= 1);
}

#[test]
fn a_home_planet_never_rebels_against_its_own_side() {
    let (mut d, home) = economy();
    d.rebellion = Default::default();
    d.tuning.ship_cost = 1e6;
    d.set_tax(home, MAX_TAX);
    d.step(60.0);
    assert_eq!(d.planets[home].owner, BLUE);
    assert!(d.planets[home].loyal());
    assert_eq!(d.revolts[0], 0);
    assert_eq!(d.ships_needed(home), Some(0));
    // Held by the enemy, it is just another planet.
    d.planets[home].owner = RED;
    assert!(!d.planets[home].loyal() && d.planets[home].restless());
}

#[test]
fn the_ships_a_planet_needs_grow_with_tax_and_occupation() {
    let (mut d, home) = economy();
    d.planets[home].home_of = None;
    d.set_tax(home, 0.1);
    assert_eq!(d.ships_needed(home), Some(0));
    d.set_tax(home, 0.2);
    let fair = d.ships_needed(home).unwrap();
    assert!(fair >= 1);
    d.planets[home].occupation = OCCUPATION_PENALTY;
    let occupied = d.ships_needed(home).unwrap_or(u32::MAX);
    assert!(occupied > fair, "{occupied} vs {fair}");
    d.set_tax(home, MAX_TAX);
    assert!(d.ships_needed(home).is_none_or(|n| n >= occupied));
}

#[test]
fn a_garrisoned_planet_at_a_fair_tax_stays_calm() {
    let (mut d, home) = economy();
    d.rebellion = Default::default();
    d.tuning.ship_cost = 1e6;
    let at = d.planets[home].pos;
    d.add_fleet(BLUE, at, 0.0, 10, Some(home));
    d.step(120.0);
    assert_eq!(d.planets[home].owner, BLUE);
    assert!(d.planets[home].unrest < 0.05, "{}", d.planets[home].unrest);
    assert_eq!(d.revolts[0], 0);
}

#[test]
fn hungry_ships_desert_and_shake_the_fleet() {
    let (mut d, _) = economy();
    d.tuning.desert_chance = 1.0;
    let (from, _) = long_road(&d);
    let id = d.add_fleet(BLUE, from + Vec2::new(0.0, 60.0), 0.0, 20, None);
    set_stores(&mut d, id, [10.0, 0.0, 2.0]);
    d.step(MAINTENANCE_SECS + 0.05);
    assert!(ships(&d, id) < 20, "nobody deserted");
    assert!(d.deserted[0] > 0);
    let e = d.fleet(id).map(|f| f.division);
    if let Some(div) = e.and_then(|e| d.world.get::<crate::sim::components::Division>(e)) {
        assert!(div.shock > 0.0);
    }
}

#[test]
fn ships_without_ammo_cannot_fire() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 30), (RED, Vec2::new(325.0, 200.0), WEST, 30));
    d.tuning.ammo_per_shot = 0.1;
    set_stores(&mut d, a, [10.0, 3.0, 2.0]);
    set_stores(&mut d, b, [0.0, 3.0, 2.0]);
    d.step(20.0);
    assert!(!d.shots.is_empty());
    assert!(d.shots.iter().all(|s| s.2 == 0), "Red fired with no ammo");
    assert!(ships(&d, b) < 30);
    assert_eq!(ships(&d, a), 30);
    // Blue spent some.
    let (stores, _) = d.fleet_stores(d.fleet(a).unwrap());
    assert!(stores[AMMO] < 300.0);
}

#[test]
fn parts_repair_ships_and_without_them_damaged_ships_break_up() {
    let (mut d, _) = economy();
    d.tuning.food_per_phase = 0.0;
    let (from, _) = long_road(&d);
    let id = d.add_fleet(BLUE, from + Vec2::new(0.0, 60.0), 0.0, 10, None);
    for mut s in d.world.query::<&mut Ship>().iter_mut(&mut d.world) {
        s.hull = 3.0;
    }
    // 10 ships × 2 parts × 2.5 hull: every ship mended (2 hull points each).
    d.step(MAINTENANCE_SECS + 0.05);
    assert!(d.world.query::<&Ship>().iter(&d.world).all(|s| s.hull == s.max_hull));
    let (stores, _) = d.fleet_stores(d.fleet(id).unwrap());
    assert!((stores[PARTS] - 12.0).abs() < 0.01, "{stores:?}");
    // No parts and badly hurt: they break up.
    set_stores(&mut d, id, [10.0, 3.0, 0.0]);
    for mut s in d.world.query::<&mut Ship>().iter_mut(&mut d.world) {
        s.hull = 0.5;
    }
    d.tuning.wreck_chance = 1.0;
    d.step(3.0 * MAINTENANCE_SECS);
    assert!(ships(&d, id) < 10, "{}", ships(&d, id));
}

#[test]
fn a_fleet_in_need_gets_one_convoy_per_kind_that_joins_and_is_used_up() {
    let (mut d, home) = economy();
    d.tuning.ship_cost = 1e6;
    // Enough for one supply ship of each kind, and no more coming.
    d.tuning.income = 0.0;
    d.planets[home].stock = [10.0; 3];
    let start = d.planets[home].pos;
    let far = d.highways.neighbors[home][0];
    let at = d.planets[home].pos + (d.planets[far].pos - start) * 0.6;
    let id = d.add_fleet(BLUE, at, 0.0, 6, None);
    set_stores(&mut d, id, [1.0, 1.0, 0.0]);
    d.step(MAINTENANCE_SECS + 0.05);
    assert_eq!(d.convoys_sent[0], 3);
    let convoys: Vec<&Fleet> = d.fleets.iter().filter(|f| f.joining == Some(id)).collect();
    assert_eq!(convoys.len(), 3);
    for c in &convoys {
        let (_, cargo) = d.fleet_stores(c);
        assert_eq!(cargo.iter().filter(|v| **v > 0.0).count(), 1, "one kind a ship: {cargo:?}");
    }
    assert!(d.planets[home].stock.iter().sum::<f32>() < 0.01, "all loaded");
    // They travel the highway and join (and may unload at once).
    let mut seen = 0;
    for _ in 0..120 {
        d.step(0.05);
        seen = seen.max(d.fleet(id).unwrap().supply);
        if !d.fleets.iter().any(|f| f.joining == Some(id)) {
            break;
        }
    }
    assert!(seen >= 1, "no convoy ever joined");
    assert!(!d.fleets.iter().any(|f| f.joining == Some(id)), "convoys still under way");
    // Their cargo is used up by the next phase.
    d.step(MAINTENANCE_SECS + 0.05);
    let f = d.fleet(id).unwrap();
    let (stores, _) = d.fleet_stores(f);
    assert!(stores[AMMO] > 10.0 && stores[FOOD] > 6.0 && stores[PARTS] > 9.0, "{stores:?}");
    assert_eq!(f.supply, 0, "the empty supply ships are used up");
}

#[test]
fn a_convoy_waits_at_enemy_lines() {
    let (mut d, home) = economy();
    d.tuning.food_per_phase = 0.0;
    let far = d.highways.neighbors[home][0];
    let (a, b) = (d.planets[home].pos, d.planets[far].pos);
    let target = d.add_fleet(BLUE, b, 0.0, 6, None);
    // A big Red fleet sits on the road: its cell covers the middle.
    let mid = a + (b - a) * 0.5;
    let red = d.add_fleet(RED, mid + (b - a).perp().normalize() * 40.0, 0.0, 60, None);
    let convoy = d.add_convoy(BLUE, a, [10.0, 0.0, 0.0], Some(target));
    d.step(8.0);
    let c = d.fleet(convoy).expect("still under way");
    assert!(c.waiting, "it should wait at the border");
    let inf = Influence::new(&d);
    assert!(!matches!(inf.at(c.pos), Some(Site::Fleet(i)) if d.fleets[i].owner == RED));
    // The Red fleet leaves: the convoy goes on and joins.
    d.clear_fleet(red);
    for _ in 0..60 {
        d.step(0.5);
        if d.fleet(convoy).is_none() {
            break;
        }
    }
    assert!(d.fleet(convoy).is_none(), "the convoy never arrived");
    assert_eq!(d.fleet(target).unwrap().supply, 1);
}

#[test]
fn supply_ships_trail_their_fleet_and_can_be_shot() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 10), (RED, Vec2::new(500.0, 200.0), WEST, 30));
    assert!(d.add_supply_ship(a, [5.0; 3]));
    d.step(6.0);
    let f = d.fleet(a).unwrap();
    let k = f.is_supply.iter().position(|s| *s).unwrap();
    let behind = (f.ship_px[k].0 - f.pos).dot(Vec2::from_angle(f.facing));
    assert!(behind < 0.0 && behind > -d.range(), "close behind: {behind}");
    assert!(d.world.get::<Supply>(f.ship_ids[k]).is_some());
    // A lone supply ship next to an enemy fleet gets shot down.
    let lone = d.add_convoy(BLUE, d.fleet(b).unwrap().pos + Vec2::new(-18.0, 0.0), [5.0; 3], None);
    assert_eq!(d.fleet(lone).unwrap().supply, 1);
    d.step(30.0);
    assert!(d.fleet(lone).is_none(), "the supply ship survived");
}

fn ids(d: &Dominion, id: u32) -> Vec<bevy::prelude::Entity> {
    d.fleet(id).unwrap().ship_ids.clone()
}

#[test]
fn one_whole_fleet_selected_just_moves() {
    let (mut d, a, _) = duel((BLUE, Vec2::new(100.0, 100.0), 0.0, 10), (RED, Vec2::new(650.0, 350.0), WEST, 10));
    let to = Vec2::new(200.0, 100.0);
    assert_eq!(d.form_fleet(&ids(&d, a), to), Some(a));
    assert_eq!(d.fleets.len(), 2);
    assert_eq!(d.fleet(a).unwrap().dest, Some(d.highways.snap(to).unwrap().point));
}

#[test]
fn captains_travel_apart_and_never_merge() {
    let mut d = world();
    d.clear_fleets();
    d.tuning.food_per_phase = 0.0;
    d.tuning.secs_per_ship = 1e6;
    let home = d.planets.iter().position(|p| p.owner == BLUE).unwrap();
    let ns = d.highways.neighbors[home].clone();
    let (p1, p2) = (d.planets[ns[0]].pos, d.planets[ns[1]].pos);
    let a = captained(&mut d, BLUE, p1, 0.0, 12);
    let b = captained(&mut d, BLUE, p2, 0.0, 5);
    let mut sel = ids(&d, a);
    sel.extend(ids(&d, b));
    let to = d.planets[home].pos;
    assert_eq!(d.form_fleet(&sel, to), Some(a), "the strongest leads");
    d.step(60.0);
    let (fa, fb) = (d.fleet(a).unwrap(), d.fleet(b).unwrap());
    assert_eq!((fa.ships, fb.ships), (12, 5));
    assert!(fa.pos.distance(to) < 30.0 && fb.pos.distance(to) < 30.0);
}

#[test]
fn ships_without_a_captain_move_or_join_a_captain_nearby() {
    let (mut d, home) = economy();
    d.tuning.food_per_phase = 0.0;
    d.tuning.ship_cost = 1e6;
    let at = d.planets[home].pos;
    let garrison = d.add_fleet(BLUE, at, 0.0, 6, Some(home));
    assert!(d.controllable_id(garrison));
    // Alone, it moves on its own.
    let to = d.planets[d.highways.neighbors[home][0]].pos;
    assert_eq!(d.form_fleet(&ids(&d, garrison), to), Some(garrison));
    assert!(d.fleet(garrison).unwrap().dest.is_some());
    // With a captain nearby, the captain takes them along.
    let cap = captained(&mut d, BLUE, at + Vec2::new(15.0, 0.0), 0.0, 3);
    let mut sel = ids(&d, garrison);
    sel.extend(ids(&d, cap));
    assert_eq!(d.form_fleet(&sel, to), Some(cap));
    assert!(d.fleet(garrison).is_none());
    assert_eq!(ships(&d, cap), 9);
}

#[test]
fn part_of_a_fleet_without_its_flagship_splits_off_and_goes_alone() {
    let (mut d, a, _) = duel((BLUE, Vec2::new(100.0, 100.0), 0.0, 10), (RED, Vec2::new(650.0, 350.0), WEST, 10));
    let flag = d.captain(d.fleet(a).unwrap().captain.unwrap()).unwrap().flagship.unwrap();
    let part: Vec<_> = ids(&d, a).into_iter().filter(|e| *e != flag).take(4).collect();
    let new = d.form_fleet(&part, Vec2::new(100.0, 300.0)).unwrap();
    assert_ne!(new, a);
    let nf = d.fleet(new).unwrap();
    assert_eq!((ships(&d, a), nf.ships), (6, 4));
    assert!(nf.captain.is_none() && nf.dest.is_some(), "it goes on its own");
    // The flagship with part of its fleet: that part goes, the rest stays.
    let mut go: Vec<_> = ids(&d, a).into_iter().filter(|e| *e != flag).take(2).collect();
    go.push(flag);
    assert_eq!(d.form_fleet(&go, Vec2::new(300.0, 100.0)), Some(a));
    assert_eq!(ships(&d, a), 3);
    assert!(d.fleet(a).unwrap().dest.is_some());
}

#[test]
fn a_mixed_selection_only_moves_one_owner_and_never_independents() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(100.0, 100.0), 0.0, 10), (RED, Vec2::new(650.0, 350.0), WEST, 10));
    let mut sel = ids(&d, a);
    sel.extend(ids(&d, b));
    assert_eq!(d.form_fleet(&sel, Vec2::new(300.0, 200.0)), Some(a));
    assert_eq!((ships(&d, a), ships(&d, b)), (10, 10));
    assert!(d.fleet(b).unwrap().dest.is_none());
    let ind = d.add_fleet(Owner::Independent(0), Vec2::new(400.0, 100.0), 0.0, 3, None);
    assert_eq!(d.form_fleet(&ids(&d, ind), Vec2::ZERO), None);
}

#[test]
fn a_flagship_counts_as_several_ships() {
    let (mut d, home) = economy();
    d.tuning.food_per_phase = 0.0;
    let at = d.planets[home].pos;
    let id = captained(&mut d, BLUE, at, 0.0, 5);
    let f = d.fleet(id).unwrap().clone();
    assert_eq!(f.ships, 5);
    assert_eq!(f.strength(&d.tuning), 4.0 + d.tuning.flagship_ships);
    assert!((f.weight(&d.tuning) - (4.0 + d.tuning.flagship_ships) * d.tuning.weight_per_ship).abs() < 1e-5);
    assert_eq!(d.police_ships(home), 4 + d.tuning.flagship_ships as u32);
    // Its stores share and its hull are the flagship's weight.
    let flag = d.captain(f.captain.unwrap()).unwrap().flagship.unwrap();
    d.step(MAINTENANCE_SECS + 0.05);
    let s = d.world.get::<Stores>(flag).unwrap().0;
    let other = d.world.get::<Stores>(*ids(&d, id).iter().find(|e| **e != flag && d.world.get::<Stores>(**e).is_some()).unwrap()).unwrap().0;
    assert!((s[AMMO] - other[AMMO] * d.tuning.flagship_ships).abs() < 0.01, "{s:?} {other:?}");
    assert_eq!(d.world.get::<Ship>(flag).unwrap().max_hull, d.battle.hull * d.tuning.flagship_ships);
}

#[test]
fn over_capacity_shakes_and_slows_a_fleet() {
    let (mut d, _) = economy();
    d.tuning.food_per_phase = 0.0;
    let (from, _) = long_road(&d);
    let id = captained(&mut d, BLUE, from + Vec2::new(0.0, 60.0), 0.0, 21);
    let e = d.fleet(id).unwrap().division;
    d.step(MAINTENANCE_SECS + 0.05);
    let div = d.world.get::<crate::sim::components::Division>(e).unwrap();
    // 20 escorts for 10: 100% over.
    assert!((div.pace - 0.5).abs() < 1e-4, "{}", div.pace);
    assert!(div.shock > 0.0);
    // A Commander handles 15 without slowing as much.
    let c = captained_with(&mut d, BLUE, from + Vec2::new(0.0, 120.0), 0.0, 16, Skill::Commander);
    assert_eq!(d.capacity(d.fleet(c).unwrap()), 15.0);
}

#[test]
fn a_lost_flagship_kills_its_captain_or_sends_them_home() {
    for death in [1.0, 0.0] {
        let (mut d, home) = economy();
        d.tuning.food_per_phase = 0.0;
        d.tuning.captain_death = death;
        d.tuning.captain_return = 5.0;
        let (from, _) = long_road(&d);
        let id = captained_with(&mut d, BLUE, from + Vec2::new(0.0, 60.0), 0.0, 6, Skill::Gunner);
        let cid = d.fleet(id).unwrap().captain.unwrap();
        let flag = d.captain(cid).unwrap().flagship.unwrap();
        let e = d.fleet(id).unwrap().division;
        crate::sim::despawn_ships(&mut d.world, e, &[flag]);
        d.step(0.1);
        let f = d.fleet(id).unwrap();
        assert!(f.captain.is_none(), "the fleet lost its captain");
        let c = d.captain(cid).unwrap().clone();
        if death == 1.0 {
            assert!(!c.alive);
            d.step(10.0);
            assert!(d.captain(cid).unwrap().flagship.is_none());
        } else {
            assert!(c.alive && c.flagship.is_none());
            d.step(6.0);
            let back = d.fleets.iter().find(|f| f.captain == Some(cid)).expect("a new flagship");
            assert!(back.pos.distance(d.planets[home].pos) < 30.0);
        }
    }
}

#[test]
fn skills_change_the_rules() {
    let (mut d, home) = economy();
    d.tuning.food_per_phase = 0.0;
    let at = d.planets[home].pos;
    d.planets[home].home_of = None;
    // Governor: ships count double as cops.
    let plain = d.cops_for(home);
    let g = captained_with(&mut d, BLUE, at, 0.0, 4, Skill::Governor);
    let gov = d.cops_for(home);
    d.clear_fleet(g);
    let p = captained_with(&mut d, BLUE, at, 0.0, 4, Skill::Survivor);
    let normal = d.cops_for(home);
    assert!(gov > normal && normal > plain, "{plain} {normal} {gov}");
    d.clear_fleet(p);
    // Navigator: faster.
    let (from, _) = long_road(&d);
    let n = captained_with(&mut d, BLUE, from + Vec2::new(0.0, 60.0), 0.0, 4, Skill::Navigator);
    d.step(MAINTENANCE_SECS + 0.05);
    let e = d.fleet(n).unwrap().division;
    assert_eq!(d.world.get::<crate::sim::components::Division>(e).unwrap().pace, 1.25);
}

#[test]
fn a_sharpshooter_spends_half_the_ammo() {
    let mut used = Vec::new();
    for skill in [Skill::Survivor, Skill::Sharpshooter] {
        let (mut d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 10), (RED, Vec2::new(325.0, 200.0), WEST, 30));
        let _ = b;
        d.tuning.ammo_per_shot = 0.1;
        // Swap the captain's skill.
        let cid = d.fleet(a).unwrap().captain.unwrap();
        d.captains.iter_mut().find(|c| c.id == cid).unwrap().skill = skill;
        set_stores(&mut d, a, [10.0, 3.0, 2.0]);
        let (before, _) = d.fleet_stores(d.fleet(a).unwrap());
        let shots_before = d.shots.len();
        d.step(3.0);
        let fired = d.shots.iter().filter(|s| s.2 == 0).count().max(shots_before);
        let (after, _) = d.fleet_stores(d.fleet(a).unwrap());
        used.push((before[AMMO] - after[AMMO]) / fired.max(1) as f32);
    }
    assert!(used[1] < used[0] * 0.75, "{used:?}");
}

#[test]
fn a_conqueror_takes_planets_twice_as_fast() {
    let mut d = world();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| matches!(p.owner, Owner::Independent(_))).map(|(i, p)| (i, *p)).unwrap();
    d.fleets.retain(|f| f.home != Some(i));
    d.clear_fleets();
    d.tuning.food_per_phase = 0.0;
    d.tuning.secs_per_ship = 1e6;
    captained_with(&mut d, BLUE, p.pos, 0.0, 5, Skill::Conqueror);
    d.step(HOLD_SECS * 0.5 + 0.2);
    assert_eq!(d.planets[i].owner, BLUE);
    assert!((d.planets[i].occupation - OCCUPATION_PENALTY * 0.5).abs() < 0.02);
}

#[test]
fn a_box_takes_ships_of_one_side_only() {
    let (d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 10), (RED, Vec2::new(340.0, 200.0), WEST, 20));
    let all = d.ships_in_box(Vec2::new(0.0, 0.0), Vec2::new(720.0, 400.0));
    assert_eq!(all, ids(&d, b));
    let left = d.ships_in_box(Vec2::new(0.0, 0.0), Vec2::new(320.0, 400.0));
    assert_eq!(left, ids(&d, a));
}
