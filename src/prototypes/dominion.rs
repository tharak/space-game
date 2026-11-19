//! prototype01 — Fleet dominion. Space is split into Voronoi regions; some
//! of them hold a planet. Every planet starts as its own independent
//! faction that only defends itself. Blue and Red fleets project influence
//! like growing seeds. A fleet's weight is ships × weight-per-ship; it
//! reaches reach-per-weight × weight, and where patches meet the border sits
//! at distance ÷ weight. At the end of each turn a planet joins the side of
//! the fleet dominating it if that fleet outweighs the planet, and every
//! planet pays its owner. The three numbers live in [`Tuning`]. Contested planets (the other side within 10%) don't change
//! hands.

use crate::voronoi::Vec2;
use crate::voronoi::grid::{NO_OWNER, OwnerGrid, voronoi_grid};
use crate::voronoi::influence::weighted_scores;
use crate::voronoi::labs::cell_color;
use crate::voronoi::metric::{Metric, distance};
use crate::voronoi::raster::{Canvas, Rgb};
use crate::voronoi::rng::ParkMiller;
use crate::voronoi::sites::{VoronoiConfig, make_sites};

pub const MAX_FLEETS_PER_SIDE: usize = 8;
pub const START_SHIPS: u32 = 50;
pub const MIN_SHIPS: u32 = 10;
pub const MAX_SHIPS: u32 = 200;
pub const REINFORCE_COST: u32 = 10;
pub const REINFORCE_SHIPS: u32 = 25;
/// The numbers that shape the prototype, set from sliders.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Influence weight each ship adds.
    pub weight_per_ship: f32,
    /// Reach per point of weight, in pixels of a 720-wide map: it scales
    /// with the map, so bigger maps don't shrink the fleets' share.
    pub reach_per_weight: f32,
    /// Every planet's weight: a fleet must outweigh it to take the planet.
    pub planet_weight: f32,
}

impl Default for Tuning {
    fn default() -> Self {
        // 50 ships = weight 5, reaching 70 px; a starting fleet outweighs a planet.
        Self { weight_per_ship: 0.1, reach_per_weight: 14.0, planet_weight: 4.0 }
    }
}
/// The other side's best fleet within this fraction contests the ground.
pub const CONTEST_MARGIN: f32 = 0.10;
pub const SIDE_NAMES: [&str; 2] = ["Blue", "Red"];
const SIDE_COLORS: [Rgb; 2] = [[70, 130, 220], [215, 80, 70]];

/// Who a planet belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner {
    /// Blue (0) or Red (1).
    Side(usize),
    /// An independent faction, numbered by the planet it started on. It
    /// has no fleet and only defends.
    Independent(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planet {
    /// The Voronoi site (region) it sits in.
    pub site: usize,
    pub pos: Vec2,
    /// Resources a turn, 1 (poor) to 3 (rich).
    pub value: u32,
    /// A fleet must outweigh this to take the planet.
    pub weight: f32,
    pub owner: Owner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FleetToken {
    pub side: usize,
    pub pos: Vec2,
    pub ships: u32,
}

impl FleetToken {
    /// Influence weight: distance ÷ weight decides who dominates.
    pub fn weight(&self, t: &Tuning) -> f32 {
        self.ships as f32 * t.weight_per_ship
    }

    /// How far its patch can grow, in map pixels.
    pub fn reach(&self, t: &Tuning, map_scale: f32) -> f32 {
        t.reach_per_weight * map_scale * self.weight(t)
    }
}

/// Map width the reach slider is measured against.
pub const REFERENCE_WIDTH: f32 = 720.0;

/// How much bigger than the reference this map is.
pub fn map_scale(cfg: &VoronoiConfig) -> f32 {
    cfg.w as f32 / REFERENCE_WIDTH
}

/// Which side's fleets dominate a point of space.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Side(usize),
    Contested,
    Neutral,
}

/// What the fleets are doing to a planet this turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pressure {
    /// No fleets at all.
    None,
    /// Both sides within the contest margin: nothing changes.
    Contested,
    /// The dominating side already owns it.
    Held(usize),
    /// The dominating fleet outweighs the planet: it falls at the end of
    /// the turn.
    Falls(usize),
    /// The dominating fleet is too weak to take it.
    Resists(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dominion {
    pub cfg: VoronoiConfig,
    pub tuning: Tuning,
    pub sites: Vec<Vec2>,
    /// Which site owns each pixel.
    pub regions: OwnerGrid,
    /// The planet in each site's region, if any.
    pub planet_at: Vec<Option<usize>>,
    pub planets: Vec<Planet>,
    pub fleets: Vec<FleetToken>,
    pub stock: [u32; 2],
    pub turn: u32,
    /// What happened at the last end of turn.
    pub last_turn: String,
}

impl Dominion {
    /// `planets` of the config's sites get a planet (inland regions first,
    /// picked at random from the seed); each side gets `fleets` fleets.
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
        let mut list = Vec::new();
        for &site in order.iter().take(planets.min(sites.len())) {
            let roll = rng.next_float();
            let value = if roll < 0.2 {
                3
            } else if roll < 0.7 {
                2
            } else {
                1
            };
            planet_at[site] = Some(list.len());
            list.push(Planet { site, pos: sites[site], value, weight: tuning.planet_weight, owner: Owner::Independent(list.len()) });
        }
        // Blue starts in the left third; Red mirrors it.
        let (w, h) = (cfg.w as f32, cfg.h as f32);
        let spots = make_sites(fleets.clamp(1, MAX_FLEETS_PER_SIDE), w / 3.0, h * 0.8, cfg.seed.wrapping_add(23));
        let fleets = (0..2)
            .flat_map(|side| {
                spots.iter().map(move |p| {
                    // Keep a margin so patches don't start half off the map.
                    let x = 30.0 + p.x * 0.85;
                    let x = if side == 0 { x } else { w - x };
                    FleetToken { side, pos: Vec2::new(x, p.y + h * 0.1), ships: START_SHIPS }
                })
            })
            .collect();
        Self { cfg: *cfg, tuning, sites, regions, planet_at, planets: list, fleets, stock: [0; 2], turn: 1, last_turn: String::new() }
    }

    /// New tuning; every planet takes the planet weight (all the same for now).
    pub fn set_tuning(&mut self, t: Tuning) {
        self.tuning = t;
        for p in &mut self.planets {
            p.weight = t.planet_weight;
        }
    }

    /// Which side's fleets dominate point `p`.
    pub fn control_at(&self, p: Vec2) -> Control {
        Influence::new(self).at(p).0
    }

    /// What the fleets are doing to each planet.
    pub fn pressure(&self) -> Vec<Pressure> {
        let inf = Influence::new(self);
        self.planets
            .iter()
            .map(|pl| match inf.at(pl.pos) {
                (Control::Neutral, _) => Pressure::None,
                (Control::Contested, _) => Pressure::Contested,
                (Control::Side(s), fleet) => {
                    if pl.owner == Owner::Side(s) {
                        Pressure::Held(s)
                    } else if fleet.is_some_and(|f| self.fleets[f].weight(&self.tuning) > pl.weight) {
                        Pressure::Falls(s)
                    } else {
                        Pressure::Resists(s)
                    }
                }
            })
            .collect()
    }

    pub fn income(&self) -> [u32; 2] {
        let mut out = [0; 2];
        for p in &self.planets {
            if let Owner::Side(s) = p.owner {
                out[s] += p.value;
            }
        }
        out
    }

    /// Planets held by Blue and Red, and how many are still independent.
    pub fn holdings(&self) -> ([u32; 2], u32) {
        let mut held = [0; 2];
        let mut free = 0;
        for p in &self.planets {
            match p.owner {
                Owner::Side(s) => held[s] += 1,
                Owner::Independent(_) => free += 1,
            }
        }
        (held, free)
    }

    /// Planets change hands, then every owned planet pays.
    pub fn end_turn(&mut self) {
        let mut taken = [0u32; 2];
        for (i, pr) in self.pressure().into_iter().enumerate() {
            if let Pressure::Falls(s) = pr {
                self.planets[i].owner = Owner::Side(s);
                taken[s] += 1;
            }
        }
        let income = self.income();
        for (stock, inc) in self.stock.iter_mut().zip(income) {
            *stock += inc;
        }
        self.last_turn = format!(
            "Turn {}: Blue took {} and earned {}, Red took {} and earned {}.",
            self.turn, taken[0], income[0], taken[1], income[1]
        );
        self.turn += 1;
    }

    /// Spends from the fleet's side to add ships. False if it can't.
    pub fn reinforce(&mut self, fleet: usize) -> bool {
        let Some(f) = self.fleets.get(fleet) else { return false };
        if self.stock[f.side] < REINFORCE_COST || f.ships >= MAX_SHIPS {
            return false;
        }
        self.stock[f.side] -= REINFORCE_COST;
        let f = &mut self.fleets[fleet];
        f.ships = (f.ships + REINFORCE_SHIPS).min(MAX_SHIPS);
        true
    }

    /// The fleet's number within its side (1-based), for labels.
    pub fn fleet_number(&self, fleet: usize) -> usize {
        let side = self.fleets[fleet].side;
        self.fleets[..=fleet].iter().filter(|g| g.side == side).count()
    }

    /// The fleet within `r` pixels of `p`, nearest first.
    pub fn fleet_at(&self, p: Vec2, r: f32) -> Option<usize> {
        let pos: Vec<Vec2> = self.fleets.iter().map(|f| f.pos).collect();
        crate::voronoi::labs::site_at(&pos, p, r)
    }
}

/// The fleets as weighted influence bases, built once per batch of queries.
pub struct Influence {
    /// Score (distance ÷ weight) beyond which a fleet doesn't reach.
    pub reach: f32,
    bases: Vec<Vec2>,
    weights: Vec<f32>,
    sides: Vec<usize>,
    metric: Metric,
}

impl Influence {
    pub fn new(d: &Dominion) -> Self {
        Self {
            reach: d.tuning.reach_per_weight * map_scale(&d.cfg),
            bases: d.fleets.iter().map(|f| f.pos).collect(),
            weights: d.fleets.iter().map(|f| f.weight(&d.tuning)).collect(),
            sides: d.fleets.iter().map(|f| f.side).collect(),
            metric: d.cfg.metric,
        }
    }

    /// The controlling side at `p` and the fleet that dominates it. Like
    /// growing seeds, a fleet only reaches reach-per-weight × weight: a
    /// score (distance ÷ weight) above reach-per-weight is out of reach.
    pub fn at(&self, p: Vec2) -> (Control, Option<usize>) {
        let (ctrl, fleet, _) = self.at_scored(p);
        (ctrl, fleet)
    }

    /// Like [`Influence::at`], plus the dominating fleet's score.
    pub fn at_scored(&self, p: Vec2) -> (Control, Option<usize>, f32) {
        let inf = weighted_scores(&self.bases, &self.weights, p, self.metric);
        let Some(best) = inf.owner.filter(|_| inf.best <= self.reach) else { return (Control::Neutral, None, inf.best) };
        let side = self.sides[best];
        // The other side's best score, if that fleet reaches here too.
        let rival = self
            .bases
            .iter()
            .zip(&self.weights)
            .zip(&self.sides)
            .filter(|((_, w), s)| **s != side && **w > 0.0)
            .map(|((b, w), _)| distance(*b, p, self.metric) / w)
            .fold(f32::INFINITY, f32::min);
        let ctrl = if rival <= self.reach && rival <= inf.best * (1.0 + CONTEST_MARGIN) { Control::Contested } else { Control::Side(side) };
        (ctrl, Some(best), inf.best)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrawOptions {
    pub regions: bool,
    pub borders: bool,
    pub selected: Option<usize>,
    /// > 1 samples influence coarsely (quick previews while dragging).
    pub step: u32,
}

impl Default for DrawOptions {
    fn default() -> Self {
        Self { regions: true, borders: true, selected: None, step: 1 }
    }
}

const SPACE: Rgb = [12, 14, 22];
const CONTESTED: Rgb = [150, 140, 160];
const WHITE: Rgb = [240, 240, 240];
const INK: Rgb = [10, 10, 14];
const GOLD: Rgb = [235, 190, 80];

fn owner_color(o: Owner) -> Rgb {
    match o {
        Owner::Side(s) => SIDE_COLORS[s],
        // Each independent faction gets its own muted colour.
        Owner::Independent(id) => Canvas::shade_px(cell_color(id), 0.55),
    }
}

/// Fleet `k` of a side gets its own tone of the side colour.
pub fn fleet_color(side: usize, k: usize) -> Rgb {
    const TONES: [f32; MAX_FLEETS_PER_SIDE] = [0.0, -0.35, 0.35, -0.6, 0.6, -0.18, 0.18, 0.8];
    let base = SIDE_COLORS[side];
    let t = TONES[k % TONES.len()];
    if t >= 0.0 {
        // Toward white.
        [0, 1, 2].map(|i| (base[i] as f32 + (255.0 - base[i] as f32) * t * 0.7) as u8)
    } else {
        Canvas::shade_px(base, 1.0 + t)
    }
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    [0, 1, 2].map(|i| (a[i] as f32 * (1.0 - t) + b[i] as f32 * t).round() as u8)
}

pub fn render(d: &Dominion, opts: DrawOptions) -> Canvas {
    let (w, h) = (d.cfg.w, d.cfg.h);
    let mut c = Canvas::new(w, h, SPACE);
    let tones: Vec<Rgb> = (0..d.fleets.len()).map(|i| fleet_color(d.fleets[i].side, d.fleet_number(i) - 1)).collect();
    // Each fleet's patch (as in "grow the seeds"), coarse when `step` > 1.
    let step = opts.step.max(1);
    let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
    let inf = Influence::new(d);
    // (control, dominating fleet, on its growth front)
    let mut cells = vec![(Control::Neutral, None, false); (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let p = Vec2::new((gx * step) as f32 + step as f32 * 0.5, (gy * step) as f32 + step as f32 * 0.5);
            let (ctrl, fleet, score) = inf.at_scored(p);
            // The last ~1.5 px of a patch's reach.
            let front = fleet.is_some_and(|f| (inf.reach - score) * d.fleets[f].weight(&d.tuning) < 1.5 * step as f32);
            cells[(gy * gw + gx) as usize] = (ctrl, fleet, front);
        }
    }
    let at = |x: u32, y: u32| cells[((y / step) * gw + x / step) as usize];
    for y in 0..h {
        for x in 0..w {
            let site = d.regions.at(x, y);
            let planet = if site == NO_OWNER { None } else { d.planet_at[site as usize] };
            // Base: planet regions in their owner's colour, else empty space.
            let base = match planet {
                Some(pi) if opts.regions => {
                    if d.regions.is_border(x, y) {
                        INK
                    } else {
                        let pl = &d.planets[pi];
                        Canvas::shade_px(owner_color(pl.owner), if matches!(pl.owner, Owner::Side(_)) { 0.45 } else { 0.8 })
                    }
                }
                _ => SPACE,
            };
            let (ctrl, fleet, front) = at(x, y);
            let mut col = match (ctrl, fleet) {
                (Control::Contested, _) if (x + y) % 8 < 4 => mix(base, CONTESTED, 0.55),
                (_, Some(f)) => mix(base, tones[f], if planet.is_some() && opts.regions { 0.45 } else { 0.6 }),
                _ => base,
            };
            if opts.borders {
                // Dark seams where two fleets' patches meet, a bright edge
                // where a patch stops growing.
                let right = if x + 1 < w { at(x + 1, y).1 } else { fleet };
                let down = if y + 1 < h { at(x, y + 1).1 } else { fleet };
                if fleet.is_some() && ((right.is_some() && right != fleet) || (down.is_some() && down != fleet)) {
                    col = INK;
                } else if front && let Some(f) = fleet {
                    col = tones[f];
                }
            }
            c.set(x as i32, y as i32, col);
        }
    }
    // Planets: size by value, grey ring by defence, gold ring if it falls
    // at the end of this turn.
    for (p, pr) in d.planets.iter().zip(d.pressure()) {
        let r = 1.5 + 1.3 * p.value as f32;
        let ring = r + 2.0 + (p.weight * 0.8).min(14.0);
        c.circle(p.pos, ring, [170, 170, 180], 0.7);
        match pr {
            Pressure::Falls(s) => {
                c.circle(p.pos, ring + 2.0, GOLD, 1.0);
                c.circle(p.pos, ring + 3.0, SIDE_COLORS[s], 1.0);
            }
            Pressure::Contested => c.circle(p.pos, ring + 2.0, WHITE, 0.8),
            _ => {}
        }
        c.disc(p.pos, r + 1.0, INK, 1.0);
        c.disc(p.pos, r, [230, 220, 170], 1.0);
    }
    // Fleets: the battle map's ships in a wedge, in the fleet's own tone,
    // facing the nearest enemy fleet.
    for (i, f) in d.fleets.iter().enumerate() {
        let facing = d
            .fleets
            .iter()
            .filter(|g| g.side != f.side)
            .map(|g| g.pos - f.pos)
            .min_by(|a, b| a.length_squared().total_cmp(&b.length_squared()))
            .and_then(|v| v.try_normalize())
            .unwrap_or(if f.side == 0 { Vec2::X } else { Vec2::NEG_X });
        let slots = fleet_slots(ship_icons(f.ships));
        if opts.selected == Some(i) {
            let r = slots.iter().map(|s| s.length()).fold(0.0, f32::max) + 8.0;
            c.circle(f.pos, r, GOLD, 1.0);
            c.circle(f.pos, r + 1.0, GOLD, 1.0);
        }
        for (k, slot) in slots.iter().enumerate() {
            let (scale, col) = if k == 0 { (1.1, WHITE) } else { (0.6, tones[i]) };
            draw_ship(&mut c, f.pos + facing.rotate(*slot), facing, scale, col);
        }
    }
    c
}

/// Ships per dart on the map.
pub const SHIPS_PER_ICON: u32 = 5;
const MAX_ICONS: usize = 40;

/// How many darts a fleet of `ships` shows (at least one, the flagship).
pub fn ship_icons(ships: u32) -> usize {
    (ships.div_ceil(SHIPS_PER_ICON) as usize).clamp(1, MAX_ICONS)
}

/// Dart positions around the fleet's centre (+x = facing): the battle's
/// wedge, centred, with the flagship (slot 0) in the middle.
pub fn fleet_slots(icons: usize) -> Vec<Vec2> {
    let mut slots = crate::sim::formation::layout(crate::sim::formation::Formation::Wedge, icons, 6.0);
    let centre = slots.iter().copied().sum::<Vec2>() / slots.len().max(1) as f32;
    for s in &mut slots {
        *s -= centre;
    }
    // The ship nearest the centre is the flagship (slot 0).
    if let Some(k) = (0..slots.len()).min_by(|a, b| slots[*a].length_squared().total_cmp(&slots[*b].length_squared())) {
        slots.swap(0, k);
    }
    slots
}

fn draw_ship(c: &mut Canvas, at: Vec2, facing: Vec2, scale: f32, col: Rgb) {
    let dart = |k: f32| -> Vec<Vec2> { crate::sim::formation::SHIP_DART.iter().map(|p| at + facing.rotate(*p * scale * k)).collect() };
    // A dark outline, then the ship a little lighter than its patch.
    c.polygon_fill(&dart(1.45), INK, 1.0);
    c.polygon_fill(&dart(1.0), mix(col, WHITE, 0.3), 1.0);
}

/// The status lines for the page.
pub fn status(d: &Dominion, selected: Option<usize>) -> String {
    let income = d.income();
    let (held, free) = d.holdings();
    let falling = d.pressure().iter().filter(|p| matches!(p, Pressure::Falls(_))).count();
    let ships = |side: usize| d.fleets.iter().filter(|f| f.side == side).map(|f| f.ships).sum::<u32>();
    let sel = match selected.filter(|i| *i < d.fleets.len()) {
        Some(i) => {
            let f = &d.fleets[i];
            format!(
                "Selected: {} fleet {}, {} ships, weight {:.2}, reach {:.0} px.",
                SIDE_NAMES[f.side],
                d.fleet_number(i),
                f.ships,
                f.weight(&d.tuning),
                f.reach(&d.tuning, map_scale(&d.cfg))
            )
        }
        None => "Click a fleet to select it.".into(),
    };
    let mut s = format!(
        "Turn {} · Blue {} (+{}/turn, {} planets, {} ships) · Red {} (+{}/turn, {}, {} ships) · {} independent · {} fall at end of turn\n{sel}",
        d.turn,
        d.stock[0],
        income[0],
        held[0],
        ships(0),
        d.stock[1],
        income[1],
        held[1],
        ships(1),
        free,
        falling
    );
    if !d.last_turn.is_empty() {
        s.push(' ');
        s.push_str(&d.last_turn);
    }
    s
}
