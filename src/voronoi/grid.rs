//! Owner maps (tutorial step 3): which site owns each pixel.

use super::Vec2;
use super::metric::{Metric, SiteIndex};

pub const NO_OWNER: u32 = u32::MAX;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnerGrid {
    pub w: u32,
    pub h: u32,
    pub owner: Vec<u32>,
}

impl OwnerGrid {
    #[inline]
    pub fn at(&self, x: u32, y: u32) -> u32 {
        self.owner[(y * self.w + x) as usize]
    }

    /// A pixel whose right or lower neighbour has another owner.
    pub fn is_border(&self, x: u32, y: u32) -> bool {
        let o = self.at(x, y);
        (x + 1 < self.w && self.at(x + 1, y) != o) || (y + 1 < self.h && self.at(x, y + 1) != o)
    }

    /// Owners of pixels on the map's edge.
    pub fn edge_owners(&self) -> Vec<u32> {
        let mut out = Vec::new();
        for x in 0..self.w {
            out.push(self.at(x, 0));
            out.push(self.at(x, self.h - 1));
        }
        for y in 0..self.h {
            out.push(self.at(0, y));
            out.push(self.at(self.w - 1, y));
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// Nearest site at every pixel centre.
pub fn voronoi_grid(sites: &[Vec2], w: u32, h: u32, metric: Metric) -> OwnerGrid {
    voronoi_grid_scaled(sites, w, h, metric, 1)
}

/// Like [`voronoi_grid`], sampling every `step`-th pixel and filling the
/// block (for quick previews while dragging).
pub fn voronoi_grid_scaled(sites: &[Vec2], w: u32, h: u32, metric: Metric, step: u32) -> OwnerGrid {
    let step = step.max(1);
    let index = SiteIndex::new(sites, metric);
    let mut owner = vec![NO_OWNER; (w * h) as usize];
    for by in (0..h).step_by(step as usize) {
        for bx in (0..w).step_by(step as usize) {
            let c = Vec2::new(bx as f32 + step as f32 * 0.5, by as f32 + step as f32 * 0.5);
            let o = index.nearest(c).map_or(NO_OWNER, |i| i as u32);
            for y in by..(by + step).min(h) {
                for x in bx..(bx + step).min(w) {
                    owner[(y * w + x) as usize] = o;
                }
            }
        }
    }
    OwnerGrid { w, h, owner }
}
