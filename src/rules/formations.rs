//! Formation templates: pure position/facing data. Formations carry no
//! modifiers; they are only where squadrons start and which way they point.

use super::config::Formation;

/// One squadron slot: forward offset (toward the enemy), lateral offset
/// (rows), and a facing hint (0 straight ahead, +1 / -1 angled).
pub type Slot = (i32, i32, i32);

pub struct Layout {
    pub slots: Vec<Slot>,
    pub flag: usize,
}

fn line_of(lo: i32, hi: i32, f: impl Fn(i32) -> Slot) -> Vec<Slot> {
    (lo..=hi).map(f).collect()
}

fn crescent_facing(l: i32) -> i32 {
    if l <= -2 {
        1
    } else if l >= 2 {
        -1
    } else {
        0
    }
}

/// Mirrors `formation_layout` in the original Python sim exactly.
pub fn layout(formation: Formation, size: usize) -> Layout {
    use Formation::*;
    let (slots, flag) = match (size, formation) {
        (5, Line) => (line_of(-2, 2, |l| (0, l, 0)), 2),
        (5, Spindle) => (vec![(1, 0, 0), (0, -1, 0), (0, 0, 0), (0, 1, 0), (-1, 0, 0)], 2),
        (5, Crescent) => (line_of(-2, 2, |l| (if l.abs() == 2 { 1 } else { 0 }, l, crescent_facing(l))), 2),
        (5, Echelon) => (line_of(-2, 2, |l| (-l, l, 0)), 2),
        (5, Sphere) => (vec![(0, 0, 0), (1, 0, 0), (0, -1, 0), (-1, 0, 0), (0, 1, 0)], 0),
        (5, Column) => (line_of(-2, 2, |f| (f, 0, 0)), 2),

        (9, Line) => (line_of(-4, 4, |l| (0, l, 0)), 4),
        (9, Spindle) => (
            vec![(2, 0, 0), (1, -1, 0), (1, 1, 0), (0, -1, 0), (0, 0, 0), (0, 1, 0), (-1, -1, 0), (-1, 1, 0), (-2, 0, 0)],
            4,
        ),
        (9, Crescent) => (
            line_of(-4, 4, |l| (if l.abs() >= 3 { 2 } else if l.abs() == 2 { 1 } else { 0 }, l, crescent_facing(l))),
            4,
        ),
        (9, Echelon) => (line_of(-4, 4, |l| (-l, l, 0)), 4),
        (9, Sphere) => (
            vec![(0, 0, 0), (1, 0, 0), (1, -1, 0), (0, -1, 0), (-1, -1, 0), (-1, 0, 0), (-1, 1, 0), (0, 1, 0), (1, 1, 0)],
            0,
        ),
        (9, Column) => (line_of(-4, 4, |f| (f, 0, 0)), 4),

        (12, Line) => (line_of(-6, 5, |l| (0, l, 0)), 6),
        (12, Spindle) => (
            vec![
                (3, 0, 0), (2, -1, 0), (2, 1, 0),
                (1, -1, 0), (1, 0, 0), (1, 1, 0),
                (0, -1, 0), (0, 0, 0), (0, 1, 0),
                (-1, -1, 0), (-1, 1, 0), (-2, 0, 0),
            ],
            7,
        ),
        (12, Crescent) => (
            line_of(-6, 5, |l| (if l.abs() >= 4 { 2 } else if l.abs() >= 2 { 1 } else { 0 }, l, crescent_facing(l))),
            6,
        ),
        (12, Echelon) => (line_of(-6, 5, |l| ((-l).clamp(-4, 4), l, 0)), 6),
        (12, Sphere) => (
            vec![
                (0, 0, 0), (1, 0, 0), (1, -1, 0), (0, -1, 0), (-1, -1, 0), (-1, 0, 0),
                (-1, 1, 0), (0, 1, 0), (1, 1, 0), (2, 0, 0), (0, -2, 0), (0, 2, 0),
            ],
            0,
        ),
        (12, Column) => ((-2..=3).flat_map(|f| [(f, 0, 0), (f, 1, 0)]).collect(), 4),
        _ => panic!("no layout for {:?} at size {size}", formation),
    };
    Layout { slots, flag }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hex::Hex;
    use crate::rules::config::{DEPLOY_ANCHOR, DEPLOY_ROW_CENTER, in_bounds};

    #[test]
    fn every_layout_fits_the_board_on_both_sides() {
        for size in [5, 9, 12] {
            for f in Formation::STANDARD {
                let l = layout(f, size);
                assert_eq!(l.slots.len(), size, "{f:?} {size}");
                for (fwd, lat, _) in &l.slots {
                    let blue = Hex::new(DEPLOY_ANCHOR[0] + fwd, DEPLOY_ROW_CENTER + lat);
                    let red = Hex::new(DEPLOY_ANCHOR[1] - fwd, DEPLOY_ROW_CENTER + lat);
                    assert!(in_bounds(blue) && in_bounds(red), "{f:?} {size}");
                }
            }
        }
    }
}
