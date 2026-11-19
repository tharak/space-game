//! A Voronoi world map (tutorial lab 4): edge cells are ocean so the land
//! is always an island; inland cells get a biome from latitude plus jitter.

use super::Vec2;
use super::grid::{NO_OWNER, OwnerGrid};
use super::rng::ParkMiller;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Biome {
    Ocean,
    Tundra,
    Forest,
    Plains,
    Swamp,
    Desert,
}

impl Biome {
    pub fn color(self) -> [u8; 3] {
        match self {
            Biome::Ocean => [0x2E, 0x5A, 0x88],
            Biome::Tundra => [0xE0, 0xFF, 0xFF],
            Biome::Forest => [0x22, 0x8B, 0x22],
            Biome::Plains => [0x98, 0xFB, 0x98],
            Biome::Swamp => [0x55, 0x6B, 0x2F],
            Biome::Desert => [0xED, 0xC9, 0xAF],
        }
    }

    /// Latitude band: `t` is 0 at the top of the map, 1 at the bottom.
    pub fn from_latitude(t: f32) -> Biome {
        if t < 0.3 {
            Biome::Tundra
        } else if t < 0.45 {
            Biome::Forest
        } else if t < 0.6 {
            Biome::Plains
        } else if t < 0.72 {
            Biome::Swamp
        } else {
            Biome::Desert
        }
    }
}

pub fn assign_biomes(sites: &[Vec2], grid: &OwnerGrid, seed: u32) -> Vec<Biome> {
    let mut rng = ParkMiller::new(seed as i64);
    let mut ocean = vec![false; sites.len()];
    for o in grid.edge_owners() {
        if o != NO_OWNER {
            ocean[o as usize] = true;
        }
    }
    sites
        .iter()
        .enumerate()
        .map(|(i, s)| {
            // Draw for every cell so a cell's biome doesn't depend on its neighbours.
            let jitter = (rng.next_float() - 0.5) * 0.3;
            if ocean[i] { Biome::Ocean } else { Biome::from_latitude(s.y / grid.h as f32 + jitter) }
        })
        .collect()
}
