#![allow(clippy::needless_range_loop)]
//! The campaign screen: the galaxy map (systems, lanes, the nebula, fleets
//! and their orders), selecting and ordering fleets, ending the turn, and
//! taking each battle to the pre-battle card, auto-resolve or the battle
//! screen and back.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;

use super::camera::CameraRig;
use super::flow::{LogBook, Phase, Tone};
use super::model::Spans;
use super::theme::*;
use crate::campaign::battle::{apply_report, battle_seed, forces_for, involves, withdraw};
use crate::campaign::map::{FORTRESS, Kind, POWERS, SysId, galaxy};
use crate::campaign::state::*;
use crate::campaign::turn::{end_turn, finish_turn, fleet_in_supply, supplied};
use crate::campaign::{Campaign, ai, battle_settings, save};
use crate::sim::battle::{Control, SimSettings};
use crate::sim::scenario::FleetShape;
use crate::sim::{AutoResolve, Report};

/// The player's side in the campaign.
pub const PLAYER: usize = 0;
pub const MAP_RECT: Rect = Rect { min: Vec2::new(-1120.0, -640.0), max: Vec2::new(1120.0, 640.0) };

#[derive(Resource, Default)]
pub struct CampaignState {
    pub c: Option<Campaign>,
    pub fleet: Option<u32>,
    pub system: Option<SysId>,
    /// The battle on the pre-battle card or being fought/resolved.
    pub battle: Option<u32>,
    /// The player's formation for that battle.
    pub shape: FleetShape,
    /// A turn has ended and its battles are being worked through.
    pub turn_open: bool,
    pub auto: Option<AutoResolve>,
    pub resolving_name: String,
}

impl CampaignState {
    pub fn pending_battle(&self) -> Option<&PendingBattle> {
        let c = self.c.as_ref()?;
        c.pending.iter().find(|b| Some(b.id) == self.battle)
    }
}

#[derive(Component)]
pub struct CampaignVisual;

#[derive(Component)]
pub struct SystemLabel(pub SysId);

#[derive(Component)]
pub struct FleetLabel(pub u32);

pub fn in_campaign(phase: Phase) -> bool {
    matches!(phase, Phase::Campaign | Phase::PreBattle | Phase::Resolving)
}

fn log_campaign(world: &mut World, lines: &[String]) {
    let mut log = world.resource_mut::<LogBook>();
    for l in lines {
        log.push(l.clone(), Tone::Plain);
    }
}

/// Removes any battle left on screen.
pub fn clear_battle(world: &mut World) {
    let old: Vec<Entity> = world
        .query_filtered::<Entity, Or<(With<crate::sim::components::Ship>, With<crate::sim::components::Division>)>>()
        .iter(world)
        .collect();
    for e in old {
        world.despawn(e);
    }
}

fn enter_campaign(world: &mut World) {
    clear_battle(world);
    *world.resource_mut::<Phase>() = Phase::Campaign;
    let mut rig = world.resource_mut::<CameraRig>();
    rig.fit_pending = true;
}

pub fn new_campaign(world: &mut World) {
    let seed = world.resource::<super::flow::SeedSource>().0 ^ 0xc0ff_ee11;
    let mut c = Campaign::new(seed);
    ai::orders(&mut c, 1 - PLAYER);
    save::save(&c);
    let first = c.log.clone();
    *world.resource_mut::<CampaignState>() = CampaignState { c: Some(c), ..default() };
    let mut log = world.resource_mut::<LogBook>();
    log.lines.clear();
    log.generation += 1;
    log.push("New campaign — you command the Meridian Compact (blue, west).", Tone::Title);
    log_campaign(world, &first);
    enter_campaign(world);
}

pub fn continue_campaign(world: &mut World) {
    let Some(c) = save::load() else { return new_campaign(world) };
    let open = !c.pending.is_empty();
    *world.resource_mut::<CampaignState>() = CampaignState { c: Some(c), turn_open: open, ..default() };
    let mut log = world.resource_mut::<LogBook>();
    log.lines.clear();
    log.generation += 1;
    log.push("Campaign resumed.", Tone::Title);
    enter_campaign(world);
}

/// Back from a fought battle: apply its result to the campaign.
pub fn return_to_campaign(world: &mut World) {
    let report = crate::sim::report(world);
    let mut st = world.resource_mut::<CampaignState>();
    if let (Some(id), Some(r), Some(c)) = (st.battle.take(), report, st.c.as_mut()) {
        apply_report(c, id, &r);
    }
    enter_campaign(world);
}

/// The player ends the turn.
pub fn end_player_turn(world: &mut World) {
    let mut st = world.resource_mut::<CampaignState>();
    let open = st.turn_open;
    let Some(c) = st.c.as_mut() else { return };
    if c.winner.is_some() || open {
        return;
    }
    end_turn(c);
    let turn = c.turn;
    save::save(c);
    st.turn_open = true;
    world.resource_mut::<LogBook>().push(format!("— Turn {turn} —"), Tone::Title);
}

/// Works through the turn's battles one at a time, then closes the turn.
pub fn process_turn(world: &mut World) {
    if *world.resource::<Phase>() != Phase::Campaign {
        return;
    }
    let mut st = world.resource_mut::<CampaignState>();
    if !st.turn_open {
        return;
    }
    let Some(c) = st.c.as_mut() else { return };
    // Log what the last battle produced.
    let lines: Vec<String> = std::mem::take(&mut c.log);
    let next = c.pending.first().cloned();
    match next {
        Some(b) if involves(&b, PLAYER) || (b.garrison[PLAYER] > 0) => {
            st.battle = Some(b.id);
            let lead = crate::campaign::battle::senior(st.c.as_ref().unwrap(), &b, PLAYER).map(|f| f.shape);
            st.shape = lead.unwrap_or(FleetShape::Line);
            *world.resource_mut::<Phase>() = Phase::PreBattle;
        }
        Some(b) => start_auto(&mut st, b),
        None => {
            let c = st.c.as_mut().unwrap();
            finish_turn(c, [PLAYER != 0, PLAYER != 1]);
            let mut lines2 = lines.clone();
            lines2.append(&mut c.log);
            st.turn_open = false;
            save::save(st.c.as_ref().unwrap());
            log_campaign(world, &lines2);
            return;
        }
    }
    if *world.resource::<Phase>() == Phase::Campaign && world.resource::<CampaignState>().auto.is_some() {
        *world.resource_mut::<Phase>() = Phase::Resolving;
    }
    log_campaign(world, &lines);
}

fn start_auto(st: &mut CampaignState, b: PendingBattle) {
    let c = st.c.as_ref().unwrap();
    let settings = battle_settings(c, &b, [None, None], Control::Spectate);
    let player_shape = if involves(&b, PLAYER) || b.garrison[PLAYER] > 0 { Some(st.shape) } else { None };
    let settings = match player_shape {
        Some(shape) => {
            let mut shapes = [None, None];
            shapes[PLAYER] = Some(shape);
            SimSettings { forces: Some(forces_for(c, &b, shapes)), ..settings }
        }
        None => settings,
    };
    st.resolving_name = place_name(b.place);
    st.auto = Some(AutoResolve::new(settings, battle_seed(c, &b)));
    st.battle = Some(b.id);
}

pub fn place_name(p: Place) -> String {
    let g = galaxy();
    match p {
        Place::System(s) => g.systems[s].name.to_string(),
        Place::Lane(a, b) => format!("{}–{} lane", g.systems[a].name, g.systems[b].name),
    }
}

/// Steps an auto-resolving battle a slice per frame.
pub fn tick_resolving(world: &mut World) {
    if *world.resource::<Phase>() != Phase::Resolving {
        return;
    }
    let mut st = world.resource_mut::<CampaignState>();
    let report: Option<Report> = st.auto.as_mut().and_then(|a| a.advance(400));
    if let Some(r) = report {
        st.auto = None;
        if let (Some(id), Some(c)) = (st.battle.take(), st.c.as_mut()) {
            apply_report(c, id, &r);
        }
        *world.resource_mut::<Phase>() = Phase::Campaign;
    }
}

/// Pre-battle choices.
pub fn prebattle_fight(world: &mut World) {
    let st = world.resource::<CampaignState>();
    let Some(b) = st.pending_battle().cloned() else { return };
    let mut shapes = [None, None];
    shapes[PLAYER] = Some(st.shape);
    let c = st.c.as_ref().unwrap();
    let settings = battle_settings(c, &b, shapes, if PLAYER == 0 { Control::PlayBlue } else { Control::PlayRed });
    let intro = vec![
        format!("Battle of {}", place_name(b.place)),
        describe_side(c, &b, PLAYER, "You"),
        describe_side(c, &b, 1 - PLAYER, "Enemy"),
    ];
    let seed = battle_seed(c, &b);
    let title = format!("Battle of {}", place_name(b.place));
    world.resource_mut::<CampaignState>().resolving_name = title;
    *world.resource_mut::<SimSettings>() = settings;
    world.resource_mut::<super::flow::SeedSource>().0 = seed;
    super::flow::start_battle_with(world, Some(intro));
}

pub fn prebattle_auto(world: &mut World) {
    let mut st = world.resource_mut::<CampaignState>();
    let Some(b) = st.pending_battle().cloned() else { return };
    start_auto(&mut st, b);
    *world.resource_mut::<Phase>() = Phase::Resolving;
}

pub fn prebattle_withdraw(world: &mut World) {
    let mut st = world.resource_mut::<CampaignState>();
    if let (Some(id), Some(c)) = (st.battle.take(), st.c.as_mut()) {
        withdraw(c, id, PLAYER);
    }
    *world.resource_mut::<Phase>() = Phase::Campaign;
}

fn describe_side(c: &Campaign, b: &PendingBattle, side: usize, who: &str) -> String {
    let fleets: Vec<String> = b.fleets[side]
        .iter()
        .filter_map(|id| c.fleet(*id))
        .map(|f| format!("{} {} ({} dmg)", f.admiral.title(), f.ships, f.damaged))
        .collect();
    let mut parts = fleets;
    if b.garrison[side] > 0 {
        parts.push(format!("garrison {}", b.garrison[side]));
    }
    let name = if b.fleets[side].is_empty() && b.garrison_owner == Some(None) { "Neutral".to_string() } else { POWERS[side].to_string() };
    format!("{who} — {name}: {}", parts.join(", "))
}

// ---------------------------------------------------------------------------
// Map drawing
// ---------------------------------------------------------------------------

fn owner_color(owner: Option<usize>) -> Color {
    match owner {
        Some(s) => side_color(s),
        None => Color::srgb(0.72, 0.72, 0.78),
    }
}

fn system_pos(s: SysId) -> Vec2 {
    let p = galaxy().systems[s].pos;
    Vec2::new(p.0, p.1)
}

fn system_radius(s: SysId) -> f32 {
    let sys = &galaxy().systems[s];
    10.0 + 5.0 * sys.value as f32 + if sys.kind == Kind::Capital { 6.0 } else { 0.0 }
}

/// Is a fleet visible to the player? (Own fleets always; enemy fleets in or
/// next to the player's systems or fleets.)
pub fn visible(c: &Campaign, f: &Fleet) -> bool {
    if f.side == PLAYER {
        return true;
    }
    let g = galaxy();
    let near = |s: SysId| {
        c.systems[s].owner == Some(PLAYER)
            || g.neighbours(s).any(|n| c.systems[n].owner == Some(PLAYER))
            || c.fleets.iter().any(|o| o.side == PLAYER && o.at().is_some_and(|x| x == s || g.lane(x, s).is_some()))
    };
    match f.loc {
        Location::At(s) => near(s),
        Location::Lane { from, to, .. } => near(from) || near(to),
    }
}

/// Where a fleet token is drawn: beside its system (stacked), or along its
/// lane.
pub fn fleet_pos(c: &Campaign, f: &Fleet) -> Vec2 {
    let g = galaxy();
    match f.loc {
        Location::At(s) => {
            let stack: Vec<u32> = c.fleets.iter().filter(|o| o.at() == Some(s) && visible(c, o)).map(|o| o.id).collect();
            let i = stack.iter().position(|x| *x == f.id).unwrap_or(0) as f32;
            let side_off = if f.side == 0 { -1.0 } else { 1.0 };
            system_pos(s) + Vec2::new(side_off * (system_radius(s) + 34.0), 18.0 - i * 22.0)
        }
        Location::Lane { from, to, left } => {
            let total = g.lane(from, to).map_or(1, |l| l.turns) as f32;
            let t = 1.0 - left as f32 / total;
            system_pos(from).lerp(system_pos(to), t.clamp(0.2, 0.8)) + Vec2::new(0.0, 14.0)
        }
    }
}

pub fn setup_campaign_map(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<ColorMaterial>>) {
    // The nebula: a few overlapping translucent bands.
    for (w, a) in [(380.0, 0.10), (300.0, 0.10), (200.0, 0.12)] {
        commands.spawn((
            CampaignVisual,
            Mesh2d(meshes.add(Rectangle::new(w, 1500.0))),
            MeshMaterial2d(materials.add(ColorMaterial::from_color(Color::srgba(0.55, 0.3, 0.75, a)))),
            Transform::from_xyz(0.0, 0.0, -5.0),
            Visibility::Hidden,
        ));
    }
    commands.spawn((
        CampaignVisual,
        Text2d::new("THE SHROUD"),
        TextFont::from_font_size(22.0),
        TextColor(Color::srgba(0.75, 0.6, 0.95, 0.35)),
        Transform::from_xyz(0.0, 0.0, -4.0),
        Visibility::Hidden,
    ));
    for s in 0..galaxy().systems.len() {
        commands.spawn((
            CampaignVisual,
            SystemLabel(s),
            Text2d::new(galaxy().systems[s].name),
            TextFont::from_font_size(13.0),
            TextColor(TEXT),
            Transform::from_translation((system_pos(s) - Vec2::new(0.0, system_radius(s) + 14.0)).extend(3.0)),
            Visibility::Hidden,
        ));
    }
}

/// Shows the campaign visuals in campaign phases and keeps the labels current.
pub fn sync_campaign_visuals(
    mut commands: Commands,
    phase: Res<Phase>,
    st: Res<CampaignState>,
    rig: Res<CameraRig>,
    mut visuals: Query<&mut Visibility, (With<CampaignVisual>, Without<FleetLabel>)>,
    mut sys_labels: Query<(&SystemLabel, &mut Text2d, &mut TextColor, &mut Transform), Without<FleetLabel>>,
    mut fleet_labels: Query<(Entity, &FleetLabel, &mut Text2d, &mut TextColor, &mut Transform, &mut Visibility), Without<SystemLabel>>,
) {
    let show = in_campaign(*phase) && st.c.is_some();
    for mut v in &mut visuals {
        v.set_if_neq(if show { Visibility::Inherited } else { Visibility::Hidden });
    }
    let Some(c) = st.c.as_ref().filter(|_| show) else {
        for (e, ..) in &fleet_labels {
            commands.entity(e).despawn();
        }
        return;
    };
    let label_scale = Vec3::splat(rig.scale.clamp(0.6, 2.5));
    let ok = [supplied(c, 0), supplied(c, 1)];
    for (l, mut text, mut color, mut t) in &mut sys_labels {
        let sys = &c.systems[l.0];
        let def = &galaxy().systems[l.0];
        let mut s = def.name.to_string();
        if sys.garrison > 0 {
            s.push_str(&format!(" ·{}", sys.garrison));
        }
        if sys.owner.is_some_and(|o| !ok[o][l.0]) {
            s.push_str(" (cut off)");
        }
        if text.0 != s {
            text.0 = s;
        }
        let c2 = owner_color(sys.owner).mix(&TEXT, 0.35);
        if color.0 != c2 {
            color.0 = c2;
        }
        t.scale = label_scale;
        t.translation = (system_pos(l.0) - Vec2::new(0.0, system_radius(l.0) + 14.0 * label_scale.x)).extend(3.0);
    }
    let mut have = Vec::new();
    for (e, l, mut text, mut color, mut t, mut vis) in &mut fleet_labels {
        let Some(f) = c.fleet(l.0) else {
            commands.entity(e).despawn();
            continue;
        };
        have.push(l.0);
        let shown = visible(c, f);
        vis.set_if_neq(if shown { Visibility::Inherited } else { Visibility::Hidden });
        let surname = f.admiral.name.split(' ').next_back().unwrap_or("").to_string();
        let s = format!("{surname} {}", f.ships);
        if text.0 != s {
            text.0 = s;
        }
        let want = if st.fleet == Some(f.id) { Color::WHITE } else { side_color(f.side) };
        if color.0 != want {
            color.0 = want;
        }
        let p = fleet_pos(c, f);
        t.translation = (p + Vec2::new(0.0, 15.0 * label_scale.x)).extend(4.0);
        t.scale = label_scale;
    }
    for f in &c.fleets {
        if !have.contains(&f.id) {
            commands.spawn((
                CampaignVisual,
                FleetLabel(f.id),
                Text2d::new(""),
                TextFont::from_font_size(12.0),
                TextColor(side_color(f.side)),
                Transform::from_translation(fleet_pos(c, f).extend(4.0)),
                Visibility::Hidden,
            ));
        }
    }
}

pub fn draw_campaign(phase: Res<Phase>, st: Res<CampaignState>, rig: Res<CameraRig>, mut gizmos: Gizmos) {
    if !in_campaign(*phase) {
        return;
    }
    let Some(c) = st.c.as_ref() else { return };
    let g = galaxy();
    // Lanes.
    for l in &g.lanes {
        let (a, b) = (system_pos(l.a), system_pos(l.b));
        let color = if l.turns > 1 { GOLD.with_alpha(0.45) } else { Color::srgba(0.6, 0.7, 0.9, 0.3) };
        gizmos.line_2d(a, b, color);
        if l.turns > 1 {
            let mid = a.lerp(b, 0.5);
            let n = (b - a).normalize().perp() * 6.0;
            gizmos.line_2d(mid - n, mid + n, color);
        }
    }
    // Systems.
    let ok = [supplied(c, 0), supplied(c, 1)];
    for (s, sys) in c.systems.iter().enumerate() {
        let p = system_pos(s);
        let r = system_radius(s);
        let mut color = owner_color(sys.owner);
        if sys.owner.is_some_and(|o| !ok[o][s]) {
            color = color.with_alpha(0.45);
        }
        gizmos.circle_2d(p, r, color);
        gizmos.circle_2d(p, r * 0.45, color.with_alpha(0.6));
        match g.systems[s].kind {
            Kind::Capital => {
                gizmos.circle_2d(p, r + 5.0, color);
            }
            Kind::Fortress => {
                gizmos.rect_2d(p, Vec2::splat(r * 2.6), color);
            }
            Kind::Hub => {
                let d = r + 7.0;
                gizmos.linestrip_2d([p + Vec2::Y * d, p + Vec2::X * d, p - Vec2::Y * d, p - Vec2::X * d, p + Vec2::Y * d], color);
            }
            Kind::Normal => {}
        }
        if st.system == Some(s) {
            gizmos.circle_2d(p, r + 10.0, Color::WHITE.with_alpha(0.8));
        }
    }
    // Fleets: a dart pointing where they're headed, and their path.
    for f in c.fleets.iter().filter(|f| visible(c, f)) {
        let p = fleet_pos(c, f);
        let next = match f.loc {
            Location::Lane { to, .. } => Some(to),
            Location::At(_) => f.path.first().copied(),
        };
        let dir = next.map_or(if f.side == 0 { Vec2::X } else { Vec2::NEG_X }, |n| (system_pos(n) - p).normalize_or(Vec2::X));
        let s = 9.0 + (f.ships as f32 / 40.0);
        let tip = p + dir * s;
        let back = p - dir * s * 0.6;
        let side_v = dir.perp() * s * 0.6;
        let color = if st.fleet == Some(f.id) { Color::WHITE } else { side_color(f.side) };
        gizmos.linestrip_2d([tip, back + side_v, p - dir * s * 0.2, back - side_v, tip], color);
        if f.supply_level() > 0 {
            gizmos.circle_2d(p, s + 4.0, if f.supply_level() == 1 { GOLD } else { BAD });
        }
        if f.side == PLAYER || st.fleet == Some(f.id) {
            let mut from = p;
            let alpha = if st.fleet == Some(f.id) { 0.9 } else { 0.35 };
            let pts: Vec<SysId> = match f.loc {
                Location::Lane { to, .. } => std::iter::once(to).chain(f.path.iter().copied()).collect(),
                _ => f.path.clone(),
            };
            for s in pts {
                let to = system_pos(s);
                gizmos.arrow_2d(from, from.lerp(to, 0.92), side_color(f.side).with_alpha(alpha));
                from = to;
            }
        }
    }
    // Pending battles: crossed swords.
    for b in &c.pending {
        let p = match b.place {
            Place::System(s) => system_pos(s) + Vec2::new(0.0, system_radius(s) + 10.0),
            Place::Lane(a, b2) => system_pos(a).lerp(system_pos(b2), 0.5),
        };
        let d = 9.0 * rig.scale.clamp(0.8, 2.0);
        gizmos.line_2d(p - Vec2::splat(d), p + Vec2::splat(d), GOLD);
        gizmos.line_2d(p + Vec2::new(-d, d), p + Vec2::new(d, -d), GOLD);
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn pick_fleet(c: &Campaign, world: Vec2, scale: f32) -> Option<u32> {
    c.fleets
        .iter()
        .filter(|f| visible(c, f))
        .map(|f| (f.id, fleet_pos(c, f).distance(world)))
        .filter(|(_, d)| *d < 16.0 * scale.max(0.7))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

fn pick_system(world: Vec2, scale: f32) -> Option<SysId> {
    (0..galaxy().systems.len())
        .map(|s| (s, system_pos(s).distance(world)))
        .filter(|(s, d)| *d < system_radius(*s) + 12.0 * scale.max(0.7))
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(s, _)| s)
}

/// Orders the selected fleet to `s` (or to hold, if it's already there).
pub fn order_move(st: &mut CampaignState, s: SysId) {
    let Some(id) = st.fleet else { return };
    let Some(c) = st.c.as_mut() else { return };
    let Some(f) = c.fleet(id).filter(|f| f.side == PLAYER) else { return };
    let start = match f.loc {
        Location::At(x) => x,
        Location::Lane { to, .. } => to,
    };
    let path = if start == s { Vec::new() } else { galaxy().path(start, s, |_| true).unwrap_or_default() };
    // A fleet in a lane finishes the crossing first, then follows the path.
    c.fleet_mut(id).unwrap().path = path;
}

#[derive(Default)]
pub struct CampPointer {
    left: Option<(Vec2, Vec2, bool)>,
    touch: Option<(u64, Vec2, bool)>,
    pinch: Option<f32>,
}

#[allow(clippy::too_many_arguments)]
pub fn campaign_pointer(
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<MouseWheel>,
    touches: Res<Touches>,
    phase: Res<Phase>,
    mut rig: ResMut<CameraRig>,
    mut st: ResMut<CampaignState>,
    mut requests: MessageWriter<super::flow::AppRequest>,
    mut state: Local<CampPointer>,
) {
    let dy: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            MouseScrollUnit::Line => ev.y * 40.0,
            MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if *phase != Phase::Campaign || st.c.is_none() {
        *state = CampPointer::default();
        return;
    }
    if keys.just_pressed(KeyCode::Enter) {
        requests.write(super::flow::AppRequest::EndTurn);
    }
    if keys.just_pressed(KeyCode::Escape) {
        st.fleet = None;
        st.system = None;
    }
    let area = rig.area;
    let cursor = window.cursor_position();
    let over = cursor.is_some_and(|c| area.contains(c));
    if over && dy != 0.0 {
        rig.zoom_at(cursor.unwrap(), 0.9f32.powf(dy / 40.0));
    }
    let mut clicks: Vec<(Vec2, bool)> = Vec::new();
    if mouse.just_pressed(MouseButton::Left)
        && over
        && let Some(c) = cursor
    {
        state.left = Some((c, c, false));
    }
    if let Some((start, last, dragging)) = state.left
        && let Some(c) = cursor
    {
        let dragging = dragging || c.distance(start) > 6.0;
        if dragging {
            rig.pan(c - last);
        }
        state.left = Some((start, c, dragging));
    }
    if mouse.just_released(MouseButton::Left)
        && let Some((start, _, dragging)) = state.left.take()
        && !dragging
    {
        clicks.push((start, false));
    }
    if mouse.just_pressed(MouseButton::Right)
        && over
        && let Some(c) = cursor
    {
        clicks.push((c, true));
    }
    // Touch: drag pans, pinch zooms, tap clicks.
    let fingers: Vec<_> = touches.iter().collect();
    if fingers.len() >= 2 {
        let d = fingers[0].position().distance(fingers[1].position());
        if let Some(prev) = state.pinch
            && d > 1.0
        {
            rig.zoom_at((fingers[0].position() + fingers[1].position()) / 2.0, prev / d);
        }
        state.pinch = Some(d);
        state.touch = None;
    } else {
        state.pinch = None;
        for t in touches.iter_just_pressed() {
            if area.contains(t.position()) {
                state.touch = Some((t.id(), t.position(), false));
            }
        }
        if let Some((id, start, dragging)) = state.touch
            && let Some(t) = touches.get_pressed(id)
        {
            let dragging = dragging || t.position().distance(start) > 12.0;
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
                    clicks.push((start, false));
                }
            }
        }
    }
    for (screen, secondary) in clicks {
        let world = rig.screen_to_world(screen);
        let c = st.c.as_ref().unwrap();
        let fleet = pick_fleet(c, world, rig.scale);
        let system = pick_system(world, rig.scale);
        let own_selected = st.fleet.and_then(|id| c.fleet(id)).is_some_and(|f| f.side == PLAYER);
        if let Some(id) = fleet.filter(|_| !secondary) {
            let at = c.fleet(id).and_then(|f| f.at());
            st.fleet = Some(id);
            st.system = at;
        } else if let Some(s) = system {
            if own_selected {
                order_move(&mut st, s);
            }
            st.system = Some(s);
        } else if !secondary {
            st.fleet = None;
            st.system = None;
        }
    }
}

// ---------------------------------------------------------------------------
// Read model for the panel
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FleetRow {
    pub id: u32,
    pub text: String,
    pub color: Color,
    pub selected: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CampaignModel {
    pub status: Vec<Spans>,
    pub info: Vec<Spans>,
    pub fleets: Vec<FleetRow>,
    pub prebattle: Vec<Spans>,
    pub resolving: String,
    pub shape: Option<FleetShape>,
    pub fleet_selected: bool,
    pub can_raise: bool,
    pub can_absorb: bool,
    pub can_end_turn: bool,
    pub over: Option<String>,
}

fn span(s: impl Into<String>, c: Color) -> (String, Color) {
    (s.into(), c)
}

pub fn build_campaign_model(st: &CampaignState, phase: Phase) -> CampaignModel {
    let mut m = CampaignModel::default();
    let Some(c) = st.c.as_ref() else { return m };
    let g = galaxy();
    let ok = [supplied(c, 0), supplied(c, 1)];
    let income = |side: usize| -> u32 { c.owned(side).filter(|s| ok[side][*s]).map(|s| g.systems[s].value).sum() };
    m.status.push(vec![span(format!("Turn {}  ", c.turn), GOLD), span("· Enter ends the turn", DIM)]);
    for side in 0..2 {
        let you = if side == PLAYER { " (you)" } else { "" };
        m.status.push(vec![
            span(format!("{}{you}: ", POWERS[side]), side_color(side)),
            span(format!("{} systems · +{}/turn · {} ships in fleets", c.owned(side).count(), income(side), c.ships(side)), TEXT),
        ]);
    }
    for f in c.fleets.iter().filter(|f| f.side == PLAYER) {
        let loc = match f.loc {
            Location::At(s) => g.systems[s].name.to_string(),
            Location::Lane { to, left, .. } => format!("→ {} ({left})", g.systems[to].name),
        };
        let supply = ["", " · LOW SUPPLY", " · NO SUPPLY"][f.supply_level() as usize];
        m.fleets.push(FleetRow {
            id: f.id,
            text: format!("{} · {} ships{} · {}{}", f.admiral.title(), f.ships, if f.damaged > 0 { format!(" ({} dmg)", f.damaged) } else { String::new() }, loc, supply),
            color: if f.supply_level() > 0 { GOLD } else { side_color(PLAYER) },
            selected: st.fleet == Some(f.id),
        });
    }

    if let Some(f) = st.fleet.and_then(|id| c.fleet(id)) {
        m.info.push(vec![span(f.admiral.title(), side_color(f.side)), span(format!(" · {}", POWERS[f.side]), DIM)]);
        let traits = if f.admiral.traits.is_empty() { "none yet".to_string() } else { f.admiral.traits.iter().map(|t| t.name()).collect::<Vec<_>>().join(", ") };
        m.info.push(vec![span(format!("Traits: {traits} · XP {}", f.admiral.xp), TEXT)]);
        m.info.push(vec![span(format!("{} ships, {} damaged · formation {}", f.ships, f.damaged, f.shape.name()), TEXT)]);
        let in_supply = fleet_in_supply(f, &ok[f.side]);
        m.info.push(vec![span(
            if in_supply { "In supply".to_string() } else { format!("Out of supply for {} turn(s)", f.unsupplied.max(1)) },
            if in_supply { DIM } else { BAD },
        )]);
        if f.side == PLAYER {
            let path: Vec<&str> = f.path.iter().map(|s| g.systems[*s].name).collect();
            m.info.push(vec![span(
                if path.is_empty() { "Holding. Click a system to send it there.".to_string() } else { format!("Orders: {}", path.join(" → ")) },
                DIM,
            )]);
            m.fleet_selected = true;
            m.shape = Some(f.shape);
            m.can_absorb = f.at().is_some_and(|s| c.systems[s].owner == Some(PLAYER) && c.systems[s].garrison > 0 && f.ships < FLEET_CAP);
        }
    }
    if let Some(s) = st.system {
        let sys = &c.systems[s];
        let def = &g.systems[s];
        let owner = sys.owner.map_or("Neutral", |o| POWERS[o]);
        let kind = match def.kind {
            Kind::Capital => " · capital",
            Kind::Fortress => " · fortress",
            Kind::Hub => " · trade hub",
            Kind::Normal => "",
        };
        m.info.push(vec![span(def.name.to_string(), GOLD), span(format!(" · {owner}{kind}"), owner_color(sys.owner))]);
        let supplied_note = match sys.owner {
            Some(o) if !ok[o][s] => " · cut off (no income)",
            _ => "",
        };
        m.info.push(vec![span(format!("+{} ships/turn · garrison {}{supplied_note}", def.value, sys.garrison), TEXT)]);
        if s == FORTRESS {
            m.info.push(vec![span("Fortress guns: its garrison fights with +25% fire and steadier morale.", DIM)]);
        }
        m.can_raise = sys.owner == Some(PLAYER) && sys.garrison >= 20 + if def.kind == Kind::Capital { 40 } else { 0 };
    }
    if m.info.is_empty() {
        m.info.push(vec![span("Select a fleet (on the map or in the list), then click a system to send it there.", DIM)]);
        m.info.push(vec![span("Hold systems for ship income; stay connected to your capital for supply. Take the enemy capital to win.", DIM)]);
    }
    m.can_end_turn = phase == Phase::Campaign && !st.turn_open && c.winner.is_none();

    if let Some(b) = st.pending_battle().filter(|_| phase == Phase::PreBattle) {
        m.prebattle.push(vec![span(format!("Battle of {}", place_name(b.place)), GOLD)]);
        for (side, who) in [(PLAYER, "You"), (1 - PLAYER, "Enemy")] {
            m.prebattle.push(vec![span(describe_side(c, b, side, who), side_color(side))]);
        }
        let f = forces_for(c, b, {
            let mut s = [None, None];
            s[PLAYER] = Some(st.shape);
            s
        });
        m.prebattle.push(vec![span(
            format!("{} ships vs {} — enemy will form {}", f[PLAYER].ships, f[1 - PLAYER].ships, f[1 - PLAYER].shape.name()),
            TEXT,
        )]);
        m.shape = Some(st.shape);
    }
    if phase == Phase::Resolving {
        let pct = st.auto.as_ref().map_or(0.0, |a| a.progress());
        m.resolving = format!("Resolving the battle of {}… {:.0}%", st.resolving_name, pct * 100.0);
    }
    if let Some(w) = c.winner {
        m.over = Some(if w == PLAYER {
            format!("Victory — the {} has won the war in {} turns.", POWERS[w], c.turn - 1)
        } else {
            format!("Defeat — the {} has won the war.", POWERS[w])
        });
    }
    m
}
