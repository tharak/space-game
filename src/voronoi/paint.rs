//! "Paint the island" (tutorial step 9): biomes, edge shading, roads and
//! towns combined into one RGB image.

use std::collections::BTreeSet;

use super::Vec2;
use super::biomes::{Biome, assign_biomes};
use super::grid::voronoi_grid;
use super::metric::{Metric, SiteIndex};
use super::raster::{Canvas, Rgb};
use super::relax::relax_step;
use super::rng::ParkMiller;
use super::sites::make_sites;

const OCEAN: Rgb = [26, 74, 122];
const SAND: Rgb = [232, 214, 160];
const ROAD: Rgb = [96, 66, 42];
const TOWN: Rgb = [60, 44, 36];
const TOWN_CENTRE: Rgb = [245, 240, 225];
/// Light comes from the upper left.
const LIGHT: Vec2 = Vec2::new(-0.6, -0.8);

/// Writes `color × light` (clamped) at a pixel: the tutorial's `setPixel`.
pub fn set_pixel(c: &mut Canvas, x: u32, y: u32, color: Rgb, light: f32) {
    c.set(x as i32, y as i32, Canvas::shade_px(color, light));
}

/// The tutorial's `stampCircle`: an alpha-blended filled circle.
pub fn stamp_circle(c: &mut Canvas, centre: Vec2, radius: f32, color: Rgb, opacity: f32) {
    c.disc(centre, radius, color, opacity);
}

/// Distance from `p` to the bisector between sites `a` (nearest) and `b`.
fn edge_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = a.distance(b);
    if ab < 1e-4 {
        return 0.0;
    }
    (p.distance_squared(b) - p.distance_squared(a)) / (2.0 * ab)
}

pub fn paint_island(w: u32, h: u32, regions: usize, seed: u32, relax: u32) -> Canvas {
    // Pass 1: base map.
    let mut sites = make_sites(regions, w as f32, h as f32, seed);
    for _ in 0..relax {
        sites = relax_step(&sites, w, h, Metric::Euclidean).0;
    }
    let grid = voronoi_grid(&sites, w, h, Metric::Euclidean);
    let biomes = assign_biomes(&sites, &grid, seed);
    let mut tone_rng = ParkMiller::new(seed as i64 + 1);
    let tones: Vec<f32> = sites.iter().map(|_| 0.92 + tone_rng.next_float() * 0.16).collect();
    let index = SiteIndex::new(&sites, Metric::Euclidean);
    let mut c = Canvas::new(w, h, OCEAN);
    if sites.is_empty() {
        return c;
    }

    // Pass 2: per-pixel shading.
    for y in 0..h {
        for x in 0..w {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let n = index.nearest_two(p);
            let (i, j) = (n.nearest, n.second);
            let biome = biomes[i];
            let (edge, other) = if j == usize::MAX {
                (f32::INFINITY, biome)
            } else {
                (edge_distance(p, sites[i], sites[j]), biomes[j])
            };
            if biome == Biome::Ocean {
                let shallow = if other != Biome::Ocean { (1.0 - edge / 16.0).max(0.0) } else { 0.0 };
                let (dx, dy) = ((x as f32 / w as f32 - 0.5) * 2.0, (y as f32 / h as f32 - 0.5) * 2.0);
                let vignette = 1.0 - 0.6 * ((dx * dx + dy * dy) * 0.5).clamp(0.0, 1.0);
                set_pixel(&mut c, x, y, OCEAN, (0.85 + 0.7 * shallow) * vignette);
            } else if other == Biome::Ocean && edge < 3.0 {
                set_pixel(&mut c, x, y, SAND, tones[i]);
            } else {
                let rim = (1.0 - edge / 9.0).max(0.0);
                let facing = (p - sites[i]).normalize_or_zero().dot(LIGHT);
                let mut light = tones[i] * (1.0 + 0.22 * facing * rim);
                if other != biome && edge < 1.5 {
                    light *= 0.55;
                }
                set_pixel(&mut c, x, y, biome.color(), light);
            }
        }
    }

    // Pass 3: dashed roads between neighbouring land regions.
    let mut pairs = BTreeSet::new();
    for y in 0..h {
        for x in 0..w {
            let a = grid.at(x, y);
            for b in [(x + 1 < w).then(|| grid.at(x + 1, y)), (y + 1 < h).then(|| grid.at(x, y + 1))].into_iter().flatten() {
                if a != b && biomes[a as usize] != Biome::Ocean && biomes[b as usize] != Biome::Ocean {
                    pairs.insert((a.min(b), a.max(b)));
                }
            }
        }
    }
    for (a, b) in pairs {
        let (pa, pb) = (sites[a as usize], sites[b as usize]);
        let steps = pa.distance(pb).ceil() as usize;
        for s in 0..=steps {
            if s % 6 > 3 {
                continue;
            }
            stamp_circle(&mut c, pa.lerp(pb, s as f32 / steps.max(1) as f32), 1.2, ROAD, 0.5);
        }
    }

    // Pass 4: a town on every land site.
    for (s, b) in sites.iter().zip(&biomes) {
        if *b != Biome::Ocean {
            stamp_circle(&mut c, *s + Vec2::new(1.5, 2.0), 4.2, [0, 0, 0], 0.4);
            stamp_circle(&mut c, *s, 4.2, TOWN, 1.0);
            stamp_circle(&mut c, *s, 2.5, TOWN_CENTRE, 1.0);
        }
    }
    c
}
