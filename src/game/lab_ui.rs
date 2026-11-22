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
use crate::prototypes::dominion::{self, DrawOptions, Dominion, Formation, MAX_FLEETS_PER_SIDE, MAX_SHIPS, MIN_SHIPS, Tuning, VIEW_H, VIEW_W, View};
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
    /// The prototype's own map size (bigger than the labs').
    size: (u32, u32),
    /// How many of the sites get a planet.
    planets: usize,
    /// Fleets per side.
    fleets: usize,
    /// Selected fleet id.
    selected: Option<u32>,
    /// A press on a fleet: its id and where the press started.
    drag: Option<(u32, MapVec)>,
    regions: bool,
    borders: bool,
    paused: bool,
    speed: u32,
    view: LabView,
    /// Cached planet-region layer and what it was drawn from.
    background: Option<Canvas>,
    background_key: String,
    /// Seconds since the image was last redrawn.
    since_render: f32,
    /// The camera: the picture is always VIEW_W × VIEW_H.
    camera: View,
    /// Panning with the right or middle button: last cursor position.
    pan_from: Option<Vec2>,
    /// Seconds since the camera last moved (coarse background until it settles).
    camera_still: f32,
    last_camera: View,
}

impl Proto {
    /// A fresh world (after a size, count or config change) and a fitted camera.
    fn rebuild(&mut self, cfg: &VoronoiConfig) {
        self.d = Dominion::new(&proto_cfg(cfg, self.size), self.planets, self.fleets, self.d.tuning);
        self.selected = None;
        self.camera = View::fit(&self.d.cfg, VIEW_W, VIEW_H);
    }
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
    /// Back to the start page.
    Back,
    /// To the Voronoi labs page.
    OpenLabs,
    /// To the ECS systems page.
    OpenEcs,
    /// prototype01 map size.
    ProtoSize(u32, u32),
    /// To the fleet battle and campaign menu.
    Game,
    // Shared config
    SeedDelta(i32),
    NewSeed,
    Metric(Metric),
    Relax(u32),
    /// −/+ on a slider: one step down or up.
    Nudge(SliderParam, i8),
    // prototype01
    ProtoPause,
    ProtoSpeed(u32),
    /// Zoom the prototype map by a factor (around its centre); 0 = fit.
    ProtoZoom(i8),
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
    Fleets,
    FleetShips,
    WeightPerShip,
    ReachPerWeight,
    Production,
    FleetSpeed,
    CombatSpeed,
    GrowRadius,
    Crack,
    Weight(usize),
}

impl SliderParam {
    fn range(self, cfg: &VoronoiConfig) -> (f32, f32, f32) {
        match self {
            SliderParam::Sites => (3.0, 250.0, 1.0),
            SliderParam::Planets => (1.0, cfg.sites as f32, 1.0),
            SliderParam::Fleets => (1.0, MAX_FLEETS_PER_SIDE as f32, 1.0),
            SliderParam::FleetShips => (MIN_SHIPS as f32, MAX_SHIPS as f32, 5.0),
            SliderParam::WeightPerShip => (0.01, 1.0, 0.01),
            SliderParam::ReachPerWeight => (1.0, 100.0, 1.0),
            SliderParam::Production => (0.5, 30.0, 0.5),
            SliderParam::FleetSpeed => (5.0, 150.0, 1.0),
            SliderParam::CombatSpeed => (0.25, 20.0, 0.25),
            SliderParam::GrowRadius => (0.0, LabState::grow_max(cfg), 1.0),
            SliderParam::Crack => (0.5, 10.0, 0.1),
            SliderParam::Weight(_) => (0.2, 4.0, 0.1),
        }
    }

    fn get(self, cfg: &VoronoiConfig, st: &LabState, proto: &Proto) -> f32 {
        match self {
            SliderParam::Sites => cfg.sites as f32,
            SliderParam::Planets => proto.planets.min(cfg.sites) as f32,
            SliderParam::Fleets => proto.fleets as f32,
            SliderParam::FleetShips => proto.selected.and_then(|id| proto.d.fleet(id)).map_or(MIN_SHIPS as f32, |f| f.ships as f32),
            SliderParam::WeightPerShip => proto.d.tuning.weight_per_ship,
            SliderParam::ReachPerWeight => proto.d.tuning.reach_per_weight,
            SliderParam::Production => proto.d.tuning.secs_per_ship,
            SliderParam::FleetSpeed => proto.d.tuning.speed,
            SliderParam::CombatSpeed => proto.d.tuning.combat_speed,
            SliderParam::GrowRadius => st.grow_radius,
            SliderParam::Crack => st.crack,
            SliderParam::Weight(i) => st.weights[i],
        }
    }

    fn label(self, v: f32) -> String {
        match self {
            SliderParam::Sites => format!("{v:.0}"),
            SliderParam::Planets => format!("{v:.0}"),
            SliderParam::Fleets => format!("{v:.0} a side"),
            SliderParam::FleetShips => format!("{v:.0} ships"),
            SliderParam::WeightPerShip => format!("{v:.2} a ship"),
            SliderParam::ReachPerWeight => format!("{v:.0} × weight (px at 720 wide)"),
            SliderParam::Production => format!("{v:.1} s a ship (÷ planet value)"),
            SliderParam::FleetSpeed => format!("{v:.0} px/s (at 720 wide)"),
            SliderParam::CombatSpeed => format!("{v:.2}× the battle's rate of fire"),
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
pub struct LabsPage;

/// Map sizes offered for the prototype.
const PROTO_SIZES: [(u32, u32); 4] = [(720, 400), (1080, 600), (1440, 800), (1920, 1080)];

/// The shared config with the prototype's own map size.
fn proto_cfg(cfg: &VoronoiConfig, size: (u32, u32)) -> VoronoiConfig {
    VoronoiConfig { w: size.0, h: size.1, ..*cfg }
}
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
        button_for(r, "−", LabAction::Nudge(param, -1), BtnKind::Normal);
        button_for(r, "+", LabAction::Nudge(param, 1), BtnKind::Normal);
        r.spawn((SliderValue(param), text("", 13.5, TEXT)));
    });
}

/// Sets a slider's value (already snapped to its step).
fn set_slider(param: SliderParam, v: f32, cfg: &mut VoronoiConfig, labs: &mut Labs) {
    let (lo, hi, _) = param.range(cfg);
    let v = v.clamp(lo, hi);
    let proto = &mut labs.proto;
    match param {
        SliderParam::Sites => {
            let n = v.round() as usize;
            if cfg.sites != n {
                cfg.sites = n;
            }
        }
        SliderParam::Planets => {
            let n = (v.round() as usize).clamp(1, cfg.sites);
            if proto.planets != n {
                proto.planets = n;
                proto.rebuild(cfg);
            }
        }
        SliderParam::Fleets => {
            let n = (v.round() as usize).clamp(1, MAX_FLEETS_PER_SIDE);
            if proto.fleets != n {
                proto.fleets = n;
                proto.rebuild(cfg);
            }
        }
        SliderParam::FleetShips => {
            if let Some(id) = proto.selected {
                proto.d.set_ships(id, v.round() as u32);
            }
        }
        SliderParam::WeightPerShip => proto.d.tuning.weight_per_ship = v,
        SliderParam::ReachPerWeight => proto.d.tuning.reach_per_weight = v,
        SliderParam::Production => proto.d.tuning.secs_per_ship = v,
        SliderParam::FleetSpeed => proto.d.tuning.speed = v,
        SliderParam::CombatSpeed => proto.d.tuning.combat_speed = v,
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

/// Snaps to the slider's step without float drift (0.01 steps stay exact).
fn snap(v: f32, lo: f32, step: f32) -> f32 {
    let n = ((v - lo) / step).round();
    ((lo + n * step) * 1000.0).round() / 1000.0
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
    let size = PROTO_SIZES[1];
    let pcfg = proto_cfg(&cfg.0, size);
    // A fixed-resolution picture, smoothed when the page shows it smaller.
    let mut picture = blank_image(&VoronoiConfig { w: VIEW_W, h: VIEW_H, ..pcfg });
    picture.sampler = ImageSampler::linear();
    let proto_image = images.add(picture);
    let (planets, fleets) = (20, 3);
    let proto = Proto {
        d: Dominion::new(&pcfg, planets, fleets, Tuning::default()),
        size,
        planets,
        fleets,
        selected: None,
        drag: None,
        regions: true,
        borders: true,
        paused: false,
        speed: 1,
        view: LabView { image: proto_image.clone(), key: String::new(), base: None, shown: String::new() },
        background: None,
        background_key: String::new(),
        since_render: 0.0,
        camera: View::fit(&pcfg, VIEW_W, VIEW_H),
        pan_from: None,
        camera_still: 0.0,
        last_camera: View::fit(&pcfg, VIEW_W, VIEW_H),
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

    // --- Start page: config and the prototypes ---
    commands.spawn((StartPage, page_root(), BackgroundColor(BG), ScrollPosition::default(), GlobalZIndex(10))).with_children(|m| {
        m.spawn(page_column(1500.0)).with_children(|c| {
            c.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(16.0), ..row() }).with_children(|r| {
                r.spawn(text("Successor Stars", 26.0, TEXT));
                button_for(r, "Voronoi labs →", LabAction::OpenLabs, BtnKind::Normal);
                button_for(r, "ECS systems →", LabAction::OpenEcs, BtnKind::Normal);
                button_for(r, "Fleet battle & campaign →", LabAction::Game, BtnKind::Primary);
            });
            c.spawn((text("Prototypes built on the Voronoi toolkit.", 14.0, DIM), Node { margin: UiRect::bottom(Val::Px(10.0)), ..default() }));
            config_card(c, "Shared by the prototypes and the labs.");
            section_heading(c, "Prototypes");
            c.spawn(panel()).with_children(|p| {
                card_title(
                    p,
                    "prototype01 — Fleet dominion",
                    "Real time. Every planet builds ships into its own garrison fleet (richer planets faster); \
                     independent planets only defend. Blue and Red fleets take orders once they have 10 ships. Every \
                     ship is an entity in the battle's ECS, run by the battle's own systems: gun range and cone, \
                     front/flank/rear hit chances, reload, hull, closing ranks and morale, so formation and facing \
                     matter and a beaten fleet turns shaken (gold), routs (grey) and flees off the map. Keys 1–6 set the selected fleet's \
                     formation. A planet changes hands once its garrison is gone and an enemy fleet holds it for 3 s \
                     (ring). Patches show each fleet's influence (ships × weight per ship, reach × weight).",
                );
                p.spawn(row()).with_children(|r| {
                    button_for(r, "Pause", LabAction::ProtoPause, BtnKind::Primary);
                    for m in [1, 2, 4] {
                        button_for(r, &format!("{m}×"), LabAction::ProtoSpeed(m), BtnKind::Choice);
                    }
                    toggle(r, "Planet regions", LabAction::ProtoRegions);
                    toggle(r, "Patch edges", LabAction::ProtoBorders);
                    button_for(r, "Reset", LabAction::ProtoReset, BtnKind::Normal);
                    button_for(r, "Zoom −", LabAction::ProtoZoom(-1), BtnKind::Normal);
                    button_for(r, "Zoom +", LabAction::ProtoZoom(1), BtnKind::Normal);
                    button_for(r, "Fit", LabAction::ProtoZoom(0), BtnKind::Normal);
                });
                p.spawn((LabText::Proto, text("", 13.0, GREEN)));
                p.spawn((
                    PrototypeImage,
                    ImageNode::new(proto_image),
                    RelativeCursorPosition::default(),
                    Node {
                        width: Val::Percent(100.0),
                        aspect_ratio: Some(VIEW_W as f32 / VIEW_H as f32),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
                let sizes: Vec<(String, LabAction)> =
                    PROTO_SIZES.iter().map(|(w, h)| (format!("{w}×{h}"), LabAction::ProtoSize(*w, *h))).collect();
                choice_row(p, "Map size", &sizes);
                slider(p, "Sites", SliderParam::Sites);
                slider(p, "Planets", SliderParam::Planets);
                slider(p, "Fleets", SliderParam::Fleets);
                slider(p, "Production", SliderParam::Production);
                slider(p, "Fleet speed", SliderParam::FleetSpeed);
                slider(p, "Combat speed", SliderParam::CombatSpeed);
                slider(p, "Selected ships", SliderParam::FleetShips);
                slider(p, "Weight per ship", SliderParam::WeightPerShip);
                slider(p, "Reach per weight", SliderParam::ReachPerWeight);
                p.spawn(text(
                    "Click a fleet to select it, then click the map to send it · or drag from a fleet to where it should go · \
                     1 Line · 2 Wedge · 3 Crescent · 4 Echelon L · 5 Echelon R · 6 Column · wheel over the map: zoom · right- or middle-drag: pan.",
                    12.5,
                    DIM,
                ));
            });
        });
    });

    // --- Labs page: only rendered while open ---
    commands.spawn((LabsPage, page_root(), BackgroundColor(BG), ScrollPosition::default(), GlobalZIndex(10))).with_children(|m| {
        m.spawn(page_column(980.0)).with_children(|c| {
            c.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(16.0), ..row() }).with_children(|r| {
                button_for(r, "← Home", LabAction::Back, BtnKind::Normal);
                r.spawn(text("Voronoi labs", 24.0, TEXT));
            });
            c.spawn((
                text("After the gamegeniuslab tutorial \"Voronoi diagrams in game development\".", 14.0, DIM),
                Node { margin: UiRect::bottom(Val::Px(10.0)), ..default() },
            ));
            config_card(c, "Shared by every lab below. Lab maps are 720 × 400; Lloyd rounds apply to the biome and painted islands.");
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

fn page_root() -> Node {
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

fn page_column(max_width: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Column,
        width: Val::Percent(100.0),
        max_width: Val::Px(max_width),
        padding: UiRect::all(Val::Px(16.0)),
        row_gap: Val::Px(4.0),
        flex_shrink: 0.0,
        ..default()
    }
}

/// The shared config card (shown on both pages; same actions).
fn config_card(c: &mut ChildSpawnerCommands, note: &str) {
    c.spawn(panel()).with_children(|p| {
        card_title(p, "Config", note);
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

fn on_labs(phase: &Phase) -> bool {
    *phase == Phase::Labs
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
                labs.playing = None;
                requests.write(AppRequest::Home);
            }
            LabAction::OpenLabs => {
                requests.write(AppRequest::LabsPage);
            }
            LabAction::OpenEcs => {
                requests.write(AppRequest::EcsPage);
            }
            LabAction::ProtoSize(w, h) => {
                if labs.proto.size != (w, h) {
                    labs.proto.size = (w, h);
                    labs.proto.rebuild(&cfg.0);
                }
            }
            LabAction::Game => {
                labs.playing = None;
                requests.write(AppRequest::Menu);
            }
            LabAction::SeedDelta(d) => cfg.0.seed = cfg.0.seed.saturating_add_signed(d).max(1),
            LabAction::NewSeed => cfg.0.seed = super::random_seed() % 2_000_000_000 + 1,
            LabAction::Metric(m) => cfg.0.metric = m,
            LabAction::Relax(n) => cfg.0.relax = n,
            LabAction::Nudge(param, dir) => {
                let (lo, _, step) = param.range(&cfg.0);
                let v = snap(param.get(&cfg.0, &labs.state, &labs.proto) + step * dir as f32, lo, step);
                set_slider(param, v, &mut cfg.0, labs);
            }
            LabAction::ProtoPause => labs.proto.paused = !labs.proto.paused,
            LabAction::ProtoSpeed(m) => {
                labs.proto.speed = m;
                labs.proto.paused = false;
            }
            LabAction::ProtoReset => labs.proto.rebuild(&cfg.0),
            LabAction::ProtoZoom(dir) => {
                let proto = &mut labs.proto;
                let centre = Vec2::new(VIEW_W as f32, VIEW_H as f32) * 0.5;
                match dir {
                    0 => proto.camera = View::fit(&proto.d.cfg, VIEW_W, VIEW_H),
                    d => proto.camera.zoom_at(centre, if d > 0 { 1.5 } else { 1.0 / 1.5 }, &proto.d.cfg),
                }
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
        labs.proto.rebuild(&c);
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
        let v = snap(lo + (hi - lo) * t, lo, step);
        if (v - track.0.get(&cfg.0, &labs.state, &labs.proto)).abs() > step * 0.25 {
            set_slider(track.0, v, &mut cfg.0, &mut labs);
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
    if !on_labs(&phase) {
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

/// prototype01: click a fleet to select it; click the map to send the
/// selected fleet there, or drag from a fleet to where it should go.
#[allow(clippy::too_many_arguments)]
pub fn proto_pointer(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    image: Query<&RelativeCursorPosition, With<PrototypeImage>>,
    phase: Res<Phase>,
    mut labs: ResMut<Labs>,
) {
    let proto = &mut labs.proto;
    if !on_start(&phase) {
        proto.drag = None;
        proto.pan_from = None;
        return;
    }
    // Keys 1–6 set the selected fleet's formation.
    const KEYS: [KeyCode; 6] = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6];
    if let Some(id) = proto.selected
        && let Some(k) = KEYS.iter().position(|k| keys.just_pressed(*k))
    {
        proto.d.set_formation(id, Formation::PLAYER[k]);
    }
    let Ok(cursor) = image.single() else { return };
    let Some(n) = cursor.normalized else { return };
    // Cursor in picture pixels, then in the world.
    let screen = Vec2::new((n.x + 0.5) * VIEW_W as f32, (n.y + 0.5) * VIEW_H as f32);
    let cfg = proto.d.cfg;
    let p = proto.camera.to_world(screen).clamp(MapVec::ZERO, MapVec::new(cfg.w as f32 - 0.5, cfg.h as f32 - 0.5));
    let over = cursor.cursor_over();
    // Wheel over the map zooms around the cursor (the page scroll skips it).
    let notches: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y / 40.0,
        })
        .sum();
    if over && notches != 0.0 {
        proto.camera.zoom_at(screen, 1.2f32.powf(notches), &cfg);
    }
    // Right or middle drag pans.
    if over && (mouse.just_pressed(MouseButton::Right) || mouse.just_pressed(MouseButton::Middle)) {
        proto.pan_from = Some(screen);
    }
    if let Some(from) = proto.pan_from {
        if mouse.pressed(MouseButton::Right) || mouse.pressed(MouseButton::Middle) {
            proto.camera.pan(screen - from, &cfg);
            proto.pan_from = Some(screen);
        } else {
            proto.pan_from = None;
        }
    }
    if mouse.just_pressed(MouseButton::Left) && over {
        // About 10 picture pixels, in world units.
        match proto.d.fleet_at(p, 10.0 / proto.camera.zoom) {
            Some(id) => {
                proto.selected = Some(id);
                proto.drag = Some((id, screen));
            }
            None => {
                if let Some(id) = proto.selected {
                    proto.d.order_move(id, p);
                }
            }
        }
    } else if mouse.just_released(MouseButton::Left)
        && let Some((id, from)) = proto.drag.take()
        && from.distance(screen) > 12.0
    {
        proto.d.order_move(id, p);
    }
}

/// Runs prototype01 in real time while the start page is open.
pub fn step_proto(time: Res<Time>, phase: Res<Phase>, mut labs: ResMut<Labs>) {
    let proto = &mut labs.proto;
    if !on_start(&phase) || proto.paused {
        return;
    }
    let dt = time.delta_secs().min(0.1) * proto.speed as f32;
    proto.d.step(dt);
}

/// Play: grow the seeds, or run Lloyd rounds until nothing moves.
pub fn animate_labs(time: Res<Time>, phase: Res<Phase>, cfg: Res<LabsConfig>, mut labs: ResMut<Labs>) {
    if !on_labs(&phase) {
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
    time: Res<Time>,
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    slider: Res<ActiveSlider>,
    mut labs: ResMut<Labs>,
    mut images: ResMut<Assets<Image>>,
) {
    let labs = &mut *labs;
    // prototype01, on the start page only, at most ~25 frames a second.
    if on_start(&phase) {
        let proto = &mut labs.proto;
        proto.since_render += time.delta_secs();
        if proto.camera != proto.last_camera {
            proto.last_camera = proto.camera;
            proto.camera_still = 0.0;
        } else {
            proto.camera_still += time.delta_secs();
        }
        // Half-resolution regions while zooming or panning, sharp once still.
        let bg_step = if proto.camera_still < 0.25 { 2 } else { 1 };
        let opts = DrawOptions { regions: proto.regions, borders: proto.borders, selected: proto.selected, step: 3 };
        let key = format!(
            "{}|{:?}|{:?}|{bg_step}|{:?}|{:?}",
            proto.d.time,
            opts,
            proto.camera,
            proto.d.tuning,
            proto.d.fleets.iter().map(|f| (f.id, f.pos, f.facing, f.ships, f.formation, f.dest)).collect::<Vec<_>>()
        );
        if key != proto.view.key && proto.since_render >= 0.04 {
            let bg_key = format!("{}|{:?}|{bg_step}", proto.d.background_key(proto.regions), proto.camera);
            if bg_key != proto.background_key || proto.background.is_none() {
                proto.background = Some(dominion::render_background(&proto.d, &proto.camera, proto.regions, bg_step));
                proto.background_key = bg_key;
            }
            let rgba = dominion::render(&proto.d, proto.background.as_ref().unwrap(), &proto.camera, opts).to_rgba();
            if let Some(mut img) = images.get_mut(&proto.view.image) {
                img.data = Some(rgba);
            }
            proto.view.key = key;
            proto.since_render = 0.0;
        }
        return;
    }
    // The labs, on their own page only.
    if !on_labs(&phase) {
        return;
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

#[allow(clippy::too_many_arguments)]
pub fn update_lab_ui(
    phase: Res<Phase>,
    cfg: Res<LabsConfig>,
    labs: Res<Labs>,
    mut texts: Query<(&LabText, &mut Text)>,
    mut fills: Query<(&SliderFill, &mut Node), (Without<StartPage>, Without<LabsPage>, Without<PrototypeImage>)>,
    mut values: Query<(&SliderValue, &mut Text), Without<LabText>>,
    mut start: Query<&mut Node, (With<StartPage>, Without<SliderFill>, Without<LabsPage>, Without<PrototypeImage>)>,
    mut lab_page: Query<&mut Node, (With<LabsPage>, Without<SliderFill>, Without<StartPage>, Without<PrototypeImage>)>,
    mut proto_image: Query<&mut Node, (With<PrototypeImage>, Without<SliderFill>, Without<StartPage>, Without<LabsPage>)>,
) {
    let show = |n: &mut Node, on: bool| {
        let d = if on { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    };
    for mut n in &mut start {
        show(&mut n, on_start(&phase));
    }
    for mut n in &mut lab_page {
        show(&mut n, on_labs(&phase));
    }
    // The prototype picture has a fixed shape.
    let aspect = Some(VIEW_W as f32 / VIEW_H as f32);
    for mut n in &mut proto_image {
        if n.aspect_ratio != aspect {
            n.aspect_ratio = aspect;
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
        let s = if v.0 == SliderParam::FleetShips && labs.proto.selected.and_then(|id| labs.proto.d.fleet(id)).is_none() {
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
    let size = proto.size;
    for (action, mut sel, mut enabled, labels) in &mut buttons {
        if !enabled.0 {
            enabled.0 = true;
        }
        let on = match *action {
            LabAction::ProtoPause => proto.paused,
            LabAction::ProtoSpeed(m) => !proto.paused && proto.speed == m,
            LabAction::ProtoSize(w, h) => size == (w, h),
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
            LabAction::ProtoPause => Some(if proto.paused { "▶ Resume".to_string() } else { "Pause".to_string() }),
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

/// Wheel scrolling for the start and labs pages.
pub fn scroll_labs(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    map: Query<&RelativeCursorPosition, With<PrototypeImage>>,
    phase: Res<Phase>,
    mut start: Query<&mut ScrollPosition, (With<StartPage>, Without<LabsPage>)>,
    mut lab_page: Query<&mut ScrollPosition, (With<LabsPage>, Without<StartPage>)>,
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
    // The wheel zooms the prototype map instead.
    if on_start(&phase) && map.iter().any(|c| c.cursor_over()) {
        return;
    }
    let target = if on_start(&phase) {
        start.single_mut().ok()
    } else if on_labs(&phase) {
        lab_page.single_mut().ok()
    } else {
        None
    };
    if let Some(mut s) = target {
        s.0.y = (s.0.y - dy).max(0.0);
    }
}
