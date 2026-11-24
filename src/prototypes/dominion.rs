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
//! One Voronoi diagram: planets and fleets are its sites (fleets move,
//! weighted by ships, so a big fleet claims more space). Highways are the
//! Delaunay edges between planets; fleets only travel along them.
//!
//! Supply: three resources, ammo, food and parts, with no global purse.
//! Each point of a planet's size produces one kind (its icons) into the
//! planet's stock, at income × tax. Ships carry their own stores, split
//! evenly within a fleet: no ammo, no shooting; no food, ships desert;
//! damaged ships without parts may break up; every shortage shakes morale.
//! Fleets at a friendly planet draw from its stock; elsewhere supply ships
//! bring it: planets load them for the nearest fleet in need, they travel
//! the highways, join the fleet, trail it at a distance, empty as the
//! fleet uses their cargo, and can be shot. A planet builds ships out of
//! the parts in its stock.
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
use super::captains::{Captain, Skill, draw_name, draw_skills};
use super::highways::Highways;
use crate::sim::components::{Division, Flagship, Heading, InDivision, Morale, Order, Pos, Ship, Stores, Supply, Weapon};
pub use crate::sim::formation::Formation;
use crate::sim::formation::SHIP_DART;
use crate::voronoi::Vec2;
use crate::voronoi::grid::voronoi_grid;
use crate::voronoi::influence::weighted_scores;
use crate::voronoi::labs::cell_color;
use crate::voronoi::metric::Metric;
use crate::voronoi::raster::{Canvas, Rgb};
use crate::voronoi::rng::ParkMiller;
use crate::voronoi::sites::VoronoiConfig;

pub const MAX_FLEETS_PER_SIDE: usize = 8;
/// Escorts each starting flagship has (it starts alone).
pub const START_SHIPS: u32 = 0;
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
/// Resources: indices into a stock.
pub const AMMO: usize = 0;
pub const FOOD: usize = 1;
pub const PARTS: usize = 2;
pub const RESOURCE_NAMES: [&str; 3] = ["ammo", "food", "parts"];
/// A resource stock: ammo, food, parts.
pub type Stock = [f32; 3];
/// What one combat ship carries at most.
pub const SHIP_CAP: Stock = [10.0, 3.0, 2.0];
/// The largest planet size (and every starting fleet's ship count).
pub const MAX_PLANET_SIZE: u32 = 3;
/// Each kind a planet starts with: home planets, then the rest.
pub const HOME_STOCK: f32 = 30.0;
pub const PLANET_STOCK: f32 = 10.0;
/// A fleet needs a kind when its ships hold less than this share of capacity.
pub const NEED_SHARE: f32 = 0.5;
/// Seconds between convoy re-routes.
pub const CONVOY_SECS: f32 = 1.0;
/// Seconds between placing the supply ships behind their fleets.
pub const SUPPLY_SECS: f32 = 0.25;
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
    /// Voronoi weight each ship adds to its fleet's cell.
    pub weight_per_ship: f32,
    /// Voronoi weight of a planet's cell per point of size.
    pub planet_weight: f32,
    /// Seconds to build one ship at a value-1 planet (value 3 is 3× faster).
    pub secs_per_ship: f32,
    /// Fleet speed, pixels a second on a 720-wide map.
    pub speed: f32,
    /// Multiplies the battle's rate of fire, so map fights take seconds,
    /// not minutes; the rules' proportions are unchanged.
    pub combat_speed: f32,
    /// Units a second each point of a planet's size produces, of which its
    /// owner takes the planet's tax.
    pub income: f32,
    /// Parts to build one ship.
    pub ship_cost: f32,
    /// Ammo each shot uses.
    pub ammo_per_shot: f32,
    /// Food each ship eats every maintenance phase.
    pub food_per_phase: f32,
    /// Hull points one part repairs.
    pub hull_per_part: f32,
    /// Units a supply ship carries.
    pub supply_load: f32,
    /// Chance a ship without food deserts, each maintenance phase.
    pub desert_chance: f32,
    /// Chance (× its damage) a damaged ship without parts breaks up, each phase.
    pub wreck_chance: f32,
    /// Shock added to a fleet per maintenance phase, × its shortage (0..1).
    pub shortage_shock: f32,
    /// How many ships a flagship counts as, for everything (weight, cops,
    /// stores, food, hull, rate of fire).
    pub flagship_ships: f32,
    /// Escorts a captain commands without penalty.
    pub command: f32,
    /// Shock per maintenance phase per 100% over command capacity.
    pub over_shock: f32,
    /// Shock when a fleet loses its flagship.
    pub flag_shock: f32,
    /// Chance a captain dies with their flagship.
    pub captain_death: f32,
    /// Seconds until a surviving captain has a new flagship at home.
    pub captain_return: f32,
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
            planet_weight: 0.5,
            secs_per_ship: 15.0,
            speed: 30.0,
            combat_speed: 3.0,
            income: 4.0,
            ship_cost: 5.0,
            ammo_per_shot: 0.1,
            food_per_phase: 0.25,
            hull_per_part: 2.5,
            supply_load: 20.0,
            desert_chance: 0.25,
            wreck_chance: 0.3,
            shortage_shock: 0.4,
            flagship_ships: 3.0,
            command: 10.0,
            over_shock: 0.3,
            flag_shock: 0.4,
            captain_death: 0.5,
            captain_return: 60.0,
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
    /// Size, 1 (small) to 3 (large): build rate, output and population.
    pub value: u32,
    /// The kind each point of size produces (the first `value` count).
    pub produces: [usize; 3],
    /// Resources on hand.
    pub stock: Stock,
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
    /// The side whose home this is: loyal to it, its people never rebel
    /// while it holds the planet.
    pub home_of: Option<usize>,
}

impl Planet {
    /// Its own side's home, held by that side: its people stay loyal.
    pub fn loyal(&self) -> bool {
        matches!((self.owner, self.home_of), (Owner::Side(s), Some(h)) if s == h)
    }

    /// Run by Blue or Red and not loyal: its people can rebel.
    pub fn restless(&self) -> bool {
        matches!(self.owner, Owner::Side(_)) && !self.loyal()
    }

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
    /// Highway waypoints still ahead (the last one is `dest`).
    pub route: Vec<Vec2>,
    /// On its way to join this fleet (by id): a supply convoy, or a new
    /// ship reinforcing it.
    pub joining: Option<u32>,
    /// The captain whose flagship leads it (by id). Only these move.
    pub captain: Option<u32>,
    /// Supply ships only: held up at the edge of the other side's cells.
    pub waiting: bool,
    /// Combat ships.
    pub ships: u32,
    /// Supply ships.
    pub supply: u32,
    pub formation: Formation,
    pub morale: Morale,
    /// The planet it garrisons (planets build into it).
    pub home: Option<usize>,
    /// Each ship's position (map pixels) and heading.
    pub ship_px: Vec<(Vec2, f32)>,
    /// Each ship's entity, in the same order as `ship_px`.
    pub ship_ids: Vec<Entity>,
    /// Whether each ship (same order) is a supply ship.
    pub is_supply: Vec<bool>,
    /// Fired or was fired on in the last second or so.
    pub engaged: bool,
}

impl Fleet {
    /// Combat ships, the flagship counting as `flagship_ships`.
    pub fn strength(&self, t: &Tuning) -> f32 {
        self.ships as f32 + if self.captain.is_some() { t.flagship_ships - 1.0 } else { 0.0 }
    }

    pub fn weight(&self, t: &Tuning) -> f32 {
        self.strength(t) * t.weight_per_ship
    }

    /// Has combat ships (a convoy of supply ships alone doesn't).
    pub fn armed(&self) -> bool {
        self.ships > 0
    }

    /// Blue and Red fleets, unless routed (with a captain or not).
    pub fn controllable(&self) -> bool {
        self.selectable()
    }

    /// Blue and Red ships the player can pick (garrisons included).
    pub fn selectable(&self) -> bool {
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
    /// The relaxed sites planets were picked from.
    pub sites: Vec<Vec2>,
    pub planets: Vec<Planet>,
    /// Delaunay edges between planets: the only ways fleets travel.
    pub highways: Highways,
    pub fleets: Vec<Fleet>,
    /// The ECS world every ship lives in.
    pub world: World,
    pub shots: Vec<Shot>,
    /// Explosions: where (pixels) and age.
    pub booms: Vec<(Vec2, f32)>,
    /// Seconds since the last maintenance phase.
    pub maintenance_timer: f32,
    /// Ships lost to shortages (deserted or broken up), per side.
    pub deserted: [u32; 2],
    /// Supply ships loaded, per side.
    pub convoys_sent: [u32; 2],
    rng: ParkMiller,
    convoy_timer: f32,
    supply_timer: f32,
    /// Every captain, dead or alive.
    pub captains: Vec<Captain>,
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
    /// Red each own the planet nearest their edge (size 3, producing all
    /// three kinds) and start with `fleets` fleets of `START_SHIPS` ships on
    /// its highways, each with a full supply ship; every planet starts with
    /// a garrison of its size.
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
            let produces = std::array::from_fn(|_| ((rng.next_float() * 3.0) as usize).min(2));
            list.push(Planet {
                site,
                pos: sites[site],
                value,
                produces,
                stock: [PLANET_STOCK; 3],
                owner: Owner::Independent(list.len()),
                build: 0.0,
                hold: None,
                tax: DEFAULT_TAX,
                occupation: 0.0,
                war: 0.0,
                unrest: 0.0,
                cops: 0,
                home_of: None,
            });
        }
        let list_len = list.len();
        let highways = Highways::new(&list.iter().map(|p| p.pos).collect::<Vec<_>>());
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
            planets: list,
            highways,
            fleets: Vec::new(),
            world,
            shots: Vec::new(),
            booms: Vec::new(),
            maintenance_timer: 0.0,
            deserted: [0; 2],
            convoys_sent: [0; 2],
            rng: ParkMiller::new(cfg.seed as i64 * 7 + 3),
            convoy_timer: 0.0,
            supply_timer: 0.0,
            captains: Vec::new(),
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
        if d.planets.len() < 2 {
            return d;
        }
        // Home planets: the westernmost for Blue, the easternmost for Red.
        let west = (0..d.planets.len()).min_by(|a, b| d.planets[*a].pos.x.total_cmp(&d.planets[*b].pos.x)).unwrap();
        let east = (0..d.planets.len()).filter(|i| *i != west).max_by(|a, b| d.planets[*a].pos.x.total_cmp(&d.planets[*b].pos.x)).unwrap();
        let scale = map_scale(cfg);
        let mut names = ParkMiller::new(cfg.seed as i64 * 13 + 5);
        for (side, home) in [(0, west), (1, east)] {
            let skills = draw_skills(&mut names, fleets.clamp(1, MAX_FLEETS_PER_SIDE));
            let p = &mut d.planets[home];
            p.owner = Owner::Side(side);
            p.home_of = Some(side);
            p.value = MAX_PLANET_SIZE;
            p.produces = [AMMO, FOOD, PARTS];
            p.stock = [HOME_STOCK; 3];
            let at = p.pos;
            let n = fleets.clamp(1, MAX_FLEETS_PER_SIDE);
            // Lone flagships at the home planet, a little way out along its
            // highways (round the roads again if there are more of them).
            let mut roads: Vec<usize> = d.highways.neighbors[home].clone();
            if roads.is_empty() {
                roads.push(home);
            }
            for k in 0..n {
                let to = d.planets[roads[k % roads.len()]].pos;
                let dir = (to - at).normalize_or_zero();
                let out = (12.0 + 5.0 * (k / roads.len()) as f32) * scale;
                let pos = at + dir * out.min(at.distance(to) * 0.5);
                let id = d.spawn(Owner::Side(side), pos, dir.to_angle(), START_SHIPS, None);
                d.add_captain_named(id, skills[k], &mut names);
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
        ((self.police_strength(i, true) * self.tuning.cops_per_ship / p.value.max(1) as f32).round() as usize).min(MAX_COPS)
    }

    /// The owner's ships policing planet `i` (within the police radius),
    /// flagships counting as `flagship_ships`; with `governors`, ships of a
    /// Governor's fleet count double.
    pub fn police_strength(&self, i: usize, governors: bool) -> f32 {
        let p = &self.planets[i];
        let police = self.police_radius();
        self.fleets
            .iter()
            .filter(|f| f.owner == p.owner && f.morale != Morale::Routed && f.pos.distance(p.pos) <= police)
            .map(|f| f.strength(&self.tuning) * if governors && self.has_skill(f, Skill::Governor) { 2.0 } else { 1.0 })
            .sum()
    }

    /// `police_strength` without skills, in whole ships (rounded down).
    pub fn police_ships(&self, i: usize) -> u32 {
        (self.police_strength(i, false) + 1e-3) as u32
    }

    /// Ships planet `i` needs nearby to keep its people calm: for Blue's
    /// and Red's planets at their legitimacy and war now, for independents
    /// what a conqueror would need (occupation included). `None`: no
    /// garrison is enough. Loyal home planets need none.
    pub fn ships_needed(&self, i: usize) -> Option<u32> {
        let p = &self.planets[i];
        if p.loyal() {
            return Some(0);
        }
        let legit = match p.owner {
            Owner::Side(_) => p.legitimacy(),
            Owner::Independent(_) => (1.0 - p.tax - OCCUPATION_PENALTY).clamp(0.0, 1.0),
        };
        let cops = super::rebellion::calm_cops(legit, p.war * self.tuning.war_hardship)?;
        if cops <= 0.0 {
            return Some(0);
        }
        if self.tuning.cops_per_ship <= 0.0 {
            return None;
        }
        Some((cops * p.value.max(1) as f32 / self.tuning.cops_per_ship).ceil() as u32)
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

    /// Removes one fleet and its ships.
    pub fn clear_fleet(&mut self, id: u32) {
        let Some(f) = self.fleet(id) else { return };
        let (e, ships) = (f.division, f.ship_ids.clone());
        crate::sim::despawn_ships(&mut self.world, e, &ships);
        self.sync();
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
        self.equip(division, SHIP_CAP);
        self.fleets.push(Fleet {
            id,
            division,
            owner,
            pos,
            facing,
            dest: None,
            route: Vec::new(),
            joining: None,
            captain: None,
            waiting: false,
            ships,
            supply: 0,
            formation: Formation::Line,
            morale: Morale::Steady,
            home,
            ship_px: Vec::new(),
            ship_ids: Vec::new(),
            is_supply: Vec::new(),
            engaged: false,
        });
        id
    }

    /// Gives the division's combat ships that have no stores yet `stores`.
    fn equip(&mut self, division: Entity, stores: Stock) {
        let bare: Vec<Entity> = self
            .world
            .query_filtered::<(Entity, &InDivision), (bevy::prelude::Without<Stores>, bevy::prelude::Without<Supply>)>()
            .iter(&self.world)
            .filter(|(_, d)| d.0 == division)
            .map(|(e, _)| e)
            .collect();
        for e in bare {
            self.world.entity_mut(e).insert(Stores(stores));
        }
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

    /// Units a second a planet adds to its stock, by kind: its tax share
    /// of what each point of size produces, less what strikes cost.
    pub fn planet_output(&self, p: &Planet) -> Stock {
        let mut out = [0.0; 3];
        for &k in p.produces.iter().take(p.value as usize) {
            out[k] += self.tuning.income * p.tax * p.working();
        }
        out
    }

    /// Units a second an owner's planets produce, by kind.
    pub fn planet_income(&self, owner: Owner) -> Stock {
        let mut out = [0.0; 3];
        for p in self.planets.iter().filter(|p| p.owner == owner) {
            for (o, v) in out.iter_mut().zip(self.planet_output(p)) {
                *o += v;
            }
        }
        out
    }

    /// Everything an owner's planets hold, by kind.
    pub fn stock_of(&self, owner: Owner) -> Stock {
        let mut out = [0.0; 3];
        for p in self.planets.iter().filter(|p| p.owner == owner) {
            for (o, v) in out.iter_mut().zip(p.stock) {
                *o += v;
            }
        }
        out
    }

    /// What a fleet's combat ships carry in all, and the cargo of its
    /// supply ships.
    pub fn fleet_stores(&self, f: &Fleet) -> (Stock, Stock) {
        let (mut stores, mut cargo) = ([0.0; 3], [0.0; 3]);
        for e in &f.ship_ids {
            if let Some(s) = self.world.get::<Stores>(*e) {
                (0..3).for_each(|k| stores[k] += s.0[k]);
            }
            if let Some(s) = self.world.get::<Supply>(*e) {
                (0..3).for_each(|k| cargo[k] += s.0[k]);
            }
        }
        (stores, cargo)
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

    /// Orders a controllable fleet to the nearest point on a highway to
    /// `to`, by the shortest way along the highways; it stops garrisoning
    /// (and a convoy stops being one).
    pub fn order_move(&mut self, id: u32, to: Vec2) -> bool {
        let Some(i) = self.fleet_index(id).filter(|i| self.fleets[*i].controllable()) else { return false };
        self.fleets[i].home = None;
        self.fleets[i].joining = None;
        self.fleets[i].waiting = false;
        self.route_to(i, to)
    }

    /// Plans fleet `i`'s highway route to `to` and starts the first leg.
    fn route_to(&mut self, i: usize, to: Vec2) -> bool {
        let route = self.highways.route(self.fleets[i].pos, to);
        self.fleets[i].dest = route.last().copied();
        self.fleets[i].route = route;
        self.next_leg(i);
        self.fleets[i].dest.is_some()
    }

    /// Sends fleet `i` toward its next waypoint, facing along the leg.
    fn next_leg(&mut self, i: usize) {
        let unit = self.unit();
        let f = &self.fleets[i];
        let (e, from) = (f.division, f.pos);
        let Some(&next) = f.route.first() else { return };
        let facing = if next.distance(from) > 0.5 { (next - from).to_angle() } else { f.facing };
        if let Some(mut d) = self.world.get_mut::<Division>(e) {
            d.order = Order::MoveTo { to: next / unit, facing };
        }
    }

    /// A new fleet of one supply ship carrying `cargo` at `pos`; with a
    /// target, a convoy bound for that fleet.
    pub fn add_convoy(&mut self, owner: Owner, pos: Vec2, cargo: Stock, target: Option<u32>) -> u32 {
        let id = self.spawn(owner, pos, 0.0, 0, None);
        let fi = self.fleets.len() - 1;
        let at = pos / self.unit();
        crate::sim::add_supply(&mut self.world, self.fleets[fi].division, at, cargo);
        self.fleets[fi].joining = target;
        self.fleets[fi].supply = 1;
        if let Some(to) = target.and_then(|t| self.fleet(t)).map(|f| f.pos) {
            self.route_to(fi, to);
        }
        self.sync();
        id
    }

    /// A supply ship carrying `cargo` joins fleet `id`.
    pub fn add_supply_ship(&mut self, id: u32, cargo: Stock) -> bool {
        let Some(f) = self.fleet(id) else { return false };
        let (e, at) = (f.division, f.pos / self.unit());
        let ok = crate::sim::add_supply(&mut self.world, e, at, cargo).is_some();
        self.sync();
        ok
    }

    /// Highway distance from `a` to `b`.
    pub fn road_distance(&self, a: Vec2, b: Vec2) -> f32 {
        self.highways.distance(a, b)
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
        for f in self.fleets.iter().filter(|f| f.selectable()) {
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

    /// Right-click with `ships` selected: they go to `to` along the
    /// highways. A selected captain's fleet goes (its unselected combat
    /// ships split off and stay; its supply ships always go with it), and
    /// selected ships without a captain near it join it on the way.
    /// Every other selected group goes on its own: a whole fleet as it is,
    /// part of one split off first. Captains never merge. Only one
    /// owner's ships count. Returns the strongest fleet that moves.
    pub fn form_fleet(&mut self, ships: &[Entity], to: Vec2) -> Option<u32> {
        let mut picked: Vec<(Entity, Owner, u32)> = ships
            .iter()
            .filter_map(|e| self.fleet_of_ship(*e).filter(|f| f.selectable()).map(|f| (*e, f.owner, f.id)))
            .collect();
        let owner = picked.first()?.1;
        picked.retain(|p| p.1 == owner);
        let mut sources: Vec<u32> = picked.iter().map(|p| p.2).collect();
        sources.sort();
        sources.dedup();
        let flag_of = |d: &Self, id: u32| d.fleet(id).and_then(|f| f.captain).and_then(|c| d.captain(c)).and_then(|c| c.flagship);
        // Captains first.
        let mut captains: Vec<u32> = Vec::new();
        let mut loose: Vec<Vec<Entity>> = Vec::new();
        for &src in &sources {
            let mine: Vec<Entity> = picked.iter().filter(|p| p.2 == src).map(|p| p.0).collect();
            match flag_of(self, src) {
                Some(flag) if mine.contains(&flag) => {
                    let f = self.fleet(src)?;
                    let rest: Vec<Entity> =
                        f.ship_ids.iter().zip(&f.is_supply).filter(|(e, s)| !**s && !mine.contains(e)).map(|(e, _)| *e).collect();
                    self.split_off(&rest);
                    captains.push(src);
                }
                _ => loose.push(mine),
            }
        }
        // Ships without a captain: into the nearest moving captain within
        // reach, or on their own.
        let reach = self.police_radius();
        let mut movers = captains.clone();
        for mine in loose {
            let Some(f) = mine.first().and_then(|e| self.fleet_of_ship(*e)) else { continue };
            let (src, at, whole) = (f.id, f.pos, f.ship_ids.len() == mine.len());
            let best = captains
                .iter()
                .filter_map(|id| self.fleet(*id).map(|g| (g.division, g.pos.distance(at))))
                .filter(|(_, dist)| *dist <= reach)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((division, _)) = best {
                crate::sim::transfer_ships(&mut self.world, &mine, division);
                self.sync();
            } else if whole {
                movers.push(src);
            } else {
                self.split_off(&mine);
                movers.push(self.fleets.last()?.id);
            }
        }
        self.sync();
        for &id in &movers {
            self.order_move(id, to);
        }
        let t = self.tuning;
        movers.into_iter().filter_map(|id| self.fleet(id)).max_by(|a, b| a.strength(&t).total_cmp(&b.strength(&t))).map(|f| f.id)
    }

    /// The owner's captain nearest `pos` (by highway) whose escorts and
    /// the ships on their way to it are fewer than it commands.
    fn fleet_with_room(&self, owner: Owner, pos: Vec2) -> Option<u32> {
        self.fleets
            .iter()
            .filter(|f| f.owner == owner && f.captain.is_some() && f.morale != Morale::Routed)
            .filter(|f| {
                let coming: u32 =
                    self.fleets.iter().filter(|g| g.joining == Some(f.id) && g.armed()).map(|g| g.ships).sum();
                ((self.escorts(f) + coming) as f32) < self.capacity(f)
            })
            .map(|f| (f.id, self.road_distance(pos, f.pos)))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// These ships (all of one fleet) become a new fleet where they are,
    /// holding; nothing when empty.
    fn split_off(&mut self, ships: &[Entity]) {
        let Some(f) = ships.first().and_then(|e| self.fleet_of_ship(*e)) else { return };
        let (owner, facing, formation) = (f.owner, f.facing, f.formation);
        let unit = self.unit();
        let centre = self
            .world
            .query::<(Entity, &Pos)>()
            .iter(&self.world)
            .filter(|(e, _)| ships.contains(e))
            .map(|(_, p)| p.0 * unit)
            .sum::<Vec2>()
            / ships.len() as f32;
        self.spawn(owner, centre, facing, 0, None);
        let e = self.fleets.last().unwrap().division;
        crate::sim::set_formation(&mut self.world, e, formation);
        crate::sim::transfer_ships(&mut self.world, ships, e);
        self.sync();
    }

    pub fn captain(&self, id: u32) -> Option<&Captain> {
        self.captains.iter().find(|c| c.id == id)
    }

    /// The skill of a fleet's captain.
    pub fn skill(&self, f: &Fleet) -> Option<Skill> {
        f.captain.and_then(|c| self.captain(c)).map(|c| c.skill)
    }

    pub fn has_skill(&self, f: &Fleet, skill: Skill) -> bool {
        self.skill(f) == Some(skill)
    }

    /// Escorts a fleet's captain commands without penalty.
    pub fn capacity(&self, f: &Fleet) -> f32 {
        self.tuning.command * if self.has_skill(f, Skill::Commander) { 1.5 } else { 1.0 }
    }

    /// A fleet's combat ships besides its flagship.
    pub fn escorts(&self, f: &Fleet) -> u32 {
        f.ships.saturating_sub(u32::from(f.captain.is_some()))
    }

    /// A new captain with `skill` takes command of fleet `id` on a new
    /// flagship; returns the captain's id.
    pub fn add_captain(&mut self, id: u32, skill: Skill) -> Option<u32> {
        let mut names = ParkMiller::new(self.cfg.seed as i64 * 17 + self.captains.len() as i64 * 7 + 1);
        self.add_captain_named(id, skill, &mut names)
    }

    fn add_captain_named(&mut self, id: u32, skill: Skill, names: &mut ParkMiller) -> Option<u32> {
        let side = match self.fleet(id)?.owner {
            Owner::Side(s) => s,
            Owner::Independent(_) => return None,
        };
        let taken: Vec<String> = self.captains.iter().map(|c| c.name.clone()).collect();
        let cid = self.captains.len() as u32 + 1;
        self.captains.push(Captain { id: cid, name: draw_name(names, &taken), side, skill, flagship: None, alive: true, back_in: 0.0 });
        let fi = self.fleet_index(id)?;
        self.make_flagship(fi, cid);
        Some(cid)
    }

    /// A new flagship for captain `cid` in fleet `fi`.
    fn make_flagship(&mut self, fi: usize, cid: u32) {
        let unit = self.unit();
        let (division, at) = (self.fleets[fi].division, self.fleets[fi].pos / unit);
        let Some(e) = crate::sim::add_ship(&mut self.world, division, at) else { return };
        let k = self.tuning.flagship_ships.max(1.0);
        let hull = self.battle.hull * k;
        self.world.entity_mut(e).insert((Flagship(cid), Ship { hull, max_hull: hull }, Stores(SHIP_CAP.map(|v| v * k))));
        if let Some(c) = self.captains.iter_mut().find(|c| c.id == cid) {
            c.flagship = Some(e);
            c.back_in = 0.0;
        }
        self.fleets[fi].captain = Some(cid);
        self.sync();
    }

    /// Captains follow their flagships between fleets; a captain whose
    /// flagship is gone loses the fleet (shaken, it becomes a garrison
    /// group), and dies or gets a new flagship later.
    fn track_captains(&mut self) {
        let t = self.tuning;
        for ci in 0..self.captains.len() {
            let c = &self.captains[ci];
            let (cid, flag) = (c.id, c.flagship);
            if !c.alive {
                continue;
            }
            let holder = flag.and_then(|e| self.fleets.iter().position(|f| f.ship_ids.contains(&e)));
            let had: Vec<usize> = (0..self.fleets.len()).filter(|i| self.fleets[*i].captain == Some(cid) && Some(*i) != holder).collect();
            for &i in &had {
                let f = &mut self.fleets[i];
                f.captain = None;
                f.route.clear();
                f.dest = None;
                let (e, inspiring) = (f.division, self.captains[ci].skill == Skill::Inspiring);
                if let Some(mut d) = self.world.get_mut::<Division>(e) {
                    d.flagship = false;
                    d.pace = 1.0;
                    if d.morale != Morale::Routed {
                        d.order = Order::Hold;
                    }
                    if holder.is_none() {
                        d.shock += t.flag_shock * if inspiring { 0.5 } else { 1.0 };
                    }
                }
            }
            if let Some(h) = holder {
                self.fleets[h].captain = Some(cid);
                if let Some(mut d) = self.world.get_mut::<Division>(self.fleets[h].division) {
                    d.flagship = true;
                }
            } else if flag.is_some() {
                // The flagship is gone.
                let survivor = self.captains[ci].skill == Skill::Survivor;
                let death = t.captain_death * if survivor { 0.5 } else { 1.0 };
                let c = &mut self.captains[ci];
                c.flagship = None;
                if self.rng.next_float() < death {
                    c.alive = false;
                } else {
                    c.back_in = t.captain_return;
                }
            }
        }
    }

    /// Surviving captains without a flagship get a new one at their home
    /// planet (or their side's nearest planet) when their time comes,
    /// taking command of the ships waiting there.
    fn return_captains(&mut self, dt: f32) {
        for ci in 0..self.captains.len() {
            let c = &self.captains[ci];
            if !c.alive || c.flagship.is_some() {
                continue;
            }
            let (cid, side) = (c.id, c.side);
            self.captains[ci].back_in -= dt;
            if self.captains[ci].back_in > 0.0 {
                continue;
            }
            let owner = Owner::Side(side);
            let home = (0..self.planets.len())
                .find(|i| self.planets[*i].owner == owner && self.planets[*i].home_of == Some(side))
                .or_else(|| (0..self.planets.len()).find(|i| self.planets[*i].owner == owner));
            let Some(pi) = home else { continue };
            let pos = self.planets[pi].pos;
            let r = self.capture_radius();
            let waiting = (0..self.fleets.len())
                .filter(|i| {
                    let f = &self.fleets[*i];
                    f.owner == owner && f.captain.is_none() && f.armed() && f.route.is_empty() && f.pos.distance(pos) <= r
                })
                .max_by_key(|i| self.fleets[*i].ships);
            let fi = match waiting {
                Some(fi) => fi,
                None => {
                    self.spawn(owner, pos, 0.0, 0, None);
                    self.fleets.len() - 1
                }
            };
            self.make_flagship(fi, cid);
        }
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
            self.equip(e, SHIP_CAP);
        } else if n < have {
            // Combat ships only, the last ones first.
            let cut: Vec<Entity> =
                f.ship_ids.iter().zip(&f.is_supply).filter(|(_, s)| !**s).map(|(e, _)| *e).rev().take((have - n) as usize).collect();
            crate::sim::despawn_ships(&mut self.world, e, &cut);
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
                SimEvent::Shot { from, to, side, hit, shooter } => {
                    self.spend_ammo(shooter);
                    self.shots.push((from * unit, to * unit, side, hit, 0.0));
                }
                SimEvent::Destroyed { pos, .. } => self.booms.push((pos * unit, 0.0)),
                _ => {}
            }
        }
        self.sync();
        self.return_captains(DT);
        self.capture(DT);
        self.unrest(DT);
        self.run_convoys(DT);
        self.hold_convoys();
        self.supply_timer += DT;
        if self.supply_timer >= SUPPLY_SECS - 1e-4 {
            self.supply_timer = 0.0;
            self.position_supply();
        }
        self.maintenance_timer += DT;
        if self.maintenance_timer >= MAINTENANCE_SECS - 1e-4 {
            self.maintenance_timer = 0.0;
            self.maintain();
        }
    }

    /// Every tick each planet adds its output to its stock.
    fn earn(&mut self, dt: f32) {
        for i in 0..self.planets.len() {
            let out = self.planet_output(&self.planets[i]);
            for (s, o) in self.planets[i].stock.iter_mut().zip(out) {
                *s += dt * o;
            }
        }
    }

    /// The shooter spends a round (a Sharpshooter's ships half). A
    /// flagship reloads `flagship_ships` times as fast, and a Gunner's
    /// ships 25% faster.
    fn spend_ammo(&mut self, shooter: Entity) {
        let skill = self
            .world
            .get::<InDivision>(shooter)
            .and_then(|d| self.fleets.iter().find(|f| f.division == d.0))
            .and_then(|f| self.skill(f));
        let per_shot = self.tuning.ammo_per_shot * if skill == Some(Skill::Sharpshooter) { 0.5 } else { 1.0 };
        if let Some(mut s) = self.world.get_mut::<Stores>(shooter) {
            s.0[AMMO] = (s.0[AMMO] - per_shot).max(0.0);
        }
        let mut faster = if skill == Some(Skill::Gunner) { 1.25 } else { 1.0 };
        if self.world.get::<Flagship>(shooter).is_some() {
            faster *= self.tuning.flagship_ships.max(1.0);
        }
        if faster != 1.0
            && let Some(mut w) = self.world.get_mut::<Weapon>(shooter)
        {
            w.cooldown /= faster;
        }
    }

    /// Ships out of ammo can't fire: their guns stay unloaded.
    fn hold_fire(&mut self) {
        let per_shot = self.tuning.ammo_per_shot;
        for (s, mut w) in self.world.query::<(&Stores, &mut Weapon)>().iter_mut(&mut self.world) {
            if s.0[AMMO] < per_shot {
                w.cooldown = w.cooldown.max(2.0 * DT);
            }
        }
    }

    /// The owned planet within the capture radius of `pos`, if any.
    fn friendly_planet(&self, owner: Owner, pos: Vec2) -> Option<usize> {
        let r = self.capture_radius();
        (0..self.planets.len()).find(|i| self.planets[*i].owner == owner && self.planets[*i].pos.distance(pos) <= r)
    }

    /// The maintenance phase, fleet by fleet:
    /// 1. A fleet at one of its owner's planets draws from the planet's
    ///    stock; then its supply ships unload into its ships (emptied ones
    ///    are used up); then its ships' stores are split evenly.
    /// 2. Every ship eats; with no food, ships may desert.
    /// 3. Out of combat, parts repair damaged ships; damaged ships with no
    ///    parts may break up.
    /// 4. The shortage (ships lacking ammo, food or parts) shakes morale.
    ///
    /// Then planets load supply ships for fleets in need.
    fn maintain(&mut self) {
        let t = self.tuning;
        let mut lost: Vec<(Entity, Vec<Entity>, Vec<Vec2>)> = Vec::new();
        for fi in 0..self.fleets.len() {
            let f = &self.fleets[fi];
            let (owner, division, engaged) = (f.owner, f.division, f.engaged);
            let skill = self.skill(f);
            let flag_k = t.flagship_ships.max(1.0);
            // (ship, stores, hull, max hull, where, share: a flagship's is
            // `flagship_ships`)
            let mut combat: Vec<(Entity, Stock, f32, f32, Vec2, f32)> = Vec::new();
            let mut supply: Vec<(Entity, Stock)> = Vec::new();
            for (k, e) in f.ship_ids.iter().enumerate() {
                if let Some(c) = self.world.get::<Supply>(*e) {
                    supply.push((*e, c.0));
                } else if let (Some(st), Some(sh)) = (self.world.get::<Stores>(*e), self.world.get::<Ship>(*e)) {
                    let share = if self.world.get::<Flagship>(*e).is_some() { flag_k } else { 1.0 };
                    combat.push((*e, st.0, sh.hull, sh.max_hull, f.ship_px[k].0, share));
                }
            }
            let n = combat.len();
            if n == 0 {
                continue;
            }
            let shares: f32 = combat.iter().map(|c| c.5).sum();
            let mut pool: Stock = [0.0; 3];
            for c in &combat {
                (0..3).for_each(|k| pool[k] += c.1[k]);
            }
            let cap: Stock = std::array::from_fn(|k| SHIP_CAP[k] * shares);
            // Over command capacity: shock, and the fleet slows.
            let over = if f.captain.is_some() { (self.escorts(f) as f32 / self.capacity(f).max(1.0) - 1.0).max(0.0) } else { 0.0 };
            let calm = if skill == Some(Skill::Inspiring) { 0.5 } else { 1.0 };
            let pace = if skill == Some(Skill::Navigator) { 1.25 } else { 1.0 } / (1.0 + over);
            if let Some(mut d) = self.world.get_mut::<Division>(division) {
                d.pace = pace;
                d.shock += over * t.over_shock * calm;
            }
            // At home: the planet's stock.
            if let Some(pi) = self.friendly_planet(owner, f.pos) {
                let stock = &mut self.planets[pi].stock;
                for k in 0..3 {
                    let take = (cap[k] - pool[k]).max(0.0).min(stock[k]);
                    stock[k] -= take;
                    pool[k] += take;
                }
            }
            // The supply train.
            let mut empty: Vec<Entity> = Vec::new();
            for (e, cargo) in &mut supply {
                for k in 0..3 {
                    let take = (cap[k] - pool[k]).max(0.0).min(cargo[k]);
                    cargo[k] -= take;
                    pool[k] += take;
                }
                if cargo.iter().sum::<f32>() < 0.01 {
                    empty.push(*e);
                } else if let Some(mut c) = self.world.get_mut::<Supply>(*e) {
                    c.0 = *cargo;
                }
            }
            // Eat.
            let need = shares * t.food_per_phase * if skill == Some(Skill::Quartermaster) { 0.7 } else { 1.0 };
            let hungry = if need > 0.0 { (1.0 - pool[FOOD] / need).clamp(0.0, 1.0) } else { 0.0 };
            pool[FOOD] = (pool[FOOD] - need).max(0.0);
            // Repair out of combat, as far as parts go.
            let missing: f32 = combat.iter().map(|c| (c.3 - c.2).max(0.0)).sum();
            let mut fixed = 0.0;
            let per_part = t.hull_per_part * if skill == Some(Skill::Engineer) { 2.0 } else { 1.0 };
            if !engaged && missing > 0.0 && per_part > 0.0 {
                fixed = missing.min(pool[PARTS] * per_part);
                pool[PARTS] -= fixed / per_part;
            }
            let share = if missing > 0.0 { fixed / missing } else { 0.0 };
            let no_parts = pool[PARTS] < 0.01;
            let each: Stock = std::array::from_fn(|k| pool[k] / shares);
            let desert = t.desert_chance * if skill == Some(Skill::Quartermaster) { 0.5 } else { 1.0 };
            let wreck = t.wreck_chance * if skill == Some(Skill::Engineer) { 0.5 } else { 1.0 };
            let mut gone: Vec<Entity> = Vec::new();
            let mut booms: Vec<Vec2> = Vec::new();
            let mut damaged_short = 0;
            for (e, _, hull, max, pos, ship_share) in &combat {
                let hull = (*hull + (*max - *hull).max(0.0) * share).min(*max);
                if let Some(mut sh) = self.world.get_mut::<Ship>(*e) {
                    sh.hull = hull;
                }
                if let Some(mut st) = self.world.get_mut::<Stores>(*e) {
                    st.0 = each.map(|v| v * ship_share);
                }
                // The flagship never deserts nor breaks up for want of
                // supplies: its captain sees to that.
                let flagship = *ship_share > 1.0 || self.world.get::<Flagship>(*e).is_some();
                if hungry > 0.0 && !flagship && self.rng.next_float() < hungry * desert {
                    gone.push(*e);
                    continue;
                }
                if hull < *max - 1e-3 && no_parts {
                    damaged_short += 1;
                    if !flagship && self.rng.next_float() < (1.0 - hull / max) * wreck {
                        gone.push(*e);
                        booms.push(*pos);
                    }
                }
            }
            // Shortage: the share of ships lacking each kind, averaged.
            let no_ammo = if each[AMMO] < t.ammo_per_shot { 1.0 } else { 0.0 };
            let shortage = (no_ammo + hungry + damaged_short as f32 / n as f32) / 3.0;
            if shortage > 0.0
                && let Some(mut d) = self.world.get_mut::<Division>(division)
            {
                d.shock += shortage * t.shortage_shock * calm;
            }
            if let Owner::Side(s) = owner {
                self.deserted[s] += gone.len() as u32;
            }
            gone.extend(empty);
            if !gone.is_empty() {
                lost.push((division, gone, booms));
            }
        }
        for (division, gone, booms) in lost {
            crate::sim::despawn_ships(&mut self.world, division, &gone);
            self.booms.extend(booms.into_iter().map(|p| (p, 0.0)));
        }
        self.sync();
        self.load_convoys();
    }

    /// Supply ships carry one kind each. For each kind, a Blue or Red
    /// planet holding at least half a load sends one to the nearest (by
    /// highway) of its owner's fleets short of that kind (and not resting
    /// at a planet that holds it), counting what is already on its way.
    fn load_convoys(&mut self) {
        let t = self.tuning;
        if t.supply_load <= 0.0 {
            return;
        }
        // What each fleet will hold once the convoys bound for it arrive.
        let mut incoming: std::collections::HashMap<u32, Stock> = std::collections::HashMap::new();
        for f in self.fleets.iter().filter(|f| f.joining.is_some() && !f.armed()) {
            let (_, cargo) = self.fleet_stores(f);
            let e = incoming.entry(f.joining.unwrap()).or_insert([0.0; 3]);
            (0..3).for_each(|k| e[k] += cargo[k]);
        }
        // Each planet, each kind: one supply ship of that kind for the
        // nearest fleet short of it, if the planet holds half a load.
        for pi in 0..self.planets.len() {
            for kind in 0..3 {
                let p = self.planets[pi];
                let Owner::Side(side) = p.owner else { break };
                if p.stock[kind] < t.supply_load * 0.5 {
                    continue;
                }
                let mut best: Option<(f32, u32, f32)> = None;
                for f in &self.fleets {
                    // A fleet at one of its planets draws that planet's stock:
                    // it needs a convoy only for what that planet lacks.
                    let at_stock = self.friendly_planet(f.owner, f.pos).map(|q| (q, self.planets[q].stock[kind]));
                    if f.owner != p.owner
                        || !f.armed()
                        || f.morale == Morale::Routed
                        || at_stock.is_some_and(|(q, held)| q == pi || held >= 1.0)
                    {
                        continue;
                    }
                    let (stores, cargo) = self.fleet_stores(f);
                    let inc = incoming.get(&f.id).map_or(0.0, |s| s[kind]);
                    let cap = SHIP_CAP[kind] * f.strength(&t);
                    let have = stores[kind] + cargo[kind] + inc;
                    if have >= cap * NEED_SHARE {
                        continue;
                    }
                    // A Logistician's fleet is served first (counts as half as far).
                    let dist = self.road_distance(p.pos, f.pos) * if self.has_skill(f, Skill::Logistician) { 0.5 } else { 1.0 };
                    if best.is_none_or(|b| dist < b.0) {
                        best = Some((dist, f.id, cap - have));
                    }
                }
                let Some((_, target, short)) = best else { continue };
                let load = t.supply_load * if self.fleet(target).is_some_and(|f| self.has_skill(f, Skill::Logistician)) { 1.5 } else { 1.0 };
                let amount = self.planets[pi].stock[kind].min(load).min(short.max(load * 0.5));
                self.planets[pi].stock[kind] -= amount;
                let mut cargo = [0.0; 3];
                cargo[kind] = amount;
                incoming.entry(target).or_insert([0.0; 3])[kind] += amount;
                self.add_convoy(p.owner, p.pos, cargo, Some(target));
                self.convoys_sent[side] += 1;
            }
        }
        self.sync();
    }

    /// Convoys and reinforcements chase their fleets along the highways
    /// and join them in range. A convoy whose fleet is gone heads for its
    /// owner's nearest planet and unloads there; a reinforcement stays.
    fn run_convoys(&mut self, dt: f32) {
        self.convoy_timer += dt;
        if self.convoy_timer < CONVOY_SECS - 1e-4 {
            return;
        }
        self.convoy_timer = 0.0;
        let reach = self.range() * 2.0;
        let mut joins: Vec<(Vec<Entity>, Entity)> = Vec::new();
        let mut unload: Vec<(usize, usize)> = Vec::new();
        for i in 0..self.fleets.len() {
            let f = &self.fleets[i];
            let Some(target) = f.joining else { continue };
            let goal = self.fleet(target).filter(|g| g.armed() && g.morale != Morale::Routed && g.owner == f.owner).map(|g| (g.pos, g.division));
            match goal {
                Some((at, division)) if at.distance(f.pos) <= reach => joins.push((f.ship_ids.clone(), division)),
                Some((at, _)) => {
                    if !f.waiting {
                        self.route_to(i, at);
                    }
                }
                None if f.armed() => {
                    // A reinforcement with nobody left to join: it stays.
                    self.fleets[i].joining = None;
                }
                None => {
                    let owner = self.fleets[i].owner;
                    let pos = self.fleets[i].pos;
                    let home = (0..self.planets.len())
                        .filter(|p| self.planets[*p].owner == owner)
                        .min_by(|a, b| self.road_distance(pos, self.planets[*a].pos).total_cmp(&self.road_distance(pos, self.planets[*b].pos)));
                    match home {
                        Some(p) if self.planets[p].pos.distance(pos) <= self.capture_radius() => unload.push((i, p)),
                        Some(p) => {
                            let to = self.planets[p].pos;
                            self.route_to(i, to);
                        }
                        None => {}
                    }
                }
            }
        }
        for (i, p) in unload {
            let (_, cargo) = self.fleet_stores(&self.fleets[i]);
            (0..3).for_each(|k| self.planets[p].stock[k] += cargo[k]);
            let (division, ships) = (self.fleets[i].division, self.fleets[i].ship_ids.clone());
            crate::sim::despawn_ships(&mut self.world, division, &ships);
        }
        for (ships, division) in joins {
            crate::sim::transfer_ships(&mut self.world, &ships, division);
        }
        self.sync();
    }

    /// Whether a supply ship of `owner` may be at `p`: anywhere but in the
    /// other side's cells.
    fn passable(&self, inf: &Influence, owner: Owner, p: Vec2) -> bool {
        let other = |o: Owner| matches!(o, Owner::Side(_)) && o != owner;
        match inf.at(p) {
            Some(Site::Planet(i)) => !other(self.planets[i].owner),
            Some(Site::Fleet(i)) => !other(self.fleets[i].owner),
            None => true,
        }
    }

    /// Fleets of supply ships alone stop at the edge of the other side's
    /// cells and go on once the way is clear.
    fn hold_convoys(&mut self) {
        if !self.fleets.iter().any(|f| !f.armed() && !f.route.is_empty()) {
            return;
        }
        let inf = Influence::new(self);
        let unit = self.unit();
        for i in 0..self.fleets.len() {
            let f = &self.fleets[i];
            if f.armed() || f.route.is_empty() {
                continue;
            }
            let Some(anchor) = self.world.get::<Division>(f.division).map(|d| d.anchor * unit) else { continue };
            let next = f.route[0];
            let ahead = anchor + (next - anchor).normalize_or_zero() * 5.0 * self.scale();
            let clear = self.passable(&inf, f.owner, ahead);
            if !clear && !f.waiting {
                self.fleets[i].waiting = true;
                if let Some(mut d) = self.world.get_mut::<Division>(self.fleets[i].division) {
                    d.order = Order::Hold;
                }
            } else if clear && f.waiting {
                self.fleets[i].waiting = false;
                self.next_leg(i);
            } else if !clear {
                // Still blocked: stay put whatever else asked it to move.
                if let Some(mut d) = self.world.get_mut::<Division>(self.fleets[i].division) {
                    d.order = Order::Hold;
                }
            }
        }
    }

    /// Each fleet's supply ships keep as close behind it as they can: just
    /// behind the rear rank, or, if that is in the other side's cells, the
    /// furthest point toward it that isn't.
    fn position_supply(&mut self) {
        if !self.fleets.iter().any(|f| f.armed() && f.supply > 0) {
            return;
        }
        let inf = Influence::new(self);
        let unit = self.unit();
        let spacing = self.battle.spacing;
        let mut slots: Vec<(Entity, Vec2)> = Vec::new();
        for f in self.fleets.iter().filter(|f| f.armed() && f.supply > 0) {
            let Some(d) = self.world.get::<Division>(f.division) else { continue };
            let train: Vec<Entity> = f.ship_ids.iter().zip(&f.is_supply).filter(|(_, s)| **s).map(|(e, _)| *e).collect();
            let n = train.len() as f32;
            for (k, e) in train.into_iter().enumerate() {
                let local = Vec2::new(d.depth - crate::sim::SUPPLY_TRAIL * spacing, (k as f32 - (n - 1.0) / 2.0) * 1.5 * spacing);
                let ideal = d.to_world(local) * unit;
                let target = if self.passable(&inf, f.owner, ideal) {
                    ideal
                } else {
                    (0..8)
                        .rev()
                        .map(|s| f.pos + (ideal - f.pos) * (s as f32 / 8.0))
                        .find(|p| self.passable(&inf, f.owner, *p))
                        .unwrap_or(f.pos)
                };
                slots.push((e, d.to_local(target / unit)));
            }
        }
        for (e, slot) in slots {
            self.world.entity_mut(e).insert(crate::sim::components::Slot(slot));
        }
    }

    /// Reads every fleet back from its division; drops the empty ones.
    /// A fleet that reaches a waypoint takes the next leg.
    fn sync(&mut self) {
        let unit = self.unit();
        let mut ships: std::collections::HashMap<Entity, Vec<(Vec2, f32, Entity, bool)>> = std::collections::HashMap::new();
        for (e, p, h, d, s) in self.world.query::<(Entity, &Pos, &Heading, &InDivision, bevy::prelude::Has<Supply>)>().iter(&self.world) {
            ships.entry(d.0).or_default().push((p.0 * unit, h.0, e, s));
        }
        let mut gone = Vec::new();
        let mut arrived = Vec::new();
        let mut legs = Vec::new();
        for (i, f) in self.fleets.iter_mut().enumerate() {
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
            // The fleet is where its combat ships are (a convoy: its ships).
            let core: Vec<Vec2> = list.iter().filter(|s| !s.3).map(|s| s.0).collect();
            let core = if core.is_empty() { list.iter().map(|s| s.0).collect() } else { core };
            f.pos = core.iter().copied().sum::<Vec2>() / core.len() as f32;
            f.ships = list.iter().filter(|s| !s.3).count() as u32;
            f.supply = list.len() as u32 - f.ships;
            f.ship_px = list.iter().map(|s| (s.0, s.1)).collect();
            f.ship_ids = list.iter().map(|s| s.2).collect();
            f.is_supply = list.iter().map(|s| s.3).collect();
            f.facing = d.facing;
            f.formation = d.formation;
            f.morale = d.morale;
            f.engaged = d.since_contact < 1.5;
            if f.morale == Morale::Routed {
                f.dest = None;
                f.route.clear();
                f.home = None;
            }
            if let Some(&next) = f.route.first()
                && (d.anchor * unit).distance(next) < 1.0
            {
                f.route.remove(0);
                if f.route.is_empty() {
                    f.dest = None;
                    arrived.push(f.division);
                } else {
                    legs.push(i);
                }
            }
        }
        for e in arrived {
            if let Some(mut d) = self.world.get_mut::<Division>(e) {
                d.order = Order::Hold;
            }
        }
        for i in legs {
            self.next_leg(i);
        }
        self.fleets.retain(|f| !gone.contains(&f.division));
        for e in gone {
            if self.world.get_entity(e).is_ok() {
                self.world.despawn(e);
            }
        }
        self.track_captains();
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
        let cost = self.tuning.ship_cost;
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
                // Ships are built of parts; short of them, the ship waits.
                if self.planets[i].stock[PARTS] < cost {
                    self.planets[i].build = 1.0;
                    break;
                }
                self.planets[i].stock[PARTS] -= cost;
                self.planets[i].build -= 1.0;
                // To the nearest captain with room: at once if the fleet is
                // here, else the new ship travels to join it.
                let r = self.capture_radius();
                if let Some(target) = self.fleet_with_room(owner, pos) {
                    let tf = self.fleet_index(target).unwrap();
                    if self.fleets[tf].pos.distance(pos) <= r {
                        let e = self.fleets[tf].division;
                        crate::sim::add_ships(&mut self.world, e, 1);
                        self.equip(e, [0.0; 3]);
                        self.fleets[tf].ships += 1;
                    } else {
                        let facing = (self.fleets[tf].pos - pos).to_angle();
                        let id = self.spawn(owner, pos, facing, 1, None);
                        let fi = self.fleet_index(id).unwrap();
                        self.fleets[fi].joining = Some(target);
                        let to = self.fleets[tf].pos;
                        self.route_to(fi, to);
                    }
                    continue;
                }
                // No room anywhere: into the ships already here, its
                // garrison or else the biggest fleet resting at the planet.
                let here = self.fleets.iter().position(|f| f.home == Some(i) && f.owner == owner).or_else(|| {
                    (0..self.fleets.len())
                        .filter(|g| {
                            let f = &self.fleets[*g];
                            f.owner == owner && f.armed() && f.route.is_empty() && f.morale != Morale::Routed && f.pos.distance(pos) <= r
                        })
                        .max_by_key(|g| self.fleets[*g].ships)
                });
                match here {
                    Some(g) => {
                        let e = self.fleets[g].division;
                        crate::sim::add_ships(&mut self.world, e, 1);
                        // Built bare: the planet's stock fills it at the next phase.
                        self.equip(e, [0.0; 3]);
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
            let near = |o: Owner| self.fleets.iter().any(|f| f.owner == o && f.armed() && f.morale != Morale::Routed && f.pos.distance(pos) <= r);
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
            // A Conqueror takes it twice as fast, and is resented half as much.
            let conqueror = self
                .fleets
                .iter()
                .any(|f| f.owner == by && f.armed() && f.pos.distance(pos) <= r && self.has_skill(f, Skill::Conqueror));
            let k = if conqueror { 0.5 } else { 1.0 };
            if held >= HOLD_SECS * k - 1e-4 {
                let p = &mut self.planets[i];
                p.owner = by;
                p.build = 0.0;
                p.hold = None;
                // A fresh conquest is resented.
                p.occupation = OCCUPATION_PENALTY * k;
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
            if p.loyal() {
                self.planets[i].unrest = 0.0;
                continue;
            }
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
    pub fn background_key(&self) -> String {
        format!("{:?}", self.cfg)
    }
}

/// A cell of the map's one Voronoi diagram.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    Planet(usize),
    Fleet(usize),
}

/// Planets and fleets as the sites of one multiplicatively weighted
/// Voronoi diagram: a planet weighs size × planet weight, a fleet ships ×
/// weight per ship, and every point belongs to the site with the lowest
/// distance ÷ weight.
pub struct Influence {
    bases: Vec<Vec2>,
    weights: Vec<f32>,
    sites: Vec<Site>,
    metric: Metric,
}

impl Influence {
    pub fn new(d: &Dominion) -> Self {
        let mut s = Self { bases: Vec::new(), weights: Vec::new(), sites: Vec::new(), metric: d.cfg.metric };
        for (i, p) in d.planets.iter().enumerate() {
            s.bases.push(p.pos);
            s.weights.push(p.value as f32 * d.tuning.planet_weight);
            s.sites.push(Site::Planet(i));
        }
        for (i, f) in d.fleets.iter().enumerate() {
            s.bases.push(f.pos);
            s.weights.push(f.weight(&d.tuning));
            s.sites.push(Site::Fleet(i));
        }
        s
    }

    /// The cell `p` lies in (none only when no site has weight).
    pub fn at(&self, p: Vec2) -> Option<Site> {
        weighted_scores(&self.bases, &self.weights, p, self.metric).owner.map(|i| self.sites[i])
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
    /// The selected planet (highlighted).
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
const PLANET: Rgb = [230, 220, 170];
const REBEL: Rgb = [235, 60, 50];
const JAILED: Rgb = [70, 45, 45];
const CALM: Rgb = [110, 220, 120];
const HIGHWAY: Rgb = [110, 120, 150];
pub const RESOURCE_COLORS: [Rgb; 3] = [[235, 80, 60], [90, 200, 100], [175, 180, 190]];
const UNREST_LOW: Rgb = [240, 210, 90];
const UNREST_HIGH: Rgb = [235, 60, 50];

/// A pie chart in a disc: each (share, colour) slice clockwise from the
/// top; what the shares leave is `rest`.
fn draw_pie(c: &mut Canvas, at: Vec2, r: f32, slices: &[(f32, Rgb)], rest: Rgb) {
    let ri = r.ceil() as i32;
    for dy in -ri..=ri {
        for dx in -ri..=ri {
            let (fx, fy) = (dx as f32 + 0.5, dy as f32 + 0.5);
            if fx * fx + fy * fy > r * r {
                continue;
            }
            // 0 at the top, growing clockwise (screen y points down).
            let turn = (fx.atan2(-fy) / std::f32::consts::TAU).rem_euclid(1.0);
            let mut acc = 0.0;
            let mut col = rest;
            for (share, slice) in slices {
                acc += share;
                if turn < acc {
                    col = *slice;
                    break;
                }
            }
            c.set(at.x as i32 + dx, at.y as i32 + dy, col);
        }
    }
}

/// A resource's icon centred on `o`: ammo a red triangle, food a green
/// disc, parts a grey square, each with an ink outline.
fn draw_icon(c: &mut Canvas, kind: usize, o: Vec2, size: f32) {
    match kind {
        AMMO => {
            let tri = |k: f32| [o + Vec2::new(0.0, -size - k), o + Vec2::new(size + k, size + k * 0.4), o + Vec2::new(-size - k, size + k * 0.4)];
            c.polygon_fill(&tri(1.5), INK, 1.0);
            c.polygon_fill(&tri(0.0), RESOURCE_COLORS[AMMO], 1.0);
        }
        FOOD => {
            c.disc(o, size + 1.0, INK, 1.0);
            c.disc(o, size, RESOURCE_COLORS[FOOD], 1.0);
        }
        _ => {
            let sq = |h: f32| [o + Vec2::new(-h, -h), o + Vec2::new(h, -h), o + Vec2::new(h, h), o + Vec2::new(-h, h)];
            c.polygon_fill(&sq(size + 1.0), INK, 1.0);
            c.polygon_fill(&sq(size - 0.3), RESOURCE_COLORS[PARTS], 1.0);
        }
    }
}

/// One icon per point of a planet's size, in a row above it.
fn draw_icons(c: &mut Canvas, p: &Planet, at: Vec2, r: f32) {
    let n = p.value as usize;
    let size = 3.0;
    for (k, kind) in p.produces.iter().take(n).enumerate() {
        let x = at.x + (k as f32 - (n as f32 - 1.0) / 2.0) * (2.0 * size + 3.0);
        draw_icon(c, *kind, Vec2::new(x, at.y - r - size - 4.0), size);
    }
}

/// A 3×5 pixel glyph: digits, capitals, '!', '/' and space.
fn glyph(ch: char) -> Option<[u8; 5]> {
    Some(match ch.to_ascii_uppercase() {
        'A' => [0b010, 0b101, 0b111, 0b101, 0b101],
        'B' => [0b110, 0b101, 0b110, 0b101, 0b110],
        'C' => [0b011, 0b100, 0b100, 0b100, 0b011],
        'D' => [0b110, 0b101, 0b101, 0b101, 0b110],
        'E' => [0b111, 0b100, 0b110, 0b100, 0b111],
        'F' => [0b111, 0b100, 0b110, 0b100, 0b100],
        'G' => [0b011, 0b100, 0b101, 0b101, 0b011],
        'H' => [0b101, 0b101, 0b111, 0b101, 0b101],
        'I' => [0b111, 0b010, 0b010, 0b010, 0b111],
        'J' => [0b001, 0b001, 0b001, 0b101, 0b010],
        'K' => [0b101, 0b101, 0b110, 0b101, 0b101],
        'L' => [0b100, 0b100, 0b100, 0b100, 0b111],
        'M' => [0b101, 0b111, 0b111, 0b101, 0b101],
        'N' => [0b110, 0b101, 0b101, 0b101, 0b101],
        'O' => [0b010, 0b101, 0b101, 0b101, 0b010],
        'P' => [0b110, 0b101, 0b110, 0b100, 0b100],
        'Q' => [0b010, 0b101, 0b101, 0b110, 0b011],
        'R' => [0b110, 0b101, 0b110, 0b101, 0b101],
        'S' => [0b011, 0b100, 0b010, 0b001, 0b110],
        'T' => [0b111, 0b010, 0b010, 0b010, 0b010],
        'U' => [0b101, 0b101, 0b101, 0b101, 0b111],
        'V' => [0b101, 0b101, 0b101, 0b101, 0b010],
        'W' => [0b101, 0b101, 0b111, 0b111, 0b101],
        'X' => [0b101, 0b101, 0b010, 0b101, 0b101],
        'Y' => [0b101, 0b101, 0b010, 0b010, 0b010],
        'Z' => [0b111, 0b001, 0b010, 0b100, 0b111],
        '0' => [0b111, 0b101, 0b101, 0b101, 0b111],
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b010, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '!' => [0b010, 0b010, 0b010, 0b000, 0b010],
        ' ' => [0b000, 0b000, 0b000, 0b000, 0b000],
        '/' => [0b001, 0b001, 0b010, 0b100, 0b100],
        _ => return None,
    })
}

/// Draws text (digits, capitals, '!', '/') centred on `at`, 2 px a dot,
/// ink-outlined.
fn draw_text(c: &mut Canvas, at: Vec2, text: &str, col: Rgb) {
    let px = 2i32;
    let glyphs: Vec<[u8; 5]> = text.chars().filter_map(glyph).collect();
    let w = glyphs.len() as i32 * 4 * px - px;
    let (x0, y0) = (at.x as i32 - w / 2, at.y as i32 - 5 * px / 2);
    for pass in 0..2 {
        for (g, rows) in glyphs.iter().enumerate() {
            for (row, bits) in rows.iter().enumerate() {
                for bit in 0..3 {
                    if bits & (0b100 >> bit) == 0 {
                        continue;
                    }
                    let (x, y) = (x0 + (g as i32 * 4 + bit) * px, y0 + row as i32 * px);
                    let (lo, hi, ink) = if pass == 0 { (-1, px, INK) } else { (0, px - 1, col) };
                    for oy in lo..=hi {
                        for ox in lo..=hi {
                            c.set(x + ox, y + oy, ink);
                        }
                    }
                }
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
pub fn render_background(d: &Dominion, view: &View, step: u32) -> Canvas {
    let mut c = Canvas::new(view.w, view.h, OUTSIDE);
    let world = Vec2::new(d.cfg.w as f32, d.cfg.h as f32);
    let step = step.max(1);
    for y in (0..view.h).step_by(step as usize) {
        for x in (0..view.w).step_by(step as usize) {
            let p = view.to_world(Vec2::new(x as f32 + step as f32 * 0.5, y as f32 + step as f32 * 0.5));
            if p.x < 0.0 || p.y < 0.0 || p.x > world.x || p.y > world.y {
                continue;
            }
            for by in y..(y + step).min(view.h) {
                for bx in x..(x + step).min(view.w) {
                    c.set(bx as i32, by as i32, SPACE);
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

/// A planet's radius on screen (big enough to show its people).
pub fn planet_radius(d: &Dominion, p: &Planet, zoom: f32) -> f32 {
    ((3.0 + 2.6 * p.value as f32) * d.scale().max(0.8) * zoom).max(4.0)
}

/// The Voronoi cells, highways, planets and ships over a cached
/// background, through `view`.
pub fn render(d: &Dominion, background: &Canvas, view: &View, opts: &DrawOptions) -> Canvas {
    let (w, h) = (view.w, view.h);
    let mut c = background.clone();
    let tones = tones(d);
    let z = view.zoom;
    // The one Voronoi diagram, sampled every `step` output pixels; cells
    // next to a different cell get every pixel computed exactly, so the
    // borders stay smooth.
    let step = opts.step.max(1);
    let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
    let inf = Influence::new(d);
    let world = Vec2::new(d.cfg.w as f32, d.cfg.h as f32);
    let sample = |x: f32, y: f32| {
        let p = view.to_world(Vec2::new(x, y));
        if p.x < 0.0 || p.y < 0.0 || p.x > world.x || p.y > world.y { None } else { inf.at(p) }
    };
    let mut cells: Vec<Option<Site>> = vec![None; (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            cells[(gy * gw + gx) as usize] = sample((gx * step) as f32 + step as f32 * 0.5, (gy * step) as f32 + step as f32 * 0.5);
        }
    }
    let coarse = |x: u32, y: u32| cells[((y / step) * gw + x / step) as usize];
    let edge_cell = |gx: u32, gy: u32| {
        let here = cells[(gy * gw + gx) as usize];
        [(gx.wrapping_sub(1), gy), (gx + 1, gy), (gx, gy.wrapping_sub(1)), (gx, gy + 1)]
            .iter()
            .any(|&(nx, ny)| nx < gw && ny < gh && cells[(ny * gw + nx) as usize] != here)
    };
    let mut fine: Vec<Option<Option<Site>>> = vec![None; (w * h) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            if !edge_cell(gx, gy) {
                continue;
            }
            for y in gy * step..((gy + 1) * step).min(h) {
                for x in gx * step..((gx + 1) * step).min(w) {
                    fine[(y * w + x) as usize] = Some(sample(x as f32 + 0.5, y as f32 + 0.5));
                }
            }
        }
    }
    let at = |x: u32, y: u32| fine[(y * w + x) as usize].unwrap_or_else(|| coarse(x, y));
    for y in 0..h {
        for x in 0..w {
            let Some(site) = at(x, y) else { continue };
            let base = c.get(x, y);
            let mut col = match site {
                Site::Planet(i) if opts.regions => {
                    let o = d.planets[i].owner;
                    Canvas::shade_px(owner_color(o), if matches!(o, Owner::Side(_)) { 0.45 } else { 0.75 })
                }
                Site::Planet(_) => base,
                Site::Fleet(f) => mix(base, tones[f], 0.4),
            };
            if opts.borders {
                let right = if x + 1 < w { at(x + 1, y) } else { Some(site) };
                let down = if y + 1 < h { at(x, y + 1) } else { Some(site) };
                if (right.is_some() && right != Some(site)) || (down.is_some() && down != Some(site)) {
                    col = INK;
                }
            }
            c.set(x as i32, y as i32, col);
        }
    }
    // Highways.
    for &(a, b) in &d.highways.edges {
        c.line(view.to_screen(d.highways.nodes[a]), view.to_screen(d.highways.nodes[b]), 1.5, INK, 0.6);
        c.line(view.to_screen(d.highways.nodes[a]), view.to_screen(d.highways.nodes[b]), 1.0, HIGHWAY, 0.7);
    }
    // Planets: their people inside, the capture ring and unrest arc
    // around, an icon per point of size above.
    for (i, p) in d.planets.iter().enumerate() {
        let at = view.to_screen(p.pos);
        let r = planet_radius(d, p, z);
        if let Some((by, t)) = p.hold {
            let frac = (t / HOLD_SECS).clamp(0.0, 1.0);
            let n = (64.0 * frac) as usize;
            for k in 0..n {
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 / 64.0 * std::f32::consts::TAU;
                c.disc(at + Vec2::from_angle(a) * (r + 6.0), 1.8, owner_color(by), 1.0);
            }
        }
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
        if p.restless() {
            // The people as a pie: rebels, prisoners, the quiet rest.
            let people = &d.people[i];
            draw_pie(&mut c, at, r, &[(people.active_share(), REBEL), (people.jailed_share(), JAILED)], PLANET);
        } else {
            c.disc(at, r, PLANET, 1.0);
        }
        if p.loyal() {
            c.circle(at, r + 2.5, owner_color(p.owner), 1.0);
        }
        draw_icons(&mut c, p, at, r);
        // Garrison: ships there / ships needed to keep its people calm.
        // Green when met, red when not, grey for an independent (what a
        // conqueror would need); "!" when no garrison is enough. Loyal
        // home planets show the ships there only.
        let there = d.police_ships(i);
        let (text, col) = if p.loyal() {
            (there.to_string(), owner_color(p.owner))
        } else {
            match d.ships_needed(i) {
                None => (format!("{there}/!"), UNREST_HIGH),
                Some(n) if p.restless() => (format!("{there}/{n}"), if there >= n { CALM } else { UNREST_HIGH }),
                Some(n) => (format!("{there}/{n}"), [170, 170, 180]),
            }
        };
        draw_text(&mut c, at + Vec2::new(0.0, r + 9.0), &text, col);
    }
    if let Some(p) = opts.planet.and_then(|i| d.planets.get(i)) {
        let at = view.to_screen(p.pos);
        let r = planet_radius(d, p, z) + 9.0;
        c.circle(at, r, GOLD, 1.0);
        c.circle(at, r + 1.0, GOLD, 1.0);
    }
    // Destinations of the wholly selected fleets.
    let whole = d.whole_fleets(&opts.selected);
    for f in whole.iter().filter_map(|id| d.fleet(*id)) {
        let mut from = view.to_screen(f.pos);
        for p in &f.route {
            let to = view.to_screen(*p);
            c.dashed_line(from, to, 6.0, 5.0, GOLD, 0.9);
            from = to;
        }
        if let Some(dest) = f.dest {
            c.circle(view.to_screen(dest), 5.0, GOLD, 1.0);
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
            // Around the combat ships (the supply train trails outside).
            let r = ships
                .iter()
                .zip(&f.is_supply)
                .filter(|(_, s)| !**s || f.ships == 0)
                .map(|(s, _)| s.0.distance(centre))
                .fold(0.0, f32::max)
                + 7.0;
            c.circle(centre, r, GOLD, 1.0);
            c.circle(centre, r + 1.0, GOLD, 1.0);
        }
        let tone = match f.morale {
            Morale::Steady => mix(tones[i], WHITE, 0.25),
            Morale::Shaken => SHAKEN,
            Morale::Routed => ROUTED,
        };
        for (k, (s, heading)) in ships.iter().enumerate() {
            if s.x < -10.0 || s.y < -10.0 || s.x > w as f32 + 10.0 || s.y > h as f32 + 10.0 {
                continue;
            }
            let selected = f.ship_ids.get(k).is_some_and(|e| opts.selected.contains(e));
            if f.is_supply.get(k).copied().unwrap_or(false) {
                // A supply ship: a round hull with its cargo's icon on top.
                let r = (dart_scale * 7.0).max(3.0);
                if selected {
                    c.disc(*s, r + 2.5, GOLD, 1.0);
                }
                c.disc(*s, r + 1.2, INK, 1.0);
                c.disc(*s, r, tone, 1.0);
                let cargo = f.ship_ids.get(k).and_then(|e| d.world.get::<Supply>(*e)).map_or([0.0; 3], |c| c.0);
                draw_icon(&mut c, supply_kind(&cargo), *s, (r * 0.6).max(2.0));
                continue;
            }
            // The flagship: bigger, white, with a gold dot.
            let flagship = f.ship_ids.get(k).is_some_and(|e| d.world.get::<Flagship>(*e).is_some());
            let (col, big) = if flagship { (WHITE, 1.6) } else { (tone, 1.0) };
            let facing = Vec2::from_angle(*heading);
            let pts = |k: f32| -> Vec<Vec2> { SHIP_DART.iter().map(|p| *s + facing.rotate(*p * dart_scale * k * big)).collect() };
            // Selected ships get a gold outline.
            if selected {
                c.polygon_fill(&pts(2.0), GOLD, 1.0);
            }
            c.polygon_fill(&pts(1.45), INK, 1.0);
            c.polygon_fill(&pts(1.0), col, 1.0);
            if flagship {
                c.disc(*s, (dart_scale * 2.5).max(1.5), GOLD, 1.0);
            }
        }
        // The captain's initial above the fleet.
        if let Some(cap) = f.captain.and_then(|id| d.captain(id)) {
            let top = ships.iter().zip(&f.is_supply).filter(|(_, s)| !**s).map(|(s, _)| s.0.y).fold(centre.y, f32::min);
            draw_text(&mut c, Vec2::new(centre.x, top - 12.0), &cap.initial().to_string(), mix(tones[i], WHITE, 0.5));
        }
    }
    for (p, age) in &d.booms {
        let k = age / BOOM_SECS;
        c.circle(view.to_screen(*p), (3.0 + 9.0 * k) * (z * d.scale()).clamp(0.6, 2.0), HIT, (1.0 - k) * 0.9);
    }
    // How many ships are selected, above the topmost of them.
    if !opts.selected.is_empty() {
        let at: Vec<Vec2> = d
            .fleets
            .iter()
            .flat_map(|f| f.ship_ids.iter().zip(&f.ship_px))
            .filter(|(e, _)| opts.selected.contains(e))
            .map(|(_, p)| view.to_screen(p.0))
            .collect();
        if !at.is_empty() {
            let top = at.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
            let x = at.iter().map(|p| p.x).sum::<f32>() / at.len() as f32;
            let y = (top - 26.0).max(8.0);
            draw_text(&mut c, Vec2::new(x, y), &format!("{} SELECTED", at.len()), GOLD);
        }
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

/// The kind a supply ship carries (its largest entry).
pub fn supply_kind(cargo: &Stock) -> usize {
    (0..3).max_by(|a, b| cargo[*a].total_cmp(&cargo[*b])).unwrap()
}

/// "2 ammo, 1 food (34 units)": a fleet's supply ships by kind.
fn supply_text(d: &Dominion, f: &Fleet, cargo: &Stock) -> String {
    let mut count = [0; 3];
    for e in f.ship_ids.iter().zip(&f.is_supply).filter(|(_, s)| **s).map(|(e, _)| e) {
        if let Some(c) = d.world.get::<Supply>(*e) {
            count[supply_kind(&c.0)] += 1;
        }
    }
    let kinds: Vec<String> = (0..3).filter(|k| count[*k] > 0).map(|k| format!("{} {}", count[k], RESOURCE_NAMES[k])).collect();
    format!("{} ({:.0} units)", kinds.join(", "), cargo.iter().sum::<f32>())
}

fn stock_text(s: &Stock) -> String {
    format!("{:.0} ammo, {:.0} food, {:.0} parts", s[AMMO], s[FOOD], s[PARTS])
}

fn rate_text(s: &Stock) -> String {
    format!("{:.1}/{:.1}/{:.1}", s[AMMO], s[FOOD], s[PARTS])
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
        let gone = if d.deserted[side] > 0 { format!(" · {} ships lost to shortages", d.deserted[side]) } else { String::new() };
        let gone = if d.revolts[side] > 0 { format!("{gone} · {} planets lost to revolts", d.revolts[side]) } else { gone };
        let transit = d.fleets.iter().filter(|f| f.owner == o && f.joining.is_some() && !f.armed()).count();
        let reinforcing: u32 = d.fleets.iter().filter(|f| f.owner == o && f.joining.is_some() && f.armed()).map(|f| f.ships).sum();
        let waiting = d.fleets.iter().filter(|f| f.owner == o && f.waiting).count();
        let waiting = if waiting > 0 { format!(" ({waiting} waiting at enemy lines)") } else { String::new() };
        let captains: Vec<String> = d
            .captains
            .iter()
            .filter(|c| c.side == side)
            .map(|c| {
                let state = if !c.alive {
                    " †".to_string()
                } else if c.flagship.is_none() {
                    format!(" (back in {:.0} s)", c.back_in.max(0.0))
                } else {
                    String::new()
                };
                format!("{} {}{state}", c.name, c.skill.name())
            })
            .collect();
        s.push_str(&format!("\n{name} captains: {}", if captains.is_empty() { "none".into() } else { captains.join(", ") }));
        s.push_str(&format!(
            "\n{name}: planets hold {} · make {}/s · {transit} supply ships, {reinforcing} reinforcements under way{waiting}{gone}",
            stock_text(&d.stock_of(o)),
            rate_text(&d.planet_income(o)),
        ));
    }
    if let Some(p) = planet.and_then(|i| d.planets.get(i)) {
        let i = planet.unwrap();
        let who = match p.owner {
            Owner::Side(s) => SIDE_NAMES[s].to_string(),
            Owner::Independent(_) => "Independent".into(),
        };
        s.push_str(&format!(
            "\nPlanet: {who}, size {}, makes {}/s, holds {} · tax {:.0}% ([ ] change it) · legitimacy {:.2}",
            p.value,
            rate_text(&d.planet_output(p)),
            stock_text(&p.stock),
            p.tax * 100.0,
            p.legitimacy()
        ));
        if p.occupation > 0.0 {
            s.push_str(&format!(" (occupied −{:.2})", p.occupation));
        }
        if p.loyal() {
            s.push_str(" · home: loyal, never rebels.");
        } else if matches!(p.owner, Owner::Side(_)) {
            match d.ships_needed(i) {
                Some(n) => s.push_str(&format!(" · needs {n} ships to stay calm (has {})", d.police_ships(i))),
                None => s.push_str(" · no garrison can keep it calm"),
            }
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
                Owner::Side(side) if f.joining.is_some() && !f.armed() => format!("{} convoy", SIDE_NAMES[side]),
                Owner::Side(side) if f.joining.is_some() => format!("{} reinforcements", SIDE_NAMES[side]),
                Owner::Side(side) => match f.captain.and_then(|c| d.captain(c)) {
                    Some(c) => format!(
                        "{} — {} [{}], {}/{:.0} escorts",
                        SIDE_NAMES[side],
                        c.title(),
                        c.skill.blurb(),
                        d.escorts(f),
                        d.capacity(f)
                    ),
                    None => format!("{} ships without a captain", SIDE_NAMES[side]),
                },
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
            s.push_str(&format!(
                "\nSelected: {} ships — {who}, {}, {}, {order}{fight}.",
                f.ships + f.supply,
                f.formation.name(),
                f.morale.name()
            ));
            let (stores, cargo) = d.fleet_stores(f);
            let n = f.ships.max(1) as f32;
            s.push_str(&format!(
                " Per ship {:.0} ammo, {:.1} food, {:.1} parts · supply ships: {}.",
                stores[AMMO] / n,
                stores[FOOD] / n,
                stores[PARTS] / n,
                if f.supply == 0 { "none".to_string() } else { supply_text(d, f, &cargo) }
            ));
            if f.morale == Morale::Routed {
                s.push_str(" Routed: it no longer takes orders.");
            } else if matches!(f.owner, Owner::Independent(_)) {
                s.push_str(" Independents only defend.");
            } else {
                s.push_str(" Right-click the map to move it · keys 1–6 set its formation.");
            }
        }
        _ => {
            let n = selected.iter().filter(|e| d.fleet_of_ship(**e).is_some()).count();
            let captains = from.iter().filter(|id| d.fleet(**id).is_some_and(|f| f.controllable())).count();
            s.push_str(&format!(
                "\nSelected: {n} ships from {} fleets, {captains} with a captain. Right-click: each captain goes there and takes along the \
                 selected ships near it; the others stay.",
                from.len()
            ));
        }
    }
    s
}
