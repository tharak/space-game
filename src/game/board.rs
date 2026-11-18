//! Board rendering and picking. Squadron entities from the rules get their
//! visuals attached as children; everything else (tiles, ghosts during
//! deployment, laser beams, outlines) is presentation-only.

use bevy::asset::RenderAssetUsages;
use bevy::input::touch::Touches;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::model::UiModel;
use super::theme::*;
use crate::hex::{self, DIR_ANGLE, HS, HW, Hex, OX, OY};
use crate::rules::battle::{BattleEvent, Phase};
use crate::rules::commands::BattleCommand;
use crate::rules::components::*;
use crate::rules::config::*;

/// Board size in board pixels.
pub const BOARD_W: f32 = OX * 2.0 + HW * (COLS as f32 + 0.5);
pub const BOARD_H: f32 = OY * 2.0 + HS * 1.5 * (ROWS as f32 - 1.0) + HS * 2.0;
pub const PANEL_W: f32 = 360.0;

/// Board pixel (y down) to world (y up).
pub fn to_world(h: Hex) -> Vec2 {
    let (x, y) = hex::hex_center_px(h);
    Vec2::new(x, -y)
}

pub fn pixel_to_hex(p: Vec2) -> Option<Hex> {
    let mut best = None;
    let mut best_d = f32::INFINITY;
    for h in board_hexes() {
        let d = to_world(h).distance_squared(p);
        if d < best_d {
            best_d = d;
            best = Some(h);
        }
    }
    (best_d <= (HS * 1.05).powi(2)).then_some(best).flatten()
}

/// Where the board is drawn on screen (logical px, origin top-left).
#[derive(Resource, Default, Clone, Copy)]
pub struct BoardView {
    pub area: Rect,
    pub scale: f32,
    pub camera: Vec2,
    pub window: Vec2,
    pub portrait: bool,
}

impl BoardView {
    pub fn world_to_screen(&self, w: Vec2) -> Vec2 {
        Vec2::new(
            (w.x - self.camera.x) / self.scale + self.window.x / 2.0,
            -(w.y - self.camera.y) / self.scale + self.window.y / 2.0,
        )
    }

    pub fn screen_to_world(&self, s: Vec2) -> Vec2 {
        Vec2::new(
            self.camera.x + (s.x - self.window.x / 2.0) * self.scale,
            self.camera.y - (s.y - self.window.y / 2.0) * self.scale,
        )
    }
}

pub fn is_portrait(window: Vec2) -> bool {
    window.x < 760.0 || window.x < window.y * 0.95
}

pub fn panel_height(window: Vec2) -> f32 {
    (window.y * 0.46).max(220.0)
}

#[derive(Resource)]
pub struct BoardAssets {
    tile_outline: Handle<Mesh>,
    tile_fill: Handle<Mesh>,
    ship_border: Handle<Mesh>,
    ship_fill: Handle<Mesh>,
    outline_ring: Handle<Mesh>,
    arrow: Handle<Mesh>,
    pip: Handle<Mesh>,
    star: Handle<Mesh>,
    grid_line: Handle<ColorMaterial>,
    side_fill: [Handle<ColorMaterial>; 2],
    dark: Handle<ColorMaterial>,
    gold: Handle<ColorMaterial>,
    pip_on: Handle<ColorMaterial>,
    pip_off: Handle<ColorMaterial>,
    state: [Handle<ColorMaterial>; 4],
}

#[derive(Component)]
pub struct Tile {
    hex: Hex,
    material: Handle<ColorMaterial>,
}

#[derive(Component)]
pub struct ShipVisual {
    border: Entity,
    arrow: Entity,
    mark: Entity,
    pips: [Entity; 4],
}

#[derive(Component)]
pub struct SetupGhost;

#[derive(Component)]
pub struct Laser {
    from: Vec2,
    to: Vec2,
    side: usize,
    hit: bool,
    age: f32,
    duration: f32,
}

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct HaloGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct ThinGizmos;

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

/// Pointy-top hex corners around the origin, counter-clockwise (y up).
fn hex_points(radius: f32) -> Vec<Vec2> {
    (0..6).map(|k| Vec2::from_angle((90.0 + 60.0 * k as f32).to_radians()) * radius).collect()
}

fn ring_mesh(outer: f32, inner: f32) -> Mesh {
    let o = hex_points(outer);
    let i = hex_points(inner);
    let positions: Vec<[f32; 3]> = o.iter().chain(i.iter()).map(|p| [p.x, p.y, 0.0]).collect();
    let mut indices = Vec::new();
    for k in 0..6u32 {
        let k1 = (k + 1) % 6;
        indices.extend([k, k1, 6 + k1, k, 6 + k1, 6 + k]);
    }
    let count = positions.len();
    Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; count])
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0, 0.0]; count])
        .with_inserted_indices(Indices::U32(indices))
}

pub fn setup_board(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut gizmo_store: ResMut<GizmoConfigStore>,
) {
    commands.spawn((Camera2d, Msaa::Sample4));
    let (config, _) = gizmo_store.config_mut::<DefaultGizmoConfigGroup>();
    config.line.width = 2.5;
    config.line.joints = GizmoLineJoint::Miter;
    let (config, _) = gizmo_store.config_mut::<HaloGizmos>();
    config.line.width = 7.0;
    let (config, _) = gizmo_store.config_mut::<ThinGizmos>();
    config.line.width = 1.6;

    // Facing arrow in the front half of the token; the id sits behind it.
    let arrow = polygon(&[Vec2::new(HS - 4.0, 0.0), Vec2::new(1.5, 5.0), Vec2::new(1.5, -5.0)]);
    let star: Vec<Vec2> = (0..10)
        .map(|k| {
            let r = if k % 2 == 0 { 5.5 } else { 2.3 };
            Vec2::from_angle((90.0 + 36.0 * k as f32).to_radians()) * r
        })
        .collect();
    let mut mat = |c: Color| materials.add(ColorMaterial::from_color(c));
    let assets = BoardAssets {
        tile_outline: meshes.add(polygon(&hex_points(HS + 0.1))),
        tile_fill: meshes.add(polygon(&hex_points(HS - 1.0))),
        ship_border: meshes.add(polygon(&hex_points(HS - 2.75))),
        ship_fill: meshes.add(polygon(&hex_points(HS - 4.25))),
        outline_ring: meshes.add(ring_mesh(HS - 0.0, HS - 2.0)),
        arrow: meshes.add(arrow),
        pip: meshes.add(Circle::new(2.0)),
        star: meshes.add(polygon(&star)),
        grid_line: mat(GRID_LINE),
        side_fill: [mat(side_fill(0)), mat(side_fill(1))],
        dark: mat(DARK),
        gold: mat(GOLD),
        pip_on: mat(PIP_FILLED),
        pip_off: mat(PIP_EMPTY),
        state: [mat(DARK), mat(STATE_SHAKEN), mat(STATE_ROUTED), mat(STATE_ACTIVATED)],
    };
    for h in board_hexes() {
        let p = to_world(h);
        commands.spawn((
            Mesh2d(assets.tile_outline.clone()),
            MeshMaterial2d(assets.grid_line.clone()),
            Transform::from_xyz(p.x, p.y, 0.0),
        ));
        let material = materials.add(ColorMaterial::from_color(Color::srgb_from_array(rgb(GRID_CELL))));
        commands.spawn((
            Tile { hex: h, material: material.clone() },
            Mesh2d(assets.tile_fill.clone()),
            MeshMaterial2d(material),
            Transform::from_xyz(p.x, p.y, 0.1),
        ));
    }
    let _ = assets.outline_ring.clone();
    commands.insert_resource(assets);
}

/// Fits the board into the screen area not covered by the side panel.
pub fn fit_camera(
    window: Single<&Window, With<PrimaryWindow>>,
    mut camera: Single<(&mut Transform, &mut Projection), With<Camera2d>>,
    mut view: ResMut<BoardView>,
) {
    let win = Vec2::new(window.width(), window.height());
    if win.x <= 1.0 || win.y <= 1.0 {
        return;
    }
    let portrait = is_portrait(win);
    let area = if portrait {
        Rect::new(0.0, 0.0, win.x, win.y - panel_height(win))
    } else {
        Rect::new(0.0, 0.0, win.x - PANEL_W, win.y)
    };
    let margin = 8.0;
    let scale = (BOARD_W / (area.width() - margin * 2.0).max(50.0)).max(BOARD_H / (area.height() - margin * 2.0).max(50.0));
    let centre = area.center();
    let cam = Vec2::new(
        BOARD_W / 2.0 - (centre.x - win.x / 2.0) * scale,
        -BOARD_H / 2.0 + (centre.y - win.y / 2.0) * scale,
    );
    *view = BoardView { area, scale, camera: cam, window: win, portrait };
    let (transform, projection) = &mut *camera;
    if transform.translation.truncate() != cam {
        transform.translation = cam.extend(transform.translation.z);
    }
    if let Projection::Orthographic(ortho) = &mut **projection
        && ortho.scale != scale
    {
        ortho.scale = scale;
    }
}

pub fn paint_tiles(model: Res<UiModel>, tiles: Query<&Tile>, mut materials: ResMut<Assets<ColorMaterial>>) {
    if model.phase == Phase::Menu {
        return;
    }
    let setup_side = model.setup.as_ref().map(|s| s.side);
    for tile in &tiles {
        let mut c = rgb(if model.fire_zone.contains(&tile.hex) { FIRE_ZONE } else { GRID_CELL });
        if let Some(s) = setup_side
            && in_setup_zone(s, tile.hex)
        {
            c = blend(c, SIDE_FILL[s], SETUP_ALPHA);
        }
        for (flagship, fill) in model.flagships.iter().zip(SIDE_FILL) {
            if flagship.is_some_and(|f| hex::dist(f, tile.hex) <= CMD_R) {
                c = blend(c, fill, COMMAND_ALPHA);
            }
        }
        let color = Color::srgb_from_array(c);
        if materials.get(&tile.material).is_some_and(|m| m.color != color)
            && let Some(mut m) = materials.get_mut(&tile.material)
        {
            m.color = color;
        }
    }
}

/// Where the id label / flagship star sits: behind the facing arrow.
fn mark_offset(facing: u8) -> Vec2 {
    Vec2::from_angle((-DIR_ANGLE[facing as usize] as f32).to_radians()) * -5.5
}

fn facing_rotation(facing: u8) -> Quat {
    Quat::from_rotation_z((-DIR_ANGLE[facing as usize] as f32).to_radians())
}

/// Spawns a squadron token's parts; returns (border, arrow, mark).
fn spawn_token(
    parent: &mut ChildSpawnerCommands,
    assets: &BoardAssets,
    side: usize,
    border: Handle<ColorMaterial>,
    flagship: bool,
    label: &str,
    facing: u8,
) -> (Entity, Entity, Entity) {
    let border = parent.spawn((Mesh2d(assets.ship_border.clone()), MeshMaterial2d(border), Transform::default())).id();
    parent.spawn((
        Mesh2d(assets.ship_fill.clone()),
        MeshMaterial2d(assets.side_fill[side].clone()),
        Transform::from_xyz(0.0, 0.0, 0.1),
    ));
    let arrow = parent
        .spawn((
            Mesh2d(assets.arrow.clone()),
            MeshMaterial2d(if flagship { assets.gold.clone() } else { assets.dark.clone() }),
            Transform::from_xyz(0.0, 0.0, 0.2).with_rotation(facing_rotation(facing)),
        ))
        .id();
    let at = mark_offset(facing).extend(0.3);
    let mark = if flagship {
        parent.spawn((Mesh2d(assets.star.clone()), MeshMaterial2d(assets.dark.clone()), Transform::from_translation(at)))
    } else {
        parent.spawn((
            Text2d::new(label),
            TextFont::from_font_size(20.0),
            TextColor(DARK),
            Transform::from_translation(at).with_scale(Vec3::splat(0.5)),
        ))
    }
    .id();
    (border, arrow, mark)
}

pub fn attach_ship_visuals(
    mut commands: Commands,
    added: Query<(Entity, &Side, &Callsign, &HexPos, &Facing, Has<Flagship>), Added<Squadron>>,
    assets: Res<BoardAssets>,
) {
    for (e, side, label, pos, facing, flagship) in &added {
        let p = to_world(pos.0);
        let mut visual = None;
        commands.entity(e).insert((Transform::from_xyz(p.x, p.y, 2.0), Visibility::Inherited)).with_children(|parent| {
            let (border, arrow, mark) =
                spawn_token(parent, &assets, side.0, assets.state[0].clone(), flagship, &label.0[1..], facing.0);
            let pips = std::array::from_fn(|i| {
                parent
                    .spawn((
                        Mesh2d(assets.pip.clone()),
                        MeshMaterial2d(assets.pip_on.clone()),
                        Transform::from_xyz(-9.0 + i as f32 * 6.0, -(HS - 8.0), 0.3),
                    ))
                    .id()
            });
            visual = Some(ShipVisual { border, arrow, mark, pips });
        });
        if let Some(v) = visual {
            commands.entity(e).insert(v);
        }
    }
}

type ShipState<'a> = (
    &'a HexPos,
    &'a Facing,
    &'a Morale,
    &'a Strength,
    Has<Alive>,
    Has<Activated>,
    &'a ShipVisual,
    &'a mut Transform,
    &'a mut Visibility,
);

pub fn sync_ships(
    mut ships: Query<ShipState, With<Squadron>>,
    mut parts: Query<(&mut MeshMaterial2d<ColorMaterial>, &mut Transform), Without<Squadron>>,
    mut marks: Query<&mut Transform, (Without<Squadron>, Without<MeshMaterial2d<ColorMaterial>>)>,
    assets: Res<BoardAssets>,
) {
    for (pos, facing, morale, strength, alive, activated, visual, mut transform, mut visibility) in &mut ships {
        let want = if alive { Visibility::Inherited } else { Visibility::Hidden };
        visibility.set_if_neq(want);
        let p = to_world(pos.0);
        if transform.translation.truncate() != p {
            transform.translation = p.extend(2.0);
        }
        let state = match morale {
            Morale::Routed => 2,
            Morale::Shaken => 1,
            Morale::Steady if activated => 3,
            Morale::Steady => 0,
        };
        if let Ok((mut mat, _)) = parts.get_mut(visual.border)
            && mat.0 != assets.state[state]
        {
            mat.0 = assets.state[state].clone();
        }
        if let Ok((_, mut t)) = parts.get_mut(visual.arrow) {
            let r = facing_rotation(facing.0);
            if t.rotation != r {
                t.rotation = r;
            }
        }
        let at = mark_offset(facing.0).extend(0.3);
        let mark = match parts.get_mut(visual.mark) {
            Ok((_, t)) => Some(t),
            Err(_) => marks.get_mut(visual.mark).ok(),
        };
        if let Some(mut t) = mark
            && t.translation != at
        {
            t.translation = at;
        }
        for (i, pip) in visual.pips.iter().enumerate() {
            let want = if (i as u8) < strength.0 { &assets.pip_on } else { &assets.pip_off };
            if let Ok((mut mat, _)) = parts.get_mut(*pip)
                && mat.0 != *want
            {
                mat.0 = want.clone();
            }
        }
    }
}

/// Rebuilds the deployment ghosts whenever the placement changes.
pub fn sync_setup_ghosts(
    mut commands: Commands,
    model: Res<UiModel>,
    ghosts: Query<Entity, With<SetupGhost>>,
    assets: Res<BoardAssets>,
    mut last: Local<Option<(usize, Vec<(Hex, u8)>, Option<usize>)>>,
) {
    let current = model.setup.as_ref().map(|s| (s.side, s.placed.iter().map(|p| (p.hex, p.facing)).collect::<Vec<_>>(), s.flagship));
    if *last == current {
        return;
    }
    *last = current.clone();
    for g in &ghosts {
        commands.entity(g).despawn();
    }
    let Some((side, placed, flag)) = current else { return };
    for (i, (h, facing)) in placed.into_iter().enumerate() {
        let p = to_world(h);
        let is_flag = flag == Some(i);
        commands.spawn((SetupGhost, Transform::from_xyz(p.x, p.y, 2.0), Visibility::Inherited)).with_children(|parent| {
            let border = if is_flag { assets.gold.clone() } else { assets.dark.clone() };
            spawn_token(parent, &assets, side, border, is_flag, &(i + 1).to_string(), facing);
        });
    }
}

fn hex_outline(gizmos: &mut Gizmos, h: Hex, radius: f32, color: Color) {
    let c = to_world(h);
    let mut pts = hex_points(radius);
    pts.push(pts[0]);
    gizmos.linestrip_2d(pts.into_iter().map(|p| p + c), color);
}

pub fn draw_outlines(
    mut gizmos: Gizmos,
    model: Res<UiModel>,
    ships: Query<&HexPos, With<Alive>>,
) {
    if model.phase == Phase::Menu {
        return;
    }
    if let Some(sel) = model.selected
        && let Ok(pos) = ships.get(sel)
    {
        hex_outline(&mut gizmos, pos.0, HS - 1.0, SELECTION);
    }
    for t in &model.targets {
        if let Ok(pos) = ships.get(*t) {
            hex_outline(&mut gizmos, pos.0, HS - 1.0, TARGET);
        }
    }
    if let Some(setup) = &model.setup
        && let Some(i) = setup.selected
        && let Some(p) = setup.placed.get(i)
    {
        hex_outline(&mut gizmos, p.hex, HS - 1.0, SELECTION);
    }
}

/// Turns shot events into fading laser beams.
pub fn spawn_lasers(mut commands: Commands, mut events: MessageReader<BattleEvent>) {
    for event in events.read() {
        if let BattleEvent::Shot { from, to, side, hits, .. } = event {
            commands.spawn(Laser {
                from: to_world(*from),
                to: to_world(*to),
                side: *side,
                hit: *hits > 0,
                age: 0.0,
                duration: if *hits > 0 { LASER_HIT_SECS } else { LASER_MISS_SECS },
            });
        }
    }
}

pub fn draw_lasers(
    mut commands: Commands,
    time: Res<Time>,
    mut lasers: Query<(Entity, &mut Laser)>,
    mut main: Gizmos,
    mut halo: Gizmos<HaloGizmos>,
    mut thin: Gizmos<ThinGizmos>,
) {
    for (e, mut laser) in &mut lasers {
        laser.age += time.delta_secs();
        let t = laser.age / laser.duration;
        if t >= 1.0 {
            commands.entity(e).despawn();
            continue;
        }
        let color = side_fill(laser.side).with_alpha(1.0 - t);
        if laser.hit {
            halo.line_2d(laser.from, laser.to, side_fill(laser.side).with_alpha((1.0 - t) * 0.5));
            main.line_2d(laser.from, laser.to, color);
        } else {
            thin.line_2d(laser.from, laser.to, color);
        }
    }
}

/// Click or tap on the board becomes a `SelectHex` command.
pub fn board_input(
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    touches: Res<Touches>,
    view: Res<BoardView>,
    phase: Res<Phase>,
    mut commands: MessageWriter<BattleCommand>,
) {
    if !matches!(*phase, Phase::Deployment | Phase::Combat) {
        return;
    }
    let mut presses: Vec<Vec2> = touches.iter_just_pressed().map(|t| t.position()).collect();
    if mouse.just_pressed(MouseButton::Left)
        && let Some(p) = window.cursor_position()
    {
        presses.push(p);
    }
    for p in presses {
        if !view.area.contains(p) {
            continue;
        }
        if let Some(h) = pixel_to_hex(view.screen_to_world(p)) {
            commands.write(BattleCommand::SelectHex(h));
        }
    }
}
