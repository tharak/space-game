//! Division formation shapes as slot layouts. Slots are in the division's
//! frame (+x forward, +y left) and centred on their centroid, so the
//! division anchor sits in the middle of its ships.

use bevy::prelude::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum Formation {
    /// Two ranks abreast: the widest frontage.
    #[default]
    Line,
    /// A filled wedge, point forward: few guns at the front, all of them
    /// close together.
    Wedge,
    /// A line swept back on its left end.
    EchelonLeft,
    /// A line swept back on its right end.
    EchelonRight,
    /// Two ranks on an arc with the horns forward.
    Crescent,
    /// Three abreast, many deep: fast to move, narrow to fight.
    Column,
    /// Concentric rings: no flank, no rear.
    Ring,
}

impl Formation {
    pub const PLAYER: [Formation; 6] = [
        Formation::Line,
        Formation::Wedge,
        Formation::Crescent,
        Formation::EchelonLeft,
        Formation::EchelonRight,
        Formation::Column,
    ];

    /// Deep formations refill their front from the ranks behind.
    pub fn closes_ranks(self) -> bool {
        matches!(self, Formation::Wedge | Formation::Column | Formation::Ring)
    }

    pub fn name(self) -> &'static str {
        match self {
            Formation::Line => "Line",
            Formation::Wedge => "Wedge",
            Formation::EchelonLeft => "Echelon L",
            Formation::EchelonRight => "Echelon R",
            Formation::Crescent => "Crescent",
            Formation::Column => "Column",
            Formation::Ring => "Ring",
        }
    }
}

/// Rows of `n` ships, `per_row` wide, centred on y; row `r` sits at
/// x = -r * spacing.
fn ranks(n: usize, per_row: usize, spacing: f32) -> Vec<Vec2> {
    let per_row = per_row.max(1);
    let mut out = Vec::with_capacity(n);
    let mut row = 0;
    while out.len() < n {
        let count = per_row.min(n - out.len());
        for i in 0..count {
            let y = (i as f32 - (count - 1) as f32 / 2.0) * spacing;
            out.push(Vec2::new(-(row as f32) * spacing, y));
        }
        row += 1;
    }
    out
}

pub fn layout(formation: Formation, n: usize, spacing: f32) -> Vec<Vec2> {
    if n == 0 {
        return Vec::new();
    }
    let s = spacing;
    let mut slots = match formation {
        Formation::Line => ranks(n, n.div_ceil(2), s),
        Formation::Column => ranks(n, 3, s),
        Formation::EchelonLeft | Formation::EchelonRight => {
            let sweep = if formation == Formation::EchelonLeft { 0.5 } else { -0.5 };
            ranks(n, n.div_ceil(2), s).into_iter().map(|p| Vec2::from_angle(sweep).rotate(p)).collect()
        }
        Formation::Wedge => {
            // Row k has 2k+1 ships, each row a spacing behind the last.
            let mut out = Vec::with_capacity(n);
            let mut k = 0;
            while out.len() < n {
                let count = (2 * k + 1).min(n - out.len());
                for i in 0..count {
                    let y = (i as f32 - (count - 1) as f32 / 2.0) * s;
                    out.push(Vec2::new(-(k as f32) * s * 0.9, y));
                }
                k += 1;
            }
            out
        }
        Formation::Crescent => {
            // Two ranks on a 120° arc, concave forward.
            let front = n.div_ceil(2);
            let span = std::f32::consts::TAU / 3.0;
            let radius = (front as f32 * s / span).max(s);
            let mut out = Vec::with_capacity(n);
            for (rank, count) in [(0, front), (1, n - front)] {
                let r = radius + rank as f32 * s;
                for i in 0..count {
                    let t = if count == 1 { 0.0 } else { i as f32 / (count - 1) as f32 - 0.5 };
                    let a = t * span * radius / r;
                    out.push(Vec2::new(-r * a.cos(), r * a.sin()));
                }
            }
            out
        }
        Formation::Ring => {
            let mut out = Vec::with_capacity(n);
            out.push(Vec2::ZERO);
            let mut ring = 1;
            while out.len() < n {
                let r = ring as f32 * s;
                let count = ((std::f32::consts::TAU * r / s) as usize).min(n - out.len());
                for i in 0..count {
                    out.push(Vec2::from_angle(i as f32 / count as f32 * std::f32::consts::TAU) * r);
                }
                ring += 1;
            }
            out
        }
    };
    let centroid = slots.iter().copied().sum::<Vec2>() / slots.len() as f32;
    for p in &mut slots {
        *p -= centroid;
    }
    slots
}

/// (depth behind the anchor of the rear-most slot, half the width).
pub fn extent(slots: &[Vec2]) -> (f32, f32) {
    let back = slots.iter().map(|p| p.x).fold(0.0, f32::min);
    let half = slots.iter().map(|p| p.y.abs()).fold(0.0, f32::max);
    (back, half)
}

/// Width of a formation of `n` ships across its front.
pub fn frontage(formation: Formation, n: usize, spacing: f32) -> f32 {
    2.0 * extent(&layout(formation, n, spacing)).1 + spacing
}
