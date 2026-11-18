//! The simulation's only source of randomness, kept as a resource so a
//! battle is reproducible from its seed.

use bevy::prelude::*;

#[derive(Resource, Debug, Clone)]
pub struct Rng {
    state: u32,
}

impl Rng {
    pub fn seeded(seed: u32) -> Self {
        Self { state: seed }
    }

    /// Mulberry32, as in the original prototype. Uniform in [0, 1).
    pub fn next_f32(&mut self) -> f32 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut v = self.state;
        v = (v ^ (v >> 15)).wrapping_mul(v | 1);
        v ^= v.wrapping_add((v ^ (v >> 7)).wrapping_mul(v | 61));
        ((v ^ (v >> 14)) >> 8) as f32 / (1u32 << 24) as f32
    }

    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    pub fn chance(&mut self, p: f32) -> bool {
        self.next_f32() < p
    }
}

impl Default for Rng {
    fn default() -> Self {
        Self::seeded(0x5eed_1234)
    }
}
