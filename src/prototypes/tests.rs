use super::dominion::*;
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
    d.fleets.clear();
    for p in &mut d.planets {
        p.value = 0;
    }
    let mk = |id, (owner, pos, facing, ships): (Owner, Vec2, f32, u32)| Fleet {
        id,
        owner,
        pos,
        facing,
        dest: None,
        ships,
        damage: 0.0,
        formation: Formation::Line,
        home: None,
        dealt: 0.0,
        taken: 0.0,
    };
    d.fleets.push(mk(1001, a));
    d.fleets.push(mk(1002, b));
    (d, 1001, 1002)
}

const BLUE: Owner = Owner::Side(0);
const RED: Owner = Owner::Side(1);
const WEST: f32 = std::f32::consts::PI;

fn rate(d: &Dominion, a: u32, b: u32) -> f32 {
    let (fa, fb) = (d.fleet(a).unwrap(), d.fleet(b).unwrap());
    d.fire_rate(fa, &d.ship_positions(fa), fb, &d.ship_positions(fb))
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
}

#[test]
fn planets_build_ships_into_their_garrison() {
    let mut d = world();
    d.tuning.secs_per_ship = 6.0;
    let home = d.planets.iter().position(|p| p.owner == BLUE).unwrap();
    d.planets[home].value = 2;
    // 6 s a ship at value 1, so value 2 builds one every 3 s.
    d.step(9.05);
    let g = d.fleets.iter().find(|f| f.home == Some(home)).expect("a new garrison");
    assert_eq!(g.ships, 3);
    assert!(!g.controllable(), "too few ships to command");
    d.step(21.0);
    let g = d.fleets.iter().find(|f| f.home == Some(home)).unwrap();
    assert!(g.ships >= CONTROL_MIN && g.controllable());
    // Sending it away starts a new garrison.
    let id = g.id;
    assert!(d.order_move(id, Vec2::new(360.0, 200.0)));
    d.step(3.1);
    assert!(d.fleets.iter().any(|f| f.home == Some(home) && f.id != id));
}

#[test]
fn independents_never_take_orders_and_cap_their_garrison() {
    let mut d = world();
    let g = d.fleets.iter().find(|f| matches!(f.owner, Owner::Independent(_))).unwrap().clone();
    assert!(!g.controllable());
    assert!(!d.order_move(g.id, Vec2::ZERO));
    d.step(400.0);
    assert!(d.fleets.iter().filter(|f| matches!(f.owner, Owner::Independent(_))).all(|f| f.ships <= INDEPENDENT_CAP));
}

#[test]
fn fleets_move_at_the_set_speed() {
    let (mut d, a, _) = duel((BLUE, Vec2::new(100.0, 200.0), 0.0, 20), (RED, Vec2::new(700.0, 30.0), 0.0, 20));
    assert!(d.order_move(a, Vec2::new(250.0, 200.0)));
    d.step(2.05);
    let x = d.fleet(a).unwrap().pos.x;
    assert!((x - 160.0).abs() < 1.0, "30 px/s for 2 s: {x}");
    d.step(10.0);
    assert_eq!(d.fleet(a).unwrap().pos, Vec2::new(250.0, 200.0));
    assert!(d.fleet(a).unwrap().dest.is_none());
}

#[test]
fn fleets_out_of_range_dont_fight() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(100.0, 200.0), 0.0, 30), (RED, Vec2::new(300.0, 200.0), WEST, 30));
    d.step(5.0);
    assert_eq!((d.fleet(a).unwrap().ships, d.fleet(b).unwrap().ships), (30, 30));
}

#[test]
fn fleets_in_range_wear_each_other_down_with_battle_hit_chances() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 30), (RED, Vec2::new(325.0, 200.0), WEST, 30));
    // Facing each other: front hits, at most 12% a shot every 3 s (× combat speed).
    let r = rate(&d, a, b);
    assert!(r > 0.0 && r <= 30.0 * 0.12 / 3.0 * d.tuning.combat_speed + 1e-4, "{r}");
    d.step(30.0);
    let (sa, sb) = (d.fleet(a).map_or(0, |f| f.ships), d.fleet(b).map_or(0, |f| f.ships));
    assert!(sa < 30 && sb < 30, "{sa} {sb}");
    assert_eq!(sa, sb, "a mirror fight stays even");
}

#[test]
fn flank_shots_hit_harder_than_front_shots() {
    let attacker = (BLUE, Vec2::new(300.0, 200.0), 0.0, 20);
    let (front, a1, b1) = duel(attacker, (RED, Vec2::new(322.0, 200.0), WEST, 20));
    let (flank, a2, b2) = duel(attacker, (RED, Vec2::new(322.0, 200.0), std::f32::consts::FRAC_PI_2, 20));
    assert!(rate(&flank, a2, b2) > rate(&front, a1, b1) * 1.5);
}

#[test]
fn a_line_brings_more_guns_to_bear_than_a_column() {
    let target = (RED, Vec2::new(325.0, 200.0), WEST, 40);
    let (mut line, a, b) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 40), target);
    let (mut column, c, e) = duel((BLUE, Vec2::new(300.0, 200.0), 0.0, 40), target);
    line.fleets[0].formation = Formation::Line;
    column.fleets[0].formation = Formation::Column;
    assert!(rate(&line, a, b) > rate(&column, c, e));
    // The menu action sets it on controllable fleets only.
    assert!(line.set_formation(a, Formation::Wedge));
    assert_eq!(line.fleet(a).unwrap().formation, Formation::Wedge);
}

#[test]
fn a_planet_falls_only_after_its_garrison_is_gone_and_it_is_held() {
    let mut d = world();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| matches!(p.owner, Owner::Independent(_))).map(|(i, p)| (i, *p)).unwrap();
    // Park a big Blue fleet next to it, facing it: the garrison fights first.
    let blue = d.fleets.iter().find(|f| f.owner == BLUE).unwrap().id;
    let fi = d.fleet_index(blue).unwrap();
    d.fleets[fi].pos = p.pos + Vec2::new(-12.0, 0.0);
    d.fleets[fi].facing = 0.0;
    d.fleets[fi].ships = 150;
    d.planets[i].value = 1;
    d.step(1.0);
    assert_eq!(d.planets[i].owner, p.owner, "garrison still there");
    d.step(120.0);
    assert!(d.fleets.iter().all(|f| f.home != Some(i) || f.owner == BLUE), "garrison destroyed");
    assert_eq!(d.planets[i].owner, BLUE);
}

#[test]
fn both_sides_present_stops_the_capture() {
    let (mut d, a, b) = duel((BLUE, Vec2::ZERO, 0.0, 10), (RED, Vec2::ZERO, 0.0, 10));
    let p = d.planets.iter().position(|p| matches!(p.owner, Owner::Independent(_))).unwrap();
    let at = d.planets[p].pos;
    // Both parked on the planet; neither can capture while the other is there.
    let ia = d.fleet_index(a).unwrap();
    d.fleets[ia].pos = at + Vec2::new(-10.0, 0.0);
    let ib = d.fleet_index(b).unwrap();
    d.fleets[ib].pos = at + Vec2::new(10.0, 0.0);
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
    assert_eq!(a, b);
    // A fixed-size picture, whatever the map size or zoom.
    let mut view = View::fit(&a.cfg, VIEW_W, VIEW_H);
    let bg = render_background(&a, &view, true, 1);
    let c = render(&a, &bg, &view, DrawOptions::default());
    assert_eq!(render_background(&a, &view, true, 2).px.len(), bg.px.len());
    assert_eq!((c.w, c.h), (VIEW_W, VIEW_H));
    view.zoom_at(Vec2::new(700.0, 400.0), 4.0, &a.cfg);
    let bg = render_background(&a, &view, true, 1);
    assert_eq!(render(&a, &bg, &view, DrawOptions::default()).px.len(), c.px.len());
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
fn moving_fleets_stop_short_of_the_enemy_and_face_it() {
    let (mut d, a, b) = duel((BLUE, Vec2::new(100.0, 200.0), 0.0, 20), (RED, Vec2::new(300.0, 200.0), WEST, 20));
    assert!(d.order_move(a, Vec2::new(300.0, 200.0)));
    d.step(15.0);
    let (fa, fb) = (d.fleet(a).unwrap(), d.fleet(b).unwrap());
    let gap = d.ship_positions(fb).iter().map(|s| s.distance(fa.pos)).fold(f32::INFINITY, f32::min);
    assert!(gap > 0.0 && gap <= d.range() * 0.6 + 1.0, "stopped short: {gap}");
    assert!(fa.dest.is_some(), "still wants to go there");
    assert!(fa.ships < 20 || fb.ships < 20, "and they fight");
}

#[test]
fn a_fleet_sent_at_a_planet_takes_it() {
    let mut d = world();
    let (i, p) = d.planets.iter().enumerate().find(|(_, p)| matches!(p.owner, Owner::Independent(_))).map(|(i, p)| (i, *p)).unwrap();
    d.planets[i].value = 1;
    let blue = d.fleets.iter().find(|f| f.owner == BLUE).unwrap().id;
    let fi = d.fleet_index(blue).unwrap();
    d.fleets[fi].pos = p.pos + Vec2::new(-80.0, 0.0);
    d.fleets[fi].ships = 80;
    assert!(d.order_move(blue, p.pos));
    d.step(90.0);
    assert_eq!(d.planets[i].owner, BLUE);
}
