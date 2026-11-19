//! The tutorial's Park-Miller "minimal standard" generator, so the same
//! seed gives the same sites as the article.

const MODULUS: i64 = 2_147_483_647;
const MULTIPLIER: i64 = 16_807;

#[derive(Clone, Debug)]
pub struct ParkMiller {
    state: i64,
}

impl ParkMiller {
    /// Seeds of 0 or below are moved into the valid range.
    pub fn new(seed: i64) -> Self {
        let mut state = seed % MODULUS;
        if state <= 0 {
            state += MODULUS - 1;
        }
        Self { state }
    }

    /// Next raw value in 1..2³¹−1.
    pub fn next_raw(&mut self) -> i64 {
        self.state = self.state * MULTIPLIER % MODULUS;
        self.state
    }

    /// Uniform in [0, 1].
    pub fn next_float(&mut self) -> f32 {
        ((self.next_raw() - 1) as f64 / (MODULUS - 2) as f64) as f32
    }
}
