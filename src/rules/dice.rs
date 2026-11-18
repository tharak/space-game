//! The battle's only source of randomness, as a resource so tests and
//! replays can inject a seed or an exact roll sequence.

use std::collections::VecDeque;

use bevy::prelude::*;

#[derive(Resource, Debug, Clone)]
pub struct Dice {
    state: u32,
    scripted: VecDeque<u8>,
}

impl Dice {
    pub fn seeded(seed: u32) -> Self {
        Self { state: seed, scripted: VecDeque::new() }
    }

    /// Rolls returned in order before falling back to the seeded generator.
    pub fn scripted(rolls: impl IntoIterator<Item = u8>) -> Self {
        Self { state: 1, scripted: rolls.into_iter().collect() }
    }

    /// Mulberry32, as in the original prototype.
    pub fn next_f64(&mut self) -> f64 {
        self.state = self.state.wrapping_add(0x6d2b_79f5);
        let mut v = self.state;
        v = (v ^ (v >> 15)).wrapping_mul(v | 1);
        v ^= v.wrapping_add((v ^ (v >> 7)).wrapping_mul(v | 61));
        ((v ^ (v >> 14)) as f64) / 4_294_967_296.0
    }

    pub fn range(&mut self, min: u32, max: u32) -> u32 {
        min + (self.next_f64() * (max - min + 1) as f64) as u32
    }

    pub fn d6(&mut self) -> u8 {
        match self.scripted.pop_front() {
            Some(v) => v,
            None => self.range(1, 6) as u8,
        }
    }

    pub fn pick<T: Copy>(&mut self, values: &[T]) -> T {
        values[self.range(0, values.len() as u32 - 1) as usize]
    }
}

impl Default for Dice {
    fn default() -> Self {
        Self::seeded(0x5eed_1234)
    }
}
