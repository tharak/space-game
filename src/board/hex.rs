//! Pure hex geometry on an "odd-r" offset grid (`c` = column, `r` = row,
//! odd rows shifted half a hex right). A faithful port of the original
//! `battle/hexmath.js`, including its JavaScript rounding semantics, so
//! line-of-sight and arc results match the playtested prototype exactly.

/// A board cell in offset coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Hex {
    pub c: i32,
    pub r: i32,
}

impl Hex {
    pub const fn new(c: i32, r: i32) -> Self {
        Self { c, r }
    }
}

/// Axial neighbour offsets, indexed by facing 0..5.
pub const CUBE_DIRS: [(i32, i32); 6] = [(1, 0), (1, -1), (0, -1), (-1, 0), (-1, 1), (0, 1)];
/// Screen-space angle (degrees, y pointing down) of each facing.
pub const DIR_ANGLE: [f64; 6] = [0.0, -60.0, -120.0, 180.0, 120.0, 60.0];

const S32: f64 = 0.866_025_403_784_438_6; // sqrt(3)/2

pub fn to_axial(h: Hex) -> (i32, i32) {
    (h.c - ((h.r - (h.r & 1)) >> 1), h.r)
}

pub fn from_axial(q: i32, r: i32) -> Hex {
    Hex::new(q + ((r - (r & 1)) >> 1), r)
}

pub fn dist(a: Hex, b: Hex) -> i32 {
    let (aq, ar) = to_axial(a);
    let (bq, br) = to_axial(b);
    let (dq, dr) = (aq - bq, ar - br);
    (dq.abs() + dr.abs() + (dq + dr).abs()) / 2
}

pub fn neighbor(h: Hex, dir: u8) -> Hex {
    let (q, r) = to_axial(h);
    let (dq, dr) = CUBE_DIRS[dir as usize % 6];
    from_axial(q + dq, r + dr)
}

fn to_cart(h: Hex) -> (f64, f64) {
    (h.c as f64 + 0.5 * (h.r & 1) as f64, h.r as f64 * S32)
}

/// Screen angle in degrees from `a` to `b`.
pub fn angle_between(a: Hex, b: Hex) -> f64 {
    let (ax, ay) = to_cart(a);
    let (bx, by) = to_cart(b);
    (by - ay).atan2(bx - ax).to_degrees()
}

/// Normalises an angle difference into [-180, 180) and returns its magnitude.
pub fn wrap_abs(deg: f64) -> f64 {
    ((deg + 180.0).rem_euclid(360.0) - 180.0).abs()
}

pub fn rel_angle(facing: u8, from: Hex, to: Hex) -> f64 {
    wrap_abs(angle_between(from, to) - DIR_ANGLE[facing as usize])
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arc {
    Front,
    Flank,
    Rear,
}

impl Arc {
    pub fn name(self) -> &'static str {
        match self {
            Arc::Front => "front",
            Arc::Flank => "flank",
            Arc::Rear => "rear",
        }
    }
}

/// Which arc of the *target* the firer sits in.
pub fn incoming_arc(target: Hex, target_facing: u8, firer: Hex) -> Arc {
    let a = rel_angle(target_facing, target, firer);
    if a < 90.0 - 1e-9 {
        Arc::Front
    } else if a < 150.0 - 1e-9 {
        Arc::Flank
    } else {
        Arc::Rear
    }
}

/// A squadron may only fire into its own 180-degree front arc.
pub fn in_fire_arc(facing: u8, from: Hex, to: Hex) -> bool {
    rel_angle(facing, from, to) <= 90.0 + 1e-9
}

/// JavaScript `Math.round`: halves round toward +infinity.
fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// Line of sight between two hexes; any occupied hex strictly between them
/// blocks. A line running exactly along a hex seam is resolved in the
/// shooter's favour (either nudge direction being clear is enough).
pub fn los_clear(a: Hex, b: Hex, occupied: impl Fn(Hex) -> bool) -> bool {
    let n = dist(a, b);
    if n <= 1 {
        return true;
    }
    let (aq, ar) = to_axial(a);
    let (bq, br) = to_axial(b);
    let (aq, ar, bq, br) = (aq as f64, ar as f64, bq as f64, br as f64);
    'nudge: for eps in [1e-6, -1e-6] {
        for i in 1..n {
            let t = i as f64 / n as f64;
            let q = aq + (bq - aq) * t + eps;
            let r = ar + (br - ar) * t + eps / 2.0;
            let s = -q - r;
            let (mut rq, mut rr, rs) = (js_round(q), js_round(r), js_round(s));
            let (dq, dr, ds) = ((rq - q).abs(), (rr - r).abs(), (rs - s).abs());
            if dq > dr && dq > ds {
                rq = -rr - rs;
            } else if dr > ds {
                rr = -rq - rs;
            }
            let h = from_axial(rq as i32, rr as i32);
            if h != a && h != b && occupied(h) {
                continue 'nudge;
            }
        }
        return true;
    }
    false
}

/// The facing (0..5) whose angle is closest to pointing from `from` at `goal`.
/// Ties resolve to the lowest index, as in the original `argmin`.
pub fn desired_dir(from: Hex, goal: Hex) -> u8 {
    let ang = angle_between(from, goal);
    let mut best = 0u8;
    let mut best_v = f64::INFINITY;
    for d in 0..6u8 {
        let v = wrap_abs(DIR_ANGLE[d as usize] - ang);
        if v < best_v {
            best_v = v;
            best = d;
        }
    }
    best
}

/// One 60-degree step from `facing` toward `target` (shortest way, ties
/// counter-clockwise in index terms).
pub fn turn_toward(facing: u8, target: u8) -> u8 {
    let diff = (target as i32 - facing as i32).rem_euclid(6);
    ((facing as i32 + if diff <= 3 { 1 } else { 5 }) % 6) as u8
}

// --- pixel geometry (board drawing, y pointing down like the canvas) -----

/// Hex size: centre to corner, in board pixels.
pub const HS: f32 = 17.0;
/// Hex width.
pub const HW: f32 = HS * 1.732_050_8;
pub const OX: f32 = 26.0;
pub const OY: f32 = 26.0;

/// Centre of a hex in board pixels (origin top-left, y down).
pub fn hex_center_px(h: Hex) -> (f32, f32) {
    (
        OX + (h.c as f32 + 0.5 * (h.r & 1) as f32) * HW,
        OY + h.r as f32 * HS * 1.5,
    )
}

/// Corners of a pointy-top hex of radius `s`, in y-down pixel space.
pub fn hex_corners_px(cx: f32, cy: f32, s: f32) -> [(f32, f32); 6] {
    std::array::from_fn(|k| {
        let a = (60.0 * k as f32 - 90.0).to_radians();
        (cx + a.cos() * s, cy + a.sin() * s)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distances_and_neighbours() {
        let o = Hex::new(5, 5);
        for d in 0..6 {
            assert_eq!(dist(o, neighbor(o, d)), 1);
        }
        assert_eq!(dist(Hex::new(0, 0), Hex::new(3, 0)), 3);
        assert_eq!(neighbor(neighbor(o, 0), 3), o);
    }

    #[test]
    fn arcs() {
        let t = Hex::new(10, 10);
        // Target faces east (0): a firer to the east is in its front arc,
        // directly west is rear.
        assert_eq!(incoming_arc(t, 0, Hex::new(12, 10)), Arc::Front);
        assert_eq!(incoming_arc(t, 0, Hex::new(8, 10)), Arc::Rear);
        assert_eq!(incoming_arc(t, 0, neighbor(t, 2)), Arc::Flank);
    }

    #[test]
    fn los_blocked_by_middle_ship() {
        let a = Hex::new(4, 4);
        let b = Hex::new(7, 4);
        let mid = Hex::new(5, 4);
        assert!(los_clear(a, b, |_| false));
        assert!(!los_clear(a, b, |h| h == mid));
    }

    #[test]
    fn turn_toward_takes_short_way() {
        assert_eq!(turn_toward(0, 1), 1);
        assert_eq!(turn_toward(0, 5), 5);
        assert_eq!(turn_toward(0, 3), 1);
    }
}
