//! prototype01 — Fleet dominion. Planets are Voronoi regions that produce
//! resources; fleets project weighted influence (distance ÷ strength), and
//! whichever side's fleet dominates a planet collects its income. Contested
//! planets pay nobody. Each turn adds income to a stockpile that buys
//! fleet strength.

use crate::voronoi::Vec2;
use crate::voronoi::grid::{NO_OWNER, OwnerGrid, voronoi_grid};
use crate::voronoi::influence::weighted_scores;
use crate::voronoi::raster::{Canvas, Rgb};
use crate::voronoi::rng::ParkMiller;
use crate::voronoi::sites::{VoronoiConfig, make_sites};

pub const FLEETS_PER_SIDE: usize = 3;
pub const MIN_STRENGTH: f32 = 0.2;
pub const MAX_STRENGTH: f32 = 4.0;
pub const REINFORCE_COST: u32 = 10;
pub const REINFORCE_STEP: f32 = 0.5;
/// The other side's best fleet within this fraction contests the ground.
pub const CONTEST_MARGIN: f32 = 0.10;
pub const SIDE_NAMES: [&str; 2] = ["Blue", "Red"];
const SIDE_COLORS: [Rgb; 2] = [[70, 130, 220], [215, 80, 70]];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Planet {
    pub pos: Vec2,
    /// Resources a turn: 0 (deep space) to 3 (rich).
    pub value: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FleetToken {
    pub side: usize,
    pub pos: Vec2,
    pub strength: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Side(usize),
    Contested,
    Neutral,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dominion {
    pub cfg: VoronoiConfig,
    pub planets: Vec<Planet>,
    /// Which planet owns each pixel (the planet regions).
    pub regions: OwnerGrid,
    pub fleets: Vec<FleetToken>,
    pub stock: [u32; 2],
    pub turn: u32,
}

impl Dominion {
    pub fn new(cfg: &VoronoiConfig) -> Self {
        let sites = cfg.relaxed_sites();
        let regions = voronoi_grid(&sites, cfg.w, cfg.h, cfg.metric);
        let edge = regions.edge_owners();
        let mut rng = ParkMiller::new(cfg.seed as i64 + 11);
        let planets = sites
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let roll = rng.next_float();
                let value = if edge.contains(&(i as u32)) {
                    0
                } else if roll < 0.2 {
                    3
                } else if roll < 0.7 {
                    2
                } else {
                    1
                };
                Planet { pos: *s, value }
            })
            .collect();
        // Blue starts in the left third, Red in the right third.
        let (w, h) = (cfg.w as f32, cfg.h as f32);
        // Red mirrors Blue so neither side starts with better positions.
        let spots = make_sites(FLEETS_PER_SIDE, w / 3.0, h * 0.8, cfg.seed.wrapping_add(23));
        let fleets = (0..2)
            .flat_map(|side| {
                spots.iter().map(move |p| {
                    let x = if side == 0 { p.x } else { w - p.x };
                    FleetToken { side, pos: Vec2::new(x, p.y + h * 0.1), strength: 1.0 }
                })
            })
            .collect();
        Self { cfg: *cfg, planets, regions, fleets, stock: [0; 2], turn: 1 }
    }

    /// Which side dominates point `p`, by its best fleet.
    pub fn control_at(&self, p: Vec2) -> Control {
        Influence::new(self).at(p)
    }

    /// Control of each planet, sampled at the planet itself.
    pub fn control(&self) -> Vec<Control> {
        let inf = Influence::new(self);
        self.planets.iter().map(|p| inf.at(p.pos)).collect()
    }

    pub fn income(&self) -> [u32; 2] {
        let mut out = [0; 2];
        for (p, c) in self.planets.iter().zip(self.control()) {
            if let Control::Side(s) = c {
                out[s] += p.value;
            }
        }
        out
    }

    /// Planets held by each side, and how many are contested.
    pub fn holdings(&self) -> ([u32; 2], u32) {
        let mut held = [0; 2];
        let mut contested = 0;
        for (p, c) in self.planets.iter().zip(self.control()) {
            if p.value == 0 {
                continue;
            }
            match c {
                Control::Side(s) => held[s] += 1,
                Control::Contested => contested += 1,
                Control::Neutral => {}
            }
        }
        (held, contested)
    }

    pub fn end_turn(&mut self) {
        let income = self.income();
        for (stock, inc) in self.stock.iter_mut().zip(income) {
            *stock += inc;
        }
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

/// The fleets as weighted influence bases, built once per query batch.
pub struct Influence {
    bases: Vec<Vec2>,
    weights: Vec<f32>,
    sides: Vec<usize>,
    metric: crate::voronoi::metric::Metric,
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

    pub fn at(&self, p: Vec2) -> Control {
        let inf = weighted_scores(&self.bases, &self.weights, p, self.metric);
        let Some(best) = inf.owner else { return Control::Neutral };
        let side = self.sides[best];
        // The other side's best score, not just the runner-up fleet.
        let rival = self
            .bases
            .iter()
            .zip(&self.weights)
            .zip(&self.sides)
            .filter(|((_, w), s)| **s != side && **w > 0.0)
            .map(|((b, w), _)| crate::voronoi::metric::distance(*b, p, self.metric) / w)
            .fold(f32::INFINITY, f32::min);
        if rival <= inf.best * (1.0 + CONTEST_MARGIN) { Control::Contested } else { Control::Side(side) }
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
const NEUTRAL: Rgb = [40, 44, 56];
const CONTESTED: Rgb = [120, 110, 130];
const WHITE: Rgb = [240, 240, 240];
const INK: Rgb = [10, 10, 14];
const GOLD: Rgb = [235, 190, 80];

pub fn render(d: &Dominion, opts: DrawOptions) -> Canvas {
    let (w, h) = (d.cfg.w, d.cfg.h);
    let mut c = Canvas::new(w, h, SPACE);
    let planet_control = d.control();
    // Territory: the side dominating each pixel, coarse when `step` > 1.
    let step = opts.step.max(1);
    let (gw, gh) = (w.div_ceil(step), h.div_ceil(step));
    let inf = Influence::new(d);
    let mut terr = vec![Control::Neutral; (gw * gh) as usize];
    for gy in 0..gh {
        for gx in 0..gw {
            let p = Vec2::new((gx * step) as f32 + step as f32 * 0.5, (gy * step) as f32 + step as f32 * 0.5);
            terr[(gy * gw + gx) as usize] = inf.at(p);
        }
    }
    let at = |x: u32, y: u32| terr[((y / step) * gw + x / step) as usize];
    for y in 0..h {
        for x in 0..w {
            let region = d.regions.at(x, y);
            let value = if region == NO_OWNER { 0 } else { d.planets[region as usize].value };
            // Planet regions take their planet's controller; deep space and
            // (with regions off) everything shows raw fleet influence, dimmer.
            let (ctrl, k) = if opts.regions && value > 0 { (planet_control[region as usize], 0.55) } else { (at(x, y), 0.22) };
            let mut col = match ctrl {
                Control::Side(s) => Canvas::shade_px(SIDE_COLORS[s], k + 0.08 * value as f32),
                Control::Contested => {
                    if (x + y) % 8 < 4 {
                        Canvas::shade_px(CONTESTED, k + 0.2)
                    } else {
                        NEUTRAL
                    }
                }
                Control::Neutral => {
                    if value > 0 {
                        NEUTRAL
                    } else {
                        SPACE
                    }
                }
            };
            if opts.regions && value > 0 && d.regions.is_border(x, y) {
                col = INK;
            }
            if opts.borders {
                let here = at(x, y);
                let right = if x + 1 < w { at(x + 1, y) } else { here };
                let down = if y + 1 < h { at(x, y + 1) } else { here };
                let side_of = |c: Control| if let Control::Side(s) = c { Some(s) } else { None };
                if (side_of(here) != side_of(right) && side_of(here).is_some() && side_of(right).is_some())
                    || (side_of(here) != side_of(down) && side_of(here).is_some() && side_of(down).is_some())
                {
                    col = WHITE;
                }
            }
            c.set(x as i32, y as i32, col);
        }
    }
    // Planets, sized by value.
    for p in &d.planets {
        if p.value > 0 {
            let r = 1.5 + 1.3 * p.value as f32;
            c.disc(p.pos, r + 1.0, INK, 1.0);
            c.disc(p.pos, r, [230, 220, 170], 1.0);
        }
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

/// One status line for the page.
pub fn status(d: &Dominion, selected: Option<usize>) -> String {
    let income = d.income();
    let (held, contested) = d.holdings();
    let sel = match selected.and_then(|i| d.fleets.get(i).map(|f| (i, f))) {
        Some((i, f)) => format!(
            " · selected: {} fleet {}, strength {:.1}",
            SIDE_NAMES[f.side],
            d.fleets[..=i].iter().filter(|g| g.side == f.side).count(),
            f.strength
        ),
        None => " · click a fleet to select it".into(),
    };
    format!(
        "Turn {} · Blue {} (+{}/turn, {} planets) · Red {} (+{}/turn, {}) · {} contested{sel}",
        d.turn, d.stock[0], income[0], held[0], d.stock[1], income[1], held[1], contested
    )
}
