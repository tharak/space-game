//! prototype01 — Fleet dominion, in real time. Space is split into Voronoi
//! regions; some hold a planet. Every planet builds ships into its own
//! garrison fleet; Blue's and Red's fleets can be commanded once they have
//! 10 ships. Fleets in range fight automatically with the battle's rules
//! (range, gun cone, front/flank/rear hit chances, reload, hull), resolved
//! as expected damage so it is deterministic. A planet changes hands once
//! its garrison is gone and an enemy fleet has held it for a few seconds.
//! Influence patches (weight = ships × weight per ship, reach = reach per
//! weight × weight) show who dominates where.

use crate::sim::battle::Tuning as BattleTuning;
use crate::sim::components::{angle_diff, turn_toward};
pub use crate::sim::formation::Formation;
use crate::sim::formation::{SHIP_DART, layout};
use crate::sim::systems::{hit_chance, incoming_arc};
use crate::voronoi::Vec2;
use crate::voronoi::grid::{NO_OWNER, OwnerGrid, voronoi_grid};
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
/// Simulation tick, seconds.
pub const TICK: f32 = 0.1;
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
        self != other && (matches!(self, Owner::Side(_)) || matches!(other, Owner::Side(_)))
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

#[derive(Clone, Debug, PartialEq)]
pub struct Fleet {
    /// Stable across removals (the UI keys menus on it).
    pub id: u32,
    pub owner: Owner,
    pub pos: Vec2,
    /// Radians; 0 = +x.
    pub facing: f32,
    pub dest: Option<Vec2>,
    pub ships: u32,
    /// Hull points taken toward the next ship lost.
    pub damage: f32,
    pub formation: Formation,
    /// The planet it garrisons (planets build into it).
    pub home: Option<usize>,
    /// Damage it dealt and took in the last tick (for status and flashes).
    pub dealt: f32,
    pub taken: f32,
}

impl Fleet {
    pub fn weight(&self, t: &Tuning) -> f32 {
        self.ships as f32 * t.weight_per_ship
    }

    /// Blue and Red fleets with 10+ ships, or that already left home.
    pub fn controllable(&self) -> bool {
        matches!(self.owner, Owner::Side(_)) && (self.ships >= CONTROL_MIN || self.home.is_none())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dominion {
    pub cfg: VoronoiConfig,
    pub tuning: Tuning,
    pub battle: BattleTuning,
    pub sites: Vec<Vec2>,
    pub regions: OwnerGrid,
    pub planet_at: Vec<Option<usize>>,
    pub planets: Vec<Planet>,
    pub fleets: Vec<Fleet>,
    next_id: u32,
    pub time: f32,
    /// Leftover time below one tick.
    pending: f32,
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
        let mut d = Self {
            cfg: *cfg,
            tuning,
            battle: BattleTuning::default(),
            sites,
            regions,
            planet_at,
            planets: list,
            fleets: Vec::new(),
            next_id: 1,
            time: 0.0,
            pending: 0.0,
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
        d
    }

    fn spawn(&mut self, owner: Owner, pos: Vec2, facing: f32, ships: u32, home: Option<usize>) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.fleets.push(Fleet {
            id,
            owner,
            pos,
            facing,
            dest: None,
            ships,
            damage: 0.0,
            formation: Formation::Line,
            home,
            dealt: 0.0,
            taken: 0.0,
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

    /// Each ship's position: the battle's formation layout (+x = facing),
    /// centred on the fleet, scaled to the map.
    pub fn ship_positions(&self, f: &Fleet) -> Vec<Vec2> {
        let slots = layout(f.formation, f.ships as usize, self.battle.spacing);
        if slots.is_empty() {
            return slots;
        }
        let centre = slots.iter().copied().sum::<Vec2>() / slots.len() as f32;
        let rot = Vec2::from_angle(f.facing);
        let unit = self.unit();
        slots.into_iter().map(|s| f.pos + rot.rotate((s - centre) * unit)).collect()
    }

    /// Orders a controllable fleet to move; it stops garrisoning.
    pub fn order_move(&mut self, id: u32, to: Vec2) -> bool {
        let (w, h) = (self.cfg.w as f32, self.cfg.h as f32);
        match self.fleets.iter_mut().find(|f| f.id == id) {
            Some(f) if f.controllable() => {
                f.dest = Some(to.clamp(Vec2::ZERO, Vec2::new(w, h)));
                f.home = None;
                true
            }
            _ => false,
        }
    }

    pub fn set_formation(&mut self, id: u32, formation: Formation) -> bool {
        match self.fleets.iter_mut().find(|f| f.id == id) {
            Some(f) if f.controllable() => {
                f.formation = formation;
                true
            }
            _ => false,
        }
    }

    /// The fleet with a ship within `r` pixels of `p`, nearest first.
    pub fn fleet_at(&self, p: Vec2, r: f32) -> Option<u32> {
        self.fleets
            .iter()
            .filter_map(|f| {
                let d = self.ship_positions(f).iter().map(|s| s.distance(p)).fold(f.pos.distance(p), f32::min);
                (d <= r).then_some((f.id, d))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// Advances the world by `dt` seconds, in fixed ticks.
    pub fn step(&mut self, dt: f32) {
        self.pending += dt;
        while self.pending >= TICK {
            self.pending -= TICK;
            self.tick(TICK);
        }
    }

    fn tick(&mut self, dt: f32) {
        self.time += dt;
        self.produce(dt);
        self.move_fleets(dt);
        self.fight(dt);
        self.fleets.retain(|f| f.ships > 0);
        self.capture(dt);
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
                    Some(g) => self.fleets[g].ships += 1,
                    None => {
                        let facing = (Vec2::new(self.cfg.w as f32, self.cfg.h as f32) * 0.5 - pos).to_angle();
                        self.spawn(owner, pos, facing, 1, Some(i));
                    }
                }
            }
        }
    }

    fn move_fleets(&mut self, dt: f32) {
        let speed = self.tuning.speed * self.scale();
        let range = self.range();
        let turn = 1.5 * dt;
        let snapshot: Vec<(Owner, Vec2)> = self.fleets.iter().map(|f| (f.owner, f.pos)).collect();
        let ships: Vec<Vec<Vec2>> = self.fleets.iter().map(|f| self.ship_positions(f)).collect();
        for (i, f) in self.fleets.iter_mut().enumerate() {
            // Like the battle's stop distance: a moving fleet halts and faces
            // a hostile fleet once any of its ships is within 60% of range.
            let blocker = (0..snapshot.len())
                .filter(|j| *j != i && f.owner.hostile(snapshot[*j].0))
                .filter_map(|j| ships[j].iter().map(|s| s.distance(f.pos)).reduce(f32::min).map(|d| (j, d)))
                .filter(|(_, d)| *d <= range * 0.6)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let (Some(_), Some((j, _))) = (f.dest, blocker) {
                f.facing = turn_toward(f.facing, (snapshot[j].1 - f.pos).to_angle(), turn * 2.0);
                continue;
            }
            if let Some(dest) = f.dest {
                let to = dest - f.pos;
                let step = speed * dt;
                if to.length() <= step {
                    f.pos = dest;
                    f.dest = None;
                } else {
                    f.pos += to.normalize() * step;
                    f.facing = turn_toward(f.facing, to.to_angle(), turn * 2.0);
                }
            } else if let Some(enemy) = snapshot
                .iter()
                .filter(|(o, p)| f.owner.hostile(*o) && p.distance(f.pos) <= range * 3.0)
                .min_by(|a, b| a.1.distance(f.pos).total_cmp(&b.1.distance(f.pos)))
            {
                // Idle: turn to face the nearest enemy close by.
                f.facing = turn_toward(f.facing, (enemy.1 - f.pos).to_angle(), turn);
            }
        }
    }

    /// Expected damage per second `a`'s ships deal to `b`'s, with the
    /// battle's rules: each ship shoots the nearest enemy ship inside its
    /// gun cone and range; hit chance by arc (front/flank/rear) and range;
    /// one shot per reload.
    pub fn fire_rate(&self, a: &Fleet, a_ships: &[Vec2], b: &Fleet, b_ships: &[Vec2]) -> f32 {
        let (range, unit) = (self.range(), self.unit());
        let mut rate = 0.0;
        for s in a_ships {
            let mut best: Option<(f32, Vec2)> = None;
            for t in b_ships {
                let d = s.distance(*t);
                if d > range || best.is_some_and(|(bd, _)| bd <= d) {
                    continue;
                }
                if angle_diff(a.facing, (*t - *s).to_angle()).abs() > self.battle.fire_arc {
                    continue;
                }
                best = Some((d, *t));
            }
            if let Some((d, t)) = best {
                let arc = incoming_arc(t, b.facing, *s);
                rate += hit_chance(&self.battle, arc, d / unit) / self.battle.cooldown * self.tuning.combat_speed;
            }
        }
        rate
    }

    fn fight(&mut self, dt: f32) {
        let range = self.range();
        let ships: Vec<Vec<Vec2>> = self.fleets.iter().map(|f| self.ship_positions(f)).collect();
        let radius: Vec<f32> = self
            .fleets
            .iter()
            .zip(&ships)
            .map(|(f, s)| s.iter().map(|p| p.distance(f.pos)).fold(0.0, f32::max))
            .collect();
        let mut taken = vec![0.0f32; self.fleets.len()];
        let mut dealt = vec![0.0f32; self.fleets.len()];
        for a in 0..self.fleets.len() {
            for b in 0..self.fleets.len() {
                let (fa, fb) = (&self.fleets[a], &self.fleets[b]);
                if !fa.owner.hostile(fb.owner) || fa.pos.distance(fb.pos) > radius[a] + radius[b] + range {
                    continue;
                }
                let dmg = self.fire_rate(fa, &ships[a], fb, &ships[b]) * dt;
                taken[b] += dmg;
                dealt[a] += dmg;
            }
        }
        let hull = self.battle.hull;
        for (i, f) in self.fleets.iter_mut().enumerate() {
            f.dealt = dealt[i];
            f.taken = taken[i];
            f.damage += taken[i];
            let lost = ((f.damage / hull).floor() as u32).min(f.ships);
            f.ships -= lost;
            f.damage -= lost as f32 * hull;
        }
    }

    fn capture(&mut self, dt: f32) {
        let r = self.capture_radius();
        for i in 0..self.planets.len() {
            let (owner, pos) = (self.planets[i].owner, self.planets[i].pos);
            let near = |o: Owner| self.fleets.iter().any(|f| f.owner == o && f.pos.distance(pos) <= r);
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
const CONTESTED: Rgb = [150, 140, 160];
const WHITE: Rgb = [240, 240, 240];
const INK: Rgb = [10, 10, 14];
const GOLD: Rgb = [235, 190, 80];
const HIT: Rgb = [255, 230, 150];

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

/// The slow-changing layer: planet regions in their owner's colours.
pub fn render_background(d: &Dominion, regions: bool) -> Canvas {
    let (w, h) = (d.cfg.w, d.cfg.h);
    let mut c = Canvas::new(w, h, SPACE);
    if !regions {
        return c;
    }
    for y in 0..h {
        for x in 0..w {
            let site = d.regions.at(x, y);
            let Some(pi) = (if site == NO_OWNER { None } else { d.planet_at[site as usize] }) else { continue };
            let col = if d.regions.is_border(x, y) {
                INK
            } else {
                let p = &d.planets[pi];
                Canvas::shade_px(owner_color(p.owner), if matches!(p.owner, Owner::Side(_)) { 0.45 } else { 0.75 })
            };
            c.set(x as i32, y as i32, col);
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

/// Patches, planets and ships over a cached background.
pub fn render(d: &Dominion, background: &Canvas, opts: DrawOptions) -> Canvas {
    let (w, h) = (d.cfg.w, d.cfg.h);
    let mut c = background.clone();
    let tones = tones(d);
    // Patches, sampled every `step` pixels.
    let step = opts.step.max(1);
    let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
    let inf = Influence::new(d);
    let mut cells: Vec<(Option<usize>, bool, bool)> = vec![(None, false, false); (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let p = Vec2::new((gx * step) as f32 + step as f32 * 0.5, (gy * step) as f32 + step as f32 * 0.5);
            let (fleet, contested, score) = inf.at(p);
            let front = fleet.is_some_and(|f| (inf.reach - score) * d.fleets[f].weight(&d.tuning) < 1.5 * step as f32);
            cells[(gy * gw + gx) as usize] = (fleet, contested, front);
        }
    }
    let at = |x: u32, y: u32| cells[((y / step) * gw + x / step) as usize];
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
    let scale = d.scale();
    // Planets, with a capture ring while someone holds them.
    for p in &d.planets {
        let r = (1.5 + 1.3 * p.value as f32) * scale.max(0.8);
        if let Some((by, t)) = p.hold {
            let col = owner_color(by);
            let frac = (t / HOLD_SECS).clamp(0.0, 1.0);
            let n = (48.0 * frac) as usize;
            for k in 0..n {
                let a = -std::f32::consts::FRAC_PI_2 + k as f32 / 48.0 * std::f32::consts::TAU;
                c.disc(p.pos + Vec2::from_angle(a) * (r + 6.0), 1.6, col, 1.0);
            }
        }
        c.disc(p.pos, r + 1.0, INK, 1.0);
        c.disc(p.pos, r, [230, 220, 170], 1.0);
    }
    // Destination of the selected fleet.
    if let Some(f) = opts.selected.and_then(|id| d.fleet(id))
        && let Some(dest) = f.dest
    {
        c.dashed_line(f.pos, dest, 5.0, 4.0, GOLD, 0.9);
        c.circle(dest, 4.0, GOLD, 1.0);
    }
    // Ships: the battle's darts at their formation slots.
    let dart_scale = 0.32 * scale;
    for (i, f) in d.fleets.iter().enumerate() {
        let ships = d.ship_positions(f);
        let facing = Vec2::from_angle(f.facing);
        if opts.selected == Some(f.id) {
            let r = ships.iter().map(|s| s.distance(f.pos)).fold(0.0, f32::max) + 6.0;
            c.circle(f.pos, r, GOLD, 1.0);
        }
        let flash = f.taken > 0.0;
        let flag = (0..ships.len()).min_by(|a, b| ships[*a].distance(f.pos).total_cmp(&ships[*b].distance(f.pos)));
        for (k, s) in ships.iter().enumerate() {
            let col = if Some(k) == flag && f.controllable() {
                WHITE
            } else if flash && k % 7 == (d.time * 10.0) as usize % 7 {
                HIT
            } else {
                mix(tones[i], WHITE, 0.25)
            };
            let pts = |k: f32| -> Vec<Vec2> { SHIP_DART.iter().map(|p| *s + facing.rotate(*p * dart_scale * k)).collect() };
            c.polygon_fill(&pts(1.5), INK, 1.0);
            c.polygon_fill(&pts(1.0), col, 1.0);
        }
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
            let order = if f.dest.is_some() {
                "moving"
            } else if f.home.is_some() {
                "garrisoning"
            } else {
                "holding"
            };
            let fight = if f.dealt > 0.0 || f.taken > 0.0 { " · ENGAGED" } else { "" };
            s.push_str(&format!("\nSelected: {who}, {} ships, {}, {order}{fight}.", f.ships, f.formation.name()));
            if !f.controllable() {
                s.push_str(&format!(" Takes orders at {CONTROL_MIN} ships."));
            } else {
                s.push_str(" Click the map to move it.");
            }
        }
        None => s.push_str("\nClick a fleet with 10+ ships to select it, then click the map to send it there."),
    }
    s
}
