//! Palette and stroke sizes, matching the original prototype.

use bevy::prelude::*;

use crate::rules::battle::Tone;

const fn hex(rgb: u32) -> Color {
    Color::srgb(
        ((rgb >> 16) & 0xff) as f32 / 255.0,
        ((rgb >> 8) & 0xff) as f32 / 255.0,
        (rgb & 0xff) as f32 / 255.0,
    )
}

pub const BG: Color = hex(0x0b0e14);
pub const PANEL: Color = hex(0x131826);
pub const PANEL2: Color = hex(0x1a2133);
pub const LINE: Color = hex(0x2a3350);
pub const TEXT: Color = hex(0xd7deef);
pub const DIM: Color = hex(0x8892ab);
pub const BLUE: Color = hex(0x4a9eff);
pub const RED: Color = hex(0xff5a5a);
pub const GOLD: Color = hex(0xffd166);
pub const GREEN: Color = hex(0x4cd97b);
pub const BAD: Color = hex(0xff9a7a);
pub const PRIMARY_BG: Color = hex(0x1d3a63);

/// Board fill per side. Side 1 is "Red" in all text but drawn green on the
/// board, as in the original.
pub const SIDE_FILL: [u32; 2] = [0x4a9eff, 0x4cd97b];
pub fn side_fill(side: usize) -> Color {
    hex(SIDE_FILL[side])
}
pub fn side_text(side: usize) -> Color {
    if side == 0 { BLUE } else { RED }
}

pub const DARK: Color = hex(0x0b0e14);
pub const STATE_SHAKEN: Color = hex(0xffd166);
pub const STATE_ROUTED: Color = hex(0xff3333);
pub const STATE_ACTIVATED: Color = hex(0x8892ab);
pub const SELECTION: Color = Color::WHITE;
pub const TARGET: Color = hex(0xff3333);
pub const PIP_FILLED: Color = Color::WHITE;
pub const PIP_EMPTY: Color = Color::srgba(1.0, 1.0, 1.0, 0.2);

pub const GRID_CELL: u32 = 0x111624;
pub const GRID_LINE: Color = hex(0x1d2438);
pub const FIRE_ZONE: u32 = 0x26203a;
pub const COMMAND_ALPHA: f32 = 0x1c as f32 / 255.0;
pub const SETUP_ALPHA: f32 = 0x14 as f32 / 255.0;

/// Canvas-style "source over" blend of `tint` at `alpha` onto `base`, in
/// sRGB space (matches how the 2D canvas composited the original).
pub fn blend(base: [f32; 3], tint: u32, alpha: f32) -> [f32; 3] {
    let t = [
        ((tint >> 16) & 0xff) as f32 / 255.0,
        ((tint >> 8) & 0xff) as f32 / 255.0,
        (tint & 0xff) as f32 / 255.0,
    ];
    std::array::from_fn(|i| base[i] * (1.0 - alpha) + t[i] * alpha)
}

pub fn rgb(rgb: u32) -> [f32; 3] {
    [((rgb >> 16) & 0xff) as f32 / 255.0, ((rgb >> 8) & 0xff) as f32 / 255.0, (rgb & 0xff) as f32 / 255.0]
}

pub fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Plain => TEXT,
        Tone::Title => GOLD,
        Tone::Bad => BAD,
        Tone::Good => GREEN,
        Tone::Side(s) => side_text(s),
    }
}

pub const LASER_HIT_SECS: f32 = 0.42;
pub const LASER_MISS_SECS: f32 = 0.32;
