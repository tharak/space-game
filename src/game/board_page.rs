//! The hex board page: the BattleStars v0.2.1 battle (see [`crate::board`])
//! played on a 27×27 hex board. The rules run in their own `World`; this
//! page turns clicks, keys and buttons into `BattleCommand`s, paces the AI,
//! keeps the log and draws the board with the software rasterizer.

use std::collections::HashSet;

use bevy::asset::RenderAssetUsages;
use bevy::ecs::message::Messages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::RelativeCursorPosition;

use super::flow::Phase;
use super::theme::*;
use super::ui::{BtnKind, Enabled, Selected, button_for, panel, row, text};
use crate::board::ai;
use crate::board::battle::{self as rules, *};
use crate::board::commands::{BattleCommand, apply, fire_zone};
use crate::board::components::Morale;
use crate::board::config::*;
use crate::board::dice::Dice;
use crate::board::hex::{self, DIR_ANGLE, HS, HW, Hex, OX, OY, hex_center_px, hex_corners_px};
use crate::voronoi::raster::{Canvas, Rgb};

/// Board pixels to picture pixels.
const K: f32 = 1.3;
const IMG_W: u32 = ((2.0 * OX + (COLS as f32 + 0.5) * HW) * K) as u32;
const IMG_H: u32 = ((2.0 * OY + (ROWS as f32 - 1.0) * 1.5 * HS) * K) as u32;
/// Seconds between AI activations against a human, and in spectate auto.
const AI_DELAY: f32 = 0.25;
const AUTO_DELAY: f32 = 0.22;
const LASER_SECS: f32 = 0.45;
const LOG_LINES: usize = 18;
const INFO_LINES: usize = 6;
const STATUS_LINES: usize = 3;

const C_BG: Rgb = [0x07, 0x0a, 0x10];
const C_CELL: Rgb = [0x11, 0x16, 0x24];
const C_GRID: Rgb = [0x1d, 0x24, 0x38];
const C_FIRE_ZONE: Rgb = [0x2e, 0x24, 0x48];
const C_SIDE: [Rgb; 2] = [[0x4a, 0x9e, 0xff], [0xff, 0x6a, 0x4a]];
const C_DARK: Rgb = [0x0b, 0x0e, 0x14];
const C_GOLD: Rgb = [0xff, 0xd1, 0x66];
const C_SHAKEN: Rgb = [0xff, 0x9a, 0x3c];
const C_ROUTED: Rgb = [0xff, 0x33, 0x33];
const C_DONE: Rgb = [0x5d, 0x64, 0x78];
const C_WHITE: Rgb = [0xff, 0xff, 0xff];
const C_TARGET: Rgb = [0xff, 0x33, 0x33];
const C_MISS: Rgb = [0x88, 0x92, 0xab];

#[derive(Component)]
pub struct BoardPage;
#[derive(Component)]
pub struct BoardImage;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoardText {
    Status(usize),
    Info(usize),
    Log(usize),
    Scenario,
    Hover,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum BoardAction {
    Home,
    Scenario(usize),
    Control(ControlMode),
    Size(usize),
    Deploy(DeployMode),
    Moves(MoveMode),
    NewBattle,
    Cmd(BattleCommand),
    Step,
    Auto,
}

struct Laser {
    from: Hex,
    to: Hex,
    side: usize,
    hit: bool,
    age: f32,
}

#[derive(Resource)]
pub struct HexBoard {
    world: World,
    image: Handle<Image>,
    settings: MatchSettings,
    timer: f32,
    auto: bool,
    step: bool,
    log: Vec<(String, Color)>,
    lasers: Vec<Laser>,
    hover: Option<Hex>,
    seed: u32,
}

impl HexBoard {
    fn new(image: Handle<Image>, seed: u32) -> Self {
        let settings = MatchSettings { deploy_mode: DeployMode::Fixed, ..default() };
        let mut world = World::new();
        rules::init_world(&mut world, settings, Dice::seeded(seed));
        let mut b = Self { world, image, settings, timer: 0.0, auto: false, step: false, log: Vec::new(), lasers: Vec::new(), hover: None, seed };
        b.restart();
        b
    }

    /// A new battle with the current settings and fresh dice.
    fn restart(&mut self) {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        self.world.insert_resource(self.settings);
        self.world.insert_resource(Dice::seeded(self.seed));
        self.log.clear();
        self.lasers.clear();
        self.timer = 0.0;
        self.auto = false;
        apply(&mut self.world, BattleCommand::Restart);
        self.drain_events();
    }

    fn command(&mut self, cmd: BattleCommand) {
        apply(&mut self.world, cmd);
        self.drain_events();
    }

    fn phase(&self) -> rules::Phase {
        *self.world.resource::<rules::Phase>()
    }

    fn spectate(&self) -> bool {
        self.settings.control == ControlMode::Spectate
    }

    /// Paces the AI like the original: one activation per delay, or per
    /// Step / Auto tick when spectating.
    fn tick(&mut self, dt: f32) {
        for l in &mut self.lasers {
            l.age += dt;
        }
        self.lasers.retain(|l| l.age < LASER_SECS);
        if let Next::Ai(side) = advance(&mut self.world) {
            self.timer += dt;
            let go = if self.spectate() { self.step || (self.auto && self.timer >= AUTO_DELAY) } else { self.timer >= AI_DELAY };
            if go {
                self.timer = 0.0;
                self.step = false;
                ai::activate_next(&mut self.world, side);
                advance(&mut self.world);
            }
        } else {
            self.step = false;
            if self.phase() == rules::Phase::GameOver {
                self.auto = false;
            }
        }
        self.drain_events();
    }

    /// Rule events become log lines and lasers.
    fn drain_events(&mut self) {
        let events: Vec<BattleEvent> = self.world.resource_mut::<Messages<BattleEvent>>().drain().collect();
        for event in events {
            let (line, color) = match event {
                BattleEvent::Note { text, tone } => (text, tone_color(tone)),
                BattleEvent::MoraleChecked { label, roll, modifiers, total, passed } => (
                    format!(
                        "  {label} morale: {roll}{} = {total} → {}",
                        if modifiers.is_empty() { String::new() } else { format!(" {}", modifiers.join(" ")) },
                        if passed { "holds" } else { "FAILS" }
                    ),
                    if passed { TEXT } else { BAD },
                ),
                BattleEvent::Routed { label } => (format!("  {label} ROUTS!"), BAD),
                BattleEvent::Destroyed { label, .. } => (format!("  {label} is DESTROYED"), BAD),
                BattleEvent::FlagshipLost { side } => {
                    (format!("  {} FLAGSHIP LOST — fleet-wide morale check, command net down", side_name(side)), BAD)
                }
                BattleEvent::Shot { attacker, target, side, arc, need, rolls, hits, from, to } => {
                    self.lasers.push(Laser { from, to, side, hit: hits > 0, age: 0.0 });
                    (
                        format!(
                            "{attacker} fires at {target} ({} arc, {need}+): [{}] → {hits} hit{}",
                            arc.name(),
                            rolls.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" "),
                            if hits == 1 { "" } else { "s" }
                        ),
                        if hits > 0 { side_color(side) } else { TEXT },
                    )
                }
                BattleEvent::Fled { label } => (format!("  {label} flees off the map"), BAD),
                BattleEvent::Rallied { label, roll, bonus } => {
                    (format!("{label} RALLIES ({roll}{}) — now Shaken", if bonus { "+1" } else { "" }), GREEN)
                }
                BattleEvent::RallyFailed { label, roll, bonus } => {
                    (format!("{label} fails to rally ({roll}{}) and keeps running", if bonus { "+1" } else { "" }), TEXT)
                }
                BattleEvent::ShakenRefused { label } => (format!("{label} is Shaken — it refuses to move toward the enemy"), BAD),
            };
            self.log.push((line, color));
        }
        let over = self.log.len().saturating_sub(400);
        self.log.drain(..over);
    }
}

fn tone_color(tone: Tone) -> Color {
    match tone {
        Tone::Plain => TEXT,
        Tone::Title => GOLD,
        Tone::Bad => BAD,
        Tone::Good => GREEN,
        Tone::Side(s) => side_color(s),
    }
}

fn side_color(side: usize) -> Color {
    if side == 0 { BLUE } else { RED }
}

// --- What the panel shows and which buttons work ---

#[derive(Default)]
struct View {
    status: Vec<(String, Color)>,
    info: Vec<(String, Color)>,
    can: HashSet<BattleCommand>,
    fire_zone: HashSet<Hex>,
    targets: HashSet<Entity>,
    selected: Option<Entity>,
    flagships: [Option<Hex>; 2],
    setup: Option<SetupState>,
}

fn view(b: &mut HexBoard) -> View {
    let world = &mut b.world;
    let phase = *world.resource::<rules::Phase>();
    let settings = *world.resource::<MatchSettings>();
    let mut v = View::default();
    let ships = snapshot(world);
    for side in 0..2 {
        v.flagships[side] = flagship_of(&ships, side).map(|s| s.hex);
    }
    if phase == rules::Phase::Deployment {
        let setup = world.resource::<Deployment>().current.clone();
        if let Some(setup) = &setup {
            let size = settings.size;
            v.status.push((format!("{} deployment — {}/{} placed", side_name(setup.side), setup.placed.len(), size), side_color(setup.side)));
            let line = match setup.selected {
                Some(i) => format!(
                    "Selected squadron{}. Turn it (Q/E), set flagship, remove it, or click an empty zone hex to place another.",
                    if setup.flagship == Some(i) { " — flagship" } else { "" }
                ),
                None if setup.placed.len() < size => {
                    format!("Click an empty hex in your shaded zone to place your next squadron ({} left).", size - setup.placed.len())
                }
                None => format!("All {size} squadrons placed. Click one to adjust, or confirm."),
            };
            v.info.push((line, TEXT));
            let sel = setup.selected.is_some();
            for (on, c) in [
                (sel, BattleCommand::TurnLeft),
                (sel, BattleCommand::TurnRight),
                (sel && setup.selected != setup.flagship, BattleCommand::SetFlagship),
                (sel, BattleCommand::RemoveDeployed),
                (setup.placed.len() == size, BattleCommand::ConfirmDeployment),
            ] {
                if on {
                    v.can.insert(c);
                }
            }
        }
        v.setup = setup;
        return v;
    }
    let battle = world.resource::<Battle>().clone();
    let losses_of = |side: usize| battle.fleets[side].roster.iter().filter(|e| find(&ships, **e).is_some_and(|s| !s.alive)).count();
    let fleet_tag = |side: usize| {
        let f = &battle.fleets[side];
        let sup = if f.supply == Supply::Ok { String::new() } else { format!(" ({} supply)", f.supply.name()) };
        format!("{} {}{}: {}/{} losses{}", side_name(side), f.formation.name(), sup, losses_of(side), battle.break_at, if f.flag_lost { " · flagship lost" } else { "" })
    };
    v.status = vec![(format!("Turn {}/{}", battle.turn, MAX_TURNS), GOLD), (fleet_tag(0), BLUE), (fleet_tag(1), RED)];

    let act = world.resource::<Activation>().0;
    if let Some(result) = &battle.result {
        v.info.push((format!("{}. {}", result.title, result.body), GOLD));
        v.info.push(("Press New battle to play again.".into(), DIM));
    } else if let Some(a) = act {
        match a.unit {
            None => v.info.push((format!("{}: click one of your un-activated squadrons.", side_name(a.side)), side_color(a.side))),
            Some(u) => {
                let me = *find(&ships, u).expect("selected ship");
                let fire_now = can_fire(world, &a);
                v.info.push((
                    format!("{}{} — str {}, {}", label_of(world, u), if me.flagship { " (flagship)" } else { "" }, me.strength, me.morale.name()),
                    GOLD,
                ));
                v.info.push((
                    if a.cmd { "In command: move AND fire".into() } else { "OUT of command: move OR fire".into() },
                    if a.cmd { TEXT } else { BAD },
                ));
                v.info.push((format!("MP {}/{}{}", a.mp, MP_MAX, if a.fired { " · has fired" } else { "" }), TEXT));
                if fire_now {
                    v.info.push(("Click a red-outlined enemy to fire.".into(), RED));
                }
                if can_switch_selection(&a) {
                    v.info.push(("Changed your mind? Click another un-activated squadron — nothing's committed yet.".into(), DIM));
                }
                for (on, c) in [
                    (can_move(&a), BattleCommand::TurnLeft),
                    (can_move(&a), BattleCommand::TurnRight),
                    (can_move(&a), BattleCommand::Forward),
                    (can_back(&a), BattleCommand::Backward),
                    (fire_now, BattleCommand::FireMode),
                    (true, BattleCommand::EndActivation),
                ] {
                    if on {
                        v.can.insert(c);
                    }
                }
                v.selected = Some(u);
                if fire_now {
                    v.targets = legal_targets(&ships, &me).iter().map(|t| t.e).collect();
                }
                v.fire_zone = fire_zone(world).into_iter().collect();
            }
        }
    } else if b.spectate() {
        v.info.push(("Spectating — press Step or Auto.".into(), DIM));
    } else {
        v.info.push(("AI is acting…".into(), DIM));
    }
    v
}

fn hover_line(b: &mut HexBoard) -> String {
    let Some(h) = b.hover else { return String::new() };
    let ships = snapshot(&mut b.world);
    let Some(s) = alive(&ships).find(|s| s.hex == h).copied() else { return format!("Hex {},{}", h.c, h.r) };
    let mut parts = vec![
        format!("{}{}", label_of(&b.world, s.e), if s.flagship { " (flagship)" } else { "" }),
        format!("str {}/{}", s.strength, MAX_STRENGTH),
        s.morale.name().to_string(),
    ];
    if s.activated {
        parts.push("activated".into());
    }
    if !in_command(&ships, &s) {
        parts.push("out of command".into());
    }
    parts.join(" · ")
}

// --- Drawing ---

fn px(h: Hex) -> Vec2 {
    let (x, y) = hex_center_px(h);
    Vec2::new(x, y) * K
}

fn hex_poly(c: Vec2, s: f32) -> Vec<Vec2> {
    hex_corners_px(c.x, c.y, s).iter().map(|(x, y)| Vec2::new(*x, *y)).collect()
}

fn facing_dir(f: u8) -> Vec2 {
    Vec2::from_angle((DIR_ANGLE[f as usize % 6] as f32).to_radians())
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|i| (a[i] as f32 + (b[i] as f32 - a[i] as f32) * t).round() as u8)
}

struct Token {
    hex: Hex,
    facing: u8,
    side: usize,
    strength: u8,
    flagship: bool,
    border: Rgb,
    dim: bool,
}

fn draw_token(c: &mut Canvas, t: &Token) {
    let at = px(t.hex);
    let s = HS * K;
    c.polygon_fill(&hex_poly(at, s * 0.86), t.border, 1.0);
    let fill = if t.dim { mix(C_SIDE[t.side], C_DARK, 0.55) } else { C_SIDE[t.side] };
    c.polygon_fill(&hex_poly(at, s * 0.72), fill, 1.0);
    // Facing arrow.
    let d = facing_dir(t.facing);
    let n = d.perp();
    let tip = at + d * s * 0.62;
    let base = at + d * s * 0.05;
    let arrow = if t.flagship { C_GOLD } else { C_DARK };
    c.polygon_fill(&[tip, base + n * s * 0.32, base - n * s * 0.32], arrow, 1.0);
    // A flagship wears a gold ring; strength pips sit behind the arrow.
    if t.flagship {
        c.polygon_stroke(&hex_poly(at, s * 0.94), 2.0, C_GOLD, 1.0);
    }
    let back = at - d * s * 0.36;
    for k in 0..MAX_STRENGTH {
        let off = (k as f32 - (MAX_STRENGTH as f32 - 1.0) / 2.0) * s * 0.19;
        let a = if k < t.strength { 1.0 } else { 0.25 };
        c.disc(back + n * off, s * 0.07, C_WHITE, a);
    }
}

fn render(b: &mut HexBoard, v: &View) -> Canvas {
    let mut c = Canvas::new(IMG_W, IMG_H, C_BG);
    let s = HS * K;
    let setup_side = v.setup.as_ref().map(|st| st.side);
    for h in board_hexes() {
        let at = px(h);
        let mut col = if v.fire_zone.contains(&h) { C_FIRE_ZONE } else { C_CELL };
        if let Some(side) = setup_side
            && in_setup_zone(side, h)
        {
            col = mix(col, C_SIDE[side], 0.14);
        }
        for (side, f) in v.flagships.iter().enumerate() {
            if f.is_some_and(|f| hex::dist(f, h) <= CMD_R) {
                col = mix(col, C_SIDE[side], 0.1);
            }
        }
        let poly = hex_poly(at, s);
        c.polygon_fill(&poly, col, 1.0);
        c.polygon_stroke(&poly, 1.0, C_GRID, 1.0);
    }
    if let Some(h) = b.hover
        && in_bounds(h)
    {
        c.polygon_stroke(&hex_poly(px(h), s), 1.5, C_WHITE, 0.45);
    }
    if let Some(setup) = &v.setup {
        for (i, p) in setup.placed.iter().enumerate() {
            let border = if setup.selected == Some(i) { C_WHITE } else { C_DARK };
            draw_token(
                &mut c,
                &Token { hex: p.hex, facing: p.facing, side: setup.side, strength: MAX_STRENGTH, flagship: setup.flagship == Some(i), border, dim: false },
            );
        }
    }
    let ships = snapshot(&mut b.world);
    for sh in alive(&ships) {
        let border = if v.selected == Some(sh.e) {
            C_WHITE
        } else if v.targets.contains(&sh.e) {
            C_TARGET
        } else {
            match sh.morale {
                Morale::Routed => C_ROUTED,
                Morale::Shaken => C_SHAKEN,
                Morale::Steady if sh.activated => C_DONE,
                Morale::Steady => C_DARK,
            }
        };
        draw_token(
            &mut c,
            &Token { hex: sh.hex, facing: sh.facing, side: sh.side, strength: sh.strength, flagship: sh.flagship, border, dim: sh.activated && v.selected != Some(sh.e) },
        );
    }
    for l in &b.lasers {
        let a = 1.0 - l.age / LASER_SECS;
        let (from, to) = (px(l.from), px(l.to));
        if l.hit {
            c.line(from, to, 2.5, C_SIDE[l.side], a);
            c.disc(to, s * 0.3, C_WHITE, a * 0.6);
        } else {
            c.dashed_line(from, to, 6.0, 5.0, C_MISS, a);
        }
    }
    c
}

// --- Page ---

fn blank_image() -> Image {
    let mut img = Image::new_fill(
        Extent3d { width: IMG_W, height: IMG_H, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0x07, 0x0a, 0x10, 0xff],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    img.sampler = ImageSampler::linear();
    img
}

pub fn setup_board_page(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(blank_image());
    commands.insert_resource(HexBoard::new(image.clone(), super::random_seed() ^ 0x9e37_79b9));

    let root = Node {
        position_type: PositionType::Absolute,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        overflow: Overflow::scroll_y(),
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        display: Display::None,
        ..default()
    };
    commands.spawn((BoardPage, root, BackgroundColor(BG), ScrollPosition::default(), GlobalZIndex(10))).with_children(|m| {
        m.spawn(Node {
            flex_direction: FlexDirection::Column,
            width: Val::Percent(100.0),
            max_width: Val::Px(1500.0),
            padding: UiRect::all(Val::Px(16.0)),
            row_gap: Val::Px(4.0),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|c| {
            c.spawn(Node { align_items: AlignItems::Center, column_gap: Val::Px(16.0), ..row() }).with_children(|r| {
                button_for(r, "← Home", BoardAction::Home, BtnKind::Normal);
                r.spawn(text("Hex board — BattleStars v0.2.1", 24.0, TEXT));
            });
            c.spawn((
                text(
                    "The alternating-activation hex rules that passed the paper playtest: squadrons of strength 4 activate one at a \
                     time, move up to 3 hexes, fire in a 3-hex forward arc (front 5+, flank 4+, rear 3+), check morale when hit and \
                     rout off their own edge. Stay within 4 hexes of your flagship to move AND fire. A fleet breaks after losing half.",
                    14.0,
                    DIM,
                ),
                Node { margin: UiRect::bottom(Val::Px(8.0)), ..default() },
            ));
            c.spawn(panel()).with_children(|p| {
                let scen: Vec<(String, BoardAction)> =
                    SCENARIOS.iter().enumerate().map(|(i, s)| (s.title.to_string(), BoardAction::Scenario(i))).collect();
                choice_row(p, "Scenario", &scen);
                p.spawn((BoardText::Scenario, text("", 13.0, DIM)));
                choice_row(
                    p,
                    "Control",
                    &[
                        ("Play Blue".into(), BoardAction::Control(ControlMode::PlayBlue)),
                        ("Play Red".into(), BoardAction::Control(ControlMode::PlayRed)),
                        ("Hotseat".into(), BoardAction::Control(ControlMode::Hotseat)),
                        ("Spectate".into(), BoardAction::Control(ControlMode::Spectate)),
                    ],
                );
                let sizes: Vec<(String, BoardAction)> = [5, 7, 9, 12].iter().map(|n| (format!("{n}"), BoardAction::Size(*n))).collect();
                choice_row(p, "Squadrons a side", &sizes);
                choice_row(
                    p,
                    "Deployment",
                    &[("Formation".into(), BoardAction::Deploy(DeployMode::Fixed)), ("Place by hand".into(), BoardAction::Deploy(DeployMode::Manual))],
                );
                choice_row(
                    p,
                    "Activation",
                    &[
                        ("Alternate".into(), BoardAction::Moves(MoveMode::Interleaved)),
                        ("Whole side".into(), BoardAction::Moves(MoveMode::AllAtOnce)),
                    ],
                );
                p.spawn(row()).with_children(|r| {
                    button_for(r, "New battle", BoardAction::NewBattle, BtnKind::Primary);
                });
            });
            c.spawn(Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(12.0), ..default() })
                .with_children(|g| {
                    g.spawn((
                        BoardImage,
                        ImageNode::new(image.clone()),
                        RelativeCursorPosition::default(),
                        Node {
                            width: Val::Px(IMG_W as f32),
                            max_width: Val::Percent(100.0),
                            aspect_ratio: Some(IMG_W as f32 / IMG_H as f32),
                            flex_shrink: 1.0,
                            margin: UiRect::bottom(Val::Px(12.0)),
                            ..default()
                        },
                    ));
                    g.spawn(panel()).insert(Node {
                        flex_direction: FlexDirection::Column,
                        padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                        border: UiRect::all(Val::Px(1.0)),
                        border_radius: BorderRadius::all(Val::Px(8.0)),
                        row_gap: Val::Px(6.0),
                        width: Val::Px(400.0),
                        flex_grow: 1.0,
                        align_self: AlignSelf::FlexStart,
                        ..default()
                    })
                    .with_children(|p| {
                        for i in 0..STATUS_LINES {
                            p.spawn((BoardText::Status(i), text("", 15.0, TEXT)));
                        }
                        for i in 0..INFO_LINES {
                            p.spawn((BoardText::Info(i), text("", 13.5, TEXT)));
                        }
                        p.spawn(row()).with_children(|r| {
                            for (label, cmd) in [
                                ("↶ Turn (Q)", BattleCommand::TurnLeft),
                                ("↷ Turn (E)", BattleCommand::TurnRight),
                                ("Forward (W)", BattleCommand::Forward),
                                ("Back (S)", BattleCommand::Backward),
                                ("Fire (F)", BattleCommand::FireMode),
                                ("End (Space)", BattleCommand::EndActivation),
                            ] {
                                button_for(r, label, BoardAction::Cmd(cmd), BtnKind::Normal);
                            }
                        });
                        p.spawn(row()).with_children(|r| {
                            button_for(r, "Set flagship", BoardAction::Cmd(BattleCommand::SetFlagship), BtnKind::Normal);
                            button_for(r, "Remove", BoardAction::Cmd(BattleCommand::RemoveDeployed), BtnKind::Normal);
                            button_for(r, "Confirm deployment", BoardAction::Cmd(BattleCommand::ConfirmDeployment), BtnKind::Primary);
                        });
                        p.spawn(row()).with_children(|r| {
                            button_for(r, "Step", BoardAction::Step, BtnKind::Normal);
                            button_for(r, "Auto", BoardAction::Auto, BtnKind::Choice);
                        });
                        p.spawn((BoardText::Hover, text("", 13.0, GREEN)));
                        p.spawn(text("Log", 13.5, GOLD));
                        for i in 0..LOG_LINES {
                            p.spawn((BoardText::Log(i), text("", 12.0, TEXT)));
                        }
                    });
                });
        });
    });
}

fn choice_row(parent: &mut ChildSpawnerCommands, title: &str, choices: &[(String, BoardAction)]) {
    super::ui::choice_row(parent, title, choices);
}

fn on_page(phase: &Phase) -> bool {
    *phase == Phase::Board
}

pub fn handle_board_buttons(
    buttons: Query<(&Interaction, &BoardAction, &Enabled), Changed<Interaction>>,
    mut board: ResMut<HexBoard>,
    mut requests: MessageWriter<super::flow::AppRequest>,
) {
    for (interaction, action, enabled) in &buttons {
        if *interaction != Interaction::Pressed || !enabled.0 {
            continue;
        }
        let b = &mut *board;
        match *action {
            BoardAction::Home => {
                requests.write(super::flow::AppRequest::Home);
            }
            BoardAction::Scenario(i) => {
                b.settings.scenario = i;
                b.restart();
            }
            BoardAction::Control(c) => {
                b.settings.control = c;
                b.restart();
            }
            BoardAction::Size(n) => {
                b.settings.size = n;
                b.restart();
            }
            BoardAction::Deploy(d) => {
                b.settings.deploy_mode = d;
                b.restart();
            }
            BoardAction::Moves(m) => {
                b.settings.move_mode = m;
                b.restart();
            }
            BoardAction::NewBattle => b.restart(),
            BoardAction::Cmd(cmd) => b.command(cmd),
            BoardAction::Step => b.step = true,
            BoardAction::Auto => b.auto = !b.auto,
        }
    }
}

/// Clicks on the board, the hovered hex, and the Q/E/W/S/F/Space keys.
pub fn board_input(
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    image: Query<&RelativeCursorPosition, With<BoardImage>>,
    phase: Res<Phase>,
    mut board: ResMut<HexBoard>,
) {
    if !on_page(&phase) {
        return;
    }
    if matches!(board.phase(), rules::Phase::Deployment | rules::Phase::Combat) {
        for (key, cmd) in [
            (KeyCode::KeyQ, BattleCommand::TurnLeft),
            (KeyCode::KeyE, BattleCommand::TurnRight),
            (KeyCode::KeyW, BattleCommand::Forward),
            (KeyCode::KeyS, BattleCommand::Backward),
            (KeyCode::KeyF, BattleCommand::FireMode),
            (KeyCode::Space, BattleCommand::EndActivation),
        ] {
            if keys.just_pressed(key) {
                board.command(cmd);
            }
        }
    }
    let Ok(cursor) = image.single() else { return };
    let hover = cursor.normalized.filter(|_| cursor.cursor_over()).and_then(|n| {
        let p = Vec2::new((n.x + 0.5) * IMG_W as f32, (n.y + 0.5) * IMG_H as f32);
        let h = board_hexes().min_by(|a, b| px(*a).distance_squared(p).total_cmp(&px(*b).distance_squared(p)))?;
        (px(h).distance(p) <= HS * K).then_some(h)
    });
    board.hover = hover;
    if let Some(h) = hover
        && mouse.just_pressed(MouseButton::Left)
    {
        board.command(BattleCommand::SelectHex(h));
    }
}

pub fn step_board(time: Res<Time>, phase: Res<Phase>, mut board: ResMut<HexBoard>) {
    if on_page(&phase) {
        board.tick(time.delta_secs().min(0.1));
    }
}

/// Redraws the board, the panel text and the button states.
pub fn render_board(
    phase: Res<Phase>,
    mut board: ResMut<HexBoard>,
    mut images: ResMut<Assets<Image>>,
    mut page: Query<&mut Node, With<BoardPage>>,
    mut texts: Query<(&BoardText, &mut Text, &mut TextColor)>,
    mut buttons: Query<(&BoardAction, &mut Selected, &mut Enabled)>,
) {
    let open = on_page(&phase);
    for mut n in &mut page {
        let d = if open { Display::Flex } else { Display::None };
        if n.display != d {
            n.display = d;
        }
    }
    if !open {
        return;
    }
    let b = &mut *board;
    let v = view(b);
    let rgba = render(b, &v).to_rgba();
    if let Some(mut img) = images.get_mut(&b.image) {
        img.data = Some(rgba);
    }
    let hover = hover_line(b);
    let log_start = b.log.len().saturating_sub(LOG_LINES);
    for (slot, mut t, mut color) in &mut texts {
        let (line, col) = match *slot {
            BoardText::Status(i) => v.status.get(i).cloned().unwrap_or_default(),
            BoardText::Info(i) => v.info.get(i).cloned().unwrap_or_default(),
            BoardText::Log(i) => b.log.get(log_start + i).cloned().unwrap_or_default(),
            BoardText::Scenario => (SCENARIOS[b.settings.scenario].note.to_string(), DIM),
            BoardText::Hover => (hover.clone(), GREEN),
        };
        if t.0 != line {
            t.0 = line;
        }
        let col = if col == Color::default() { TEXT } else { col };
        if color.0 != col {
            color.0 = col;
        }
    }
    let spectate = b.spectate();
    let s = b.settings;
    for (action, mut sel, mut enabled) in &mut buttons {
        let (on, en) = match *action {
            BoardAction::Scenario(i) => (s.scenario == i, true),
            BoardAction::Control(c) => (s.control == c, true),
            BoardAction::Size(n) => (s.size == n, true),
            BoardAction::Deploy(d) => (s.deploy_mode == d, true),
            BoardAction::Moves(m) => (s.move_mode == m, true),
            BoardAction::Cmd(cmd) => (false, v.can.contains(&cmd)),
            BoardAction::Step => (false, spectate && !b.auto && b.phase() == rules::Phase::Combat),
            BoardAction::Auto => (b.auto, spectate && b.phase() == rules::Phase::Combat),
            BoardAction::Home | BoardAction::NewBattle => (false, true),
        };
        if sel.0 != on {
            sel.0 = on;
        }
        if enabled.0 != en {
            enabled.0 = en;
        }
    }
}

/// Wheel scrolling for the page.
pub fn scroll_board_page(
    mut wheel: MessageReader<bevy::input::mouse::MouseWheel>,
    phase: Res<Phase>,
    mut page: Query<&mut ScrollPosition, With<BoardPage>>,
) {
    let dy: f32 = wheel
        .read()
        .map(|ev| match ev.unit {
            bevy::input::mouse::MouseScrollUnit::Line => ev.y * 28.0,
            bevy::input::mouse::MouseScrollUnit::Pixel => ev.y,
        })
        .sum();
    if dy == 0.0 || !on_page(&phase) {
        return;
    }
    if let Ok(mut s) = page.single_mut() {
        s.0.y = (s.0.y - dy).max(0.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A spectated battle on the page plays to the end and draws; a
    /// human click selects and a key moves.
    #[test]
    fn page_plays_and_renders() {
        let mut b = HexBoard::new(Handle::default(), 7);
        b.settings.control = ControlMode::Spectate;
        b.restart();
        b.auto = true;
        for _ in 0..20_000 {
            b.tick(0.25);
            if b.phase() == rules::Phase::GameOver {
                break;
            }
        }
        assert_eq!(b.phase(), rules::Phase::GameOver);
        let v = view(&mut b);
        let c = render(&mut b, &v);
        if let Ok(dir) = std::env::var("SG_BOARD_SHOTS") {
            std::fs::write(format!("{dir}/board-over.ppm"), c.to_ppm()).unwrap();
        }
        assert!(b.log.iter().any(|(l, _)| l.contains("BATTLE OVER")));

        let mut b = HexBoard::new(Handle::default(), 9);
        let ships = snapshot(&mut b.world);
        // Blue moves first on turn 1 in alternate mode only if it is Blue's go;
        // tick until a human pick is pending.
        for _ in 0..40 {
            b.tick(0.3);
            if matches!(b.world.resource::<Activation>().0, Some(a) if a.unit.is_none()) {
                break;
            }
        }
        let mine = alive(&ships).find(|s| s.side == 0).unwrap().hex;
        b.command(BattleCommand::SelectHex(mine));
        assert!(b.world.resource::<Activation>().0.and_then(|a| a.unit).is_some());
        b.command(BattleCommand::Forward);
        let v = view(&mut b);
        let c = render(&mut b, &v);
        if let Ok(dir) = std::env::var("SG_BOARD_SHOTS") {
            std::fs::write(format!("{dir}/board-turn.ppm"), c.to_ppm()).unwrap();
        }
    }
}
