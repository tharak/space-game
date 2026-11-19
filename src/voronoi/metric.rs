//! Distance metrics and nearest-site queries (tutorial step 2).

use super::Vec2;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Metric {
    /// Straight-line distance: borders at any angle.
    #[default]
    Euclidean,
    /// |dx| + |dy|: borders of horizontal, vertical and 45° pieces.
    Manhattan,
    /// max(|dx|, |dy|): 8-way movement, boxy borders.
    Chebyshev,
}

impl Metric {
    pub const ALL: [Metric; 3] = [Metric::Euclidean, Metric::Manhattan, Metric::Chebyshev];

    pub fn name(self) -> &'static str {
        match self {
            Metric::Euclidean => "Euclidean",
            Metric::Manhattan => "Manhattan",
            Metric::Chebyshev => "Chebyshev",
        }
    }
}

pub fn distance(a: Vec2, b: Vec2, metric: Metric) -> f32 {
    let (dx, dy) = ((a.x - b.x).abs(), (a.y - b.y).abs());
    match metric {
        Metric::Euclidean => (dx * dx + dy * dy).sqrt(),
        Metric::Manhattan => dx + dy,
        Metric::Chebyshev => dx.max(dy),
    }
}

/// Checks every site; strict `<`, so ties go to the lower index.
pub fn nearest_site(sites: &[Vec2], p: Vec2, metric: Metric) -> Option<usize> {
    let mut best = None;
    let mut best_d = f32::INFINITY;
    for (i, s) in sites.iter().enumerate() {
        let d = distance(*s, p, metric);
        if d < best_d {
            best_d = d;
            best = Some(i);
        }
    }
    best
}

/// The nearest (F1) and second-nearest (F2) sites, found in one pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearestTwo {
    pub nearest: usize,
    pub second: usize,
    pub d1: f32,
    pub d2: f32,
}

impl NearestTwo {
    pub const NONE: NearestTwo = NearestTwo { nearest: usize::MAX, second: usize::MAX, d1: f32::INFINITY, d2: f32::INFINITY };

    /// Folds in candidate `i` at distance `d` (ties keep the lower index).
    #[inline]
    pub fn offer(&mut self, i: usize, d: f32) {
        if d < self.d1 || (d == self.d1 && i < self.nearest) {
            self.second = self.nearest;
            self.d2 = self.d1;
            self.nearest = i;
            self.d1 = d;
        } else if d < self.d2 || (d == self.d2 && i < self.second) {
            self.second = i;
            self.d2 = d;
        }
    }
}

pub fn nearest_two(sites: &[Vec2], p: Vec2, metric: Metric) -> NearestTwo {
    let mut n = NearestTwo::NONE;
    for (i, s) in sites.iter().enumerate() {
        n.offer(i, distance(*s, p, metric));
    }
    n
}

/// Sites sorted by x, for fast nearest queries over whole images. Every
/// metric is at least |dx|, so a sweep outward from the query's x can stop
/// once |dx| passes the current answer. Same results as the brute-force
/// functions above, ties included.
#[derive(Clone, Debug)]
pub struct SiteIndex {
    sorted: Vec<(f32, f32, usize)>,
    pub metric: Metric,
}

impl SiteIndex {
    pub fn new(sites: &[Vec2], metric: Metric) -> Self {
        let mut sorted: Vec<(f32, f32, usize)> = sites.iter().enumerate().map(|(i, s)| (s.x, s.y, i)).collect();
        sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
        Self { sorted, metric }
    }

    pub fn nearest_two(&self, p: Vec2) -> NearestTwo {
        let mut n = NearestTwo::NONE;
        let start = self.sorted.partition_point(|s| s.0 < p.x);
        // Right of p.
        for &(x, y, i) in &self.sorted[start..] {
            if x - p.x > n.d2 {
                break;
            }
            n.offer(i, distance(Vec2::new(x, y), p, self.metric));
        }
        // Left of p.
        for &(x, y, i) in self.sorted[..start].iter().rev() {
            if p.x - x > n.d2 {
                break;
            }
            n.offer(i, distance(Vec2::new(x, y), p, self.metric));
        }
        n
    }

    pub fn nearest(&self, p: Vec2) -> Option<usize> {
        let n = self.nearest_two(p);
        (n.nearest != usize::MAX).then_some(n.nearest)
    }
}
