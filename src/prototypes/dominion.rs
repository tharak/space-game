//! prototype01 — Fleet dominion. Space is split into Voronoi regions; some
//! of them hold a planet. Every planet starts as its own independent
//! faction that only defends itself. Blue and Red fleets project weighted
//! influence (distance ÷ strength). At the end of each turn, a planet
//! joins the faction of the fleet dominating it if that fleet is stronger
//! than the planet's defence, and every planet pays its owner. Contested
//! planets (the other side within 10%) don't change hands.

use crate::voronoi::Vec2;
use crate::voronoi::grid::{NO_OWNER, OwnerGrid, voronoi_grid};
use crate::voronoi::influence::weighted_scores;
use crate::voronoi::labs::cell_color;
use crate::voronoi::metric::{Metric, distance};
use crate::voronoi::raster::{Canvas, Rgb};
use crate::voronoi::rng::ParkMiller;
use crate::voronoi::sites::{VoronoiConfig, make_sites};

pub const FLEETS_PER_SIDE: usize = 3;
pub const MIN_STRENGTH: f32 = 0.2;
pub const MAX_STRENGTH: f32 = 4.0;
pub const REINFORCE_COST: u32 = 10;
pub const REINFORCE_STEP: f32 = 0.5;
/// Planet defence range.
pub const MIN_DEFENCE: f32 = 0.5;
pub const MAX_DEFENCE: f32 = 3.0;
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
    /// A fleet must be stronger than this to take the planet.
    pub defence: f32,
    pub owner: Owner,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FleetToken {
    pub side: usize,
    pub pos: Vec2,
    pub strength: f32,
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
    /// The dominating fleet is stronger than the defence: it falls at the
    /// end of the turn.
    Falls(usize),
    /// The dominating fleet is too weak to take it.
    Resists(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dominion {
    pub cfg: VoronoiConfig,
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
    /// `planets` of the config's sites get a planet: inland regions first,
    /// picked at random from the seed.
    pub fn new(cfg: &VoronoiConfig, planets: usize) -> Self {
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
            let defence = ((MIN_DEFENCE + rng.next_float() * (MAX_DEFENCE - MIN_DEFENCE)) * 10.0).round() / 10.0;
            planet_at[site] = Some(list.len());
            list.push(Planet { site, pos: sites[site], value, defence, owner: Owner::Independent(list.len()) });
        }
        // Blue starts in the left third; Red mirrors it.
        let (w, h) = (cfg.w as f32, cfg.h as f32);
        let spots = make_sites(FLEETS_PER_SIDE, w / 3.0, h * 0.8, cfg.seed.wrapping_add(23));
        let fleets = (0..2)
            .flat_map(|side| {
                spots.iter().map(move |p| {
                    let x = if side == 0 { p.x } else { w - p.x };
                    FleetToken { side, pos: Vec2::new(x, p.y + h * 0.1), strength: 1.0 }
                })
            })
            .collect();
        Self { cfg: *cfg, sites, regions, planet_at, planets: list, fleets, stock: [0; 2], turn: 1, last_turn: String::new() }
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
                    } else if fleet.is_some_and(|f| self.fleets[f].strength > pl.defence) {
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

    /// Spends from the fleet's side to add strength. False if it can't.
    pub fn reinforce(&mut self, fleet: usize) -> bool {
        let Some(f) = self.fleets.get(fleet) else { return false };
        if self.stock[f.side] < REINFORCE_COST || f.strength >= MAX_STRENGTH {
            return false;
        }
        self.stock[f.side] -= REINFORCE_COST;
        let f = &mut self.fleets[fleet];
        f.strength = (f.strength + REINFORCE_STEP).min(MAX_STRENGTH);
        true
    }

    /// The fleet within `r` pixels of `p`, nearest first.
    pub fn fleet_at(&self, p: Vec2, r: f32) -> Option<usize> {
        let pos: Vec<Vec2> = self.fleets.iter().map(|f| f.pos).collect();
        crate::voronoi::labs::site_at(&pos, p, r)
    }
}

/// The fleets as weighted influence bases, built once per batch of queries.
pub struct Influence {
    bases: Vec<Vec2>,
    weights: Vec<f32>,
    sides: Vec<usize>,
    metric: Metric,
}

impl Influence {
    pub fn new(d: &Dominion) -> Self {
        Self {
            bases: d.fleets.iter().map(|f| f.pos).collect(),
            weights: d.fleets.iter().map(|f| f.strength).collect(),
            sides: d.fleets.iter().map(|f| f.side).collect(),
            metric: d.cfg.metric,
        }
    }

    /// The controlling side at `p` and the fleet that dominates it.
    pub fn at(&self, p: Vec2) -> (Control, Option<usize>) {
        let inf = weighted_scores(&self.bases, &self.weights, p, self.metric);
        let Some(best) = inf.owner else { return (Control::Neutral, None) };
        let side = self.sides[best];
        // The other side's best score, not just the runner-up fleet.
        let rival = self
            .bases
            .iter()
            .zip(&self.weights)
            .zip(&self.sides)
            .filter(|((_, w), s)| **s != side && **w > 0.0)
            .map(|((b, w), _)| distance(*b, p, self.metric) / w)
            .fold(f32::INFINITY, f32::min);
        if rival <= inf.best * (1.0 + CONTEST_MARGIN) { (Control::Contested, Some(best)) } else { (Control::Side(side), Some(best)) }
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
const CONTESTED: Rgb = [120, 110, 130];
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

pub fn render(d: &Dominion, opts: DrawOptions) -> Canvas {
    let (w, h) = (d.cfg.w, d.cfg.h);
    let mut c = Canvas::new(w, h, SPACE);
    // Fleet territory, coarse when `step` > 1.
    let step = opts.step.max(1);
    let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
    let inf = Influence::new(d);
    let mut terr = vec![Control::Neutral; (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let p = Vec2::new((gx * step) as f32 + step as f32 * 0.5, (gy * step) as f32 + step as f32 * 0.5);
            terr[(gy * gw + gx) as usize] = inf.at(p).0;
        }
    }
    let at = |x: u32, y: u32| terr[((y / step) * gw + x / step) as usize];
    let side_of = |c: Control| if let Control::Side(s) = c { Some(s) } else { None };
    for y in 0..h {
        for x in 0..w {
            let site = d.regions.at(x, y);
            let planet = if site == NO_OWNER { None } else { d.planet_at[site as usize] };
            let mut col = match planet {
                // Planet regions show their owner.
                Some(pi) if opts.regions => {
                    let pl = &d.planets[pi];
                    let k = if matches!(pl.owner, Owner::Side(_)) { 0.5 + 0.08 * pl.value as f32 } else { 1.0 };
                    if d.regions.is_border(x, y) { INK } else { Canvas::shade_px(owner_color(pl.owner), k) }
                }
                // Empty space (and everything, with regions off) shows the
                // fleets' reach, dimly.
                _ => match at(x, y) {
                    Control::Side(s) => Canvas::shade_px(SIDE_COLORS[s], 0.2),
                    Control::Contested if (x + y) % 8 < 4 => Canvas::shade_px(CONTESTED, 0.35),
                    _ => SPACE,
                },
            };
            if opts.borders {
                let here = side_of(at(x, y));
                let right = if x + 1 < w { side_of(at(x + 1, y)) } else { here };
                let down = if y + 1 < h { side_of(at(x, y + 1)) } else { here };
                if here.is_some() && ((right.is_some() && right != here) || (down.is_some() && down != here)) {
                    col = WHITE;
                }
            }
            c.set(x as i32, y as i32, col);
        }
    }
    // Planets: size by value, grey ring by defence, gold ring if it falls
    // at the end of this turn.
    for (p, pr) in d.planets.iter().zip(d.pressure()) {
        let r = 1.5 + 1.3 * p.value as f32;
        c.circle(p.pos, r + 2.0 + 2.0 * p.defence, [170, 170, 180], 0.7);
        match pr {
            Pressure::Falls(s) => {
                c.circle(p.pos, r + 4.0 + 2.0 * p.defence, GOLD, 1.0);
                c.circle(p.pos, r + 5.0 + 2.0 * p.defence, SIDE_COLORS[s], 1.0);
            }
            Pressure::Contested => c.circle(p.pos, r + 4.0 + 2.0 * p.defence, WHITE, 0.8),
            _ => {}
        }
        c.disc(p.pos, r + 1.0, INK, 1.0);
        c.disc(p.pos, r, [230, 220, 170], 1.0);
    }
    // Fleets: a ring sized by strength.
    for (i, f) in d.fleets.iter().enumerate() {
        let col = SIDE_COLORS[f.side];
        c.circle(f.pos, 9.0 + 6.0 * f.strength, col, 0.9);
        if opts.selected == Some(i) {
            c.circle(f.pos, 11.0 + 6.0 * f.strength, GOLD, 1.0);
            c.disc(f.pos, 10.0, GOLD, 1.0);
        }
        c.disc(f.pos, 8.0, INK, 1.0);
        c.disc(f.pos, 6.5, col, 1.0);
        c.disc(f.pos, 2.0, WHITE, 1.0);
    }
    c
}

/// The status lines for the page.
pub fn status(d: &Dominion, selected: Option<usize>) -> String {
    let income = d.income();
    let (held, free) = d.holdings();
    let falling = d.pressure().iter().filter(|p| matches!(p, Pressure::Falls(_))).count();
    let sel = match selected.and_then(|i| d.fleets.get(i).map(|f| (i, f))) {
        Some((i, f)) => format!(
            "Selected: {} fleet {}, strength {:.1}.",
            SIDE_NAMES[f.side],
            d.fleets[..=i].iter().filter(|g| g.side == f.side).count(),
            f.strength
        ),
        None => "Click a fleet to select it.".into(),
    };
    let mut s = format!(
        "Turn {} · Blue {} (+{}/turn, {} planets) · Red {} (+{}/turn, {}) · {} independent · {} fall at end of turn\n{sel}",
        d.turn, d.stock[0], income[0], held[0], d.stock[1], income[1], held[1], free, falling
    );
    if !d.last_turn.is_empty() {
        s.push(' ');
        s.push_str(&d.last_turn);
    }
    s
}
