//! Seeded sites (tutorial step 1) and the config every lab shares.

use super::Vec2;
use super::metric::Metric;
use super::relax::relax_step;
use super::rng::ParkMiller;

/// `count` sites spread over a `w`×`h` map, repeatable from `seed`.
pub fn make_sites(count: usize, w: f32, h: f32, seed: u32) -> Vec<Vec2> {
    let mut rng = ParkMiller::new(seed as i64);
    (0..count).map(|_| Vec2::new(rng.next_float() * w, rng.next_float() * h)).collect()
}

/// The shared settings at the top of the labs page.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VoronoiConfig {
    pub seed: u32,
    pub sites: usize,
    pub metric: Metric,
    /// Lloyd rounds applied where a lab wants even cells (biomes, island).
    pub relax: u32,
    pub w: u32,
    pub h: u32,
}

impl Default for VoronoiConfig {
    fn default() -> Self {
        Self { seed: 123_456, sites: 40, metric: Metric::Euclidean, relax: 2, w: 720, h: 400 }
    }
}

impl VoronoiConfig {
    pub fn raw_sites(&self) -> Vec<Vec2> {
        make_sites(self.sites, self.w as f32, self.h as f32, self.seed)
    }

    /// The sites after `relax` Lloyd rounds.
    pub fn relaxed_sites(&self) -> Vec<Vec2> {
        let mut sites = self.raw_sites();
        for _ in 0..self.relax {
            sites = relax_step(&sites, self.w, self.h, self.metric).0;
        }
        sites
    }
}
