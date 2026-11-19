//! Lloyd relaxation: move each site to its cell's centroid, then rebuild.

use super::Vec2;
use super::grid::{NO_OWNER, OwnerGrid, voronoi_grid};
use super::metric::Metric;

/// Each cell's centroid (mean of its pixel centres); `None` for an empty cell.
pub fn centroids(n: usize, grid: &OwnerGrid) -> Vec<Option<Vec2>> {
    let mut sum = vec![(0.0f64, 0.0f64, 0u32); n];
    for y in 0..grid.h {
        for x in 0..grid.w {
            let o = grid.at(x, y);
            if o != NO_OWNER {
                let s = &mut sum[o as usize];
                s.0 += x as f64 + 0.5;
                s.1 += y as f64 + 0.5;
                s.2 += 1;
            }
        }
    }
    sum.into_iter().map(|(x, y, c)| (c > 0).then(|| Vec2::new((x / c as f64) as f32, (y / c as f64) as f32))).collect()
}

/// One Lloyd round. Returns the new sites and the largest move.
pub fn relax_step(sites: &[Vec2], w: u32, h: u32, metric: Metric) -> (Vec<Vec2>, f32) {
    let grid = voronoi_grid(sites, w, h, metric);
    let cs = centroids(sites.len(), &grid);
    let mut moved: f32 = 0.0;
    let out = sites
        .iter()
        .zip(cs)
        .map(|(s, c)| {
            let c = c.unwrap_or(*s);
            moved = moved.max(c.distance(*s));
            c
        })
        .collect();
    (out, moved)
}

/// The tutorial's `relaxSites`: one round, new sites only.
pub fn relax_sites(sites: &[Vec2], w: u32, h: u32, metric: Metric) -> Vec<Vec2> {
    relax_step(sites, w, h, metric).0
}
