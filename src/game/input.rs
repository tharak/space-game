//! Mouse, touch and keyboard on the battle map.
//!
//! * Left click / tap a ship: select its division. With one of your
//!   divisions selected, clicking an enemy ship orders it to engage.
//! * Right click (or tap in Move mode): move the selected division there;
//!   drag while holding to set the facing.
//! * Left drag / one-finger drag: pan. Wheel / pinch: zoom.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::camera::CameraRig;
use super::flow::{OverlayHidden, Phase, Speed};
use crate::sim::Command;
use crate::sim::battle::SimSettings;
use crate::sim::components::{Division, Morale, Order};
use crate::sim::formation::Formation;
use crate::sim::spatial::SpatialHash;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PointerMode {
    #[default]
    Normal,
    /// The next click on the map moves the selected division there.
    Move,
}

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Selection {
    pub division: Option<Entity>,
    pub mode: PointerMode,
}

/// An in-progress move order being dragged out (for the preview).
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct MoveDrag {
    pub from: Option<Vec2>,
    pub to: Vec2,
}

#[derive(Default)]
pub struct PointerState {
    left: Option<(Vec2, Vec2, bool)>,
    right: Option<Vec2>,
    touch: Option<(u64, Vec2, bool)>,
    pinch: Option<f32>,
}

const DRAG_PX: f32 = 6.0;
const PICK_PX: f32 = 14.0;

/// The division of the ship nearest `world` within a few pixels.
pub fn pick_division(hash: &SpatialHash, world: Vec2, scale: f32) -> Option<(Entity, usize)> {
    let mut best: Option<(Entity, usize, f32)> = None;
    hash.for_each_within(world, PICK_PX * scale.max(0.5), |e, d2| {
        if best.is_none_or(|b| d2 < b.2) {
            best = Some((e.division, e.side, d2));
        }
    });
    best.map(|(d, s, _)| (d, s))
}

/// What a click at `world` means: select, engage or move.
#[allow(clippy::too_many_arguments)]
pub fn click_commands(
    world: Vec2,
    scale: f32,
    sel: &mut Selection,
    hash: &SpatialHash,
    divs: &Query<(Entity, &Division)>,
    settings: &SimSettings,
    secondary: bool,
    facing: Option<f32>,
) -> Option<Command> {
    let own = sel
        .division
        .and_then(|e| divs.get(e).ok())
        .filter(|(_, d)| settings.control.human_controls(d.side) && d.morale != Morale::Routed && d.alive > 0);
    let picked = pick_division(hash, world, scale);
    if let (Some((me, d)), Some((target, side))) = (own, picked)
        && side != d.side
    {
        sel.mode = PointerMode::Normal;
        return Some(Command::Order { division: me, order: Order::Engage(target) });
    }
    if let Some((me, d)) = own
        && (secondary || sel.mode == PointerMode::Move)
    {
        sel.mode = PointerMode::Normal;
        let travel = world - d.anchor;
        let facing = facing.unwrap_or(if travel.length() > 40.0 { travel.to_angle() } else { d.facing });
        return Some(Command::Order { division: me, order: Order::MoveTo { to: world, facing } });
    }
    if !secondary && let Some((div, _)) = picked {
        sel.division = Some(div);
        sel.mode = PointerMode::Normal;
    }
    None
}

#[allow(clippy::too_many_arguments)]
pub fn pointer(
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut wheel: MessageReader<MouseWheel>,
    touches: Res<Touches>,
    phase: Res<Phase>,
    overlay_hidden: Res<OverlayHidden>,
    mut rig: ResMut<CameraRig>,
    mut sel: ResMut<Selection>,
    mut drag: ResMut<MoveDrag>,
    hash: Res<SpatialHash>,
    divs: Query<(Entity, &Division)>,
    settings: Res<SimSettings>,
    mut out: MessageWriter<Command>,
    mut state: Local<PointerState>,
) {
    let wheel_dy: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            MouseScrollUnit::Line => ev.y * 40.0,
            MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    let active = *phase == Phase::Battle || (*phase == Phase::Over && overlay_hidden.0);
    if !active {
        *state = PointerState::default();
        drag.from = None;
        return;
    }
    let area = rig.area;
    let cursor = window.cursor_position();
    let over_map = cursor.is_some_and(|c| area.contains(c));

    if over_map && wheel_dy != 0.0 {
        rig.zoom_at(cursor.unwrap(), 0.9f32.powf(wheel_dy / 40.0));
    }

    // Left button: drag to pan, click to select / engage / move.
    if mouse.just_pressed(MouseButton::Left)
        && over_map
        && let Some(c) = cursor
    {
        state.left = Some((c, c, false));
    }
    if let Some((start, last, dragging)) = state.left
        && let Some(c) = cursor
    {
        let dragging = dragging || c.distance(start) > DRAG_PX;
        if dragging {
            rig.pan(c - last);
        }
        state.left = Some((start, c, dragging));
    }
    if mouse.just_released(MouseButton::Left)
        && let Some((start, _, dragging)) = state.left.take()
        && !dragging
    {
        let world = rig.screen_to_world(start);
        if let Some(cmd) = click_commands(world, rig.scale, &mut sel, &hash, &divs, &settings, false, None) {
            out.write(cmd);
        }
    }

    // Right button: move (drag sets facing) or engage.
    if mouse.just_pressed(MouseButton::Right)
        && over_map
        && let Some(c) = cursor
    {
        let w = rig.screen_to_world(c);
        state.right = Some(w);
        drag.from = Some(w);
        drag.to = w;
    }
    if state.right.is_some()
        && let Some(c) = cursor
    {
        drag.to = rig.screen_to_world(c);
    }
    if mouse.just_released(MouseButton::Right)
        && let Some(from) = state.right.take()
    {
        drag.from = None;
        let to = cursor.map_or(from, |c| rig.screen_to_world(c));
        let facing = (to.distance(from) > 12.0 * rig.scale).then(|| (to - from).to_angle());
        if let Some(cmd) = click_commands(from, rig.scale, &mut sel, &hash, &divs, &settings, true, facing) {
            out.write(cmd);
        }
    }

    // Touch: one finger pans or taps; two fingers pinch-zoom.
    let fingers: Vec<_> = touches.iter().collect();
    if fingers.len() >= 2 {
        let d = fingers[0].position().distance(fingers[1].position());
        if let Some(prev) = state.pinch
            && d > 1.0
        {
            let mid = (fingers[0].position() + fingers[1].position()) / 2.0;
            rig.zoom_at(mid, prev / d);
        }
        state.pinch = Some(d);
        state.touch = None;
        return;
    }
    state.pinch = None;
    for t in touches.iter_just_pressed() {
        if area.contains(t.position()) {
            state.touch = Some((t.id(), t.position(), false));
        }
    }
    if let Some((id, start, dragging)) = state.touch
        && let Some(t) = touches.get_pressed(id)
    {
        let dragging = dragging || t.position().distance(start) > DRAG_PX * 2.0;
        if dragging {
            rig.pan(t.delta());
        }
        state.touch = Some((id, start, dragging));
    }
    for t in touches.iter_just_released() {
        if let Some((id, start, dragging)) = state.touch
            && id == t.id()
        {
            state.touch = None;
            if !dragging {
                let world = rig.screen_to_world(start);
                if let Some(cmd) = click_commands(world, rig.scale, &mut sel, &hash, &divs, &settings, false, None) {
                    out.write(cmd);
                }
            }
        }
    }
}

/// Divisions the player can order, in fleet order.
pub fn own_divisions(divs: &Query<(Entity, &Division)>, settings: &SimSettings) -> Vec<Entity> {
    let mut own: Vec<(usize, usize, Entity)> = divs
        .iter()
        .filter(|(_, d)| settings.control.human_controls(d.side) && d.alive > 0 && d.morale != Morale::Routed)
        .map(|(e, d)| (d.side, d.number, e))
        .collect();
    own.sort();
    own.into_iter().map(|(_, _, e)| e).collect()
}

#[allow(clippy::too_many_arguments)]
pub fn keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    phase: Res<Phase>,
    mut speed: ResMut<Speed>,
    mut sel: ResMut<Selection>,
    mut rig: ResMut<CameraRig>,
    divs: Query<(Entity, &Division)>,
    settings: Res<SimSettings>,
    mut out: MessageWriter<Command>,
) {
    if !matches!(*phase, Phase::Battle | Phase::Over) {
        return;
    }
    if keys.just_pressed(KeyCode::Space) {
        speed.paused = !speed.paused;
    }
    for (key, mult) in [(KeyCode::Digit1, 1), (KeyCode::Digit2, 2), (KeyCode::Digit3, 4), (KeyCode::Digit4, 8)] {
        if keys.just_pressed(key) {
            speed.mult = mult;
            speed.paused = false;
        }
    }
    // Pan with arrows (WASD is kept for orders-free panning too).
    let mut pan = Vec2::ZERO;
    for (key, dir) in [
        (KeyCode::ArrowLeft, Vec2::NEG_X),
        (KeyCode::ArrowRight, Vec2::X),
        (KeyCode::ArrowUp, Vec2::Y),
        (KeyCode::ArrowDown, Vec2::NEG_Y),
        (KeyCode::KeyA, Vec2::NEG_X),
        (KeyCode::KeyD, Vec2::X),
        (KeyCode::KeyW, Vec2::Y),
        (KeyCode::KeyS, Vec2::NEG_Y),
    ] {
        if keys.pressed(key) {
            pan += dir;
        }
    }
    if pan != Vec2::ZERO {
        let step = pan * 600.0 * time.delta_secs();
        rig.pan(Vec2::new(-step.x, step.y));
    }
    let centre = rig.area.center();
    if keys.just_pressed(KeyCode::Equal) {
        rig.zoom_at(centre, 0.8);
    }
    if keys.just_pressed(KeyCode::Minus) {
        rig.zoom_at(centre, 1.25);
    }
    if keys.just_pressed(KeyCode::Escape) {
        if sel.mode != PointerMode::Normal {
            sel.mode = PointerMode::Normal;
        } else {
            sel.division = None;
        }
    }
    if keys.just_pressed(KeyCode::Tab) {
        let own = own_divisions(&divs, &settings);
        if !own.is_empty() {
            let i = sel.division.and_then(|d| own.iter().position(|e| *e == d)).map_or(0, |i| (i + 1) % own.len());
            sel.division = Some(own[i]);
        }
    }
    let Some(me) = sel.division.filter(|d| own_divisions(&divs, &settings).contains(d)) else { return };
    for (key, order) in
        [(KeyCode::KeyH, Order::Hold), (KeyCode::KeyG, Order::Advance), (KeyCode::KeyC, Order::Charge), (KeyCode::KeyX, Order::Withdraw)]
    {
        if keys.just_pressed(key) {
            out.write(Command::Order { division: me, order });
        }
    }
    if keys.just_pressed(KeyCode::KeyM) {
        sel.mode = if sel.mode == PointerMode::Move { PointerMode::Normal } else { PointerMode::Move };
    }
    if keys.just_pressed(KeyCode::KeyF)
        && let Ok((_, d)) = divs.get(me)
    {
        let all = Formation::PLAYER;
        let i = all.iter().position(|f| *f == d.formation).map_or(0, |i| (i + 1) % all.len());
        out.write(Command::Formation { division: me, formation: all[i] });
    }
}
