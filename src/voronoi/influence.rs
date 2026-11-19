//! Weighted faction territories (tutorial lab 5): a multiplicatively
//! weighted Voronoi diagram. Each point goes to the base with the lowest
//! distance ÷ weight, so stronger bases reach further and borders curve.

use super::Vec2;
use super::metric::{Metric, distance};

/// The best base, its score and the runner-up's score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Influence {
    pub owner: Option<usize>,
    pub best: f32,
    pub second: f32,
}

impl Influence {
    /// Contested ground: the runner-up is within `margin` (e.g. 0.1 = 10%).
    pub fn contested(&self, margin: f32) -> bool {
        self.owner.is_some() && self.second.is_finite() && self.second <= self.best * (1.0 + margin)
    }
}

/// Bases with weight ≤ 0 (destroyed) are skipped.
pub fn weighted_scores(bases: &[Vec2], weights: &[f32], p: Vec2, metric: Metric) -> Influence {
    let mut r = Influence { owner: None, best: f32::INFINITY, second: f32::INFINITY };
    for (i, (b, w)) in bases.iter().zip(weights).enumerate() {
        if *w <= 0.0 {
            continue;
        }
        let s = distance(*b, p, metric) / w;
        if s < r.best {
            r.second = r.best;
            r.best = s;
            r.owner = Some(i);
        } else if s < r.second {
            r.second = s;
        }
    }
    r
}

/// The tutorial's `weightedOwner`; `None` when no base remains.
pub fn weighted_owner(bases: &[Vec2], weights: &[f32], p: Vec2, metric: Metric) -> Option<usize> {
    weighted_scores(bases, weights, p, metric).owner
}
