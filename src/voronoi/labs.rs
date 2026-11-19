//! The nine tutorial labs as pure renders: the shared config plus a lab's
//! own state in, one RGB image out. The Bevy page and the PPM dumper both
//! call [`render`].

use super::Vec2;
use super::biomes::assign_biomes;
use super::delaunay::Delaunay;
use super::grid::{NO_OWNER, voronoi_grid_scaled};
use super::influence::weighted_scores;
use super::metric::{Metric, SiteIndex, distance};
use super::noise::{NoiseMode, NoisePreset, shade};
use super::paint::paint_island;
use super::raster::{Canvas, Rgb};
use super::relax::{centroids, relax_step};
use super::sites::{VoronoiConfig, make_sites};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LabId {
    Grow,
    Nearest,
    OwnerMap,
    Delaunay,
    Lloyd,
    Biomes,
    Influence,
    CellNoise,
    Island,
}

impl LabId {
    pub const ALL: [LabId; 9] = [
        LabId::Grow,
        LabId::Nearest,
        LabId::OwnerMap,
        LabId::Delaunay,
        LabId::Lloyd,
        LabId::Biomes,
        LabId::Influence,
        LabId::CellNoise,
        LabId::Island,
    ];

    pub fn title(self) -> &'static str {
        match self {
            LabId::Grow => "Grow the seeds",
            LabId::Nearest => "Nearest site",
            LabId::OwnerMap => "Owner map",
            LabId::Delaunay => "Delaunay & exact cells",
            LabId::Lloyd => "Lloyd relaxation",
            LabId::Biomes => "Biome island",
            LabId::Influence => "Faction territories",
            LabId::CellNoise => "Cell noise",
            LabId::Island => "Paint the island",
        }
    }

    pub fn blurb(self) -> &'static str {
        match self {
            LabId::Grow => "Every seed grows at the same speed; patches stop where they meet, halfway between seeds.",
            LabId::Nearest => "Drag the probe: the nearest site wins. The outline shows 'equally far' for the metric.",
            LabId::OwnerMap => "Every pixel coloured by its nearest site; borders are where neighbours differ.",
            LabId::Delaunay => "Exact cell polygons from the Delaunay dual (Euclidean). Click a site for its neighbours, drag it, click empty space to add one.",
            LabId::Lloyd => "Move each site to its cell's centroid and rebuild: cells even out toward a honeycomb.",
            LabId::Biomes => "Edge cells are ocean so land is an island; inland cells get a biome from latitude plus jitter.",
            LabId::Influence => "Faction territories: distance ÷ weight, so strong bases reach further and borders curve. Drag or destroy bases.",
            LabId::CellNoise => "Worley noise from the nearest (F1) and second-nearest (F2) distances; F2 − F1 finds the cracks.",
            LabId::Island => "Biomes, edge shading, roads and towns painted into one image. Hover for a closer look.",
        }
    }
}

pub const FACTIONS: [(&str, Rgb); 5] =
    [("Tide", [70, 140, 220]), ("Ember", [222, 110, 60]), ("Grove", [90, 170, 90]), ("Frost", [170, 200, 230]), ("Dusk", [150, 100, 190])];

/// Every lab's own controls and edits. Reset when the shared config changes.
#[derive(Clone, Debug, PartialEq)]
pub struct LabState {
    // Grow the seeds
    pub grow_radius: f32,
    pub grow_fronts: bool,
    // Nearest site
    pub probe: Vec2,
    pub distance_lines: bool,
    // Owner map
    pub owner_borders: bool,
    // Delaunay
    pub show_cells: bool,
    pub show_triangles: bool,
    pub show_circles: bool,
    pub selected: Option<usize>,
    /// Sites after adds and drags; `None` = the config's sites.
    pub edited: Option<Vec<Vec2>>,
    // Lloyd
    pub lloyd: Option<Vec<Vec2>>,
    pub lloyd_round: u32,
    pub lloyd_moved: f32,
    pub show_centroids: bool,
    // Biomes
    pub biome_borders: bool,
    pub biome_sites: bool,
    // Influence
    pub bases: Vec<Vec2>,
    pub weights: Vec<f32>,
    pub alive: Vec<bool>,
    pub use_weights: bool,
    // Cell noise
    pub noise_mode: NoiseMode,
    pub preset: NoisePreset,
    pub crack: f32,
    // Island
    pub world: u32,
}

impl LabState {
    pub fn new(cfg: &VoronoiConfig) -> Self {
        Self {
            grow_radius: 40.0,
            grow_fronts: true,
            probe: Vec2::new(cfg.w as f32 * 0.5, cfg.h as f32 * 0.5),
            distance_lines: true,
            owner_borders: true,
            show_cells: true,
            show_triangles: false,
            show_circles: false,
            selected: None,
            edited: None,
            lloyd: None,
            lloyd_round: 0,
            lloyd_moved: f32::INFINITY,
            show_centroids: true,
            biome_borders: true,
            biome_sites: false,
            bases: make_sites(FACTIONS.len(), cfg.w as f32, cfg.h as f32, cfg.seed.wrapping_add(7)),
            weights: vec![2.2, 1.0, 1.0, 1.0, 1.0],
            alive: vec![true; FACTIONS.len()],
            use_weights: true,
            noise_mode: NoiseMode::F2MinusF1,
            preset: NoisePreset::Stone,
            crack: 2.5,
            world: 0,
        }
    }

    /// After the shared config changes: drop edits that belonged to the old
    /// sites, keep the toggles.
    pub fn reseed(&mut self, cfg: &VoronoiConfig) {
        let fresh = LabState::new(cfg);
        self.probe = fresh.probe;
        self.selected = None;
        self.edited = None;
        self.lloyd = None;
        self.lloyd_round = 0;
        self.lloyd_moved = f32::INFINITY;
        self.bases = fresh.bases;
        self.alive = fresh.alive;
        self.grow_radius = self.grow_radius.min(LabState::grow_max(cfg));
    }

    /// The sites a lab works on.
    pub fn sites(&self, cfg: &VoronoiConfig, lab: LabId) -> Vec<Vec2> {
        match lab {
            LabId::Delaunay => self.edited.clone().unwrap_or_else(|| cfg.raw_sites()),
            LabId::Lloyd => self.lloyd.clone().unwrap_or_else(|| cfg.raw_sites()),
            LabId::Biomes => cfg.relaxed_sites(),
            LabId::Influence => self.bases.clone(),
            _ => cfg.raw_sites(),
        }
    }

    /// One Lloyd round on the Lloyd lab's sites.
    pub fn lloyd_step(&mut self, cfg: &VoronoiConfig) {
        let sites = self.sites(cfg, LabId::Lloyd);
        let (next, moved) = relax_step(&sites, cfg.w, cfg.h, cfg.metric);
        self.lloyd = Some(next);
        self.lloyd_round += 1;
        self.lloyd_moved = moved;
    }

    pub fn lloyd_settled(&self) -> bool {
        self.lloyd_moved < 0.5
    }

    /// Weights as the territory map uses them (destroyed bases are 0).
    pub fn effective_weights(&self) -> Vec<f32> {
        self.weights
            .iter()
            .zip(&self.alive)
            .map(|(w, a)| if !a { 0.0 } else if self.use_weights { *w } else { 1.0 })
            .collect()
    }

    /// Radius at which every pixel is reached (for the grow animation).
    pub fn grow_max(cfg: &VoronoiConfig) -> f32 {
        (cfg.w as f32).hypot(cfg.h as f32)
    }
}

const BG: Rgb = [14, 17, 26];
const INK: Rgb = [17, 17, 17];
const WHITE: Rgb = [240, 240, 240];
const GOLD: Rgb = [235, 190, 80];

/// A distinct colour per cell.
pub fn cell_color(i: usize) -> Rgb {
    const P: [Rgb; 12] = [
        [88, 129, 87],
        [214, 140, 69],
        [70, 130, 180],
        [191, 99, 120],
        [140, 110, 190],
        [200, 180, 90],
        [80, 160, 160],
        [170, 90, 70],
        [110, 150, 210],
        [150, 170, 100],
        [210, 120, 150],
        [120, 120, 160],
    ];
    Canvas::shade_px(P[i % P.len()], [1.0, 0.82, 1.15][(i / P.len()) % 3])
}

/// The nearest site within `r` pixels of `p`.
pub fn site_at(sites: &[Vec2], p: Vec2, r: f32) -> Option<usize> {
    sites
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.distance(p)))
        .filter(|(_, d)| *d <= r)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

fn dots(c: &mut Canvas, sites: &[Vec2], r: f32) {
    for s in sites {
        c.disc(*s, r + 1.0, INK, 1.0);
        c.disc(*s, r, WHITE, 1.0);
    }
}

/// Fills cells from an owner grid; `borders` darkens cell edges.
fn paint_cells(c: &mut Canvas, cfg: &VoronoiConfig, sites: &[Vec2], step: u32, brightness: f32, borders: bool) {
    let grid = voronoi_grid_scaled(sites, cfg.w, cfg.h, cfg.metric, step);
    for y in 0..cfg.h {
        for x in 0..cfg.w {
            let o = grid.at(x, y);
            if o == NO_OWNER {
                continue;
            }
            let col = if borders && grid.is_border(x, y) { INK } else { Canvas::shade_px(cell_color(o as usize), brightness) };
            c.set(x as i32, y as i32, col);
        }
    }
}

/// Renders one lab. `step` > 1 samples per-pixel work coarsely (quick
/// previews while dragging).
pub fn render(cfg: &VoronoiConfig, lab: LabId, st: &LabState, step: u32) -> Canvas {
    let (w, h) = (cfg.w, cfg.h);
    let mut c = Canvas::new(w, h, BG);
    let sites = st.sites(cfg, lab);
    match lab {
        LabId::Grow => {
            let index = SiteIndex::new(&sites, cfg.metric);
            let r = st.grow_radius;
            let grid = voronoi_grid_scaled(&sites, w, h, cfg.metric, step);
            for y in 0..h {
                for x in 0..w {
                    let n = index.nearest_two(Vec2::new(x as f32 + 0.5, y as f32 + 0.5));
                    if n.nearest == usize::MAX || n.d1 > r {
                        continue;
                    }
                    let col = if grid.is_border(x, y) {
                        INK
                    } else if st.grow_fronts && n.d1 > r - 1.5 {
                        WHITE
                    } else {
                        cell_color(n.nearest)
                    };
                    c.set(x as i32, y as i32, col);
                }
            }
            dots(&mut c, &sites, 2.5);
        }
        LabId::Nearest => {
            paint_cells(&mut c, cfg, &sites, step, 0.45, true);
            let p = st.probe;
            if let Some(n) = super::metric::nearest_site(&sites, p, cfg.metric) {
                if st.distance_lines {
                    for s in &sites {
                        c.line(p, *s, 1.0, WHITE, 0.25);
                    }
                }
                let d = distance(sites[n], p, cfg.metric);
                c.line(p, sites[n], 2.0, GOLD, 1.0);
                let shape: Vec<Vec2> = match cfg.metric {
                    Metric::Euclidean => (0..96).map(|k| p + Vec2::from_angle(k as f32 / 96.0 * std::f32::consts::TAU) * d).collect(),
                    Metric::Manhattan => vec![p + Vec2::new(d, 0.0), p + Vec2::new(0.0, d), p - Vec2::new(d, 0.0), p - Vec2::new(0.0, d)],
                    Metric::Chebyshev => vec![p + Vec2::new(d, d), p + Vec2::new(-d, d), p - Vec2::new(d, d), p + Vec2::new(d, -d)],
                };
                c.polygon_stroke(&shape, 1.5, GOLD, 0.9);
                dots(&mut c, &sites, 2.5);
                c.disc(sites[n], 5.0, GOLD, 1.0);
            }
            c.disc(p, 5.0, INK, 1.0);
            c.disc(p, 4.0, [255, 80, 80], 1.0);
        }
        LabId::OwnerMap => {
            paint_cells(&mut c, cfg, &sites, step, 1.0, st.owner_borders);
            dots(&mut c, &sites, 2.0);
        }
        LabId::Delaunay => {
            let d = Delaunay::build(&sites);
            let (fw, fh) = (w as f32, h as f32);
            if st.show_cells {
                for i in 0..sites.len() {
                    let poly = d.cell_polygon(i, fw, fh);
                    c.polygon_fill(&poly, cell_color(i), 1.0);
                }
                if let Some(s) = st.selected.filter(|s| *s < sites.len()) {
                    for &n in &d.neighbors[s] {
                        c.polygon_fill(&d.cell_polygon(n, fw, fh), GOLD, 0.35);
                    }
                    c.polygon_fill(&d.cell_polygon(s, fw, fh), WHITE, 0.5);
                }
                for i in 0..sites.len() {
                    c.polygon_stroke(&d.cell_polygon(i, fw, fh), 1.5, INK, 1.0);
                }
            }
            if st.show_triangles {
                for t in 0..d.triangle_count() {
                    let tri = d.triangle(t);
                    c.polygon_stroke(&tri, 1.0, WHITE, 0.7);
                }
            }
            if st.show_circles {
                for t in 0..d.triangle_count() {
                    if let Some((centre, r)) = d.circumcircle(t) {
                        c.circle(centre, r, GOLD, 0.45);
                    }
                }
            }
            if let Some(s) = st.selected.filter(|s| *s < sites.len()) {
                for &n in &d.neighbors[s] {
                    c.line(sites[s], sites[n], 2.0, GOLD, 1.0);
                }
            }
            c.polygon_stroke(&[Vec2::ZERO, Vec2::new(fw - 1.0, 0.0), Vec2::new(fw - 1.0, fh - 1.0), Vec2::new(0.0, fh - 1.0)], 1.0, INK, 1.0);
            dots(&mut c, &sites, 3.0);
        }
        LabId::Lloyd => {
            paint_cells(&mut c, cfg, &sites, step, 1.0, true);
            if st.show_centroids {
                let grid = voronoi_grid_scaled(&sites, w, h, cfg.metric, step);
                for (s, ce) in sites.iter().zip(centroids(sites.len(), &grid)) {
                    if let Some(ce) = ce {
                        c.disc(ce, 2.0, GOLD, 1.0);
                        c.arrow(*s, ce, WHITE, 1.0);
                    }
                }
            }
            dots(&mut c, &sites, 2.5);
        }
        LabId::Biomes => {
            let grid = voronoi_grid_scaled(&sites, w, h, cfg.metric, step);
            let biomes = assign_biomes(&sites, &grid, cfg.seed);
            for y in 0..h {
                for x in 0..w {
                    let o = grid.at(x, y);
                    if o == NO_OWNER {
                        continue;
                    }
                    let mut col = biomes[o as usize].color();
                    if st.biome_borders && grid.is_border(x, y) {
                        col = Canvas::shade_px(col, 0.6);
                    }
                    c.set(x as i32, y as i32, col);
                }
            }
            if st.biome_sites {
                dots(&mut c, &sites, 2.0);
            }
        }
        LabId::Influence => {
            let weights = st.effective_weights();
            let step = step.max(1);
            for by in (0..h).step_by(step as usize) {
                for bx in (0..w).step_by(step as usize) {
                    let p = Vec2::new(bx as f32 + step as f32 * 0.5, by as f32 + step as f32 * 0.5);
                    let inf = weighted_scores(&sites, &weights, p, cfg.metric);
                    let Some(o) = inf.owner else { continue };
                    let col = Canvas::shade_px(FACTIONS[o % FACTIONS.len()].1, 0.75);
                    for y in by..(by + step).min(h) {
                        for x in bx..(bx + step).min(w) {
                            let px = if inf.contested(0.06) && (x + y) % 6 < 3 { Canvas::shade_px(col, 0.55) } else { col };
                            c.set(x as i32, y as i32, px);
                        }
                    }
                }
            }
            for (i, b) in sites.iter().enumerate() {
                if st.alive.get(i).copied().unwrap_or(true) {
                    let col = FACTIONS[i % FACTIONS.len()].1;
                    if st.use_weights {
                        c.circle(*b, 8.0 + 6.0 * st.weights[i], WHITE, 0.6);
                    }
                    c.disc(*b, 8.0, INK, 1.0);
                    c.disc(*b, 6.5, col, 1.0);
                } else {
                    c.cross(*b, 6.0, [120, 120, 120]);
                }
            }
        }
        LabId::CellNoise => {
            let index = SiteIndex::new(&sites, cfg.metric);
            let scale = ((w * h) as f32 / sites.len().max(1) as f32).sqrt() * 0.6;
            let step = step.max(1);
            for by in (0..h).step_by(step as usize) {
                for bx in (0..w).step_by(step as usize) {
                    let p = Vec2::new(bx as f32 + 0.5, by as f32 + 0.5);
                    let col = shade(&index, p, st.noise_mode, st.preset, st.crack, scale);
                    for y in by..(by + step).min(h) {
                        for x in bx..(bx + step).min(w) {
                            c.set(x as i32, y as i32, col);
                        }
                    }
                }
            }
        }
        LabId::Island => {
            c = paint_island(w, h, cfg.sites, cfg.seed.wrapping_add(st.world), cfg.relax);
        }
    }
    c
}

/// A 2× zoom of the area around `at`, drawn in the corner away from it.
pub fn zoom_inset(base: &Canvas, at: Vec2) -> Canvas {
    let mut c = base.clone();
    let (sw, sh) = (110u32, 70u32);
    let (bw, bh) = (sw * 2, sh * 2);
    if base.w < bw + 20 || base.h < bh + 20 {
        return c;
    }
    let sx = (at.x as i32 - sw as i32 / 2).clamp(0, (base.w - sw) as i32) as u32;
    let sy = (at.y as i32 - sh as i32 / 2).clamp(0, (base.h - sh) as i32) as u32;
    let ox = if at.x < base.w as f32 * 0.5 { base.w - bw - 10 } else { 10 };
    let oy = if at.y < base.h as f32 * 0.5 { base.h - bh - 10 } else { 10 };
    for y in 0..bh {
        for x in 0..bw {
            c.set((ox + x) as i32, (oy + y) as i32, base.get(sx + x / 2, sy + y / 2));
        }
    }
    let r = |x: u32, y: u32| Vec2::new(x as f32, y as f32);
    c.polygon_stroke(&[r(ox, oy), r(ox + bw, oy), r(ox + bw, oy + bh), r(ox, oy + bh)], 2.0, WHITE, 1.0);
    c.polygon_stroke(&[r(sx, sy), r(sx + sw, sy), r(sx + sw, sy + sh), r(sx, sy + sh)], 1.0, WHITE, 0.8);
    c
}

/// One line about the lab's current state, for the page.
pub fn status(cfg: &VoronoiConfig, lab: LabId, st: &LabState) -> String {
    let sites = st.sites(cfg, lab);
    match lab {
        LabId::Grow => format!("Grow radius {:.0} px · {} seeds · {}", st.grow_radius, sites.len(), cfg.metric.name()),
        LabId::Nearest => match super::metric::nearest_site(&sites, st.probe, cfg.metric) {
            Some(n) => format!(
                "Probe ({:.0}, {:.0}) → site #{n}, {} distance {:.1}",
                st.probe.x,
                st.probe.y,
                cfg.metric.name(),
                distance(sites[n], st.probe, cfg.metric)
            ),
            None => "No sites.".into(),
        },
        LabId::OwnerMap => format!(
            "{}×{} pixels × {} sites ≈ {:.1} million distance checks (brute force)",
            cfg.w,
            cfg.h,
            sites.len(),
            (cfg.w * cfg.h) as f32 * sites.len() as f32 / 1e6
        ),
        LabId::Delaunay => {
            let d = Delaunay::build(&sites);
            match st.selected.filter(|s| *s < sites.len()) {
                Some(s) => format!("Site #{s}: {} neighbours {:?} · {} triangles", d.neighbors[s].len(), d.neighbors[s], d.triangle_count()),
                None => format!("{} sites · {} triangles · click a site to see its neighbours", sites.len(), d.triangle_count()),
            }
        }
        LabId::Lloyd => {
            if st.lloyd_round == 0 {
                "Round 0 · arrows point from each site to its cell's centroid".into()
            } else {
                format!("Round {} · largest move {:.2} px{}", st.lloyd_round, st.lloyd_moved, if st.lloyd_settled() { " · settled" } else { "" })
            }
        }
        LabId::Biomes => format!("{} regions after {} Lloyd round(s) · seed {}", sites.len(), cfg.relax, cfg.seed),
        LabId::Influence => {
            let alive = st.alive.iter().filter(|a| **a).count();
            format!("{alive} of {} bases standing · weights {}", FACTIONS.len(), if st.use_weights { "on" } else { "off" })
        }
        LabId::CellNoise => format!("{} · {} · crack width {:.1}", st.noise_mode.name(), st.preset.name(), st.crack),
        LabId::Island => format!("{} regions · seed {} · {} Lloyd round(s)", cfg.sites, cfg.seed.wrapping_add(st.world), cfg.relax),
    }
}
