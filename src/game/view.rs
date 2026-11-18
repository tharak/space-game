//! Drawing the battle: every sim ship gets a small triangle mesh (shared
//! mesh and a handful of shared materials, so Bevy batches hundreds of
//! them), divisions get a label, and weapon fire, explosions, the
//! selection and orders are drawn with gizmos.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::camera::CameraRig;
use super::flow::Phase;
use super::input::{MoveDrag, PointerMode, Selection};
use super::theme::*;
use crate::sim::battle::{Battle, SimEvent, SimSettings, Tuning};
use crate::sim::components::*;
use crate::sim::spatial::SpatialHash;

#[derive(Resource)]
pub struct ViewAssets {
    ship: Handle<Mesh>,
    flagship: Handle<Mesh>,
    /// [side][steady, shaken, routed, selected]
    mats: [[Handle<ColorMaterial>; 4]; 2],
    flag_mat: Handle<ColorMaterial>,
}

#[derive(Component)]
pub struct ShipVisual;

#[derive(Component)]
pub struct DivisionLabel(pub Entity);

struct Laser {
    from: Vec2,
    to: Vec2,
    side: usize,
    hit: bool,
    age: f32,
}

struct Boom {
    at: Vec2,
    side: usize,
    big: bool,
    age: f32,
}

#[derive(Resource, Default)]
pub struct Fx {
    lasers: Vec<Laser>,
    booms: Vec<Boom>,
}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ThinGizmos;

const MAX_LASERS: usize = 1500;

fn polygon(points: &[Vec2]) -> Mesh {
    let mut positions = vec![[0.0, 0.0, 0.0]];
    positions.extend(points.iter().map(|p| [p.x, p.y, 0.0]));
    let n = points.len() as u32;
    let indices: Vec<u32> = (0..n).flat_map(|i| [0, i + 1, (i + 1) % n + 1]).collect();
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
        .with_inserted_indices(Indices::U32(indices))
}

pub fn setup_view(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut gizmo_store: ResMut<GizmoConfigStore>,
) {
    let (config, _) = gizmo_store.config_mut::<DefaultGizmoConfigGroup>();
    config.line.width = 2.0;
    let (config, _) = gizmo_store.config_mut::<ThinGizmos>();
    config.line.width = 1.0;
    let dart = [Vec2::new(6.0, 0.0), Vec2::new(-4.5, 3.6), Vec2::new(-2.5, 0.0), Vec2::new(-4.5, -3.6)];
    let mut mat = |c: Color| materials.add(ColorMaterial::from_color(c));
    let mats = std::array::from_fn(|side| {
        [mat(SHIP[side][0]), mat(SHIP[side][1]), mat(SHIP[side][2]), mat(SHIP_SELECTED[side])]
    });
    commands.insert_resource(ViewAssets {
        ship: meshes.add(polygon(&dart)),
        flagship: meshes.add(polygon(&dart.map(|p| p * 1.8))),
        mats,
        flag_mat: mat(FLAGSHIP),
    });
}

pub fn attach_ship_visuals(
    mut commands: Commands,
    added: Query<(Entity, &Pos, &Heading, &Side, Has<FleetFlagship>), Added<Ship>>,
    assets: Res<ViewAssets>,
) {
    for (e, pos, heading, side, flagship) in &added {
        let (mesh, mat) = if flagship {
            (assets.flagship.clone(), assets.flag_mat.clone())
        } else {
            (assets.ship.clone(), assets.mats[side.0][0].clone())
        };
        commands.entity(e).insert((
            ShipVisual,
            Mesh2d(mesh),
            MeshMaterial2d(mat),
            Transform::from_translation(pos.0.extend(if flagship { 2.0 } else { 1.0 }))
                .with_rotation(Quat::from_rotation_z(heading.0)),
            Visibility::Inherited,
        ));
    }
}

pub fn sync_ships(
    time: Res<Time>,
    rig: Res<CameraRig>,
    sel: Res<Selection>,
    mut ships: Query<(&Pos, &Heading, &Side, &InDivision, Has<FleetFlagship>, &mut Transform, &mut MeshMaterial2d<ColorMaterial>)>,
    divs: Query<&Division>,
    assets: Res<ViewAssets>,
) {
    // The sim moves in 50 ms steps; ease toward them so motion is smooth.
    let k = 1.0 - (-time.delta_secs() * 18.0).exp();
    // Zoomed far out, draw ships a little larger so they stay visible.
    let size = Vec3::splat((rig.scale / 1.6).clamp(1.0, 2.2));
    for (pos, heading, side, div, flagship, mut t, mut mat) in &mut ships {
        let p = t.translation.truncate();
        let target = if p.distance_squared(pos.0) > 900.0 { pos.0 } else { p.lerp(pos.0, k) };
        t.translation = target.extend(t.translation.z);
        t.rotation = Quat::from_rotation_z(heading.0);
        t.scale = size;
        if flagship {
            continue;
        }
        let morale = divs.get(div.0).map_or(Morale::Steady, |d| d.morale);
        let i = if sel.division == Some(div.0) && morale == Morale::Steady { 3 } else { morale_index(morale) };
        let want = &assets.mats[side.0][i];
        if mat.0 != *want {
            mat.0 = want.clone();
        }
    }
}

/// One floating label per division: "B3 34" in its morale colour.
pub fn sync_labels(
    mut commands: Commands,
    rig: Res<CameraRig>,
    phase: Res<Phase>,
    hash: Res<SpatialHash>,
    divs: Query<(Entity, &Division)>,
    mut labels: Query<(Entity, &DivisionLabel, &mut Text2d, &mut TextColor, &mut Transform, &mut Visibility)>,
) {
    let mut have: Vec<Entity> = Vec::new();
    for (e, label, mut text, mut color, mut t, mut vis) in &mut labels {
        let Ok((_, d)) = divs.get(label.0) else {
            commands.entity(e).despawn();
            continue;
        };
        have.push(label.0);
        let shown = *phase != Phase::Menu && d.alive > 0;
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        if !shown {
            continue;
        }
        let want = format!("{} {}", d.label(), d.alive);
        if text.0 != want {
            text.0 = want;
        }
        let c = match d.morale {
            Morale::Steady => side_color(d.side),
            m => morale_color(m),
        };
        if color.0 != c {
            color.0 = c;
        }
        let centre = hash.centroid(label.0).unwrap_or(d.anchor);
        let behind = centre - d.forward() * (-d.depth + 26.0 * rig.scale.max(1.0));
        t.translation = behind.extend(5.0);
        t.scale = Vec3::splat(rig.scale);
    }
    for (e, d) in &divs {
        if !have.contains(&e) {
            commands.spawn((
                DivisionLabel(e),
                Text2d::new(d.label()),
                TextFont::from_font_size(13.0),
                TextColor(side_color(d.side)),
                Transform::from_translation(d.anchor.extend(5.0)),
                Visibility::Hidden,
            ));
        }
    }
}

pub fn collect_fx(mut events: MessageReader<SimEvent>, mut fx: ResMut<Fx>) {
    for event in events.read() {
        match event {
            SimEvent::Shot { from, to, side, hit } => {
                if fx.lasers.len() < MAX_LASERS {
                    fx.lasers.push(Laser { from: *from, to: *to, side: *side, hit: *hit, age: 0.0 });
                }
            }
            SimEvent::Destroyed { pos, side, flagship, .. } => {
                fx.booms.push(Boom { at: *pos, side: *side, big: *flagship, age: 0.0 });
            }
            _ => {}
        }
    }
}

pub fn draw_fx(time: Res<Time>, phase: Res<Phase>, mut fx: ResMut<Fx>, mut gizmos: Gizmos<ThinGizmos>) {
    if *phase == Phase::Menu {
        fx.lasers.clear();
        fx.booms.clear();
        return;
    }
    let dt = time.delta_secs();
    fx.lasers.retain_mut(|l| {
        l.age += dt;
        let life = if l.hit { LASER_HIT_SECS } else { LASER_MISS_SECS };
        if l.age >= life {
            return false;
        }
        let fade = 1.0 - l.age / life;
        let c = if l.hit { side_color(l.side).lighter(0.2) } else { side_color(l.side) };
        gizmos.line_2d(l.from, l.to, c.with_alpha(fade * if l.hit { 0.9 } else { 0.35 }));
        true
    });
    fx.booms.retain_mut(|b| {
        b.age += dt;
        if b.age >= BOOM_SECS {
            return false;
        }
        let t = b.age / BOOM_SECS;
        let r = if b.big { 10.0 + 40.0 * t } else { 3.0 + 10.0 * t };
        gizmos.circle_2d(b.at, r, GOLD.mix(&side_color(b.side), t).with_alpha(1.0 - t));
        true
    });
}

/// World-space box around a division's ships, in its own frame.
fn division_box(d: &Division, hash: &SpatialHash, e: Entity, pad: f32) -> Option<[Vec2; 5]> {
    let ships = hash.divisions.get(&e)?;
    let mut min = Vec2::splat(f32::MAX);
    let mut max = Vec2::splat(f32::MIN);
    for i in &ships.members {
        let l = d.to_local(hash.entries[*i as usize].pos);
        min = min.min(l);
        max = max.max(l);
    }
    min -= Vec2::splat(pad);
    max += Vec2::splat(pad);
    let corners = [min, Vec2::new(max.x, min.y), max, Vec2::new(min.x, max.y), min];
    Some(corners.map(|c| d.to_world(c)))
}

#[allow(clippy::too_many_arguments)]
pub fn draw_overlays(
    phase: Res<Phase>,
    battle: Res<Battle>,
    tuning: Res<Tuning>,
    settings: Res<SimSettings>,
    sel: Res<Selection>,
    drag: Res<MoveDrag>,
    rig: Res<CameraRig>,
    hash: Res<SpatialHash>,
    divs: Query<(Entity, &Division)>,
    flagships: Query<(&Pos, &Side), With<FleetFlagship>>,
    mut gizmos: Gizmos,
    mut thin: Gizmos<ThinGizmos>,
) {
    if *phase == Phase::Menu {
        return;
    }
    // Backdrop grid and the edge of the battle area.
    let b = battle.bounds;
    let step = 200.0;
    let mut x = (b.min.x / step).ceil() * step;
    while x <= b.max.x {
        thin.line_2d(Vec2::new(x, b.min.y), Vec2::new(x, b.max.y), GRID);
        x += step;
    }
    let mut y = (b.min.y / step).ceil() * step;
    while y <= b.max.y {
        thin.line_2d(Vec2::new(b.min.x, y), Vec2::new(b.max.x, y), GRID);
        y += step;
    }
    thin.rect_2d(b.center(), b.size(), BOUNDS);

    // Command radius around the player's flagship(s).
    for (p, s) in &flagships {
        if settings.control.human_controls(s.0) {
            thin.circle_2d(p.0, tuning.command_radius, side_color(s.0).with_alpha(0.18));
        }
    }

    // Own divisions: faint order lines; the selected one in full.
    for (e, d) in &divs {
        if d.alive == 0 || d.morale == Morale::Routed {
            continue;
        }
        let selected = sel.division == Some(e);
        let mine = settings.control.human_controls(d.side) || settings.control == crate::sim::battle::Control::Spectate;
        if !selected && !mine {
            continue;
        }
        let centre = hash.centroid(e).unwrap_or(d.anchor);
        let alpha = if selected { 0.9 } else { 0.25 };
        let color = side_color(d.side).with_alpha(alpha);
        match d.order {
            Order::MoveTo { to, facing } => {
                thin.line_2d(centre, to, color);
                let tip = to + Vec2::from_angle(facing) * 30.0;
                thin.arrow_2d(to, tip, color);
            }
            Order::Engage(t) => {
                if let Some(tc) = hash.centroid(t) {
                    thin.line_2d(centre, tc, RED.with_alpha(alpha * 0.8));
                }
            }
            _ => {}
        }
        if selected {
            if let Some(corners) = division_box(d, &hash, e, 10.0 * rig.scale.max(1.0)) {
                gizmos.linestrip_2d(corners, Color::WHITE.with_alpha(0.8));
            }
            gizmos.arrow_2d(centre, centre + d.forward() * (60.0 + 20.0 * rig.scale), Color::WHITE.with_alpha(0.8));
        }
    }

    // Move order being dragged out.
    if let Some(from) = drag.from {
        let c = if sel.mode == PointerMode::Move { GOLD } else { Color::WHITE };
        thin.circle_2d(from, 8.0 * rig.scale, c);
        if drag.to.distance(from) > 12.0 * rig.scale {
            gizmos.arrow_2d(from, drag.to, c.with_alpha(0.8));
        }
    }
}
