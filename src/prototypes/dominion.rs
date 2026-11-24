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
//!
//! Economy: one resource (credits), a stockpile per owner. A planet of
//! size x produces x × income; its owner takes its tax share. A ship costs
//! credits to build, every shot costs credits, and every few seconds a
//! maintenance phase pays the fleets' upkeep (unpaid ships desert) and
//! repairs damaged ships out of combat. An owner out of credits can't fire.
//!
//! Unrest: every Blue or Red planet's people run Epstein's civil-violence
//! model ([`super::rebellion`]). The owner's ships nearby are its cops,
//! legitimacy is 1 − tax − occupation (fresh conquests start resented),
//! and fighting nearby adds hardship. Active rebels strike (income and
//! building drop by their share); when the smoothed rebel share passes the
//! revolt share the planet goes independent with a rebel garrison.

use bevy::ecs::message::Messages;
use bevy::prelude::{Entity, Rect, World};

use super::rebellion::{Rebellion, RebellionParams};
use crate::sim::battle::{Battle, DT, SimEvent, Tuning as BattleTuning};
use crate::sim::components::{Division, Heading, InDivision, Morale, Order, Pos, Ship, Weapon};
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
pub const START_SHIPS: u32 = 10;
pub const MIN_SHIPS: u32 = 1;
pub const MAX_SHIPS: u32 = 100;
/// Independent garrisons build up to this many ships per point of size
/// (every planet starts with `size` ships).
pub const INDEPENDENT_CAP_PER_SIZE: u32 = 5;
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
/// Seconds between maintenance phases (upkeep and repairs).
pub const MAINTENANCE_SECS: f32 = 5.0;
/// Credits Blue and Red start with.
pub const START_CREDITS: f32 = 50.0;
/// Credits each independent planet starts with.
pub const INDEPENDENT_CREDITS: f32 = 10.0;
const SIDE_COLORS: [Rgb; 2] = [[70, 130, 220], [215, 80, 70]];
/// A planet's tax until its owner changes it (legitimacy 0.8, near the
/// NetLogo model's default of 0.82).
pub const DEFAULT_TAX: f32 = 0.2;
pub const MAX_TAX: f32 = 0.9;
/// Legitimacy lost when a side conquers a planet, fading over `OCCUPATION_SECS`.
pub const OCCUPATION_PENALTY: f32 = 0.3;
pub const OCCUPATION_SECS: f32 = 60.0;
/// War hardship rises to full in this many seconds of fighting nearby and
/// fades over `WAR_FADE_SECS`.
pub const WAR_RISE_SECS: f32 = 3.0;
pub const WAR_FADE_SECS: f32 = 20.0;
/// Seconds of game time per step of a planet's rebellion model.
pub const UNREST_STEP_SECS: f32 = 0.5;
/// Time constant of the smoothed rebel share (single bursts don't topple).
pub const UNREST_SMOOTH_SECS: f32 = 10.0;
/// Model steps the home planets' people take before the game starts.
pub const WARM_UP_STEPS: usize = 40;
/// Most cops a planet's model gets.
pub const MAX_COPS: usize = 200;
/// Rebel ships per point of size for a planet whose people all rebelled.
pub const REBELS_PER_SIZE: f32 = 10.0;

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
    /// Income multiplier: a planet of size x produces x × this credits a
    /// second, of which its owner takes the planet's tax.
    pub income: f32,
    /// Credits to build one ship.
    pub ship_cost: f32,
    /// Credits a second each ship costs (debts are settled at the
    /// maintenance phase: unpaid ships desert).
    pub upkeep: f32,
    /// Credits per shot fired.
    pub shot_cost: f32,
    /// Credits per hull point repaired at the maintenance phase.
    pub repair_cost: f32,
    /// Cops in a planet's rebellion model per ship nearby, divided by the
    /// planet's size (bigger populations need more ships).
    pub cops_per_ship: f32,
    /// Hardship added to everyone while fighting rages nearby.
    pub war_hardship: f32,
    /// Smoothed rebel share at which a planet revolts.
    pub revolt_share: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        // Building must stay slower than a fleet in range can kill: 60 ships
        // firing at the front kill about 0.5 ships a second.
        Self {
            weight_per_ship: 0.1,
            reach_per_weight: 14.0,
            secs_per_ship: 15.0,
            speed: 30.0,
            combat_speed: 3.0,
            income: 4.0,
            ship_cost: 10.0,
            upkeep: 0.05,
            shot_cost: 0.01,
            repair_cost: 0.2,
            cops_per_ship: 40.0,
            war_hardship: 0.3,
            revolt_share: 0.25,
        }
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
    /// Size, 1 (small) to 3 (large): build rate, income and population.
    pub value: u32,
    pub owner: Owner,
    /// Progress toward the next ship, 0..1.
    pub build: f32,
    /// An attacker holding the ungarrisoned planet, and for how long.
    pub hold: Option<(Owner, f32)>,
    /// Share of production its owner takes, 0..=MAX_TAX.
    pub tax: f32,
    /// Legitimacy lost to a recent conquest, fading to 0.
    pub occupation: f32,
    /// Fighting nearby, 0..1 (adds hardship).
    pub war: f32,
    /// Smoothed share of its people actively rebelling.
    pub unrest: f32,
    /// Cops its rebellion model was last given.
    pub cops: usize,
}

impl Planet {
    /// 1 − tax − occupation.
    pub fn legitimacy(&self) -> f32 {
        (1.0 - self.tax - self.occupation).clamp(0.0, 1.0)
    }

    /// What the strikes leave of production and building.
    pub fn working(&self) -> f32 {
        1.0 - self.unrest.clamp(0.0, 1.0)
    }
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
    /// Each ship's entity, in the same order as `ship_px`.
    pub ship_ids: Vec<Entity>,
    /// Fired or was fired on in the last second or so.
    pub engaged: bool,
}

impl Fleet {
    pub fn weight(&self, t: &Tuning) -> f32 {
        self.ships as f32 * t.weight_per_ship
    }

    /// Blue and Red fleets, unless routed.
    pub fn controllable(&self) -> bool {
        matches!(self.owner, Owner::Side(_)) && self.morale != Morale::Routed
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
    /// Credits per owner: Blue, Red, then each planet's independents
    /// (see `purse`). Below zero only between maintenance phases.
    pub credits: Vec<f32>,
    /// Seconds since the last maintenance phase.
    pub maintenance_timer: f32,
    /// Ships lost to unpaid upkeep, per side (Blue, Red).
    pub deserted: [u32; 2],
    /// Each planet's people (rebellion model), stepped while Blue or Red own it.
    pub people: Vec<Rebellion>,
    pub rebellion: RebellionParams,
    /// Planets lost to revolts, per side.
    pub revolts: [u32; 2],
    unrest_timer: f32,
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
            list.push(Planet {
                site,
                pos: sites[site],
                value,
                owner: Owner::Independent(list.len()),
                build: 0.0,
                hold: None,
                tax: DEFAULT_TAX,
                occupation: 0.0,
                war: 0.0,
                unrest: 0.0,
                cops: 0,
            });
        }
        let list_len = list.len();
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
            credits: Vec::new(),
            maintenance_timer: 0.0,
            deserted: [0; 2],
            people: (0..list_len).map(|i| Rebellion::new(cfg.seed as i64 * 31 + i as i64 + 101)).collect(),
            rebellion: RebellionParams::default(),
            revolts: [0; 2],
            unrest_timer: 0.0,
            next_id: 1,
            time: 0.0,
            pending: 0.0,
            applied: Some(tuning),
            face_timer: 0.0,
        };
        d.credits = [START_CREDITS; 2].into_iter().chain(std::iter::repeat_n(INDEPENDENT_CREDITS, d.planets.len())).collect();
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
        // Every planet starts with a garrison of `size` ships.
        for i in 0..d.planets.len() {
            let (owner, pos, size) = (d.planets[i].owner, d.planets[i].pos, d.planets[i].value);
            let facing = (Vec2::new(cfg.w as f32, cfg.h as f32) * 0.5 - pos).to_angle();
            d.spawn(owner, pos, facing, size, Some(i));
        }
        d.sync();
        // The home planets' people settle under their garrisons before the
        // game starts (a fresh model opens with a burst of rebels).
        for i in 0..d.planets.len() {
            if matches!(d.planets[i].owner, Owner::Side(_)) {
                let (legit, cops) = (d.planets[i].legitimacy(), d.cops_for(i));
                for _ in 0..WARM_UP_STEPS {
                    d.people[i].step(legit, 0.0, cops, &d.rebellion);
                }
                d.planets[i].cops = cops;
            }
        }
        d
    }

    /// Cops for planet `i`'s model: its owner's ships within the police
    /// radius × cops per ship ÷ size.
    pub fn cops_for(&self, i: usize) -> usize {
        let p = &self.planets[i];
        let police = self.police_radius();
        let ships: u32 = self
            .fleets
            .iter()
            .filter(|f| f.owner == p.owner && f.morale != Morale::Routed && f.pos.distance(p.pos) <= police)
            .map(|f| f.ships)
            .sum();
        ((ships as f32 * self.tuning.cops_per_ship / p.value.max(1) as f32).round() as usize).min(MAX_COPS)
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
            ship_ids: Vec::new(),
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

    /// Radius around a planet within which its owner's ships police it.
    pub fn police_radius(&self) -> f32 {
        2.0 * self.capture_radius()
    }

    /// Index of an owner's stockpile in `credits`.
    pub fn purse(owner: Owner) -> usize {
        match owner {
            Owner::Side(s) => s,
            Owner::Independent(i) => 2 + i,
        }
    }

    pub fn credits_of(&self, owner: Owner) -> f32 {
        self.credits.get(Self::purse(owner)).copied().unwrap_or(0.0)
    }

    /// Credits a second a planet earns its owner: its tax share of what it
    /// produces, less what strikes cost.
    pub fn planet_earning(&self, p: &Planet) -> f32 {
        p.value as f32 * self.tuning.income * p.tax * p.working()
    }

    /// Credits a second an owner's planets earn.
    pub fn planet_income(&self, owner: Owner) -> f32 {
        self.planets.iter().filter(|p| p.owner == owner).map(|p| self.planet_earning(p)).sum()
    }

    /// Sets a planet's tax (clamped to 0..=MAX_TAX).
    pub fn set_tax(&mut self, i: usize, tax: f32) {
        if let Some(p) = self.planets.get_mut(i) {
            p.tax = tax.clamp(0.0, MAX_TAX);
        }
    }

    /// The planet within `r` pixels of `p`, nearest first.
    pub fn planet_near(&self, p: Vec2, r: f32) -> Option<usize> {
        (0..self.planets.len())
            .filter(|i| self.planets[*i].pos.distance(p) <= r)
            .min_by(|a, b| self.planets[*a].pos.distance(p).total_cmp(&self.planets[*b].pos.distance(p)))
    }

    pub fn ships_of(&self, owner: Owner) -> u32 {
        self.fleets.iter().filter(|f| f.owner == owner).map(|f| f.ships).sum()
    }

    /// Credits a second after planets and upkeep (shots not counted).
    pub fn net_income(&self, owner: Owner) -> f32 {
        self.planet_income(owner) - self.ships_of(owner) as f32 * self.tuning.upkeep
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

    /// The fleet a ship belongs to.
    pub fn fleet_of_ship(&self, ship: Entity) -> Option<&Fleet> {
        self.fleets.iter().find(|f| f.ship_ids.contains(&ship))
    }

    /// Ships of controllable fleets inside the box (map pixels). With both
    /// sides inside, only the side with more ships there.
    pub fn ships_in_box(&self, a: Vec2, b: Vec2) -> Vec<Entity> {
        let (lo, hi) = (a.min(b), a.max(b));
        let mut found: [Vec<Entity>; 2] = [Vec::new(), Vec::new()];
        for f in self.fleets.iter().filter(|f| f.controllable()) {
            let side = f.owner.side();
            for (p, e) in f.ship_px.iter().zip(&f.ship_ids) {
                if p.0.cmpge(lo).all() && p.0.cmple(hi).all() {
                    found[side].push(*e);
                }
            }
        }
        let [blue, red] = found;
        if red.len() > blue.len() { red } else { blue }
    }

    /// Fleets every one of whose ships is in `ships`.
    pub fn whole_fleets(&self, ships: &[Entity]) -> Vec<u32> {
        self.fleets.iter().filter(|f| !f.ship_ids.is_empty() && f.ship_ids.iter().all(|e| ships.contains(e))).map(|f| f.id).collect()
    }

    /// Right-click with `ships` selected: they become one fleet sent to `to`.
    /// One whole fleet just moves. Otherwise the biggest wholly selected
    /// fleet takes in the rest (merge), or, if none is whole, the ships
    /// split off into a new fleet. Only one owner's controllable ships
    /// count. Returns the fleet that moves.
    pub fn form_fleet(&mut self, ships: &[Entity], to: Vec2) -> Option<u32> {
        let mut picked: Vec<(Entity, Owner)> = ships
            .iter()
            .filter_map(|e| self.fleet_of_ship(*e).filter(|f| f.controllable()).map(|f| (*e, f.owner)))
            .collect();
        let owner = picked.first()?.1;
        picked.retain(|p| p.1 == owner);
        let ships: Vec<Entity> = picked.iter().map(|p| p.0).collect();
        let whole = self.whole_fleets(&ships);
        let sources: Vec<u32> = {
            let mut v: Vec<u32> = ships.iter().filter_map(|e| self.fleet_of_ship(*e)).map(|f| f.id).collect();
            v.sort();
            v.dedup();
            v
        };
        let target = if sources.len() == 1 && whole.len() == 1 {
            whole[0]
        } else if let Some(&big) = whole.iter().max_by_key(|id| self.fleet(**id).map_or(0, |f| f.ships)) {
            big
        } else {
            // A new fleet where the ships are, facing like their first fleet.
            let unit = self.unit();
            let centre = self
                .world
                .query::<(Entity, &Pos)>()
                .iter(&self.world)
                .filter(|(e, _)| ships.contains(e))
                .map(|(_, p)| p.0 * unit)
                .sum::<Vec2>()
                / ships.len() as f32;
            let first = self.fleet_of_ship(ships[0])?;
            let (facing, formation) = (first.facing, first.formation);
            let id = self.spawn(owner, centre, facing, 0, None);
            if let Some(e) = self.fleet(id).map(|f| f.division) {
                crate::sim::set_formation(&mut self.world, e, formation);
            }
            id
        };
        let division = self.fleet(target)?.division;
        crate::sim::transfer_ships(&mut self.world, &ships, division);
        self.sync();
        self.order_move(target, to);
        Some(target)
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
        self.earn(DT);
        self.produce(DT);
        self.hold_fire();
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
                SimEvent::Shot { from, to, side, hit } => {
                    self.pay_for_shot(from * unit, side);
                    self.shots.push((from * unit, to * unit, side, hit, 0.0));
                }
                SimEvent::Destroyed { pos, .. } => self.booms.push((pos * unit, 0.0)),
                _ => {}
            }
        }
        self.sync();
        self.capture(DT);
        self.unrest(DT);
        self.maintenance_timer += DT;
        if self.maintenance_timer >= MAINTENANCE_SECS - 1e-4 {
            self.maintenance_timer = 0.0;
            self.maintain();
        }
    }

    /// Every tick planets earn their owners' share, and
    /// fleets pay their upkeep.
    fn earn(&mut self, dt: f32) {
        let t = self.tuning;
        for i in 0..self.planets.len() {
            let earned = self.planet_earning(&self.planets[i]);
            if let Some(c) = self.credits.get_mut(Self::purse(self.planets[i].owner)) {
                *c += dt * earned;
            }
        }
        for f in &self.fleets {
            if let Some(c) = self.credits.get_mut(Self::purse(f.owner)) {
                *c -= dt * f.ships as f32 * t.upkeep;
            }
        }
    }

    /// The shooter's owner pays for the shot (independents: the nearest
    /// independent fleet's planet).
    fn pay_for_shot(&mut self, from: Vec2, side: usize) {
        let owner = if side < 2 {
            Some(Owner::Side(side))
        } else {
            self.fleets
                .iter()
                .filter(|f| f.owner.side() == side)
                .min_by(|a, b| a.pos.distance(from).total_cmp(&b.pos.distance(from)))
                .map(|f| f.owner)
        };
        if let Some(c) = owner.and_then(|o| self.credits.get_mut(Self::purse(o))) {
            *c -= self.tuning.shot_cost;
        }
    }

    /// Owners out of credits can't fire: their guns stay unloaded.
    fn hold_fire(&mut self) {
        let broke: Vec<Entity> = self.fleets.iter().filter(|f| self.credits_of(f.owner) <= 0.0).map(|f| f.division).collect();
        if broke.is_empty() {
            return;
        }
        for (d, mut w) in self.world.query::<(&InDivision, &mut Weapon)>().iter_mut(&mut self.world) {
            if broke.contains(&d.0) {
                w.cooldown = w.cooldown.max(2.0 * DT);
            }
        }
    }

    /// The maintenance phase: an owner in debt loses the ships it couldn't
    /// pay for since the last phase (they desert, from its biggest fleets),
    /// then damaged ships of fleets out of combat are repaired while
    /// credits last.
    fn maintain(&mut self) {
        let per_ship = self.tuning.upkeep * MAINTENANCE_SECS;
        for purse in 0..self.credits.len() {
            let owner = if purse < 2 { Owner::Side(purse) } else { Owner::Independent(purse - 2) };
            let ships = self.ships_of(owner);
            if self.credits[purse] >= 0.0 {
                continue;
            }
            let short = -self.credits[purse];
            self.credits[purse] = 0.0;
            if per_ship <= 0.0 {
                continue;
            }
            let mut cut = ((short / per_ship).ceil() as u32).min(ships);
            if let Owner::Side(s) = owner {
                self.deserted[s] += cut;
            }
            // One at a time from whichever fleet is biggest.
            let mut fleets: Vec<(Entity, u32)> = self.fleets.iter().filter(|f| f.owner == owner).map(|f| (f.division, f.ships)).collect();
            let mut take = vec![0u32; fleets.len()];
            while cut > 0 {
                let Some(i) = (0..fleets.len()).filter(|i| fleets[*i].1 > 0).max_by_key(|i| fleets[*i].1) else { break };
                fleets[i].1 -= 1;
                take[i] += 1;
                cut -= 1;
            }
            for (i, (e, _)) in fleets.iter().enumerate() {
                if take[i] > 0 {
                    crate::sim::remove_ships(&mut self.world, *e, take[i] as usize);
                }
            }
        }
        // Repairs, for fleets not fighting.
        let owners: std::collections::HashMap<Entity, Owner> =
            self.fleets.iter().filter(|f| !f.engaged).map(|f| (f.division, f.owner)).collect();
        let cost = self.tuning.repair_cost;
        let credits = &mut self.credits;
        for (d, mut ship) in self.world.query::<(&InDivision, &mut Ship)>().iter_mut(&mut self.world) {
            let Some(owner) = owners.get(&d.0) else { continue };
            let missing = ship.max_hull - ship.hull;
            if missing <= 0.0 {
                continue;
            }
            let c = &mut credits[Self::purse(*owner)];
            let fix = if cost > 0.0 { missing.min(*c / cost) } else { missing };
            if fix <= 0.0 {
                continue;
            }
            ship.hull += fix;
            *c -= fix * cost;
        }
        self.sync();
    }

    /// Reads every fleet back from its division; drops the empty ones.
    fn sync(&mut self) {
        let unit = self.unit();
        let mut ships: std::collections::HashMap<Entity, Vec<(Vec2, f32, Entity)>> = std::collections::HashMap::new();
        for (e, p, h, d) in self.world.query::<(Entity, &Pos, &Heading, &InDivision)>().iter(&self.world) {
            ships.entry(d.0).or_default().push((p.0 * unit, h.0, e));
        }
        let mut gone = Vec::new();
        let mut arrived = Vec::new();
        for f in &mut self.fleets {
            let Some(d) = self.world.get::<Division>(f.division) else {
                gone.push(f.division);
                continue;
            };
            let mut list = ships.remove(&f.division).unwrap_or_default();
            if list.is_empty() {
                gone.push(f.division);
                continue;
            }
            list.sort_by_key(|s| s.2);
            f.pos = list.iter().map(|s| s.0).sum::<Vec2>() / list.len() as f32;
            f.ships = list.len() as u32;
            f.ship_px = list.iter().map(|s| (s.0, s.1)).collect();
            f.ship_ids = list.iter().map(|s| s.2).collect();
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

    /// Whether an owner can pay for one more ship now and keep paying its
    /// upkeep from its planets' income.
    fn can_build(&self, owner: Owner) -> bool {
        let t = &self.tuning;
        self.credits_of(owner) >= t.ship_cost && self.planet_income(owner) >= (self.ships_of(owner) + 1) as f32 * t.upkeep
    }

    fn produce(&mut self, dt: f32) {
        let secs = self.tuning.secs_per_ship.max(0.1);
        for i in 0..self.planets.len() {
            let (owner, value, pos) = (self.planets[i].owner, self.planets[i].value, self.planets[i].pos);
            let garrison = self.fleets.iter().position(|f| f.home == Some(i) && f.owner == owner);
            let full = matches!(owner, Owner::Independent(_))
                && garrison.is_some_and(|g| self.fleets[g].ships >= INDEPENDENT_CAP_PER_SIZE * value);
            if full {
                self.planets[i].build = 0.0;
                continue;
            }
            self.planets[i].build += dt * value as f32 / secs * self.planets[i].working();
            while self.planets[i].build >= 1.0 {
                // Short of credits or of income for its upkeep: the ship waits.
                if !self.can_build(owner) {
                    self.planets[i].build = 1.0;
                    break;
                }
                self.planets[i].build -= 1.0;
                self.credits[Self::purse(owner)] -= self.tuning.ship_cost;
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
                // A fresh conquest is resented.
                p.occupation = OCCUPATION_PENALTY;
                p.unrest = 0.0;
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

    /// War and occupation evolve; every half second the people of Blue's
    /// and Red's planets take a step of the rebellion model; a planet whose
    /// smoothed rebel share passes the revolt share goes independent.
    fn unrest(&mut self, dt: f32) {
        let r = self.capture_radius();
        for p in &mut self.planets {
            let fighting = self.fleets.iter().any(|f| f.engaged && f.pos.distance(p.pos) <= r);
            p.war = if fighting { p.war + dt / WAR_RISE_SECS } else { p.war - dt / WAR_FADE_SECS }.clamp(0.0, 1.0);
            p.occupation = (p.occupation - dt * OCCUPATION_PENALTY / OCCUPATION_SECS).max(0.0);
        }
        self.unrest_timer += dt;
        if self.unrest_timer < UNREST_STEP_SECS - 1e-4 {
            return;
        }
        self.unrest_timer = 0.0;
        let t = self.tuning;
        let blend = UNREST_STEP_SECS / UNREST_SMOOTH_SECS;
        for i in 0..self.planets.len() {
            let p = self.planets[i];
            let Owner::Side(side) = p.owner else { continue };
            let cops = self.cops_for(i);
            let people = &mut self.people[i];
            people.step(p.legitimacy(), p.war * t.war_hardship, cops, &self.rebellion);
            let share = people.active_share();
            let rebels = share + people.jailed_share();
            let pl = &mut self.planets[i];
            pl.cops = cops;
            pl.unrest += (share - pl.unrest) * blend;
            if pl.unrest > t.revolt_share {
                self.revolt(i, side, rebels);
            }
        }
    }

    /// The planet goes independent; its rebels (active and jailed) arm a
    /// garrison that fights whatever of the old owner's is there.
    fn revolt(&mut self, i: usize, side: usize, rebels: f32) {
        self.revolts[side] += 1;
        let (pos, value) = (self.planets[i].pos, self.planets[i].value);
        let p = &mut self.planets[i];
        p.owner = Owner::Independent(i);
        p.build = 0.0;
        p.hold = None;
        p.tax = DEFAULT_TAX;
        p.occupation = 0.0;
        p.unrest = 0.0;
        p.cops = 0;
        self.people[i].reset();
        for f in &mut self.fleets {
            if f.home == Some(i) {
                f.home = None;
            }
        }
        let purse = Self::purse(Owner::Independent(i));
        self.credits[purse] = self.credits[purse].max(INDEPENDENT_CREDITS);
        let ships = ((rebels * value as f32 * REBELS_PER_SIZE).round() as u32).max(1);
        let facing = (Vec2::new(self.cfg.w as f32, self.cfg.h as f32) * 0.5 - pos).to_angle();
        self.spawn(Owner::Independent(i), pos, facing, ships, Some(i));
        self.sync();
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

#[derive(Clone, Debug, PartialEq)]
pub struct DrawOptions {
    pub regions: bool,
    pub borders: bool,
    /// Selected ships.
    pub selected: Vec<Entity>,
    /// A selection box being dragged, two corners in picture pixels.
    pub marquee: Option<(Vec2, Vec2)>,
    /// Patch sampling step in pixels.
    pub step: u32,
    /// The selected planet: highlighted, its people drawn in an inset.
    pub planet: Option<usize>,
}

impl Default for DrawOptions {
    fn default() -> Self {
        Self { regions: true, borders: true, selected: Vec::new(), marquee: None, step: 3, planet: None }
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
const UNREST_LOW: Rgb = [240, 210, 90];
const UNREST_HIGH: Rgb = [235, 60, 50];

/// A planet's rebellion model as an inset: quiet people grey, rebels red,
/// prisoners dark, cops in the owner's colour.
fn draw_people(c: &mut Canvas, people: &Rebellion, owner: Owner) {
    use super::rebellion::{Cell, SIDE};
    let (px, margin) = (3i32, 6i32);
    let size = SIDE as i32 * px;
    for y in margin - 2..margin + size + 2 {
        for x in margin - 2..margin + size + 2 {
            c.set(x, y, INK);
        }
    }
    let cop = match owner {
        Owner::Side(s) => mix(SIDE_COLORS[s], WHITE, 0.3),
        o => owner_color(o),
    };
    for (k, cell) in people.cells().iter().enumerate() {
        let col = match cell {
            Cell::Empty => [24, 26, 36],
            Cell::Quiet => [120, 124, 136],
            Cell::Active => [235, 60, 50],
            Cell::Jailed => [60, 50, 50],
            Cell::Cop => cop,
        };
        let (x, y) = ((k % SIDE) as i32 * px + margin, (k / SIDE) as i32 * px + margin);
        for dy in 0..px - 1 {
            for dx in 0..px - 1 {
                c.set(x + dx, y + dy, col);
            }
        }
    }
}
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
pub fn render(d: &Dominion, background: &Canvas, view: &View, opts: &DrawOptions) -> Canvas {
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
        // Unrest: an arc around Blue's and Red's planets, yellow to red,
        // full at the revolt share.
        if matches!(p.owner, Owner::Side(_)) && p.unrest > 0.01 {
            let frac = (p.unrest / d.tuning.revolt_share.max(0.01)).clamp(0.0, 1.0);
            let col = mix(UNREST_LOW, UNREST_HIGH, frac);
            let n = (64.0 * frac).ceil() as usize;
            for k in 0..n {
                let a = -std::f32::consts::FRAC_PI_2 - k as f32 / 64.0 * std::f32::consts::TAU;
                c.disc(at + Vec2::from_angle(a) * (r + 3.5), 1.3, col, 0.95);
            }
        }
        c.disc(at, r + 1.0, INK, 1.0);
        c.disc(at, r, [230, 220, 170], 1.0);
    }
    if let Some(p) = opts.planet.and_then(|i| d.planets.get(i)) {
        let at = view.to_screen(p.pos);
        let r = ((1.5 + 1.3 * p.value as f32) * d.scale().max(0.8) * z).max(2.5) + 9.0;
        c.circle(at, r, GOLD, 1.0);
        c.circle(at, r + 1.0, GOLD, 1.0);
    }
    // Destinations of the wholly selected fleets.
    let whole = d.whole_fleets(&opts.selected);
    for f in whole.iter().filter_map(|id| d.fleet(*id)) {
        if let Some(dest) = f.dest {
            let (a, b) = (view.to_screen(f.pos), view.to_screen(dest));
            c.dashed_line(a, b, 6.0, 5.0, GOLD, 0.9);
            c.circle(b, 5.0, GOLD, 1.0);
        }
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
        if whole.contains(&f.id) {
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
            // Selected ships get a gold outline.
            if f.ship_ids.get(k).is_some_and(|e| opts.selected.contains(e)) {
                c.polygon_fill(&pts(2.0), GOLD, 1.0);
            }
            c.polygon_fill(&pts(1.45), INK, 1.0);
            c.polygon_fill(&pts(1.0), col, 1.0);
        }
    }
    for (p, age) in &d.booms {
        let k = age / BOOM_SECS;
        c.circle(view.to_screen(*p), (3.0 + 9.0 * k) * (z * d.scale()).clamp(0.6, 2.0), HIT, (1.0 - k) * 0.9);
    }
    // The selected planet's people, in the top-left corner.
    if let Some(i) = opts.planet.filter(|i| *i < d.people.len()) {
        draw_people(&mut c, &d.people[i], d.planets[i].owner);
    }
    // The selection box being dragged.
    if let Some((a, b)) = opts.marquee {
        let (lo, hi) = (a.min(b), a.max(b));
        let corners = [lo, Vec2::new(hi.x, lo.y), hi, Vec2::new(lo.x, hi.y)];
        for k in 0..4 {
            c.line(corners[k], corners[(k + 1) % 4], 1.0, [120, 220, 140], 0.9);
        }
    }
    c
}

/// The status lines for the page.
pub fn status(d: &Dominion, selected: &[Entity], planet: Option<usize>) -> String {
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
    for (side, name) in SIDE_NAMES.iter().enumerate() {
        let o = Owner::Side(side);
        let net = d.net_income(o);
        let broke = if d.credits_of(o) <= 0.0 { " · OUT OF CREDITS, can't fire" } else { "" };
        let gone = if d.deserted[side] > 0 { format!(" · {} ships deserted", d.deserted[side]) } else { String::new() };
        let gone = if d.revolts[side] > 0 { format!("{gone} · {} planets lost to revolts", d.revolts[side]) } else { gone };
        s.push_str(&format!(
            "\n{}: {:.0} credits, {:+.1}/s (planets {:+.1}, upkeep −{:.1}){broke}{gone}",
            name,
            d.credits_of(o),
            net,
            d.planet_income(o),
            d.ships_of(o) as f32 * d.tuning.upkeep,
        ));
    }
    if let Some(p) = planet.and_then(|i| d.planets.get(i)) {
        let i = planet.unwrap();
        let who = match p.owner {
            Owner::Side(s) => SIDE_NAMES[s].to_string(),
            Owner::Independent(_) => "Independent".into(),
        };
        s.push_str(&format!(
            "\nPlanet: {who}, size {}, tax {:.0}% ([ ] change it), earns {:+.2}/s · legitimacy {:.2}",
            p.value,
            p.tax * 100.0,
            d.planet_earning(p),
            p.legitimacy()
        ));
        if p.occupation > 0.0 {
            s.push_str(&format!(" (occupied −{:.2})", p.occupation));
        }
        if matches!(p.owner, Owner::Side(_)) {
            let people = &d.people[i];
            s.push_str(&format!(
                " · rebels {:.0}% now, unrest {:.0}% (revolt at {:.0}%) · jailed {:.0}% · {} cops{}",
                people.active_share() * 100.0,
                p.unrest * 100.0,
                d.tuning.revolt_share * 100.0,
                people.jailed_share() * 100.0,
                p.cops,
                if p.war > 0.05 { " · war hardship" } else { "" }
            ));
        } else {
            s.push_str(" · its own people: no unrest.");
        }
    }
    let whole = d.whole_fleets(selected);
    let mut from: Vec<u32> = selected.iter().filter_map(|e| d.fleet_of_ship(*e)).map(|f| f.id).collect();
    from.sort();
    from.dedup();
    match (&from[..], &whole[..]) {
        ([], _) => s.push_str(
            "\nLeft-drag a box around ships (or click a fleet) to select them, then right-click the map: \
             ships from several fleets merge into one, part of a fleet splits off.",
        ),
        ([one], [w]) if one == w => {
            let i = d.fleet_index(*one).unwrap();
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
                s.push_str(" Independents only defend.");
            } else {
                s.push_str(" Right-click the map to move it · keys 1–6 set its formation.");
            }
        }
        _ => {
            let n = selected.iter().filter(|e| d.fleet_of_ship(**e).is_some()).count();
            let what = if from.len() == 1 { "split them off as a new fleet" } else { "form them into one fleet" };
            s.push_str(&format!("\nSelected: {n} ships from {} fleets. Right-click the map to {what} and send it there.", from.len()));
        }
    }
    s
}
