//! The 2D camera: pan and zoom over the battle area, with the side panel
//! taking part of the window.

use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use crate::sim::battle::Battle;
use crate::sim::components::Division;

pub const PANEL_W: f32 = 380.0;

pub fn is_portrait(window: Vec2) -> bool {
    window.x < 760.0 || window.x < window.y * 0.95
}

pub fn panel_height(window: Vec2) -> f32 {
    (window.y * 0.46).max(220.0)
}

/// Screen area (logical px, origin top-left) that shows the battle.
pub fn map_area(window: Vec2) -> Rect {
    if is_portrait(window) {
        Rect::new(0.0, 0.0, window.x, window.y - panel_height(window))
    } else {
        Rect::new(0.0, 0.0, window.x - PANEL_W, window.y)
    }
}

#[derive(Resource, Clone, Copy, Debug)]
pub struct CameraRig {
    /// World point at the window centre.
    pub center: Vec2,
    /// World units per logical pixel.
    pub scale: f32,
    pub window: Vec2,
    pub area: Rect,
    pub fit_pending: bool,
}

impl Default for CameraRig {
    fn default() -> Self {
        Self { center: Vec2::ZERO, scale: 2.0, window: Vec2::new(1280.0, 820.0), area: Rect::default(), fit_pending: true }
    }
}

impl CameraRig {
    pub fn world_to_screen(&self, w: Vec2) -> Vec2 {
        Vec2::new(
            (w.x - self.center.x) / self.scale + self.window.x / 2.0,
            -(w.y - self.center.y) / self.scale + self.window.y / 2.0,
        )
    }

    pub fn screen_to_world(&self, s: Vec2) -> Vec2 {
        Vec2::new(
            self.center.x + (s.x - self.window.x / 2.0) * self.scale,
            self.center.y - (s.y - self.window.y / 2.0) * self.scale,
        )
    }

    /// Shows world rect `region` centred in the map area.
    pub fn fit(&mut self, region: Rect) {
        let area = self.area;
        if area.width() < 10.0 || area.height() < 10.0 {
            return;
        }
        self.scale = (region.width() / area.width()).max(region.height() / area.height()).clamp(MIN_SCALE, MAX_SCALE);
        let offset = area.center() - self.window / 2.0;
        self.center = region.center() - Vec2::new(offset.x, -offset.y) * self.scale;
    }

    /// Zooms by `factor` keeping the world point under `screen` fixed.
    pub fn zoom_at(&mut self, screen: Vec2, factor: f32) {
        let before = self.screen_to_world(screen);
        self.scale = (self.scale * factor).clamp(MIN_SCALE, MAX_SCALE);
        let after = self.screen_to_world(screen);
        self.center += before - after;
    }

    /// Pans by a screen-space drag.
    pub fn pan(&mut self, screen_delta: Vec2) {
        self.center += Vec2::new(-screen_delta.x, screen_delta.y) * self.scale;
    }
}

pub const MIN_SCALE: f32 = 0.25;
pub const MAX_SCALE: f32 = 8.0;

pub fn setup_camera(mut commands: Commands) {
    commands.spawn((Camera2d, Msaa::Sample4));
}

pub fn apply_camera(
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<(&mut Transform, &mut Projection), With<Camera2d>>,
    mut rig: ResMut<CameraRig>,
    battle: Res<Battle>,
    divs: Query<&Division>,
    phase: Res<super::flow::Phase>,
) {
    let campaign = super::campaign_ui::in_campaign(*phase);
    let win = Vec2::new(window.width(), window.height());
    if win.x <= 1.0 || win.y <= 1.0 {
        return;
    }
    if rig.window != win {
        rig.window = win;
        rig.area = map_area(win);
    }
    if rig.area.width() < 1.0 {
        rig.area = map_area(win);
    }
    if rig.fit_pending && campaign {
        rig.fit_pending = false;
        rig.fit(super::campaign_ui::MAP_RECT);
    } else if rig.fit_pending && !divs.is_empty() {
        rig.fit_pending = false;
        let (mut min, mut max) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
        for d in &divs {
            let r = Vec2::splat(d.half_width.max(-d.depth) + 20.0);
            min = min.min(d.anchor - r);
            max = max.max(d.anchor + r);
        }
        rig.fit(Rect::from_corners(min, max).inflate(60.0));
    }
    // Keep the battle area in view.
    let b = if campaign { super::campaign_ui::MAP_RECT } else { battle.bounds };
    if b.width() > 0.0 {
        rig.center = rig.center.clamp(b.min, b.max);
    }
    let (transform, projection) = &mut *camera;
    if transform.translation.truncate() != rig.center {
        transform.translation = rig.center.extend(transform.translation.z);
    }
    if let Projection::Orthographic(ortho) = &mut **projection
        && ortho.scale != rig.scale
    {
        ortho.scale = rig.scale;
    }
}
