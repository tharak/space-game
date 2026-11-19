//! The start page: the shared Voronoi config, then every lab as a card
//! with its controls, a status line and its image, then a link to the
//! fleet battle. Every image comes from [`crate::voronoi::labs::render`];
//! this module only turns clicks, drags and sliders into config and
//! lab-state changes.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::RelativeCursorPosition;

use super::flow::{AppRequest, Phase};
use super::theme::*;
use super::ui::{BtnKind, BtnLabel, Enabled, Selected, button_for, card_title, choice_row, panel, row, text};
use crate::prototypes::dominion::{self, DrawOptions, Dominion, MAX_STRENGTH, MIN_STRENGTH};
use crate::voronoi::Vec2 as MapVec;
use crate::voronoi::labs::{FACTIONS, LabId, LabState, render, site_at, status, zoom_inset};
use crate::voronoi::metric::Metric;
use crate::voronoi::noise::{NoiseMode, NoisePreset};
use crate::voronoi::raster::Canvas;
use crate::voronoi::sites::VoronoiConfig;

const LABS: usize = LabId::ALL.len();
/// Full-quality lab renders per frame, so a config change doesn't freeze
/// the page while all nine redraw.
const RENDERS_PER_FRAME: usize = 2;

/// The config every lab reads.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct LabsConfig(pub VoronoiConfig);

#[derive(Clone, Copy, PartialEq, Debug)]
enum Drag {
    Probe,
    Site(usize),
    Base(usize),
}

/// One lab's image and what it was drawn from.
struct LabView {
    image: Handle<Image>,
    /// What the cached base image was rendered from.
    key: String,
    base: Option<Canvas>,
    /// What the texture currently shows (base key + hover).
    shown: String,
}

/// prototype01's state, selection and image.
struct Proto {
    d: Dominion,
    /// How many of the sites get a planet.
    planets: usize,
    selected: Option<usize>,
    drag: Option<usize>,
    regions: bool,
    borders: bool,
    view: LabView,
}

/// Lab state plus each lab's cached image.
#[derive(Resource)]
pub struct Labs {
    pub state: LabState,
    proto: Proto,
    views: Vec<LabView>,
    hover: Option<(LabId, MapVec)>,
    drag: Option<(LabId, Drag)>,
    playing: Option<LabId>,
    timer: f32,
    last_cfg: VoronoiConfig,
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub enum LabAction {
    /// Back to the start page (from the game menu).
    Back,
    /// To the fleet battle and campaign menu.
    Game,
    // Shared config
    SeedDelta(i32),
    NewSeed,
    Metric(Metric),
    Relax(u32),
    // prototype01
    EndTurn,
    Reinforce,
    ProtoReset,
    ProtoRegions,
    ProtoBorders,
    // Per lab
    Play(LabId),
    Reset(LabId),
    GrowFronts,
    DistanceLines,
    OwnerBorders,
    Cells,
    Triangles,
    Circles,
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
    Planets,
    FleetStrength,
    GrowRadius,
    Crack,
    Weight(usize),
}

impl SliderParam {
    fn range(self, cfg: &VoronoiConfig) -> (f32, f32, f32) {
        match self {
            SliderParam::Sites => (3.0, 250.0, 1.0),
            SliderParam::Planets => (1.0, cfg.sites as f32, 1.0),
            SliderParam::FleetStrength => (MIN_STRENGTH, MAX_STRENGTH, 0.1),
            SliderParam::GrowRadius => (0.0, LabState::grow_max(cfg), 1.0),
            SliderParam::Crack => (0.5, 10.0, 0.1),
            SliderParam::Weight(_) => (0.2, 4.0, 0.1),
        }
    }

    fn get(self, cfg: &VoronoiConfig, st: &LabState, proto: &Proto) -> f32 {
        match self {
            SliderParam::Sites => cfg.sites as f32,
            SliderParam::Planets => proto.planets.min(cfg.sites) as f32,
            SliderParam::FleetStrength => proto.selected.and_then(|i| proto.d.fleets.get(i)).map_or(MIN_STRENGTH, |f| f.strength),
            SliderParam::GrowRadius => st.grow_radius,
            SliderParam::Crack => st.crack,
            SliderParam::Weight(i) => st.weights[i],
        }
    }

    fn label(self, v: f32) -> String {
        match self {
            SliderParam::Sites => format!("{v:.0}"),
            SliderParam::Planets => format!("{v:.0}"),
            SliderParam::FleetStrength => format!("{v:.1}×"),
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
    Proto,
    Status(LabId),
}

#[derive(Component)]
pub struct StartPage;
#[derive(Component)]
pub struct LabImage(LabId);
#[derive(Component)]
pub struct PrototypeImage;

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

fn toggle(parent: &mut ChildSpawnerCommands, label: &str, action: LabAction) {
    button_for(parent, label, action, BtnKind::Choice);
}

fn slider(parent: &mut ChildSpawnerCommands, title: &str, param: SliderParam) {
    slider_sized(parent, title, param, 160.0, 280.0);
}

/// A slider in a half-width grid card.
fn small_slider(parent: &mut ChildSpawnerCommands, title: &str, param: SliderParam) {
    slider_sized(parent, title, param, 110.0, 200.0);
}

fn slider_sized(parent: &mut ChildSpawnerCommands, title: &str, param: SliderParam, label_w: f32, track_w: f32) {
    parent.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(8.0), ..row() }).with_children(|r| {
        r.spawn((text(title, 13.5, GOLD), Node { width: Val::Px(label_w), ..default() }));
        r.spawn((
            SliderTrack(param),
            Button,
            Interaction::default(),
            RelativeCursorPosition::default(),
            Node {
                width: Val::Px(track_w),
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

pub fn setup_labs(mut commands: Commands, mut images: ResMut<Assets<Image>>, cfg: Res<LabsConfig>) {
    let views: Vec<LabView> = LabId::ALL
        .iter()
        .map(|_| LabView { image: images.add(blank_image(&cfg.0)), key: String::new(), base: None, shown: String::new() })
        .collect();
    let handles: Vec<Handle<Image>> = views.iter().map(|v| v.image.clone()).collect();
    // The labs are drawn at half size: smooth the downscale.
    for h in &handles {
        if let Some(mut img) = images.get_mut(h) {
            img.sampler = ImageSampler::linear();
        }
    }
    let proto_image = images.add(blank_image(&cfg.0));
    let planets = 20;
    let proto = Proto {
        d: Dominion::new(&cfg.0, planets),
        planets,
        selected: None,
        drag: None,
        regions: true,
        borders: true,
        view: LabView { image: proto_image.clone(), key: String::new(), base: None, shown: String::new() },
    };
    commands.insert_resource(Labs {
        state: LabState::new(&cfg.0),
        proto,
        views,
        hover: None,
        drag: None,
        playing: None,
        timer: 0.0,
        last_cfg: cfg.0,
    });

    commands
        .spawn((
            StartPage,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                overflow: Overflow::scroll_y(),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(BG),
            ScrollPosition::default(),
            GlobalZIndex(10),
        ))
        .with_children(|m| {
            m.spawn(Node {
                flex_direction: FlexDirection::Column,
                width: Val::Percent(100.0),
                max_width: Val::Px(980.0),
                padding: UiRect::all(Val::Px(16.0)),
                row_gap: Val::Px(4.0),
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|c| {
                c.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(16.0), ..row() }).with_children(|r| {
                    r.spawn(text("Successor Stars", 26.0, TEXT));
                    button_for(r, "Fleet battle & campaign →", LabAction::Game, BtnKind::Primary);
                });
                c.spawn((
                    text("Voronoi labs for procedural maps, territories and effects.", 14.0, DIM),
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
                section_heading(c, "Prototypes");
                c.spawn(panel()).with_children(|p| {
                    card_title(
                        p,
                        "prototype01 — Fleet dominion",
                        "Some Voronoi regions hold a planet worth 1–3 resources a turn. Every planet starts as its own \
                         independent faction that only defends (grey ring = defence). Fleets project influence by \
                         distance ÷ strength; at the end of the turn a planet joins the side whose fleet dominates it if \
                         that fleet is stronger than its defence (gold ring = falls this turn). Contested planets don't \
                         change hands. Owned planets pay every turn; spend 10 to reinforce a fleet.",
                    );
                    p.spawn(row()).with_children(|r| {
                        button_for(r, "End turn", LabAction::EndTurn, BtnKind::Primary);
                        button_for(r, "Reinforce selected (10)", LabAction::Reinforce, BtnKind::Normal);
                        toggle(r, "Planet regions", LabAction::ProtoRegions);
                        toggle(r, "Front lines", LabAction::ProtoBorders);
                        button_for(r, "Reset", LabAction::ProtoReset, BtnKind::Normal);
                    });
                    slider(p, "Sites", SliderParam::Sites);
                    slider(p, "Planets", SliderParam::Planets);
                    slider(p, "Selected strength", SliderParam::FleetStrength);
                    p.spawn(text("Drag a fleet to move it · click a fleet to select it.", 12.5, DIM));
                    p.spawn((LabText::Proto, text("", 13.0, GREEN)));
                    p.spawn((
                        PrototypeImage,
                        ImageNode::new(proto_image),
                        RelativeCursorPosition::default(),
                        Node {
                            width: Val::Px(MAP_W),
                            max_width: Val::Percent(100.0),
                            aspect_ratio: Some(MAP_W / MAP_H),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                });
                section_heading(c, "Labs");
                c.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(12.0),
                    justify_content: JustifyContent::SpaceBetween,
                    width: Val::Percent(100.0),
                    ..default()
                })
                .with_children(|g| {
                    for (i, lab) in LabId::ALL.iter().enumerate() {
                        g.spawn(panel())
                            .insert(Node {
                                flex_direction: FlexDirection::Column,
                                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                                margin: UiRect::bottom(Val::Px(12.0)),
                                border: UiRect::all(Val::Px(1.0)),
                                border_radius: BorderRadius::all(Val::Px(8.0)),
                                row_gap: Val::Px(6.0),
                                width: Val::Percent(49.0),
                                min_width: Val::Px(340.0),
                                flex_grow: 1.0,
                                ..default()
                            })
                            .with_children(|p| {
                                card_title(p, &format!("{}. {}", i + 1, lab.title()), lab.blurb());
                                lab_controls(p, *lab);
                                p.spawn((LabText::Status(*lab), text("", 12.5, GREEN)));
                                p.spawn((
                                    LabImage(*lab),
                                    ImageNode::new(handles[i].clone()),
                                    RelativeCursorPosition::default(),
                                    Node {
                                        width: Val::Px(MAP_W / 2.0),
                                        max_width: Val::Percent(100.0),
                                        aspect_ratio: Some(MAP_W / MAP_H),
                                        flex_shrink: 0.0,
                                        ..default()
                                    },
                                ));
                            });
                    }
                });
            });
        });
}

fn section_heading(c: &mut ChildSpawnerCommands, title: &str) {
    c.spawn((text(title, 18.0, TEXT), Node { margin: UiRect::new(Val::Px(0.0), Val::Px(0.0), Val::Px(10.0), Val::Px(6.0)), ..default() }));
}

fn metric_row(p: &mut ChildSpawnerCommands) {
    let metrics: Vec<(String, LabAction)> = Metric::ALL.iter().map(|m| (m.name().to_string(), LabAction::Metric(*m))).collect();
    choice_row(p, "Distance metric", &metrics);
}

fn lab_controls(l: &mut ChildSpawnerCommands, lab: LabId) {
    match lab {
        LabId::Grow => {
            small_slider(l, "Grow radius", SliderParam::GrowRadius);
            l.spawn(row()).with_children(|r| {
                button_for(r, "▶ Play", LabAction::Play(lab), BtnKind::Primary);
                toggle(r, "Growth fronts", LabAction::GrowFronts);
            });
        }
        LabId::Nearest => {
            metric_row(l);
            l.spawn(row()).with_children(|r| toggle(r, "Distance lines", LabAction::DistanceLines));
            l.spawn(text("Drag anywhere on the map to move the probe.", 12.5, DIM));
        }
        LabId::OwnerMap => {
            l.spawn(row()).with_children(|r| toggle(r, "Borders", LabAction::OwnerBorders));
        }
        LabId::Delaunay => {
            l.spawn(row()).with_children(|r| {
                toggle(r, "Voronoi cells", LabAction::Cells);
                toggle(r, "Delaunay triangles", LabAction::Triangles);
                toggle(r, "Circumcircles", LabAction::Circles);
                button_for(r, "Reset", LabAction::Reset(lab), BtnKind::Normal);
            });
            l.spawn(text("Click a site to select it and show its neighbours · drag a site to move it · click empty space to add one.", 12.5, DIM));
        }
        LabId::Lloyd => {
            l.spawn(row()).with_children(|r| {
                button_for(r, "Step", LabAction::LloydStep, BtnKind::Normal);
                button_for(r, "▶ Play", LabAction::Play(lab), BtnKind::Primary);
                button_for(r, "Reset", LabAction::Reset(lab), BtnKind::Normal);
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
                small_slider(l, &format!("{name} weight"), SliderParam::Weight(i));
            }
            l.spawn(row()).with_children(|r| {
                for (i, (name, _)) in FACTIONS.iter().enumerate() {
                    button_for(r, &format!("Destroy {name}"), LabAction::Destroy(i), BtnKind::Danger);
                }
            });
            l.spawn(row()).with_children(|r| {
                toggle(r, "Use weights", LabAction::UseWeights);
                button_for(r, "Reset", LabAction::Reset(lab), BtnKind::Normal);
            });
            l.spawn(text("Drag a base to move it. Hatched ground is contested (runner-up within 6%).", 12.5, DIM));
        }
        LabId::CellNoise => {
            let modes: Vec<(String, LabAction)> = NoiseMode::ALL.iter().map(|m| (m.name().to_string(), LabAction::Mode(*m))).collect();
            choice_row(l, "Mode", &modes);
            let presets: Vec<(String, LabAction)> =
                NoisePreset::ALL.iter().map(|p| (p.name().to_string(), LabAction::Preset(*p))).collect();
            choice_row(l, "Preset", &presets);
            small_slider(l, "Crack width", SliderParam::Crack);
        }
        LabId::Island => {
            l.spawn(row()).with_children(|r| {
                button_for(r, "New world", LabAction::NewWorld, BtnKind::Primary);
            });
            l.spawn(text("Hover the map for a 2× closer look.", 12.5, DIM));
        }
    }
}

fn on_start(phase: &Phase) -> bool {
    *phase == Phase::Start
}

pub fn handle_lab_buttons(
    buttons: Query<(&Interaction, &LabAction, &Enabled), Changed<Interaction>>,
    mut cfg: ResMut<LabsConfig>,
    mut labs: ResMut<Labs>,
    mut requests: MessageWriter<AppRequest>,
) {
    let labs = &mut *labs;
    for (interaction, action, enabled) in &buttons {
        if *interaction != Interaction::Pressed || !enabled.0 {
            continue;
        }
        let st = &mut labs.state;
        match *action {
            LabAction::Back => {
                requests.write(AppRequest::Labs);
            }
            LabAction::Game => {
                labs.playing = None;
                requests.write(AppRequest::Menu);
            }
            LabAction::SeedDelta(d) => cfg.0.seed = cfg.0.seed.saturating_add_signed(d).max(1),
            LabAction::NewSeed => cfg.0.seed = super::random_seed() % 2_000_000_000 + 1,
            LabAction::Metric(m) => cfg.0.metric = m,
            LabAction::Relax(n) => cfg.0.relax = n,
            LabAction::EndTurn => labs.proto.d.end_turn(),
            LabAction::Reinforce => {
                if let Some(i) = labs.proto.selected {
                    labs.proto.d.reinforce(i);
                }
            }
            LabAction::ProtoReset => {
                labs.proto.d = Dominion::new(&cfg.0, labs.proto.planets);
                labs.proto.selected = None;
            }
            LabAction::ProtoRegions => labs.proto.regions = !labs.proto.regions,
            LabAction::ProtoBorders => labs.proto.borders = !labs.proto.borders,
            LabAction::Play(lab) => {
                if labs.playing == Some(lab) {
                    labs.playing = None;
                } else {
                    if lab == LabId::Grow && st.grow_radius >= LabState::grow_max(&cfg.0) {
                        st.grow_radius = 0.0;
                    }
                    if lab == LabId::Lloyd && st.lloyd_settled() {
                        st.lloyd = None;
                        st.lloyd_round = 0;
                        st.lloyd_moved = f32::INFINITY;
                    }
                    labs.playing = Some(lab);
                }
            }
            LabAction::Reset(lab) => {
                let fresh = LabState::new(&cfg.0);
                match lab {
                    LabId::Delaunay => {
                        st.edited = None;
                        st.selected = None;
                    }
                    LabId::Lloyd => {
                        st.lloyd = None;
                        st.lloyd_round = 0;
                        st.lloyd_moved = f32::INFINITY;
                        if labs.playing == Some(lab) {
                            labs.playing = None;
                        }
                    }
                    LabId::Influence => {
                        st.bases = fresh.bases;
                        st.weights = fresh.weights;
                        st.alive = fresh.alive;
                        st.use_weights = true;
                    }
                    _ => {}
                }
            }
            LabAction::GrowFronts => st.grow_fronts = !st.grow_fronts,
            LabAction::DistanceLines => st.distance_lines = !st.distance_lines,
            LabAction::OwnerBorders => st.owner_borders = !st.owner_borders,
            LabAction::Cells => st.show_cells = !st.show_cells,
            LabAction::Triangles => st.show_triangles = !st.show_triangles,
            LabAction::Circles => st.show_circles = !st.show_circles,
            LabAction::LloydStep => {
                st.lloyd_step(&cfg.0);
                if labs.playing == Some(LabId::Lloyd) {
                    labs.playing = None;
                }
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
        labs.proto.planets = labs.proto.planets.min(c.sites);
        labs.proto.d = Dominion::new(&c, labs.proto.planets);
        labs.proto.selected = None;
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
            SliderParam::Planets => {
                let n = (v as usize).clamp(1, cfg.0.sites);
                if labs.proto.planets != n {
                    labs.proto.planets = n;
                    labs.proto.d = Dominion::new(&cfg.0, n);
                    labs.proto.selected = None;
                }
            }
            SliderParam::FleetStrength => {
                if let Some(i) = labs.proto.selected
                    && let Some(f) = labs.proto.d.fleets.get_mut(i)
                {
                    f.strength = v;
                }
            }
            SliderParam::GrowRadius => {
                labs.state.grow_radius = v;
                if labs.playing == Some(LabId::Grow) {
                    labs.playing = None;
                }
            }
            SliderParam::Crack => labs.state.crack = v,
            SliderParam::Weight(i) => labs.state.weights[i] = v,
        }
    }
}

/// Clicks and drags on the lab images.
pub fn lab_pointer(
    mouse: Res<ButtonInput<MouseButton>>,
    images: Query<(&LabImage, &RelativeCursorPosition)>,
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    mut labs: ResMut<Labs>,
) {
    if !on_start(&phase) {
        labs.drag = None;
        labs.hover = None;
        return;
    }
    let cfg = cfg.0;
    let to_map = |n: Vec2| MapVec::new((n.x + 0.5) * cfg.w as f32, (n.y + 0.5) * cfg.h as f32);
    let clamp = |p: MapVec| p.clamp(MapVec::ZERO, MapVec::new(cfg.w as f32 - 0.5, cfg.h as f32 - 0.5));

    // Hover zoom on the painted island.
    let hover = images
        .iter()
        .find(|(img, c)| img.0 == LabId::Island && c.cursor_over())
        .and_then(|(_, c)| c.normalized.map(|n| (LabId::Island, to_map(n))));
    if labs.hover != hover {
        labs.hover = hover;
    }

    if mouse.just_pressed(MouseButton::Left) {
        let Some((lab, p)) = images.iter().find(|(_, c)| c.cursor_over()).and_then(|(img, c)| c.normalized.map(|n| (img.0, to_map(n))))
        else {
            return;
        };
        let st = &mut labs.state;
        let drag = match lab {
            LabId::Nearest => {
                st.probe = clamp(p);
                Some(Drag::Probe)
            }
            LabId::Delaunay => {
                let mut sites = st.sites(&cfg, lab);
                match site_at(&sites, p, 9.0) {
                    Some(i) => {
                        st.selected = Some(i);
                        Some(Drag::Site(i))
                    }
                    None => {
                        sites.push(clamp(p));
                        st.selected = Some(sites.len() - 1);
                        st.edited = Some(sites);
                        None
                    }
                }
            }
            LabId::Influence => site_at(&st.bases, p, 12.0).map(Drag::Base),
            _ => None,
        };
        labs.drag = drag.map(|d| (lab, d));
    } else if mouse.pressed(MouseButton::Left)
        && let Some((lab, drag)) = labs.drag
    {
        // Keep following the pointer even when it leaves the image.
        let Some(p) = images.iter().find(|(img, _)| img.0 == lab).and_then(|(_, c)| c.normalized).map(to_map) else { return };
        let p = clamp(p);
        let st = &mut labs.state;
        match drag {
            Drag::Probe => st.probe = p,
            Drag::Site(i) => {
                let mut sites = st.sites(&cfg, LabId::Delaunay);
                if i < sites.len() && sites[i] != p {
                    sites[i] = p;
                    st.edited = Some(sites);
                }
            }
            Drag::Base(i) => st.bases[i] = p,
        }
    } else if !mouse.pressed(MouseButton::Left) {
        labs.drag = None;
    }
}

/// prototype01: click a fleet to select it, drag to move it.
pub fn proto_pointer(
    mouse: Res<ButtonInput<MouseButton>>,
    image: Query<&RelativeCursorPosition, With<PrototypeImage>>,
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    mut labs: ResMut<Labs>,
) {
    let proto = &mut labs.proto;
    if !on_start(&phase) || !mouse.pressed(MouseButton::Left) {
        proto.drag = None;
        return;
    }
    let Ok(cursor) = image.single() else { return };
    let cfg = cfg.0;
    let Some(n) = cursor.normalized else { return };
    let p = MapVec::new((n.x + 0.5) * cfg.w as f32, (n.y + 0.5) * cfg.h as f32)
        .clamp(MapVec::ZERO, MapVec::new(cfg.w as f32 - 0.5, cfg.h as f32 - 0.5));
    if mouse.just_pressed(MouseButton::Left) {
        if !cursor.cursor_over() {
            return;
        }
        let hit = proto.d.fleet_at(p, 14.0);
        proto.selected = hit;
        proto.drag = hit;
    } else if let Some(i) = proto.drag
        && proto.d.fleets[i].pos != p
    {
        proto.d.fleets[i].pos = p;
    }
}

/// Play: grow the seeds, or run Lloyd rounds until nothing moves.
pub fn animate_labs(time: Res<Time>, phase: Res<Phase>, cfg: Res<LabsConfig>, mut labs: ResMut<Labs>) {
    if !on_start(&phase) {
        labs.playing = None;
    }
    let dt = time.delta_secs();
    match labs.playing {
        Some(LabId::Grow) => {
            let max = LabState::grow_max(&cfg.0);
            labs.state.grow_radius = (labs.state.grow_radius + 90.0 * dt).min(max);
            if labs.state.grow_radius >= max {
                labs.playing = None;
            }
        }
        Some(LabId::Lloyd) => {
            labs.timer += dt;
            if labs.timer >= 0.35 {
                labs.timer = 0.0;
                labs.state.lloyd_step(&cfg.0);
                if labs.state.lloyd_settled() {
                    labs.playing = None;
                }
            }
        }
        _ => {}
    }
}

/// Re-renders the lab images whose inputs changed: the lab being dragged,
/// animated or hovered first, then at most a couple of others per frame.
pub fn render_labs(
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    slider: Res<ActiveSlider>,
    mut labs: ResMut<Labs>,
    mut images: ResMut<Assets<Image>>,
) {
    if !on_start(&phase) {
        return;
    }
    let labs = &mut *labs;
    // prototype01 always renders first (full resolution unless dragging).
    {
        let proto = &mut labs.proto;
        let opts = DrawOptions {
            regions: proto.regions,
            borders: proto.borders,
            selected: proto.selected,
            step: if proto.drag.is_some() { 3 } else { 1 },
        };
        let key = format!("{:?}|{:?}|{:?}|{:?}|{}", proto.d.cfg, proto.d.fleets, opts, proto.d.turn, proto.d.planets.len());
        if key != proto.view.key {
            let rgba = dominion::render(&proto.d, opts).to_rgba();
            if let Some(mut img) = images.get_mut(&proto.view.image) {
                img.data = Some(rgba);
            }
            proto.view.key = key;
        }
    }
    let busy = labs.drag.map(|d| d.0).or(labs.playing);
    let sliding_config = slider.0 == Some(SliderParam::Sites);
    let mut budget = RENDERS_PER_FRAME;
    let mut order: Vec<usize> = (0..LABS).collect();
    if let Some(b) = busy {
        order.sort_by_key(|i| LabId::ALL[*i] != b);
    }
    for i in order {
        let lab = LabId::ALL[i];
        // Coarse previews while dragging, animating or sliding the site count.
        let quick = busy == Some(lab) || sliding_config;
        let step = if quick { 2 } else { 1 };
        let key = format!("{:?}|{}|{step}", cfg.0, labs.state.key_for(lab));
        let view = &mut labs.views[i];
        if key != view.key || view.base.is_none() {
            if budget == 0 && busy != Some(lab) {
                continue;
            }
            budget = budget.saturating_sub(1);
            view.base = Some(render(&cfg.0, lab, &labs.state, step));
            view.key = key;
        }
        let hover = labs.hover.filter(|h| h.0 == lab).map(|h| h.1);
        let shown = format!("{}|{:?}", view.key, hover.map(|h| (h.x as i32, h.y as i32)));
        if shown == view.shown {
            continue;
        }
        let Some(base) = view.base.as_ref() else { continue };
        let rgba = match hover {
            Some(h) => zoom_inset(base, h).to_rgba(),
            None => base.to_rgba(),
        };
        if let Some(mut img) = images.get_mut(&view.image) {
            img.data = Some(rgba);
        }
        view.shown = shown;
    }
}

pub fn update_lab_ui(
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    labs: Res<Labs>,
    mut texts: Query<(&LabText, &mut Text)>,
    mut fills: Query<(&SliderFill, &mut Node), Without<StartPage>>,
    mut values: Query<(&SliderValue, &mut Text), Without<LabText>>,
    mut start: Query<&mut Node, (With<StartPage>, Without<SliderFill>)>,
) {
    for mut n in &mut start {
        let d = if on_start(&phase) { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    }
    let c = cfg.0;
    let st = &labs.state;
    for (which, mut t) in &mut texts {
        let s = match which {
            LabText::Seed => c.seed.to_string(),
            LabText::Proto => dominion::status(&labs.proto.d, labs.proto.selected),
            LabText::Status(l) => status(&c, *l, st),
        };
        if t.0 != s {
            t.0 = s;
        }
    }
    for (f, mut n) in &mut fills {
        let (lo, hi, _) = f.0.range(&c);
        let pct = ((f.0.get(&c, st, &labs.proto) - lo) / (hi - lo)).clamp(0.0, 1.0) * 100.0;
        if n.width != Val::Percent(pct) {
            n.width = Val::Percent(pct);
        }
    }
    for (v, mut t) in &mut values {
        let s = if v.0 == SliderParam::FleetStrength && labs.proto.selected.is_none() {
            "select a fleet".to_string()
        } else {
            v.0.label(v.0.get(&c, st, &labs.proto))
        };
        if t.0 != s {
            t.0 = s;
        }
    }
}

/// Selected/label state of the lab buttons.
pub fn lab_button_state(
    cfg: Res<LabsConfig>,
    labs: Res<Labs>,
    mut buttons: Query<(&LabAction, &mut Selected, &mut Enabled, &BtnLabel)>,
    mut texts: Query<&mut Text>,
) {
    let st = &labs.state;
    let c = cfg.0;
    let proto = &labs.proto;
    let can_reinforce = proto.selected.and_then(|i| proto.d.fleets.get(i)).is_some_and(|f| {
        proto.d.stock[f.side] >= dominion::REINFORCE_COST && f.strength < MAX_STRENGTH
    });
    for (action, mut sel, mut enabled, labels) in &mut buttons {
        let en = !matches!(*action, LabAction::Reinforce) || can_reinforce;
        if enabled.0 != en {
            enabled.0 = en;
        }
        let on = match *action {
            LabAction::ProtoRegions => proto.regions,
            LabAction::ProtoBorders => proto.borders,
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
            _ => false,
        };
        if sel.0 != on {
            sel.0 = on;
        }
        let label = match *action {
            LabAction::Play(l) => Some(if labs.playing == Some(l) { "⏸ Pause".to_string() } else { "▶ Play".to_string() }),
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

/// Wheel scrolling for the start page.
pub fn scroll_labs(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    phase: Res<Phase>,
    mut start: Query<&mut ScrollPosition, With<StartPage>>,
) {
    let dy: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y * 28.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if dy == 0.0 || !on_start(&phase) {
        return;
    }
    if let Ok(mut s) = start.single_mut() {
        s.0.y = (s.0.y - dy).max(0.0);
    }
}
