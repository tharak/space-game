//! Palette and sizes.

use bevy::prelude::*;

use super::flow::Tone;
use crate::sim::components::Morale;

const fn hex(rgb: u32) -> Color {
    Color::srgb(
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
    )
}

pub const BG: Color = hex(0x070a10);
pub const PANEL: Color = hex(0x131826);
pub const PANEL2: Color = hex(0x1a2133);
pub const LINE: Color = hex(0x2a3350);
pub const TEXT: Color = hex(0xd7deef);
pub const DIM: Color = hex(0x8892ab);
pub const BLUE: Color = hex(0x4a9eff);
pub const RED: Color = hex(0xff6a4a);
pub const GOLD: Color = hex(0xffd166);
pub const GREEN: Color = hex(0x4cd97b);
pub const BAD: Color = hex(0xff9a7a);
pub const PRIMARY_BG: Color = hex(0x1d3a63);
pub const GRID: Color = Color::srgba(0.35, 0.45, 0.7, 0.07);
pub const BOUNDS: Color = Color::srgba(0.35, 0.45, 0.7, 0.25);

/// Ship colours per side: steady, shaken, routed.
pub const SHIP: [[Color; 3]; 2] = [
    [hex(0x4a9eff), hex(0xffd166), hex(0x5d6478)],
    [hex(0xff6a4a), hex(0xffd166), hex(0x5d6478)],
];
/// Ships of the selected division.
pub const SHIP_SELECTED: [Color; 2] = [hex(0xb8dcff), hex(0xffc4b0)];
pub const FLAGSHIP: Color = Color::WHITE;

pub fn side_color(side: usize) -> Color {
    if side == 0 { BLUE } else { RED }
}

pub fn morale_color(morale: Morale) -> Color {
    match morale {
        Morale::Steady => GREEN,
        Morale::Shaken => GOLD,
        Morale::Routed => BAD,
    }
}

pub fn morale_index(morale: Morale) -> usize {
    match morale {
        Morale::Steady => 0,
        Morale::Shaken => 1,
        Morale::Routed => 2,
    }
}

pub fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Plain => TEXT,
        Tone::Title => GOLD,
        Tone::Bad => BAD,
        Tone::Good => GREEN,
        Tone::Side(s) => side_color(s),
    }
}

pub const LASER_HIT_SECS: f32 = 0.3;
pub const LASER_MISS_SECS: f32 = 0.18;
pub const BOOM_SECS: f32 = 0.6;
