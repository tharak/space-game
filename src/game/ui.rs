//! Bevy UI: the menu, the side panel (status, division list, orders,
//! speed, log, rules) and the result overlay. Widgets read `UiModel`;
//! button presses become orders, speed changes or settings.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;

use super::camera::{PANEL_W, is_portrait, panel_height};
use super::flow::{AppRequest, LogBook, OverlayHidden, Phase, Speed, copy_to_clipboard};
use super::input::{PointerMode, Selection};
use super::model::{Spans, UiModel};
use super::theme::*;
use crate::sim::Command;
use crate::sim::battle::{AiKind, Control, SimSettings};
use crate::sim::components::Order;
use crate::sim::formation::Formation;
use crate::sim::scenario::{SCENARIOS, Scenario};

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OrderKind {
    Hold,
    Advance,
    Charge,
    Withdraw,
}

#[derive(Component, Clone, Copy, PartialEq, Debug)]
pub enum UiAction {
    Menu,
    Restart,
    Copy,
    CloseOverlay,
    ToggleCrib,
    Control(Control),
    Ships(usize),
    Ai(AiKind),
    Scenario(usize),
    Pause,
    Speed(u32),
    Division(usize, usize),
    Order(OrderKind),
    MoveMode,
    Formation(Formation),
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    Normal,
    Primary,
    Danger,
    Choice,
    Scenario,
    List,
}

#[derive(Component)]
pub struct Enabled(pub bool);

#[derive(Component)]
pub struct Selected(pub bool);

/// The text entities inside a button (dimmed when disabled).
#[derive(Component)]
pub struct BtnLabel(Vec<Entity>);

#[derive(Component)]
/// A column of lines, each made of coloured spans.
pub struct RichText {
    lines: Vec<Spans>,
    size: f32,
}

impl RichText {
    fn sized(size: f32) -> Self {
        Self { lines: Vec::new(), size }
    }
}

#[derive(Component)]
pub struct MenuScreen;
#[derive(Component)]
pub struct SidePanel;
#[derive(Component)]
pub struct StatusText;
#[derive(Component)]
pub struct InfoText;
#[derive(Component)]
pub struct LogView;
/// Which part of the screen a UI node is; drives show/hide and layout.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    Menu,
    Panel,
    Overlay,
    Orders,
    Crib,
    Log,
}

#[derive(Component, Clone, Copy)]
pub enum OverlayText {
    Title,
    Body,
    Line,
}

#[derive(Resource, Default)]
pub struct CribOpen(pub bool);

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

fn text(s: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (Text::new(s), TextFont::from_font_size(size), TextColor(color))
}

fn panel() -> impl Bundle {
    (
        Node {
            flex_direction: FlexDirection::Column,
            padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
            margin: UiRect::bottom(Val::Px(10.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            row_gap: Val::Px(6.0),
            flex_shrink: 0.0,
            ..default()
        },
        BackgroundColor(PANEL),
        BorderColor::all(LINE),
    )
}

fn row() -> Node {
    Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, column_gap: Val::Px(6.0), row_gap: Val::Px(6.0), ..default() }
}

fn button(parent: &mut ChildSpawnerCommands, label: &str, action: UiAction, kind: BtnKind) -> Entity {
    let mut labels = Vec::new();
    let mut entity = parent.spawn((
        Button,
        Node {
            padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(PANEL2),
        BorderColor::all(LINE),
        action,
        kind,
        Enabled(true),
        Selected(false),
    ));
    entity.with_children(|b| labels.push(b.spawn(text(label, 14.0, TEXT)).id()));
    entity.insert(BtnLabel(labels));
    entity.id()
}

/// A full-width menu button with a gold title and a dim note.
fn scenario_button(parent: &mut ChildSpawnerCommands, index: usize, s: &Scenario) {
    let mut labels = Vec::new();
    let mut entity = parent.spawn((
        Button,
        Node {
            flex_direction: FlexDirection::Column,
            width: Val::Percent(100.0),
            padding: UiRect::axes(Val::Px(14.0), Val::Px(10.0)),
            margin: UiRect::vertical(Val::Px(4.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(6.0)),
            row_gap: Val::Px(2.0),
            ..default()
        },
        BackgroundColor(PANEL2),
        BorderColor::all(LINE),
        UiAction::Scenario(index),
        BtnKind::Scenario,
        Enabled(true),
        Selected(false),
    ));
    entity.with_children(|b| {
        labels.push(b.spawn(text(format!("{}. {}", index + 1, s.title), 15.0, GOLD)).id());
        labels.push(b.spawn(text(format!("{} vs {}", s.fleets[0].name(), s.fleets[1].name()), 13.0, TEXT)).id());
        labels.push(b.spawn(text(s.blurb, 13.0, DIM)).id());
    });
    entity.insert(BtnLabel(labels));
}

/// A full-width, left-aligned row button for the division list.
fn list_button(parent: &mut ChildSpawnerCommands, action: UiAction) {
    let mut labels = Vec::new();
    let mut entity = parent.spawn((
        Button,
        Node {
            width: Val::Percent(100.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(3.0)),
            border: UiRect::all(Val::Px(1.0)),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(PANEL2),
        BorderColor::all(LINE),
        action,
        BtnKind::List,
        Enabled(true),
        Selected(false),
    ));
    entity.with_children(|b| labels.push(b.spawn(text("", 13.0, TEXT)).id()));
    entity.insert(BtnLabel(labels));
}

fn option_group(parent: &mut ChildSpawnerCommands, title: &str, choices: &[(&str, UiAction)]) {
    parent.spawn(panel()).with_children(|p| {
        p.spawn(row()).with_children(|r| {
            r.spawn((text(title, 14.0, GOLD), Node { margin: UiRect::right(Val::Px(6.0)), align_self: AlignSelf::Center, ..default() }));
            for (label, action) in choices {
                button(r, label, *action, BtnKind::Choice);
            }
        });
    });
}

pub fn setup_ui(mut commands: Commands) {
    // --- Menu ---
    commands
        .spawn((
            MenuScreen,
            Section::Menu,
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
                c.spawn(text("Successor Stars — fleet battle", 22.0, TEXT));
                c.spawn(text(
                    "Hundreds of ships a side, in divisions you command in real time (pause any time). \
                     Formation is everything: a deep arrow can tear a hole in a thin line, a crescent can close on an \
                     arrow's flanks, and shots into a ship's flank or rear hit far more often than shots into its front.",
                    14.0,
                    DIM,
                ));
                option_group(c, "Control:", &[
                    ("Command Blue (left)", UiAction::Control(Control::PlayBlue)),
                    ("Command Red (right)", UiAction::Control(Control::PlayRed)),
                    ("Spectate (AI vs AI)", UiAction::Control(Control::Spectate)),
                ]);
                option_group(c, "Fleet size:", &[
                    ("120 ships a side", UiAction::Ships(120)),
                    ("240 ships a side", UiAction::Ships(240)),
                    ("480 ships a side", UiAction::Ships(480)),
                ]);
                option_group(c, "Opponent:", &[
                    ("Commander — pins, flanks, keeps a reserve", UiAction::Ai(AiKind::Commander)),
                    ("Formation only — opening orders, never adapts", UiAction::Ai(AiKind::Advance)),
                ]);
                for (i, s) in SCENARIOS.iter().enumerate() {
                    scenario_button(c, i, s);
                }
            });
        });

    // --- Side panel ---
    commands
        .spawn((
            SidePanel,
            Section::Panel,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                width: Val::Px(PANEL_W),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(10.0)),
                overflow: Overflow::scroll_y(),
                ..default()
            },
            BackgroundColor(BG.with_alpha(0.94)),
            ScrollPosition::default(),
            RelativeCursorPosition::default(),
            GlobalZIndex(5),
        ))
        .with_children(|s| {
            s.spawn(panel()).with_children(|p| {
                p.spawn((StatusText, RichText::sized(14.0), Node { flex_direction: FlexDirection::Column, ..default() }));
                p.spawn(row()).with_children(|r| {
                    button(r, "Pause", UiAction::Pause, BtnKind::Primary);
                    for (label, mult) in [("1×", 1), ("2×", 2), ("4×", 4), ("8×", 8)] {
                        button(r, label, UiAction::Speed(mult), BtnKind::Choice);
                    }
                });
            });
            s.spawn(panel()).with_children(|p| {
                p.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(3.0), ..default() }).with_children(|l| {
                    for side in 0..2 {
                        for number in 1..=6 {
                            list_button(l, UiAction::Division(side, number));
                        }
                    }
                });
            });
            s.spawn(panel()).with_children(|p| {
                p.spawn((
                    InfoText,
                    RichText::sized(13.5),
                    Node { flex_direction: FlexDirection::Column, min_height: Val::Px(36.0), row_gap: Val::Px(2.0), ..default() },
                ));
                p.spawn((Section::Orders, Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(6.0), ..default() }))
                    .with_children(|o| {
                        o.spawn(row()).with_children(|r| {
                            button(r, "Move…", UiAction::MoveMode, BtnKind::Primary);
                            button(r, "Hold", UiAction::Order(OrderKind::Hold), BtnKind::Normal);
                            button(r, "Advance", UiAction::Order(OrderKind::Advance), BtnKind::Normal);
                            button(r, "Charge", UiAction::Order(OrderKind::Charge), BtnKind::Danger);
                            button(r, "Withdraw", UiAction::Order(OrderKind::Withdraw), BtnKind::Normal);
                        });
                        o.spawn(row()).with_children(|r| {
                            for f in Formation::PLAYER {
                                button(r, f.name(), UiAction::Formation(f), BtnKind::Choice);
                            }
                        });
                        o.spawn(text(
                            "Right-click: move there (drag to set facing) · click an enemy: engage · drag: pan · \
                             wheel: zoom · Space: pause · 1–4: speed · Tab: next division · H/G/C/X: hold/advance/charge/withdraw · F: formation",
                            11.5,
                            DIM,
                        ));
                    });
            });
            s.spawn(panel()).with_children(|p| {
                p.spawn((
                    LogView,
                    Section::Log,
                    Node {
                        flex_direction: FlexDirection::Column,
                        height: Val::Px(220.0),
                        overflow: Overflow::scroll_y(),
                        ..default()
                    },
                    ScrollPosition::default(),
                    RelativeCursorPosition::default(),
                ));
            });
            s.spawn(panel()).with_children(|p| {
                p.spawn(row()).with_children(|r| {
                    button(r, "Menu", UiAction::Menu, BtnKind::Normal);
                    button(r, "Restart", UiAction::Restart, BtnKind::Normal);
                    button(r, "Rules ▸", UiAction::ToggleCrib, BtnKind::Normal);
                });
                p.spawn((
                    Section::Crib,
                    Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), display: Display::None, ..default() },
                ))
                .with_children(|c| {
                    for (line, color) in [
                        ("Hit chance by the target's arc you fire into:", GOLD),
                        ("  front 12% · flank 25% · rear 40%  (less at long range)", TEXT),
                        ("Guns fire in a 60° cone ahead. Ships can only turn a little away from their division's facing, so a division hit from behind must wheel to answer.", DIM),
                        ("Enemy ships block movement: to get through a line you must destroy the ships in the way. Deep formations (wedge, column) close ranks as they lose ships; a line keeps its holes.", DIM),
                        ("Morale: recent losses, total losses, a lost flagship and being broken through push a division to Shaken (slower fire, won't advance) and then Routed (flees, counts as lost). Staying within the flagship's command radius steadies it.", DIM),
                        ("A fleet breaks when more than half its ships are destroyed, fled or routed. After 10 minutes the fleet with more hull in the fight wins.", DIM),
                    ] {
                        c.spawn(text(line, 12.5, color));
                    }
                });
            });
        });

    // --- Result overlay ---
    commands
        .spawn((
            Section::Overlay,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                padding: UiRect::all(Val::Px(16.0)),
                display: Display::None,
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.62)),
            GlobalZIndex(20),
        ))
        .with_children(|o| {
            o.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    max_width: Val::Px(560.0),
                    padding: UiRect::axes(Val::Px(30.0), Val::Px(24.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(10.0)),
                    row_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(PANEL),
                BorderColor::all(GOLD),
            ))
            .with_children(|card| {
                card.spawn((OverlayText::Title, text("", 24.0, GOLD)));
                card.spawn((OverlayText::Body, text("", 14.0, TEXT)));
                card.spawn((OverlayText::Line, text("", 12.0, DIM)));
                card.spawn(row()).with_children(|r| {
                    button(r, "Copy result line", UiAction::Copy, BtnKind::Normal);
                    button(r, "Play again", UiAction::Restart, BtnKind::Primary);
                    button(r, "View battlefield", UiAction::CloseOverlay, BtnKind::Normal);
                    button(r, "Menu", UiAction::Menu, BtnKind::Normal);
                });
            });
        });
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub fn handle_buttons(
    buttons: Query<(&Interaction, &UiAction, &Enabled), Changed<Interaction>>,
    mut settings: ResMut<SimSettings>,
    mut requests: MessageWriter<AppRequest>,
    mut orders: MessageWriter<Command>,
    mut speed: ResMut<Speed>,
    mut sel: ResMut<Selection>,
    mut crib: ResMut<CribOpen>,
    mut overlay_hidden: ResMut<OverlayHidden>,
    model: Res<UiModel>,
) {
    for (interaction, action, enabled) in &buttons {
        if *interaction != Interaction::Pressed || !enabled.0 {
            continue;
        }
        let me = sel.division.filter(|_| model.can_order);
        match *action {
            UiAction::Menu => {
                requests.write(AppRequest::Menu);
            }
            UiAction::Restart => {
                requests.write(AppRequest::Start);
            }
            UiAction::Copy => {
                if let Some(r) = &model.result {
                    copy_to_clipboard(&r.line);
                }
            }
            UiAction::CloseOverlay => overlay_hidden.0 = true,
            UiAction::ToggleCrib => crib.0 = !crib.0,
            UiAction::Control(c) => settings.control = c,
            UiAction::Ships(n) => settings.ships = n,
            UiAction::Ai(a) => settings.ai = [a; 2],
            UiAction::Scenario(i) => {
                settings.scenario = i;
                requests.write(AppRequest::Start);
            }
            UiAction::Pause => speed.paused = !speed.paused,
            UiAction::Speed(m) => {
                speed.mult = m;
                speed.paused = false;
            }
            UiAction::Division(side, number) => {
                if let Some(row) = model.divisions.iter().find(|r| r.side == side && r.number == number) {
                    sel.division = Some(row.entity);
                    sel.mode = PointerMode::Normal;
                }
            }
            UiAction::MoveMode => {
                if me.is_some() {
                    sel.mode = if sel.mode == PointerMode::Move { PointerMode::Normal } else { PointerMode::Move };
                }
            }
            UiAction::Order(kind) => {
                if let Some(division) = me {
                    let order = match kind {
                        OrderKind::Hold => Order::Hold,
                        OrderKind::Advance => Order::Advance,
                        OrderKind::Charge => Order::Charge,
                        OrderKind::Withdraw => Order::Withdraw,
                    };
                    orders.write(Command::Order { division, order });
                }
            }
            UiAction::Formation(formation) => {
                if let Some(division) = me {
                    orders.write(Command::Formation { division, formation });
                }
            }
        }
    }
}

fn set_label(texts: &mut Query<(&mut Text, &mut TextColor)>, labels: &BtnLabel, s: &str, color: Option<Color>) {
    if let Some(t) = labels.0.first()
        && let Ok((mut text, mut c)) = texts.get_mut(*t)
    {
        if text.0 != s {
            text.0 = s.to_string();
        }
        if let Some(color) = color
            && c.0 != color
        {
            c.0 = color;
        }
    }
}

/// Which buttons are enabled/selected/visible right now, and their labels.
pub fn update_button_state(
    model: Res<UiModel>,
    settings: Res<SimSettings>,
    crib: Res<CribOpen>,
    mut buttons: Query<(&UiAction, &mut Enabled, &mut Selected, &mut Node, &BtnLabel)>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
) {
    for (action, mut enabled, mut selected, mut node, labels) in &mut buttons {
        let (on, sel, visible) = match *action {
            UiAction::Pause => {
                set_label(&mut texts, labels, if model.paused { "▶ Resume" } else { "Pause" }, None);
                (model.phase == Phase::Battle, model.paused, true)
            }
            UiAction::Speed(m) => (model.phase == Phase::Battle, model.mult == m, true),
            UiAction::ToggleCrib => {
                set_label(&mut texts, labels, if crib.0 { "Rules ▾" } else { "Rules ▸" }, None);
                (true, false, true)
            }
            UiAction::Division(side, number) => match model.divisions.iter().find(|r| r.side == side && r.number == number) {
                Some(row) => {
                    set_label(&mut texts, labels, &row.text, Some(row.color));
                    (!row.gone, row.selected, true)
                }
                None => (false, false, false),
            },
            UiAction::MoveMode => (model.can_order, model.move_mode, true),
            UiAction::Order(_) => (model.can_order, false, true),
            UiAction::Formation(f) => (model.can_order, model.formation == Some(f), true),
            UiAction::Copy => (model.result.is_some(), false, true),
            UiAction::Control(v) => (true, settings.control == v, true),
            UiAction::Ships(v) => (true, settings.ships == v, true),
            UiAction::Ai(v) => (true, settings.ai[0] == v, true),
            _ => (true, false, true),
        };
        if enabled.0 != on {
            enabled.0 = on;
        }
        if selected.0 != sel {
            selected.0 = sel;
        }
        let display = if visible { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
    }
}

type ButtonLook<'a> = (
    &'a Interaction,
    &'a BtnKind,
    &'a Enabled,
    &'a Selected,
    &'a BtnLabel,
    &'a mut BackgroundColor,
    &'a mut BorderColor,
);

pub fn style_buttons(mut buttons: Query<ButtonLook>, mut colors: Query<&mut TextColor>, mut originals: Local<std::collections::HashMap<Entity, Color>>) {
    for (interaction, kind, enabled, selected, labels, mut bg, mut border) in &mut buttons {
        let hovered = enabled.0 && matches!(interaction, Interaction::Hovered | Interaction::Pressed);
        let (mut b, mut line) = match kind {
            BtnKind::Primary => (PRIMARY_BG, BLUE),
            BtnKind::Danger => (PANEL2, RED),
            BtnKind::List => (PANEL, PANEL),
            _ => (PANEL2, LINE),
        };
        if selected.0 {
            b = PRIMARY_BG;
            line = BLUE;
        }
        if hovered {
            line = BLUE;
        }
        if !enabled.0 {
            b = b.mix(&BG, 0.6);
            line = line.mix(&BG, 0.6);
        }
        if bg.0 != b {
            bg.0 = b;
        }
        let want = BorderColor::all(line);
        if *border != want {
            *border = want;
        }
        for t in &labels.0 {
            if let Ok(mut c) = colors.get_mut(*t) {
                let original = *originals.entry(*t).or_insert(c.0);
                let want = if enabled.0 { original } else { original.mix(&BG, 0.6) };
                if c.0 != want {
                    c.0 = want;
                }
            }
        }
    }
}

fn set_rich(commands: &mut Commands, entity: Entity, rich: &mut RichText, lines: &[Spans]) {
    if rich.lines == lines {
        return;
    }
    rich.lines = lines.to_vec();
    let size = rich.size;
    let mut e = commands.entity(entity);
    e.despawn_related::<Children>();
    e.with_children(|p| {
        for spans in lines {
            p.spawn((Text::new(""), TextFont::from_font_size(size), TextColor(TEXT))).with_children(|t| {
                for (s, color) in spans {
                    t.spawn((TextSpan::new(s.clone()), TextFont::from_font_size(size), TextColor(*color)));
                }
            });
        }
    });
}

pub fn update_texts(
    mut commands: Commands,
    model: Res<UiModel>,
    mut status: Query<(Entity, &mut RichText), (With<StatusText>, Without<InfoText>)>,
    mut info: Query<(Entity, &mut RichText), (With<InfoText>, Without<StatusText>)>,
) {
    if let Ok((e, mut r)) = status.single_mut() {
        set_rich(&mut commands, e, &mut r, &model.status);
    }
    if let Ok((e, mut r)) = info.single_mut() {
        set_rich(&mut commands, e, &mut r, &model.info);
    }
}

pub fn update_layout(
    window: Single<&Window, With<PrimaryWindow>>,
    model: Res<UiModel>,
    crib: Res<CribOpen>,
    overlay_hidden: Res<OverlayHidden>,
    mut sections: Query<(&mut Node, &Section)>,
    mut overlay_texts: Query<(&mut Text, &OverlayText)>,
) {
    let win = Vec2::new(window.width(), window.height());
    let portrait = is_portrait(win);
    let in_menu = model.phase == Phase::Menu;
    let over = model.phase == Phase::Over && model.result.is_some() && !overlay_hidden.0;
    for (mut node, section) in &mut sections {
        let visible = match section {
            Section::Menu => in_menu,
            Section::Panel => !in_menu,
            Section::Overlay => over,
            Section::Orders => !model.spectate,
            Section::Crib => crib.0,
            Section::Log => true,
        };
        let display = if visible { Display::Flex } else { Display::None };
        if node.display != display {
            node.display = display;
        }
        match section {
            Section::Panel => {
                let (w, h, top) = if portrait {
                    (Val::Percent(100.0), Val::Px(panel_height(win)), Val::Px(win.y - panel_height(win)))
                } else {
                    (Val::Px(PANEL_W), Val::Percent(100.0), Val::Px(0.0))
                };
                if node.width != w || node.height != h || node.top != top {
                    node.width = w;
                    node.height = h;
                    node.top = top;
                }
            }
            Section::Log => {
                let h = Val::Px(if portrait { 120.0 } else { 220.0 });
                if node.height != h {
                    node.height = h;
                }
            }
            _ => {}
        }
    }
    if let Some(r) = &model.result {
        for (mut t, which) in &mut overlay_texts {
            let want = match which {
                OverlayText::Title => &r.title,
                OverlayText::Body => &r.body,
                OverlayText::Line => &r.line,
            };
            if t.0 != *want {
                t.0 = want.clone();
            }
        }
    }
}

/// Appends new log lines and keeps the log scrolled to the bottom.
pub fn sync_log(
    mut commands: Commands,
    log: Res<LogBook>,
    view: Single<(Entity, Option<&Children>, &mut ScrollPosition), With<LogView>>,
    mut shown: Local<usize>,
    mut generation: Local<u64>,
) {
    if !log.is_changed() {
        return;
    }
    let (entity, children, mut scroll) = view.into_inner();
    if *generation != log.generation {
        *generation = log.generation;
        commands.entity(entity).despawn_related::<Children>();
        *shown = 0;
    }
    let _ = children;
    if log.lines.len() > *shown {
        commands.entity(entity).with_children(|p| {
            for (line, tone) in &log.lines[*shown..] {
                p.spawn((text(line.clone(), 12.5, tone_color(*tone)), Node { flex_shrink: 0.0, ..default() }));
            }
        });
        *shown = log.lines.len();
        scroll.0.y = 1.0e7;
    }
}

/// Mouse-wheel and touch-drag scrolling for the menu, panel and log.
pub fn scroll(
    mut wheel: MessageReader<MouseWheel>,
    touches: Res<Touches>,
    model: Res<UiModel>,
    mut menu: Query<&mut ScrollPosition, (With<MenuScreen>, Without<SidePanel>, Without<LogView>)>,
    mut panel: Query<(&mut ScrollPosition, &RelativeCursorPosition), (With<SidePanel>, Without<MenuScreen>, Without<LogView>)>,
    mut log: Query<(&mut ScrollPosition, &RelativeCursorPosition), (With<LogView>, Without<MenuScreen>, Without<SidePanel>)>,
) {
    let mut dy = 0.0;
    for ev in wheel.read() {
        dy += match ev.unit {
            MouseScrollUnit::Line => ev.y * 28.0,
            MouseScrollUnit::Pixel => ev.y,
        };
    }
    let touch_dy: f32 = touches.iter().map(|t| t.delta().y).sum();
    if model.phase == Phase::Menu {
        if let Ok(mut s) = menu.single_mut() {
            s.0.y = (s.0.y - dy - touch_dy).max(0.0);
        }
        return;
    }
    if dy != 0.0 {
        if let Ok((mut s, cursor)) = log.single_mut()
            && cursor.cursor_over()
        {
            s.0.y = (s.0.y - dy).max(0.0);
            return;
        }
        if let Ok((mut s, cursor)) = panel.single_mut()
            && cursor.cursor_over()
        {
            s.0.y = (s.0.y - dy).max(0.0);
        }
    }
    if touch_dy != 0.0
        && let Ok((mut s, cursor)) = panel.single_mut()
        && cursor.cursor_over()
    {
        s.0.y = (s.0.y - touch_dy).max(0.0);
    }
}
