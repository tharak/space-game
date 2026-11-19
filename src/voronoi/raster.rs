//! A tiny RGB canvas, so every lab renders to one plain image.

use super::Vec2;

pub type Rgb = [u8; 3];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Canvas {
    pub w: u32,
    pub h: u32,
    /// RGB bytes, row by row.
    pub px: Vec<u8>,
}

impl Canvas {
    pub fn new(w: u32, h: u32, fill: Rgb) -> Self {
        let mut px = Vec::with_capacity((w * h * 3) as usize);
        for _ in 0..w * h {
            px.extend_from_slice(&fill);
        }
        Self { w, h, px }
    }

    #[inline]
    pub fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = ((y as u32 * self.w + x as u32) * 3) as usize;
            self.px[i..i + 3].copy_from_slice(&c);
        }
    }

    #[inline]
    pub fn get(&self, x: u32, y: u32) -> Rgb {
        let i = ((y * self.w + x) * 3) as usize;
        [self.px[i], self.px[i + 1], self.px[i + 2]]
    }

    /// Alpha-blends `c` over the pixel.
    #[inline]
    pub fn blend(&mut self, x: i32, y: i32, c: Rgb, a: f32) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            let i = ((y as u32 * self.w + x as u32) * 3) as usize;
            for (dst, src) in self.px[i..i + 3].iter_mut().zip(c) {
                *dst = (*dst as f32 * (1.0 - a) + src as f32 * a).round() as u8;
            }
        }
    }

    /// A filled circle, blended at `a`.
    pub fn disc(&mut self, c: Vec2, r: f32, col: Rgb, a: f32) {
        let (x0, x1) = ((c.x - r).floor() as i32, (c.x + r).ceil() as i32);
        let (y0, y1) = ((c.y - r).floor() as i32, (c.y + r).ceil() as i32);
        for y in y0..=y1 {
            for x in x0..=x1 {
                if Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(c) <= r {
                    self.blend(x, y, col, a);
                }
            }
        }
    }

    /// A line `width` pixels wide.
    pub fn line(&mut self, a: Vec2, b: Vec2, width: f32, col: Rgb, alpha: f32) {
        let len = a.distance(b).max(1.0);
        let steps = (len * 2.0).ceil() as usize;
        let r = (width * 0.5).max(0.5);
        let mut last = (i32::MIN, i32::MIN);
        for k in 0..=steps {
            let p = a.lerp(b, k as f32 / steps as f32);
            if r <= 0.75 {
                let q = (p.x.floor() as i32, p.y.floor() as i32);
                if q != last {
                    self.blend(q.0, q.1, col, alpha);
                    last = q;
                }
            } else {
                self.disc(p, r, col, alpha);
            }
        }
    }

    /// Dashes of `on` pixels separated by `off`.
    pub fn dashed_line(&mut self, a: Vec2, b: Vec2, on: f32, off: f32, col: Rgb, alpha: f32) {
        let len = a.distance(b);
        let mut t = 0.0;
        while t < len {
            let e = (t + on).min(len);
            self.line(a.lerp(b, t / len), a.lerp(b, e / len), 1.0, col, alpha);
            t += on + off;
        }
    }

    pub fn circle(&mut self, c: Vec2, r: f32, col: Rgb, alpha: f32) {
        let n = ((r * 6.3).ceil() as usize).clamp(12, 2000);
        let pts: Vec<Vec2> = (0..n).map(|k| c + Vec2::from_angle(k as f32 / n as f32 * std::f32::consts::TAU) * r).collect();
        self.polygon_stroke(&pts, 1.0, col, alpha);
    }

    pub fn polygon_stroke(&mut self, pts: &[Vec2], width: f32, col: Rgb, alpha: f32) {
        for k in 0..pts.len() {
            self.line(pts[k], pts[(k + 1) % pts.len()], width, col, alpha);
        }
    }

    /// Scanline fill of a simple polygon, sampling pixel centres.
    pub fn polygon_fill(&mut self, pts: &[Vec2], col: Rgb, alpha: f32) {
        if pts.len() < 3 {
            return;
        }
        let y0 = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min).floor().max(0.0) as i32;
        let y1 = pts.iter().map(|p| p.y).fold(f32::NEG_INFINITY, f32::max).ceil().min(self.h as f32) as i32;
        let mut xs = Vec::new();
        for y in y0..y1 {
            let sy = y as f32 + 0.5;
            xs.clear();
            for k in 0..pts.len() {
                let (a, b) = (pts[k], pts[(k + 1) % pts.len()]);
                if (a.y <= sy && b.y > sy) || (b.y <= sy && a.y > sy) {
                    xs.push(a.x + (sy - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
            xs.sort_by(f32::total_cmp);
            for pair in xs.chunks(2) {
                if let [l, r] = pair {
                    for x in (l - 0.5).ceil() as i32..=(r - 0.5).floor() as i32 {
                        self.blend(x, y, col, alpha);
                    }
                }
            }
        }
    }

    /// A line with an arrowhead at `b`.
    pub fn arrow(&mut self, a: Vec2, b: Vec2, col: Rgb, alpha: f32) {
        self.line(a, b, 1.0, col, alpha);
        let d = b - a;
        if d.length() < 2.0 {
            return;
        }
        let back = -d.normalize() * 5.0;
        for turn in [0.5f32, -0.5] {
            self.line(b, b + Vec2::from_angle(turn).rotate(back), 1.0, col, alpha);
        }
    }

    /// A small X.
    pub fn cross(&mut self, c: Vec2, r: f32, col: Rgb) {
        self.line(c - Vec2::splat(r), c + Vec2::splat(r), 2.0, col, 1.0);
        self.line(c + Vec2::new(-r, r), c + Vec2::new(r, -r), 2.0, col, 1.0);
    }

    /// Scales every channel by `k` (clamped).
    pub fn shade_px(c: Rgb, k: f32) -> Rgb {
        [(c[0] as f32 * k).clamp(0.0, 255.0) as u8, (c[1] as f32 * k).clamp(0.0, 255.0) as u8, (c[2] as f32 * k).clamp(0.0, 255.0) as u8]
    }

    /// RGBA bytes (alpha 255), as a GPU texture wants them.
    pub fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity((self.w * self.h * 4) as usize);
        for p in self.px.as_chunks::<3>().0 {
            out.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
        out
    }

    /// Binary PPM (P6), as in the tutorial's "Show it" step.
    pub fn to_ppm(&self) -> Vec<u8> {
        let mut out = format!("P6\n{} {}\n255\n", self.w, self.h).into_bytes();
        out.extend_from_slice(&self.px);
        out
    }
}
