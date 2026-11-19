//! Cell (Worley) noise (tutorial lab 6): textures from the distances to
//! the nearest (F1) and second-nearest (F2) sites.

use super::Vec2;
use super::metric::SiteIndex;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NoiseMode {
    /// Distance to the nearest site: dark at the sites.
    F1,
    /// Distance to the second-nearest site.
    F2,
    /// Zero on borders, growing toward cell centres: an edge detector.
    #[default]
    F2MinusF1,
}

impl NoiseMode {
    pub const ALL: [NoiseMode; 3] = [NoiseMode::F1, NoiseMode::F2, NoiseMode::F2MinusF1];

    pub fn name(self) -> &'static str {
        match self {
            NoiseMode::F1 => "F1",
            NoiseMode::F2 => "F2",
            NoiseMode::F2MinusF1 => "F2 − F1",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NoisePreset {
    /// The raw value as grey.
    #[default]
    Grey,
    /// Grey stones with dark mortar.
    Stone,
    /// Glowing cracks on black.
    Lava,
}

impl NoisePreset {
    pub const ALL: [NoisePreset; 3] = [NoisePreset::Grey, NoisePreset::Stone, NoisePreset::Lava];

    pub fn name(self) -> &'static str {
        match self {
            NoisePreset::Grey => "Raw",
            NoisePreset::Stone => "Stone",
            NoisePreset::Lava => "Lava",
        }
    }
}

/// The tutorial's `cellNoise`.
pub fn cell_noise(index: &SiteIndex, p: Vec2, mode: NoiseMode) -> f32 {
    let n = index.nearest_two(p);
    match mode {
        NoiseMode::F1 => n.d1,
        NoiseMode::F2 => n.d2,
        NoiseMode::F2MinusF1 => n.d2 - n.d1,
    }
}

/// Colour for one pixel. `cell` is the owning site, `scale` the typical
/// cell radius (used to normalise raw distances).
pub fn shade(index: &SiteIndex, p: Vec2, mode: NoiseMode, preset: NoisePreset, crack: f32, scale: f32) -> [u8; 3] {
    let n = index.nearest_two(p);
    let edge = n.d2 - n.d1;
    match preset {
        NoisePreset::Grey => {
            let v = match mode {
                NoiseMode::F1 => n.d1 / scale,
                NoiseMode::F2 => n.d2 / (scale * 1.6),
                NoiseMode::F2MinusF1 => edge / scale,
            };
            let g = (v.clamp(0.0, 1.0) * 255.0) as u8;
            [g, g, g]
        }
        NoisePreset::Stone => {
            if edge < crack {
                [38, 34, 32]
            } else {
                // Each stone has its own tone, darker toward its rim.
                let tone = 0.75 + 0.25 * hash01(n.nearest as u32);
                let rim = (edge / (crack * 4.0)).min(1.0);
                let g = (150.0 * tone * (0.8 + 0.2 * rim)) as u8;
                [g, (g as f32 * 0.97) as u8, (g as f32 * 0.92) as u8]
            }
        }
        NoisePreset::Lava => {
            let glow = (1.0 - edge / (crack * 2.5)).clamp(0.0, 1.0);
            let hot = (1.0 - edge / crack).clamp(0.0, 1.0);
            [(255.0 * glow) as u8, (60.0 * glow + 170.0 * hot * hot) as u8, (40.0 * hot * hot * hot) as u8]
        }
    }
}

/// A stable pseudo-random value in [0, 1) per integer.
pub fn hash01(i: u32) -> f32 {
    let mut v = i.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    v ^= v >> 15;
    v = v.wrapping_mul(0x2C1B_3C6D);
    v ^= v >> 12;
    (v >> 8) as f32 / (1u32 << 24) as f32
}
