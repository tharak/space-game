//! prototype01 — Fleet dominion, in real time. Space is split into Voronoi
//! regions; some hold a planet. Every planet builds ships into its own
//! garrison fleet; Blue's and Red's fleets can be commanded once they have
//! 10 ships. Every ship is an entity in the battle's ECS world and every
//! fleet is one of its divisions, run by the battle's own `SimTick`
//! systems: steering, blocking, gunnery (cone, front/flank/rear hit
//! chances, reload, hull), closing ranks and morale, so a fleet can be
//! shaken, rout and flee off the map. Independents are a third side that
//! fights Blue and Red but not each other. A planet changes hands once its
//! garrison is gone and an enemy fleet has held it for a few seconds.
//! Influence patches (weight = ships × weight per ship, reach = reach per
//! weight × weight) show who dominates where.

use bevy::ecs::message::Messages;
use bevy::prelude::{Entity, Rect, World};

use crate::sim::battle::{Battle, DT, SimEvent, Tuning as BattleTuning};
use crate::sim::components::{Division, Heading, InDivision, Morale, Order, Pos};
pub use crate::sim::formation::Formation;
use crate::sim::formation::SHIP_DART;
use crate::voronoi::Vec2;
use crate::voronoi::grid::{OwnerGrid, voronoi_grid};
use crate::voronoi::influence::weighted_scores;
use crate::voronoi::labs::cell_color;
use crate::voronoi::metric::{Metric, distance};
use crate::voronoi::raster::{Canvas, Rgb};
use crate::voronoi::rng::ParkMiller;
use crate::voronoi::sites::VoronoiConfig;

pub const MAX_FLEETS_PER_SIDE: usize = 8;
pub const START_SHIPS: u32 = 50;
pub const MIN_SHIPS: u32 = 1;
pub const MAX_SHIPS: u32 = 300;
/// A Blue or Red fleet can take orders once it has this many ships.
pub const CONTROL_MIN: u32 = 10;
/// Independent planets start with this garrison and build up to the cap.
pub const INDEPENDENT_START: u32 = 10;
pub const INDEPENDENT_CAP: u32 = 30;
/// Seconds an attacker must hold an ungarrisoned planet to take it.
pub const HOLD_SECS: f32 = 3.0;
/// Battle units to map pixels on a 720-wide map (range 150 → 30 px).
pub const UNIT: f32 = 0.2;
/// The ECS side independents fight on.
pub const INDEPENDENT_SIDE: usize = 2;
/// Map width the pixel-based settings are measured against.
pub const REFERENCE_WIDTH: f32 = 720.0;
/// The other side's best fleet within this fraction contests the ground.
pub const CONTEST_MARGIN: f32 = 0.10;
pub const SIDE_NAMES: [&str; 2] = ["Blue", "Red"];
const SIDE_COLORS: [Rgb; 2] = [[70, 130, 220], [215, 80, 70]];

/// The prototype's sliders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Influence weight each ship adds (patches).
    pub weight_per_ship: f32,
    /// Patch reach per point of weight, in pixels of a 720-wide map.
    pub reach_per_weight: f32,
    /// Seconds to build one ship at a value-1 planet (value 3 is 3× faster).
    pub secs_per_ship: f32,
    /// Fleet speed, pixels a second on a 720-wide map.
    pub speed: f32,
    /// Multiplies the battle's rate of fire, so map fights take seconds,
    /// not minutes; the rules' proportions are unchanged.
    pub combat_speed: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        // Building must stay slower than a fleet in range can kill: 60 ships
        // firing at the front kill about 0.5 ships a second.
        Self { weight_per_ship: 0.1, reach_per_weight: 14.0, secs_per_ship: 15.0, speed: 30.0, combat_speed: 3.0 }
    }
}

/// How much bigger than the reference this map is.
pub fn map_scale(cfg: &VoronoiConfig) -> f32 {
    cfg.w as f32 / REFERENCE_WIDTH
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Owner {
    /// Blue (0) or Red (1).
    Side(usize),
    /// An independent faction, numbered by its planet. It only defends.
    Independent(usize),
}

impl Owner {
    /// Blue and Red fight everyone; independents only fight Blue and Red.
    pub fn hostile(self, other: Owner) -> bool {
        self.side() != other.side()
    }

    /// The side its ships fight on in the ECS.
    pub fn side(self) -> usize {
        match self {
            Owner::Side(s) => s,
            Owner::Independent(_) => INDEPENDENT_SIDE,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planet {
    pub site: usize,
    pub pos: Vec2,
    /// Build rate multiplier, 1 (poor) to 3 (rich).
    pub value: u32,
    pub owner: Owner,
    /// Progress toward the next ship, 0..1.
    pub build: f32,
    /// An attacker holding the ungarrisoned planet, and for how long.
    pub hold: Option<(Owner, f32)>,
}

/// A fleet as the map sees it: one ECS division, read back every tick.
#[derive(Clone, Debug, PartialEq)]
pub struct Fleet {
    /// Stable across removals (the UI keys selection on it).
    pub id: u32,
    pub division: Entity,
    pub owner: Owner,
    /// Centre of its ships, map pixels.
    pub pos: Vec2,
    /// Radians; 0 = +x.
    pub facing: f32,
    pub dest: Option<Vec2>,
    pub ships: u32,
    pub formation: Formation,
    pub morale: Morale,
    /// The planet it garrisons (planets build into it).
    pub home: Option<usize>,
    /// Each ship's position (map pixels) and heading.
    pub ship_px: Vec<(Vec2, f32)>,
    /// Fired or was fired on in the last second or so.
    pub engaged: bool,
}

impl Fleet {
    pub fn weight(&self, t: &Tuning) -> f32 {
        self.ships as f32 * t.weight_per_ship
    }

    /// Blue and Red fleets with 10+ ships, or that already left home, unless routed.
    pub fn controllable(&self) -> bool {
        matches!(self.owner, Owner::Side(_)) && self.morale != Morale::Routed && (self.ships >= CONTROL_MIN || self.home.is_none())
    }
}

/// A shot on the map: from, to (pixels), the shooter's side, hit, age (s).
pub type Shot = (Vec2, Vec2, usize, bool, f32);

/// How long shots and explosions stay on the map, seconds.
const SHOT_SECS: f32 = 0.3;
const BOOM_SECS: f32 = 0.6;

pub struct Dominion {
    pub cfg: VoronoiConfig,
    pub tuning: Tuning,
    /// The battle tuning the ECS world runs with (from `tuning`).
    pub battle: BattleTuning,
    pub sites: Vec<Vec2>,
    pub regions: OwnerGrid,
    pub planet_at: Vec<Option<usize>>,
    pub planets: Vec<Planet>,
    pub fleets: Vec<Fleet>,
    /// The ECS world every ship lives in.
    pub world: World,
    pub shots: Vec<Shot>,
    /// Explosions: where (pixels) and age.
    pub booms: Vec<(Vec2, f32)>,
    next_id: u32,
    pub time: f32,
    /// Leftover time below one tick.
    pending: f32,
    /// The tuning the world was last given.
    applied: Option<Tuning>,
    face_timer: f32,
}

/// The battle's rules with the prototype's speeds: map fleets cross the map
/// in seconds, and combat speed shortens the reload.
pub fn battle_tuning(t: &Tuning) -> BattleTuning {
    let base = BattleTuning::default();
    // Pixels a second on a 720-wide map, in battle units (UNIT px each).
    let anchor = t.speed / UNIT;
    BattleTuning {
        anchor_speed: anchor,
        ship_speed: anchor * base.ship_speed / base.anchor_speed,
        rout_speed: anchor * base.rout_speed / base.anchor_speed,
        div_turn_rate: 1.2,
        ship_turn_rate: 3.0,
        cooldown: base.cooldown / t.combat_speed.max(0.05),
        ..base
    }
}

impl Dominion {
    /// `planets` of the sites get a planet (inland first, seeded). Blue and
    /// Red each own the planet nearest their edge and start with `fleets`
    /// fleets of 50 ships there; every other planet is independent with a
    /// garrison of 10.
    pub fn new(cfg: &VoronoiConfig, planets: usize, fleets: usize, tuning: Tuning) -> Self {
        let sites = cfg.relaxed_sites();
        let regions = voronoi_grid(&sites, cfg.w, cfg.h, cfg.metric);
        let edge = regions.edge_owners();
        let mut rng = ParkMiller::new(cfg.seed as i64 + 11);
        let mut order: Vec<usize> = (0..sites.len()).collect();
        for i in (1..order.len()).rev() {
            let j = (rng.next_float() * (i + 1) as f32) as usize;
            order.swap(i, j.min(i));
        }
        order.sort_by_key(|i| edge.contains(&(*i as u32)));
        let mut planet_at = vec![None; sites.len()];
        let mut list: Vec<Planet> = Vec::new();
        for &site in order.iter().take(planets.max(2).min(sites.len())) {
            let roll = rng.next_float();
            let value = if roll < 0.2 {
                3
            } else if roll < 0.7 {
                2
            } else {
                1
            };
            planet_at[site] = Some(list.len());
            list.push(Planet { site, pos: sites[site], value, owner: Owner::Independent(list.len()), build: 0.0, hold: None });
        }
        let battle = battle_tuning(&tuning);
        let unit = UNIT * map_scale(cfg);
        let bounds = Rect::new(0.0, 0.0, cfg.w as f32 / unit, cfg.h as f32 / unit);
        let mut world = crate::sim::open_world(battle.clone(), cfg.seed.wrapping_add(7), bounds);
        world.resource_mut::<Battle>().endless = true;
        let mut d = Self {
            cfg: *cfg,
            tuning,
            battle,
            sites,
            regions,
            planet_at,
            planets: list,
            fleets: Vec::new(),
            world,
            shots: Vec::new(),
            booms: Vec::new(),
            next_id: 1,
            time: 0.0,
            pending: 0.0,
            applied: Some(tuning),
            face_timer: 0.0,
        };
        if d.planets.len() < 2 {
            return d;
        }
        // Home planets: the westernmost for Blue, the easternmost for Red.
        let west = (0..d.planets.len()).min_by(|a, b| d.planets[*a].pos.x.total_cmp(&d.planets[*b].pos.x)).unwrap();
        let east = (0..d.planets.len()).filter(|i| *i != west).max_by(|a, b| d.planets[*a].pos.x.total_cmp(&d.planets[*b].pos.x)).unwrap();
        d.planets[west].owner = Owner::Side(0);
        d.planets[east].owner = Owner::Side(1);
        let scale = map_scale(cfg);
        for (side, home) in [(0, west), (1, east)] {
            let at = d.planets[home].pos;
            let toward = Vec2::new(cfg.w as f32 * 0.5, cfg.h as f32 * 0.5) - at;
            let facing = toward.to_angle();
            let n = fleets.clamp(1, MAX_FLEETS_PER_SIDE);
            // Side by side in front of the home planet, a formation's width apart.
            let width = crate::sim::formation::frontage(Formation::Line, START_SHIPS as usize, d.battle.spacing) * UNIT * scale + 8.0;
            let ahead = at + Vec2::from_angle(facing) * 45.0 * scale;
            let across = Vec2::from_angle(facing).perp();
            for k in 0..n {
                let pos = ahead + across * (k as f32 - (n as f32 - 1.0) / 2.0) * width;
                let pos = pos.clamp(Vec2::splat(10.0), Vec2::new(cfg.w as f32 - 10.0, cfg.h as f32 - 10.0));
                d.spawn(Owner::Side(side), pos, facing, START_SHIPS, None);
            }
        }
        for i in 0..d.planets.len() {
            if let Owner::Independent(_) = d.planets[i].owner {
                let (owner, pos) = (d.planets[i].owner, d.planets[i].pos);
                d.spawn(owner, pos, 0.0, INDEPENDENT_START, Some(i));
            }
        }
        d.sync();
        d
    }

    /// Removes every fleet and its ships (for tests and set-pieces).
    pub fn clear_fleets(&mut self) {
        for f in std::mem::take(&mut self.fleets) {
            let ships: Vec<Entity> = self
                .world
                .query::<(Entity, &InDivision)>()
                .iter(&self.world)
                .filter(|(_, d)| d.0 == f.division)
                .map(|(e, _)| e)
                .collect();
            for e in ships.into_iter().chain([f.division]) {
                self.world.despawn(e);
            }
        }
    }

    /// A new fleet of `ships` for `owner` at `pos` (pixels), facing `facing`.
    pub fn add_fleet(&mut self, owner: Owner, pos: Vec2, facing: f32, ships: u32, home: Option<usize>) -> u32 {
        let id = self.spawn(owner, pos, facing, ships, home);
        self.sync();
        id
    }

    /// A new division of `ships` for `owner` at `pos` (pixels).
    fn spawn(&mut self, owner: Owner, pos: Vec2, facing: f32, ships: u32, home: Option<usize>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let unit = self.unit();
        let division =
            crate::sim::spawn_division(&mut self.world, owner.side(), Formation::Line, pos / unit, facing, ships as usize, Order::Hold);
        self.fleets.push(Fleet {
            id,
            division,
            owner,
            pos,
            facing,
            dest: None,
            ships,
            formation: Formation::Line,
            morale: Morale::Steady,
            home,
            ship_px: Vec::new(),
            engaged: false,
        });
        id
    }

    pub fn scale(&self) -> f32 {
        map_scale(&self.cfg)
    }

    /// Battle units to map pixels on this map.
    pub fn unit(&self) -> f32 {
        UNIT * self.scale()
    }

    /// Gun range in map pixels.
    pub fn range(&self) -> f32 {
        self.battle.range * self.unit()
    }

    /// Radius around a planet that counts as holding it.
    pub fn capture_radius(&self) -> f32 {
        25.0 * self.scale()
    }

    pub fn fleet(&self, id: u32) -> Option<&Fleet> {
        self.fleets.iter().find(|f| f.id == id)
    }

    pub fn fleet_index(&self, id: u32) -> Option<usize> {
        self.fleets.iter().position(|f| f.id == id)
    }

    /// Each ship's position, map pixels.
    pub fn ship_positions(&self, f: &Fleet) -> Vec<Vec2> {
        f.ship_px.iter().map(|s| s.0).collect()
    }

    /// Whether the fleet with this id takes orders.
    pub fn controllable_id(&self, id: u32) -> bool {
        self.fleet(id).is_some_and(Fleet::controllable)
    }

    fn division_mut(&mut self, id: u32) -> Option<bevy::prelude::Mut<'_, Division>> {
        let e = self.fleet(id)?.division;
        self.world.get_mut::<Division>(e)
    }

    /// Orders a controllable fleet to move; it stops garrisoning.
    pub fn order_move(&mut self, id: u32, to: Vec2) -> bool {
        let (w, h) = (self.cfg.w as f32, self.cfg.h as f32);
        let unit = self.unit();
        let Some(i) = self.fleet_index(id).filter(|i| self.fleets[*i].controllable()) else { return false };
        let to = to.clamp(Vec2::ZERO, Vec2::new(w, h));
        let f = &mut self.fleets[i];
        f.dest = Some(to);
        f.home = None;
        let facing = (to - f.pos).to_angle();
        if let Some(mut d) = self.division_mut(id) {
            d.order = Order::MoveTo { to: to / unit, facing };
        }
        true
    }

    pub fn set_formation(&mut self, id: u32, formation: Formation) -> bool {
        let Some(f) = self.fleet(id).filter(|f| f.controllable()) else { return false };
        let e = f.division;
        crate::sim::set_formation(&mut self.world, e, formation);
        self.sync();
        true
    }

    /// Sets a fleet's ship count (the "Selected ships" slider).
    pub fn set_ships(&mut self, id: u32, n: u32) {
        let Some(f) = self.fleet(id) else { return };
        let (e, have) = (f.division, f.ships);
        let n = n.max(1);
        if n > have {
            crate::sim::add_ships(&mut self.world, e, (n - have) as usize);
        } else if n < have {
            crate::sim::remove_ships(&mut self.world, e, (have - n) as usize);
        }
        self.sync();
    }

    /// The fleet with a ship within `r` pixels of `p`, nearest first.
    pub fn fleet_at(&self, p: Vec2, r: f32) -> Option<u32> {
        self.fleets
            .iter()
            .filter_map(|f| {
                let d = f.ship_px.iter().map(|s| s.0.distance(p)).fold(f.pos.distance(p), f32::min);
                (d <= r).then_some((f.id, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// Advances the world by `dt` seconds, in the battle's fixed ticks.
    pub fn step(&mut self, dt: f32) {
        self.pending += dt;
        while self.pending >= DT {
            self.pending -= DT;
            self.tick();
        }
    }

    fn tick(&mut self) {
        if self.applied != Some(self.tuning) {
            self.battle = battle_tuning(&self.tuning);
            self.world.insert_resource(self.battle.clone());
            self.applied = Some(self.tuning);
        }
        self.time += DT;
        self.produce(DT);
        self.face_timer += DT;
        if self.face_timer >= 0.5 {
            self.face_timer = 0.0;
            self.face_threats();
        }
        crate::sim::step(&mut self.world);
        let unit = self.unit();
        for s in &mut self.shots {
            s.4 += DT;
        }
        for b in &mut self.booms {
            b.1 += DT;
        }
        self.shots.retain(|s| s.4 < SHOT_SECS);
        self.booms.retain(|b| b.1 < BOOM_SECS);
        let events: Vec<SimEvent> = self.world.resource_mut::<Messages<SimEvent>>().drain().collect();
        for ev in events {
            match ev {
                SimEvent::Shot { from, to, side, hit } => self.shots.push((from * unit, to * unit, side, hit, 0.0)),
                SimEvent::Destroyed { pos, .. } => self.booms.push((pos * unit, 0.0)),
                _ => {}
            }
        }
        self.sync();
        self.capture(DT);
    }

    /// Reads every fleet back from its division; drops the empty ones.
    fn sync(&mut self) {
        let unit = self.unit();
        let mut ships: std::collections::HashMap<Entity, Vec<(Vec2, f32)>> = std::collections::HashMap::new();
        for (p, h, d) in self.world.query::<(&Pos, &Heading, &InDivision)>().iter(&self.world) {
            ships.entry(d.0).or_default().push((p.0 * unit, h.0));
        }
        let mut gone = Vec::new();
        let mut arrived = Vec::new();
        for f in &mut self.fleets {
            let Some(d) = self.world.get::<Division>(f.division) else {
                gone.push(f.division);
                continue;
            };
            let list = ships.remove(&f.division).unwrap_or_default();
            if list.is_empty() {
                gone.push(f.division);
                continue;
            }
            f.pos = list.iter().map(|s| s.0).sum::<Vec2>() / list.len() as f32;
            f.ships = list.len() as u32;
            f.ship_px = list;
            f.facing = d.facing;
            f.formation = d.formation;
            f.morale = d.morale;
            f.engaged = d.since_contact < 1.5;
            if f.morale == Morale::Routed {
                f.dest = None;
                f.home = None;
            }
            if let Some(dest) = f.dest
                && (d.anchor * unit).distance(dest) < 1.0
            {
                f.dest = None;
                arrived.push(f.division);
            }
        }
        for e in arrived {
            if let Some(mut d) = self.world.get_mut::<Division>(e) {
                d.order = Order::Hold;
            }
        }
        self.fleets.retain(|f| !gone.contains(&f.division));
        for e in gone {
            if self.world.get_entity(e).is_ok() {
                self.world.despawn(e);
            }
        }
    }

    /// Fleets that aren't moving turn to face the nearest enemy close by.
    fn face_threats(&mut self) {
        let reach = self.range() * 2.5;
        let unit = self.unit();
        let snapshot: Vec<(Owner, Vec2, Morale)> = self.fleets.iter().map(|f| (f.owner, f.pos, f.morale)).collect();
        for f in &self.fleets {
            if f.dest.is_some() || f.morale == Morale::Routed {
                continue;
            }
            let enemy = snapshot
                .iter()
                .filter(|(o, p, m)| f.owner.hostile(*o) && *m != Morale::Routed && p.distance(f.pos) <= reach)
                .min_by(|a, b| a.1.distance(f.pos).total_cmp(&b.1.distance(f.pos)));
            let Some((_, at, _)) = enemy else { continue };
            let facing = (*at - f.pos).to_angle();
            if let Some(mut d) = self.world.get_mut::<Division>(f.division) {
                let anchor = d.anchor;
                d.order = Order::MoveTo { to: anchor, facing };
            }
        }
        let _ = unit;
    }

    fn produce(&mut self, dt: f32) {
        let secs = self.tuning.secs_per_ship.max(0.1);
        for i in 0..self.planets.len() {
            let (owner, value, pos) = (self.planets[i].owner, self.planets[i].value, self.planets[i].pos);
            let garrison = self.fleets.iter().position(|f| f.home == Some(i) && f.owner == owner);
            let full = matches!(owner, Owner::Independent(_)) && garrison.is_some_and(|g| self.fleets[g].ships >= INDEPENDENT_CAP);
            if full {
                self.planets[i].build = 0.0;
                continue;
            }
            self.planets[i].build += dt * value as f32 / secs;
            while self.planets[i].build >= 1.0 {
                self.planets[i].build -= 1.0;
                match self.fleets.iter().position(|f| f.home == Some(i) && f.owner == owner) {
                    Some(g) => {
                        crate::sim::add_ships(&mut self.world, self.fleets[g].division, 1);
                        self.fleets[g].ships += 1;
                    }
                    None => {
                        let facing = (Vec2::new(self.cfg.w as f32, self.cfg.h as f32) * 0.5 - pos).to_angle();
                        self.spawn(owner, pos, facing, 1, Some(i));
                    }
                }
            }
        }
    }

    fn capture(&mut self, dt: f32) {
        let r = self.capture_radius();
        for i in 0..self.planets.len() {
            let (owner, pos) = (self.planets[i].owner, self.planets[i].pos);
            let near = |o: Owner| self.fleets.iter().any(|f| f.owner == o && f.morale != Morale::Routed && f.pos.distance(pos) <= r);
            if near(owner) {
                self.planets[i].hold = None;
                continue;
            }
            let sides: Vec<usize> = (0..2).filter(|s| Owner::Side(*s) != owner && near(Owner::Side(*s))).collect();
            if sides.len() != 1 {
                // Nobody, or both sides: no progress.
                self.planets[i].hold = None;
                continue;
            }
            let by = Owner::Side(sides[0]);
            let held = match self.planets[i].hold {
                Some((o, t)) if o == by => t + dt,
                _ => dt,
            };
            if held >= HOLD_SECS - 1e-4 {
                let p = &mut self.planets[i];
                p.owner = by;
                p.build = 0.0;
                p.hold = None;
                for f in &mut self.fleets {
                    if f.home == Some(i) {
                        f.home = None;
                    }
                }
            } else {
                self.planets[i].hold = Some((by, held));
            }
        }
    }

    /// Planets, fleets and ships per side, and how many planets are independent.
    pub fn tally(&self) -> ([u32; 2], [u32; 2], [u32; 2], u32) {
        let (mut planets, mut fleets, mut ships, mut free) = ([0; 2], [0; 2], [0; 2], 0);
        for p in &self.planets {
            match p.owner {
                Owner::Side(s) => planets[s] += 1,
                Owner::Independent(_) => free += 1,
            }
        }
        for f in &self.fleets {
            if let Owner::Side(s) = f.owner {
                fleets[s] += 1;
                ships[s] += f.ships;
            }
        }
        (planets, fleets, ships, free)
    }

    /// The fleet's number among its owner's fleets (1-based).
    pub fn fleet_number(&self, idx: usize) -> usize {
        let owner = self.fleets[idx].owner;
        self.fleets[..=idx].iter().filter(|g| g.owner == owner).count()
    }

    /// A key for everything the background layer depends on.
    pub fn background_key(&self, regions: bool) -> String {
        let owners: Vec<Owner> = self.planets.iter().map(|p| p.owner).collect();
        format!("{:?}|{:?}|{regions}", self.cfg, owners)
    }
}

/// The fleets as weighted influence bases (the grow-the-seeds patches).
pub struct Influence {
    /// Score (distance ÷ weight) beyond which a fleet doesn't reach.
    pub reach: f32,
    bases: Vec<Vec2>,
    weights: Vec<f32>,
    owners: Vec<Owner>,
    metric: Metric,
}

impl Influence {
    pub fn new(d: &Dominion) -> Self {
        Self {
            reach: d.tuning.reach_per_weight * d.scale(),
            bases: d.fleets.iter().map(|f| f.pos).collect(),
            weights: d.fleets.iter().map(|f| f.weight(&d.tuning)).collect(),
            owners: d.fleets.iter().map(|f| f.owner).collect(),
            metric: d.cfg.metric,
        }
    }

    /// The dominating fleet at `p` (if any reaches), whether a hostile
    /// fleet contests it, and the score.
    pub fn at(&self, p: Vec2) -> (Option<usize>, bool, f32) {
        let inf = weighted_scores(&self.bases, &self.weights, p, self.metric);
        let Some(best) = inf.owner.filter(|_| inf.best <= self.reach) else { return (None, false, inf.best) };
        let owner = self.owners[best];
        let rival = self
            .bases
            .iter()
            .zip(&self.weights)
            .zip(&self.owners)
            .filter(|((_, w), o)| owner.hostile(**o) && **w > 0.0)
            .map(|((b, w), _)| distance(*b, p, self.metric) / w)
            .fold(f32::INFINITY, f32::min);
        (Some(best), rival <= self.reach && rival <= inf.best * (1.0 + CONTEST_MARGIN), inf.best)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrawOptions {
    pub regions: bool,
    pub borders: bool,
    pub selected: Option<u32>,
    /// Patch sampling step in pixels.
    pub step: u32,
}

impl Default for DrawOptions {
    fn default() -> Self {
        Self { regions: true, borders: true, selected: None, step: 3 }
    }
}

const SPACE: Rgb = [12, 14, 22];
/// Beyond the edge of the world.
const OUTSIDE: Rgb = [6, 7, 11];
const CONTESTED: Rgb = [150, 140, 160];
const WHITE: Rgb = [240, 240, 240];
const INK: Rgb = [10, 10, 14];
const GOLD: Rgb = [235, 190, 80];
const HIT: Rgb = [255, 230, 150];
const SHAKEN: Rgb = [255, 209, 102];
const ROUTED: Rgb = [93, 100, 120];

pub fn owner_color(o: Owner) -> Rgb {
    match o {
        Owner::Side(s) => SIDE_COLORS[s],
        // Each independent faction gets its own muted colour.
        Owner::Independent(id) => Canvas::shade_px(cell_color(id), 0.6),
    }
}

/// Fleet `k` of a side gets its own tone of the side colour.
pub fn fleet_color(side: usize, k: usize) -> Rgb {
    const TONES: [f32; MAX_FLEETS_PER_SIDE] = [0.0, -0.35, 0.35, -0.6, 0.6, -0.18, 0.18, 0.8];
    let base = SIDE_COLORS[side];
    let t = TONES[k % TONES.len()];
    if t >= 0.0 {
        [0, 1, 2].map(|i| (base[i] as f32 + (255.0 - base[i] as f32) * t * 0.7) as u8)
    } else {
        Canvas::shade_px(base, 1.0 + t)
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8)
}

/// What part of the world the picture shows: a fixed-size output image,
/// the world point at its centre, and output pixels per world pixel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub w: u32,
    pub h: u32,
    pub center: Vec2,
    pub zoom: f32,
}

/// The fixed output resolution of the prototype map.
pub const VIEW_W: u32 = 1440;
pub const VIEW_H: u32 = 800;
pub const MAX_ZOOM: f32 = 8.0;

impl View {
    /// The whole world, fitted into a `w`×`h` picture.
    pub fn fit(cfg: &VoronoiConfig, w: u32, h: u32) -> Self {
        let zoom = (w as f32 / cfg.w as f32).min(h as f32 / cfg.h as f32);
        Self { w, h, center: Vec2::new(cfg.w as f32, cfg.h as f32) * 0.5, zoom }
    }

    pub fn to_world(&self, screen: Vec2) -> Vec2 {
        self.center + (screen - Vec2::new(self.w as f32, self.h as f32) * 0.5) / self.zoom
    }

    pub fn to_screen(&self, world: Vec2) -> Vec2 {
        (world - self.center) * self.zoom + Vec2::new(self.w as f32, self.h as f32) * 0.5
    }

    /// Zooms by `factor`, keeping the world point under `screen` in place.
    /// Never zooms out past the whole world, nor in past `MAX_ZOOM` × that.
    pub fn zoom_at(&mut self, screen: Vec2, factor: f32, cfg: &VoronoiConfig) {
        let min = View::fit(cfg, self.w, self.h).zoom;
        let anchor = self.to_world(screen);
        self.zoom = (self.zoom * factor).clamp(min, min * MAX_ZOOM);
        self.center = anchor - (screen - Vec2::new(self.w as f32, self.h as f32) * 0.5) / self.zoom;
        self.clamp(cfg);
    }

    /// Moves the view by `screen_delta` output pixels.
    pub fn pan(&mut self, screen_delta: Vec2, cfg: &VoronoiConfig) {
        self.center -= screen_delta / self.zoom;
        self.clamp(cfg);
    }

    /// Keeps the world's centre-line inside the picture.
    fn clamp(&mut self, cfg: &VoronoiConfig) {
        let half = Vec2::new(self.w as f32, self.h as f32) * 0.5 / self.zoom;
        let world = Vec2::new(cfg.w as f32, cfg.h as f32);
        let lo = (world * 0.5).min(half);
        let hi = (world * 0.5).max(world - half);
        self.center = self.center.clamp(lo, hi);
    }
}

/// The slow-changing layer: planet regions in their owner's colours,
/// computed per output pixel so borders stay sharp at any zoom. `step` > 1
/// samples coarsely (while the camera is moving).
pub fn render_background(d: &Dominion, view: &View, regions: bool, step: u32) -> Canvas {
    let mut c = Canvas::new(view.w, view.h, OUTSIDE);
    let index = crate::voronoi::metric::SiteIndex::new(&d.sites, d.cfg.metric);
    let world = Vec2::new(d.cfg.w as f32, d.cfg.h as f32);
    let step = step.max(1);
    for y in (0..view.h).step_by(step as usize) {
        for x in (0..view.w).step_by(step as usize) {
            let p = view.to_world(Vec2::new(x as f32 + step as f32 * 0.5, y as f32 + step as f32 * 0.5));
            if p.x < 0.0 || p.y < 0.0 || p.x > world.x || p.y > world.y {
                continue;
            }
            let n = index.nearest_two(p);
            let planet = d.planet_at.get(n.nearest).copied().flatten();
            let col = match planet {
                Some(pi) if regions => {
                    // On a border when the two nearest sites are almost tied
                    // (about one output pixel either side).
                    if (n.d2 - n.d1) * view.zoom < 1.2 * step as f32 && d.planet_at.get(n.second).copied().flatten() != Some(pi) {
                        INK
                    } else {
                        let pl = &d.planets[pi];
                        Canvas::shade_px(owner_color(pl.owner), if matches!(pl.owner, Owner::Side(_)) { 0.45 } else { 0.75 })
                    }
                }
                _ => SPACE,
            };
            for by in y..(y + step).min(view.h) {
                for bx in x..(x + step).min(view.w) {
                    c.set(bx as i32, by as i32, col);
                }
            }
        }
    }
    c
}

/// Each fleet's display tone.
pub fn tones(d: &Dominion) -> Vec<Rgb> {
    (0..d.fleets.len())
        .map(|i| match d.fleets[i].owner {
            Owner::Side(s) => fleet_color(s, (d.fleet_number(i) - 1) % MAX_FLEETS_PER_SIDE),
            o => owner_color(o),
        })
        .collect()
}

/// Patches, planets and ships over a cached background, through `view`.
pub fn render(d: &Dominion, background: &Canvas, view: &View, opts: DrawOptions) -> Canvas {
    let (w, h) = (view.w, view.h);
    let mut c = background.clone();
    let tones = tones(d);
    let z = view.zoom;
    // Patches, sampled every `step` output pixels.
    let step = opts.step.max(1);
    let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
    let inf = Influence::new(d);
    let world = Vec2::new(d.cfg.w as f32, d.cfg.h as f32);
    let mut cells: Vec<(Option<usize>, bool, bool)> = vec![(None, false, false); (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let p = view.to_world(Vec2::new((gx * step) as f32 + step as f32 * 0.5, (gy * step) as f32 + step as f32 * 0.5));
            if p.x < 0.0 || p.y < 0.0 || p.x > world.x || p.y > world.y {
                continue;
            }
            let (fleet, contested, score) = inf.at(p);
            // The last ~1.5 output pixels of a patch's reach.
            let front = fleet.is_some_and(|f| (inf.reach - score) * d.fleets[f].weight(&d.tuning) * z < 1.5 * step as f32);
            cells[(gy * gw + gx) as usize] = (fleet, contested, front);
        }
    }
    let coarse = |x: u32, y: u32| cells[((y / step) * gw + x / step) as usize];
    // Cells next to a different cell sit on an edge: those get every pixel
    // computed exactly, so patch outlines and seams stay smooth.
    let edge_cell = |gx: u32, gy: u32| {
        let here = cells[(gy * gw + gx) as usize];
        here.2
            || [(gx.wrapping_sub(1), gy), (gx + 1, gy), (gx, gy.wrapping_sub(1)), (gx, gy + 1)]
                .iter()
                .any(|&(nx, ny)| nx < gw && ny < gh && cells[(ny * gw + nx) as usize].0 != here.0)
    };
    let mut fine: Vec<Option<(Option<usize>, bool, bool)>> = vec![None; (w * h) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            if !edge_cell(gx, gy) {
                continue;
            }
            for y in gy * step..((gy + 1) * step).min(h) {
                for x in gx * step..((gx + 1) * step).min(w) {
                    let p = view.to_world(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                    let v = if p.x < 0.0 || p.y < 0.0 || p.x > world.x || p.y > world.y {
                        (None, false, false)
                    } else {
                        let (fleet, contested, score) = inf.at(p);
                        let front = fleet.is_some_and(|f| (inf.reach - score) * d.fleets[f].weight(&d.tuning) * z < 1.5);
                        (fleet, contested, front)
                    };
                    fine[(y * w + x) as usize] = Some(v);
                }
            }
        }
    }
    let at = |x: u32, y: u32| fine[(y * w + x) as usize].unwrap_or_else(|| coarse(x, y));
    for y in 0..h {
        for x in 0..w {
            let (fleet, contested, front) = at(x, y);
            let Some(f) = fleet else { continue };
            let base = c.get(x, y);
            let mut col = if contested && (x + y) % 8 < 4 { mix(base, CONTESTED, 0.5) } else { mix(base, tones[f], 0.35) };
            if opts.borders {
                let right = if x + 1 < w { at(x + 1, y).0 } else { fleet };
                let down = if y + 1 < h { at(x, y + 1).0 } else { fleet };
                if (right.is_some() && right != fleet) || (down.is_some() && down != fleet) {
                    col = INK;
                } else if front {
                    col = tones[f];
                }
            }
            c.set(x as i32, y as i32, col);
        }
    }
    // Planets (world size, at least a few pixels), with a capture ring.
    for p in &d.planets {
        let at = view.to_screen(p.pos);
        let r = ((1.5 + 1.3 * p.value as f32) * d.scale().max(0.8) * z).max(2.5);
        if let Some((by, t)) = p.hold {
            let frac = (t / HOLD_SECS).clamp(0.0, 1.0);
            let n = (64.0 * frac) as usize;
            for k in 0..n {
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 / 64.0 * std::f32::consts::TAU;
                c.disc(at + Vec2::from_angle(a) * (r + 6.0), 1.8, owner_color(by), 1.0);
            }
        }
        c.disc(at, r + 1.0, INK, 1.0);
        c.disc(at, r, [230, 220, 170], 1.0);
    }
    // Destination of the selected fleet.
    if let Some(f) = opts.selected.and_then(|id| d.fleet(id))
        && let Some(dest) = f.dest
    {
        let (a, b) = (view.to_screen(f.pos), view.to_screen(dest));
        c.dashed_line(a, b, 6.0, 5.0, GOLD, 0.9);
        c.circle(b, 5.0, GOLD, 1.0);
    }
    // Shots, then ships (each ECS ship where it is), then explosions.
    for (from, to, side, hit, age) in &d.shots {
        let col = if *side < 2 { mix(SIDE_COLORS[*side], WHITE, 0.3) } else { CONTESTED };
        let a = (1.0 - age / SHOT_SECS).clamp(0.0, 1.0) * if *hit { 0.95 } else { 0.35 };
        c.line(view.to_screen(*from), view.to_screen(*to), if *hit { 1.5 } else { 1.0 }, col, a);
    }
    let dart_scale = (0.32 * d.scale() * z).max(0.35);
    for (i, f) in d.fleets.iter().enumerate() {
        let ships: Vec<(Vec2, f32)> = f.ship_px.iter().map(|(p, h)| (view.to_screen(*p), *h)).collect();
        let centre = view.to_screen(f.pos);
        if opts.selected == Some(f.id) {
            let r = ships.iter().map(|s| s.0.distance(centre)).fold(0.0, f32::max) + 7.0;
            c.circle(centre, r, GOLD, 1.0);
            c.circle(centre, r + 1.0, GOLD, 1.0);
        }
        let flag = (0..ships.len()).min_by(|a, b| ships[*a].0.distance(centre).total_cmp(&ships[*b].0.distance(centre)));
        let tone = match f.morale {
            Morale::Steady => mix(tones[i], WHITE, 0.25),
            Morale::Shaken => SHAKEN,
            Morale::Routed => ROUTED,
        };
        for (k, (s, heading)) in ships.iter().enumerate() {
            if s.x < -10.0 || s.y < -10.0 || s.x > w as f32 + 10.0 || s.y > h as f32 + 10.0 {
                continue;
            }
            let col = if Some(k) == flag && f.controllable() { WHITE } else { tone };
            let facing = Vec2::from_angle(*heading);
            let pts = |k: f32| -> Vec<Vec2> { SHIP_DART.iter().map(|p| *s + facing.rotate(*p * dart_scale * k)).collect() };
            c.polygon_fill(&pts(1.45), INK, 1.0);
            c.polygon_fill(&pts(1.0), col, 1.0);
        }
    }
    for (p, age) in &d.booms {
        let k = age / BOOM_SECS;
        c.circle(view.to_screen(*p), (3.0 + 9.0 * k) * (z * d.scale()).clamp(0.6, 2.0), HIT, (1.0 - k) * 0.9);
    }
    c
}

/// The status lines for the page.
pub fn status(d: &Dominion, selected: Option<u32>) -> String {
    let (planets, fleets, ships, free) = d.tally();
    let secs = d.time as u32;
    let mut s = format!(
        "{}:{:02} · Blue {} planets, {} fleets, {} ships · Red {} planets, {} fleets, {} ships · {} independent planets",
        secs / 60,
        secs % 60,
        planets[0],
        fleets[0],
        ships[0],
        planets[1],
        fleets[1],
        ships[1],
        free
    );
    match selected.and_then(|id| d.fleet_index(id)) {
        Some(i) => {
            let f = &d.fleets[i];
            let who = match f.owner {
                Owner::Side(side) => format!("{} fleet {}", SIDE_NAMES[side], d.fleet_number(i)),
                Owner::Independent(_) => "Independent garrison".into(),
            };
            let order = if f.morale == Morale::Routed {
                "fleeing"
            } else if f.dest.is_some() {
                "moving"
            } else if f.home.is_some() {
                "garrisoning"
            } else {
                "holding"
            };
            let fight = if f.engaged { " · ENGAGED" } else { "" };
            s.push_str(&format!("\nSelected: {who}, {} ships, {}, {}, {order}{fight}.", f.ships, f.formation.name(), f.morale.name()));
            if f.morale == Morale::Routed {
                s.push_str(" Routed: it no longer takes orders.");
            } else if !f.controllable() {
                s.push_str(&format!(" Takes orders at {CONTROL_MIN} ships."));
            } else {
                s.push_str(" Click the map to move it · keys 1–6 set its formation.");
            }
        }
        None => s.push_str("\nClick a fleet with 10+ ships to select it, then click the map to send it there."),
    }
    s
}
