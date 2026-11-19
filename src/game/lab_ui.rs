//! The start page (shared Voronoi config + the list of labs) and the lab
//! page: the lab's controls, a status line and its image. Every frame of
//! the image comes from [`crate::voronoi::labs::render`]; this module only
//! turns clicks, drags and sliders into config and lab-state changes.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::RelativeCursorPosition;

use super::flow::{AppRequest, Phase};
use super::theme::*;
use super::ui::{BtnKind, BtnLabel, Enabled, Selected, button_for, card_title, choice_row, panel, row, text};
use crate::voronoi::Vec2 as MapVec;
use crate::voronoi::labs::{FACTIONS, LabId, LabState, render, site_at, status, zoom_inset};
use crate::voronoi::metric::Metric;
use crate::voronoi::noise::{NoiseMode, NoisePreset};
use crate::voronoi::raster::Canvas;
use crate::voronoi::sites::VoronoiConfig;

/// The config every lab reads.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct LabsConfig(pub VoronoiConfig);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    Probe,
    Site(usize),
    Base(usize),
}

/// Lab state plus what the page needs to redraw only on change.
#[derive(Resource)]
pub struct Labs {
    pub state: LabState,
    image: Handle<Image>,
    /// What the cached base image was rendered from.
    key: String,
    base: Option<Canvas>,
    /// What the texture currently shows (base key + hover).
    shown: String,
    hover: Option<MapVec>,
    drag: Option<Drag>,
    playing: bool,
    timer: f32,
    last_cfg: VoronoiConfig,
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub enum LabAction {
    // Navigation
    Open(LabId),
    Back,
    Game,
    // Shared config
    SeedDelta(i32),
    NewSeed,
    Metric(Metric),
    Relax(u32),
    // Per lab
    Play,
    GrowFronts,
    DistanceLines,
    OwnerBorders,
    Cells,
    Triangles,
    Circles,
    ResetLab,
    LloydStep,
    Centroids,
    BiomeBorders,
    BiomeSites,
    Destroy(usize),
    UseWeights,
    Mode(NoiseMode),
    Preset(NoisePreset),
    NewWorld,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SliderParam {
    Sites,
    GrowRadius,
    Crack,
    Weight(usize),
}

impl SliderParam {
    fn range(self, cfg: &VoronoiConfig) -> (f32, f32, f32) {
        match self {
            SliderParam::Sites => (3.0, 250.0, 1.0),
            SliderParam::GrowRadius => (0.0, LabState::grow_max(cfg), 1.0),
            SliderParam::Crack => (0.5, 10.0, 0.1),
            SliderParam::Weight(_) => (0.2, 4.0, 0.1),
        }
    }

    fn get(self, cfg: &VoronoiConfig, st: &LabState) -> f32 {
        match self {
            SliderParam::Sites => cfg.sites as f32,
            SliderParam::GrowRadius => st.grow_radius,
            SliderParam::Crack => st.crack,
            SliderParam::Weight(i) => st.weights[i],
        }
    }

    fn label(self, v: f32) -> String {
        match self {
            SliderParam::Sites => format!("{v:.0}"),
            SliderParam::GrowRadius => format!("{v:.0} px"),
            SliderParam::Crack => format!("{v:.1}"),
            SliderParam::Weight(_) => format!("{v:.1}×"),
        }
    }
}

#[derive(Component)]
pub struct SliderTrack(SliderParam);
#[derive(Component)]
pub struct SliderFill(SliderParam);
#[derive(Component)]
pub struct SliderValue(SliderParam);

/// Which slider is being dragged.
#[derive(Resource, Default)]
pub struct ActiveSlider(Option<SliderParam>);

#[derive(Component, Clone, Copy, PartialEq)]
pub enum LabText {
    Seed,
    Title,
    Blurb,
    Config,
    Status,
}

#[derive(Component)]
pub struct StartPage;
#[derive(Component)]
pub struct LabPage;
/// A lab's own controls, shown on its page only.
#[derive(Component)]
pub struct LabControls(LabId);
#[derive(Component)]
pub struct LabImage;

const MAP_W: f32 = 720.0;
const MAP_H: f32 = 400.0;

fn blank_image(cfg: &VoronoiConfig) -> Image {
    let mut img = Image::new(
        Extent3d { width: cfg.w, height: cfg.h, depth_or_array_layers: 1 },
        TextureDimension::D2,
        vec![0; (cfg.w * cfg.h * 4) as usize],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::nearest();
    img
}

fn page_node() -> Node {
    Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        overflow: Overflow::scroll_y(),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        ..default()
    }
}

fn column() -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        width: Val::Percent(100.0),
        max_width: Val::Px(980.0),
        padding: UiRect::all(Val::Px(16.0)),
        row_gap: Val::Px(4.0),
        flex_shrink: 0.0,
        ..default()
    }
}

fn toggle(parent: &mut ChildSpawnerCommands, label: &str, action: LabAction) {
    button_for(parent, label, action, BtnKind::Choice);
}

fn slider(parent: &mut ChildSpawnerCommands, title: &str, param: SliderParam) {
    parent.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..row() }).with_children(|r| {
        r.spawn((text(title, 13.5, GOLD), Node { width: Val::Px(160.0), ..default() }));
        r.spawn((
            SliderTrack(param),
            Button,
            Interaction::default(),
            RelativeCursorPosition::default(),
            Node {
                width: Val::Px(280.0),
                height: Val::Px(16.0),
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(PANEL2),
            BorderColor::all(LINE),
        ))
        .with_children(|t| {
            t.spawn((SliderFill(param), Node { width: Val::Percent(0.0), height: Val::Percent(100.0), ..default() }, BackgroundColor(PRIMARY_BG)));
        });
        r.spawn((SliderValue(param), text("", 13.5, TEXT)));
    });
}

/// A full-width row button with a gold title and a dim note.
fn lab_row(parent: &mut ChildSpawnerCommands, lab: LabId, n: usize) {
    let mut labels = Vec::new();
    let mut e = parent.spawn((
        Button,
        Node {
            flex_direction: FlexDirection::Column,
            width: Val::Percent(100.0),
            padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
            margin: UiRect::vertical(Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            row_gap: Val::Px(2.0),
            ..default()
        },
        BackgroundColor(PANEL2),
        BorderColor::all(LINE),
        LabAction::Open(lab),
        BtnKind::Choice,
        Enabled(true),
        Selected(false),
    ));
    e.with_children(|b| {
        labels.push(b.spawn(text(format!("{n}. {}", lab.title()), 15.0, GOLD)).id());
        labels.push(b.spawn(text(lab.blurb(), 13.0, DIM)).id());
    });
    e.insert(BtnLabel::new(labels));
}

pub fn setup_labs(mut commands: Commands, mut images: ResMut<Assets<Image>>, cfg: Res<LabsConfig>) {
    let image = images.add(blank_image(&cfg.0));
    commands.insert_resource(Labs {
        state: LabState::new(&cfg.0),
        image: image.clone(),
        key: String::new(),
        base: None,
        shown: String::new(),
        hover: None,
        drag: None,
        playing: false,
        timer: 0.0,
        last_cfg: cfg.0,
    });

    // --- Start page ---
    commands.spawn((StartPage, page_node(), BackgroundColor(BG), ScrollPosition::default(), GlobalZIndex(10))).with_children(|m| {
        m.spawn(column()).with_children(|c| {
            c.spawn(text("Successor Stars", 26.0, TEXT));
            c.spawn((
                text("Voronoi labs for procedural maps, territories and effects — and the fleet battle.", 14.0, DIM),
                Node { margin: UiRect::bottom(Val::Px(10.0)), ..default() },
            ));
            c.spawn(panel()).with_children(|p| {
                card_title(p, "Config", "Shared by every lab below: change it here and each lab redraws from it.");
                p.spawn(Node { align_items: AlignItems::Center, ..row() }).with_children(|r| {
                    r.spawn((text("Seed", 13.5, GOLD), Node { width: Val::Px(160.0), ..default() }));
                    r.spawn((LabText::Seed, text("", 14.0, TEXT), Node { width: Val::Px(110.0), ..default() }));
                    button_for(r, "−1", LabAction::SeedDelta(-1), BtnKind::Normal);
                    button_for(r, "+1", LabAction::SeedDelta(1), BtnKind::Normal);
                    button_for(r, "New seed", LabAction::NewSeed, BtnKind::Primary);
                });
                slider(p, "Sites", SliderParam::Sites);
                metric_row(p);
                let relax: Vec<(String, LabAction)> = (0..=5).map(|n| (n.to_string(), LabAction::Relax(n))).collect();
                choice_row(p, "Lloyd rounds", &relax);
                p.spawn(text("Map 720 × 400. Lloyd rounds apply to the biome island and the painted island.", 12.5, DIM));
            });
            for (i, lab) in LabId::ALL.iter().enumerate() {
                lab_row(c, *lab, i + 1);
            }
            c.spawn(Node { height: Val::Px(8.0), ..default() });
            c.spawn(row()).with_children(|r| {
                button_for(r, "Fleet battle & campaign →", LabAction::Game, BtnKind::Primary);
            });
        });
    });

    // --- Lab page ---
    commands.spawn((LabPage, page_node(), BackgroundColor(BG), ScrollPosition::default(), GlobalZIndex(10))).with_children(|m| {
        m.spawn(column()).with_children(|c| {
            c.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(12.0), ..row() }).with_children(|r| {
                button_for(r, "← Labs", LabAction::Back, BtnKind::Normal);
                r.spawn((LabText::Title, text("", 20.0, TEXT)));
            });
            c.spawn((LabText::Blurb, text("", 13.0, DIM)));
            c.spawn((LabText::Config, text("", 12.5, DIM)));
            c.spawn(panel()).with_children(|p| {
                for lab in LabId::ALL {
                    p.spawn((LabControls(lab), Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default() }))
                        .with_children(|l| lab_controls(l, lab));
                }
                p.spawn((LabText::Status, text("", 13.0, GREEN)));
            });
            c.spawn((
                LabImage,
                ImageNode::new(image),
                RelativeCursorPosition::default(),
                Interaction::default(),
                Node { width: Val::Px(MAP_W), max_width: Val::Percent(100.0), aspect_ratio: Some(MAP_W / MAP_H), flex_shrink: 0.0, ..default() },
            ));
        });
    });
}

fn metric_row(p: &mut ChildSpawnerCommands) {
    let metrics: Vec<(String, LabAction)> = Metric::ALL.iter().map(|m| (m.name().to_string(), LabAction::Metric(*m))).collect();
    choice_row(p, "Distance metric", &metrics);
}

fn lab_controls(l: &mut ChildSpawnerCommands, lab: LabId) {
    match lab {
        LabId::Grow => {
            slider(l, "Grow radius", SliderParam::GrowRadius);
            l.spawn(row()).with_children(|r| {
                button_for(r, "▶ Play", LabAction::Play, BtnKind::Primary);
                toggle(r, "Growth fronts", LabAction::GrowFronts);
            });
        }
        LabId::Nearest => {
            metric_row(l);
            l.spawn(row()).with_children(|r| toggle(r, "Distance lines", LabAction::DistanceLines));
            l.spawn(text("Drag anywhere on the map to move the probe.", 12.5, DIM));
        }
        LabId::OwnerMap => {
            metric_row(l);
            l.spawn(row()).with_children(|r| toggle(r, "Borders", LabAction::OwnerBorders));
        }
        LabId::Delaunay => {
            l.spawn(row()).with_children(|r| {
                toggle(r, "Voronoi cells", LabAction::Cells);
                toggle(r, "Delaunay triangles", LabAction::Triangles);
                toggle(r, "Circumcircles", LabAction::Circles);
                button_for(r, "Reset", LabAction::ResetLab, BtnKind::Normal);
            });
            l.spawn(text("Click a site to select it and show its neighbours · drag a site to move it · click empty space to add one.", 12.5, DIM));
        }
        LabId::Lloyd => {
            l.spawn(row()).with_children(|r| {
                button_for(r, "Step", LabAction::LloydStep, BtnKind::Normal);
                button_for(r, "▶ Play", LabAction::Play, BtnKind::Primary);
                button_for(r, "Reset", LabAction::ResetLab, BtnKind::Normal);
                toggle(r, "Show centroids", LabAction::Centroids);
            });
        }
        LabId::Biomes => {
            l.spawn(row()).with_children(|r| {
                toggle(r, "Region borders", LabAction::BiomeBorders);
                toggle(r, "Sites", LabAction::BiomeSites);
            });
        }
        LabId::Influence => {
            for (i, (name, _)) in FACTIONS.iter().enumerate() {
                slider(l, &format!("{name} weight"), SliderParam::Weight(i));
            }
            l.spawn(row()).with_children(|r| {
                for (i, (name, _)) in FACTIONS.iter().enumerate() {
                    button_for(r, &format!("Destroy {name}"), LabAction::Destroy(i), BtnKind::Danger);
                }
            });
            l.spawn(row()).with_children(|r| {
                toggle(r, "Use weights", LabAction::UseWeights);
                button_for(r, "Reset", LabAction::ResetLab, BtnKind::Normal);
            });
            l.spawn(text("Drag a base to move it. Hatched ground is contested (runner-up within 6%).", 12.5, DIM));
        }
        LabId::CellNoise => {
            let modes: Vec<(String, LabAction)> = NoiseMode::ALL.iter().map(|m| (m.name().to_string(), LabAction::Mode(*m))).collect();
            choice_row(l, "Mode", &modes);
            let presets: Vec<(String, LabAction)> =
                NoisePreset::ALL.iter().map(|p| (p.name().to_string(), LabAction::Preset(*p))).collect();
            choice_row(l, "Preset", &presets);
            slider(l, "Crack width", SliderParam::Crack);
        }
        LabId::Island => {
            l.spawn(row()).with_children(|r| {
                button_for(r, "New world", LabAction::NewWorld, BtnKind::Primary);
            });
            l.spawn(text("Hover the map for a 2× closer look.", 12.5, DIM));
        }
    }
}

fn current_lab(phase: Phase) -> Option<LabId> {
    match phase {
        Phase::Lab(l) => Some(l),
        _ => None,
    }
}

pub fn handle_lab_buttons(
    buttons: Query<(&Interaction, &LabAction, &Enabled), Changed<Interaction>>,
    mut cfg: ResMut<LabsConfig>,
    mut labs: ResMut<Labs>,
    mut requests: MessageWriter<AppRequest>,
    phase: Res<Phase>,
) {
    let lab = current_lab(*phase);
    for (interaction, action, enabled) in &buttons {
        if *interaction != Interaction::Pressed || !enabled.0 {
            continue;
        }
        let st = &mut labs.state;
        match *action {
            LabAction::Open(l) => {
                requests.write(AppRequest::OpenLab(l));
                labs.playing = false;
            }
            LabAction::Back => {
                requests.write(AppRequest::Labs);
                labs.playing = false;
            }
            LabAction::Game => {
                requests.write(AppRequest::Menu);
            }
            LabAction::SeedDelta(d) => cfg.0.seed = cfg.0.seed.saturating_add_signed(d).max(1),
            LabAction::NewSeed => cfg.0.seed = super::random_seed() % 2_000_000_000 + 1,
            LabAction::Metric(m) => cfg.0.metric = m,
            LabAction::Relax(n) => cfg.0.relax = n,
            LabAction::Play => {
                if lab == Some(LabId::Grow) && st.grow_radius >= LabState::grow_max(&cfg.0) {
                    st.grow_radius = 0.0;
                }
                if lab == Some(LabId::Lloyd) && st.lloyd_settled() {
                    *st = LabState { lloyd: None, lloyd_round: 0, lloyd_moved: f32::INFINITY, ..st.clone() };
                }
                labs.playing = !labs.playing;
            }
            LabAction::GrowFronts => st.grow_fronts = !st.grow_fronts,
            LabAction::DistanceLines => st.distance_lines = !st.distance_lines,
            LabAction::OwnerBorders => st.owner_borders = !st.owner_borders,
            LabAction::Cells => st.show_cells = !st.show_cells,
            LabAction::Triangles => st.show_triangles = !st.show_triangles,
            LabAction::Circles => st.show_circles = !st.show_circles,
            LabAction::ResetLab => {
                let fresh = LabState::new(&cfg.0);
                match lab {
                    Some(LabId::Delaunay) => {
                        st.edited = None;
                        st.selected = None;
                    }
                    Some(LabId::Lloyd) => {
                        st.lloyd = None;
                        st.lloyd_round = 0;
                        st.lloyd_moved = f32::INFINITY;
                        labs.playing = false;
                    }
                    Some(LabId::Influence) => {
                        st.bases = fresh.bases;
                        st.weights = fresh.weights;
                        st.alive = fresh.alive;
                        st.use_weights = true;
                    }
                    _ => {}
                }
            }
            LabAction::LloydStep => {
                st.lloyd_step(&cfg.0);
                labs.playing = false;
            }
            LabAction::Centroids => st.show_centroids = !st.show_centroids,
            LabAction::BiomeBorders => st.biome_borders = !st.biome_borders,
            LabAction::BiomeSites => st.biome_sites = !st.biome_sites,
            LabAction::Destroy(i) => st.alive[i] = !st.alive[i],
            LabAction::UseWeights => st.use_weights = !st.use_weights,
            LabAction::Mode(m) => st.noise_mode = m,
            LabAction::Preset(p) => st.preset = p,
            LabAction::NewWorld => st.world = st.world.wrapping_add(1),
        }
    }
    // A new config invalidates edits made on the old sites.
    if cfg.0 != labs.last_cfg {
        let c = cfg.0;
        labs.state.reseed(&c);
        labs.last_cfg = c;
    }
}

pub fn drag_sliders(
    mouse: Res<ButtonInput<MouseButton>>,
    tracks: Query<(&SliderTrack, &Interaction, &RelativeCursorPosition)>,
    mut active: ResMut<ActiveSlider>,
    mut cfg: ResMut<LabsConfig>,
    mut labs: ResMut<Labs>,
) {
    if !mouse.pressed(MouseButton::Left) {
        active.0 = None;
        return;
    }
    for (track, interaction, cursor) in &tracks {
        if active.0.is_none() && *interaction == Interaction::Pressed {
            active.0 = Some(track.0);
        }
        if active.0 != Some(track.0) {
            continue;
        }
        let Some(n) = cursor.normalized else { continue };
        let (lo, hi, step) = track.0.range(&cfg.0);
        let t = (n.x + 0.5).clamp(0.0, 1.0);
        let v = ((lo + (hi - lo) * t) / step).round() * step;
        match track.0 {
            SliderParam::Sites => {
                let n = v as usize;
                if cfg.0.sites != n {
                    cfg.0.sites = n;
                }
            }
            SliderParam::GrowRadius => {
                labs.state.grow_radius = v;
                labs.playing = false;
            }
            SliderParam::Crack => labs.state.crack = v,
            SliderParam::Weight(i) => labs.state.weights[i] = v,
        }
    }
}

/// Clicks and drags on the lab image.
pub fn lab_pointer(
    mouse: Res<ButtonInput<MouseButton>>,
    image: Query<&RelativeCursorPosition, With<LabImage>>,
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    mut labs: ResMut<Labs>,
) {
    let Some(lab) = current_lab(*phase) else {
        labs.drag = None;
        return;
    };
    let Ok(cursor) = image.single() else { return };
    let cfg = cfg.0;
    let p = cursor.normalized.map(|n| MapVec::new((n.x + 0.5) * cfg.w as f32, (n.y + 0.5) * cfg.h as f32));
    let over = cursor.cursor_over();
    let hover = if over && lab == LabId::Island { p } else { None };
    if labs.hover != hover {
        labs.hover = hover;
    }
    let Some(p) = p else { return };
    let clamped = p.clamp(MapVec::ZERO, MapVec::new(cfg.w as f32 - 0.5, cfg.h as f32 - 0.5));
    if mouse.just_pressed(MouseButton::Left) && over {
        let st = &mut labs.state;
        let drag = match lab {
            LabId::Nearest => {
                st.probe = clamped;
                Some(Drag::Probe)
            }
            LabId::Delaunay => {
                let sites = st.sites(&cfg, lab);
                match site_at(&sites, p, 9.0) {
                    Some(i) => {
                        st.selected = Some(i);
                        Some(Drag::Site(i))
                    }
                    None => {
                        let mut sites = sites;
                        sites.push(clamped);
                        st.selected = Some(sites.len() - 1);
                        st.edited = Some(sites);
                        None
                    }
                }
            }
            LabId::Influence => site_at(&st.bases, p, 12.0).map(Drag::Base),
            _ => None,
        };
        labs.drag = drag;
    } else if mouse.pressed(MouseButton::Left)
        && let Some(drag) = labs.drag
    {
        let st = &mut labs.state;
        match drag {
            Drag::Probe => st.probe = clamped,
            Drag::Site(i) => {
                let mut sites = st.sites(&cfg, LabId::Delaunay);
                if i < sites.len() && sites[i] != clamped {
                    sites[i] = clamped;
                    st.edited = Some(sites);
                }
            }
            Drag::Base(i) => st.bases[i] = clamped,
        }
    } else if !mouse.pressed(MouseButton::Left) {
        labs.drag = None;
    }
}

/// Play: grow the seeds, or run Lloyd rounds until nothing moves.
pub fn animate_labs(time: Res<Time>, phase: Res<Phase>, cfg: Res<LabsConfig>, mut labs: ResMut<Labs>) {
    if !labs.playing {
        return;
    }
    let dt = time.delta_secs();
    match current_lab(*phase) {
        Some(LabId::Grow) => {
            let max = LabState::grow_max(&cfg.0);
            labs.state.grow_radius = (labs.state.grow_radius + 90.0 * dt).min(max);
            if labs.state.grow_radius >= max {
                labs.playing = false;
            }
        }
        Some(LabId::Lloyd) => {
            labs.timer += dt;
            if labs.timer >= 0.35 {
                labs.timer = 0.0;
                labs.state.lloyd_step(&cfg.0);
                if labs.state.lloyd_settled() {
                    labs.playing = false;
                }
            }
        }
        _ => labs.playing = false,
    }
}

/// Re-renders the lab image when its inputs change.
pub fn render_lab(phase: Res<Phase>, cfg: Res<LabsConfig>, mut labs: ResMut<Labs>, mut images: ResMut<Assets<Image>>) {
    let Some(lab) = current_lab(*phase) else { return };
    // Coarse previews while something is being dragged or animated.
    let quick = labs.drag.is_some() || (labs.playing && lab == LabId::Grow);
    let step = if quick { 2 } else { 1 };
    let key = format!("{:?}|{:?}|{:?}|{step}", cfg.0, lab, labs.state);
    if key != labs.key || labs.base.is_none() {
        let canvas = render(&cfg.0, lab, &labs.state, step);
        labs.base = Some(canvas);
        labs.key = key;
    }
    let shown = format!("{}|{:?}", labs.key, labs.hover.map(|h| (h.x as i32, h.y as i32)));
    if shown == labs.shown {
        return;
    }
    let Some(base) = labs.base.as_ref() else { return };
    let rgba = match labs.hover {
        Some(h) if lab == LabId::Island => zoom_inset(base, h).to_rgba(),
        _ => base.to_rgba(),
    };
    let handle = labs.image.clone();
    if let Some(mut img) = images.get_mut(&handle) {
        img.data = Some(rgba);
    }
    labs.shown = shown;
}

#[allow(clippy::too_many_arguments)]
pub fn update_lab_ui(
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    labs: Res<Labs>,
    mut texts: Query<(&LabText, &mut Text)>,
    mut fills: Query<(&SliderFill, &mut Node), (Without<SliderValue>, Without<LabControls>, Without<StartPage>, Without<LabPage>)>,
    mut values: Query<(&SliderValue, &mut Text), Without<LabText>>,
    mut controls: Query<(&LabControls, &mut Node), (Without<SliderFill>, Without<StartPage>, Without<LabPage>)>,
    mut start: Query<&mut Node, (With<StartPage>, Without<LabPage>, Without<SliderFill>, Without<LabControls>)>,
    mut page: Query<&mut Node, (With<LabPage>, Without<StartPage>, Without<SliderFill>, Without<LabControls>)>,
) {
    let lab = current_lab(*phase);
    let show = |node: &mut Node, on: bool| {
        let d = if on { Display::Flex } else { Display::None };
        if node.display != d {
            node.display = d;
        }
    };
    for mut n in &mut start {
        show(&mut n, *phase == Phase::Start);
    }
    for mut n in &mut page {
        show(&mut n, lab.is_some());
    }
    for (c, mut n) in &mut controls {
        show(&mut n, lab == Some(c.0));
    }
    let c = cfg.0;
    let st = &labs.state;
    for (which, mut t) in &mut texts {
        let s = match which {
            LabText::Seed => c.seed.to_string(),
            LabText::Title => lab.map_or(String::new(), |l| l.title().to_string()),
            LabText::Blurb => lab.map_or(String::new(), |l| l.blurb().to_string()),
            LabText::Config => format!(
                "Config: seed {} · {} sites · {} · {} Lloyd round(s) · {}×{}  (change it on the labs page)",
                c.seed,
                c.sites,
                c.metric.name(),
                c.relax,
                c.w,
                c.h
            ),
            LabText::Status => lab.map_or(String::new(), |l| status(&c, l, st)),
        };
        if t.0 != s {
            t.0 = s;
        }
    }
    for (f, mut n) in &mut fills {
        let (lo, hi, _) = f.0.range(&c);
        let pct = ((f.0.get(&c, st) - lo) / (hi - lo)).clamp(0.0, 1.0) * 100.0;
        if n.width != Val::Percent(pct) {
            n.width = Val::Percent(pct);
        }
    }
    for (v, mut t) in &mut values {
        let s = v.0.label(v.0.get(&c, st));
        if t.0 != s {
            t.0 = s;
        }
    }
}

/// Selected/label state of the lab buttons.
pub fn lab_button_state(
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    labs: Res<Labs>,
    mut buttons: Query<(&LabAction, &mut Selected, &BtnLabel)>,
    mut texts: Query<&mut Text>,
) {
    let st = &labs.state;
    let c = cfg.0;
    for (action, mut sel, labels) in &mut buttons {
        let on = match *action {
            LabAction::Metric(m) => c.metric == m,
            LabAction::Relax(n) => c.relax == n,
            LabAction::GrowFronts => st.grow_fronts,
            LabAction::DistanceLines => st.distance_lines,
            LabAction::OwnerBorders => st.owner_borders,
            LabAction::Cells => st.show_cells,
            LabAction::Triangles => st.show_triangles,
            LabAction::Circles => st.show_circles,
            LabAction::Centroids => st.show_centroids,
            LabAction::BiomeBorders => st.biome_borders,
            LabAction::BiomeSites => st.biome_sites,
            LabAction::UseWeights => st.use_weights,
            LabAction::Mode(m) => st.noise_mode == m,
            LabAction::Preset(p) => st.preset == p,
            LabAction::Open(l) => *phase == Phase::Lab(l),
            _ => false,
        };
        if sel.0 != on {
            sel.0 = on;
        }
        let label = match *action {
            LabAction::Play => Some(if labs.playing { "⏸ Pause".to_string() } else { "▶ Play".to_string() }),
            LabAction::Destroy(i) => Some(if st.alive[i] { format!("Destroy {}", FACTIONS[i].0) } else { format!("Rebuild {}", FACTIONS[i].0) }),
            _ => None,
        };
        if let Some(label) = label
            && let Some(e) = labels.first()
            && let Ok(mut t) = texts.get_mut(e)
            && t.0 != label
        {
            t.0 = label;
        }
    }
}

/// Wheel scrolling for the start and lab pages.
pub fn scroll_labs(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    phase: Res<Phase>,
    mut start: Query<&mut ScrollPosition, (With<StartPage>, Without<LabPage>)>,
    mut page: Query<&mut ScrollPosition, (With<LabPage>, Without<StartPage>)>,
) {
    let dy: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y * 28.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if dy == 0.0 {
        return;
    }
    let target = match *phase {
        Phase::Start => start.single_mut().ok(),
        Phase::Lab(_) => page.single_mut().ok(),
        _ => None,
    };
    if let Some(mut s) = target {
        s.0.y = (s.0.y - dy).max(0.0);
    }
}
