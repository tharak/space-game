//! Bevy UI: scenario menu, side panel (status, controls, log, crib sheet)
//! and the result overlay. Every widget reads from `UiModel`; button
//! presses become `BattleCommand`s or settings changes.

use bevy::input::mouse::{MouseScrollUnit, MouseWheel};
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::ui::RelativeCursorPosition;
use bevy::window::PrimaryWindow;

use super::board::{PANEL_W, is_portrait, panel_height};
use super::flow::{AiPacing, LogBook, OverlayHidden, copy_to_clipboard};
use super::model::{Spans, UiModel};
use super::theme::*;
use crate::rules::battle::{MatchSettings, Phase};
use crate::rules::commands::BattleCommand;
use crate::rules::config::*;

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum UiAction {
    Cmd(BattleCommand),
    Menu,
    Restart,
    Step,
    Auto,
    Copy,
    CloseOverlay,
    ToggleCrib,
    Control(ControlMode),
    Size(usize),
    Move(MoveMode),
    Deploy(DeployMode),
    Ai(AiKind),
    Scenario(usize),
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub enum BtnKind {
    Normal,
    Primary,
    Danger,
    Choice,
    Scenario,
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
    Controls,
    SetupControls,
    KbdHint,
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
        labels.push(b.spawn(text(s.note, 13.0, DIM)).id());
    });
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
                c.spawn(text("Successor Stars — battle rules prototype  v0.2.1", 22.0, TEXT));
                c.spawn(text(
                    "Rust + Bevy ECS rebuild of the paper rules (docs/battle-rules.md). Zero formation modifiers — \
                     formations are only positions and facings. Pick your options, then a scenario.",
                    14.0,
                    DIM,
                ));
                option_group(c, "Control:", &[
                    ("Play Blue (left)", UiAction::Control(ControlMode::PlayBlue)),
                    ("Play Red (right)", UiAction::Control(ControlMode::PlayRed)),
                    ("Hotseat (both)", UiAction::Control(ControlMode::Hotseat)),
                    ("Spectate (AI vs AI)", UiAction::Control(ControlMode::Spectate)),
                ]);
                option_group(c, "Fleet size:", &[
                    ("5 squadrons (breaks at 3)", UiAction::Size(5)),
                    ("9 squadrons (breaks at 5)", UiAction::Size(9)),
                    ("12 squadrons (breaks at 7)", UiAction::Size(12)),
                ]);
                option_group(c, "Movement:", &[
                    ("Interleaved (alternate squadrons)", UiAction::Move(MoveMode::Interleaved)),
                    ("All-at-once (whole side, you first)", UiAction::Move(MoveMode::AllAtOnce)),
                ]);
                option_group(c, "Deployment:", &[
                    ("Manual placement", UiAction::Deploy(DeployMode::Manual)),
                    ("Battle formation (scenario's fixed setup)", UiAction::Deploy(DeployMode::Fixed)),
                ]);
                option_group(c, "Opponent AI:", &[
                    ("Doctrine — guards its flagship, hunts flanks", UiAction::Ai(AiKind::Doctrine)),
                    ("Baseline — the sim's advance-and-shoot", UiAction::Ai(AiKind::Baseline)),
                ]);
                c.spawn(text(
                    "Manual placement: each side you control places its squadrons in its shaded zone; the AI then \
                     picks a random formation. Battle formation: both sides start in the scenario's exact setup — use \
                     it to test your play against the benchmarked matchup. Spectate always uses the fixed setup.",
                    12.5,
                    DIM,
                ));
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
            BackgroundColor(BG),
            ScrollPosition::default(),
            RelativeCursorPosition::default(),
        ))
        .with_children(|s| {
            s.spawn(panel()).with_children(|p| {
                p.spawn((StatusText, RichText::sized(15.0), Node { flex_direction: FlexDirection::Column, ..default() }));
            });
            s.spawn(panel()).with_children(|p| {
                p.spawn((
                    InfoText,
                    RichText::sized(13.5),
                    Node { flex_direction: FlexDirection::Column, min_height: Val::Px(40.0), row_gap: Val::Px(2.0), ..default() },
                ));
                p.spawn((Section::Controls, row())).with_children(|r| {
                    button(r, "Turn left", UiAction::Cmd(BattleCommand::TurnLeft), BtnKind::Normal);
                    button(r, "↑ Forward", UiAction::Cmd(BattleCommand::Forward), BtnKind::Normal);
                    button(r, "Turn right", UiAction::Cmd(BattleCommand::TurnRight), BtnKind::Normal);
                    button(r, "↓ Back (3 MP)", UiAction::Cmd(BattleCommand::Backward), BtnKind::Normal);
                    button(r, "Fire", UiAction::Cmd(BattleCommand::FireMode), BtnKind::Danger);
                    button(r, "End activation", UiAction::Cmd(BattleCommand::EndActivation), BtnKind::Primary);
                });
                p.spawn((Section::SetupControls, row())).with_children(|r| {
                    button(r, "Turn left", UiAction::Cmd(BattleCommand::TurnLeft), BtnKind::Normal);
                    button(r, "Turn right", UiAction::Cmd(BattleCommand::TurnRight), BtnKind::Normal);
                    button(r, "Set flagship", UiAction::Cmd(BattleCommand::SetFlagship), BtnKind::Normal);
                    button(r, "Remove", UiAction::Cmd(BattleCommand::RemoveDeployed), BtnKind::Danger);
                    button(r, "Confirm deployment", UiAction::Cmd(BattleCommand::ConfirmDeployment), BtnKind::Primary);
                });
                p.spawn((
                    Section::KbdHint,
                    text("Q/E turn · W forward · S back · F fire · Space end · click a unit to select, a red-outlined enemy to shoot", 12.0, DIM),
                ));
            });
            s.spawn(panel()).with_children(|p| {
                p.spawn((
                    LogView,
                    Section::Log,
                    Node {
                        flex_direction: FlexDirection::Column,
                        height: Val::Px(300.0),
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
                    button(r, "Step", UiAction::Step, BtnKind::Normal);
                    button(r, "Auto ▶", UiAction::Auto, BtnKind::Normal);
                });
            });
            s.spawn(panel()).with_children(|p| {
                button(p, "Rules crib sheet ▸", UiAction::ToggleCrib, BtnKind::Normal);
                p.spawn((
                    Section::Crib,
                    Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(4.0), display: Display::None, ..default() },
                ))
                .with_children(|c| {
                    c.spawn(text("Fire into target's…   hit on   (critical supply)", 12.5, GOLD));
                    c.spawn(text("  Front arc            5+       6", 12.5, TEXT));
                    c.spawn(text("  Flank arc            4+       5+", 12.5, TEXT));
                    c.spawn(text("  Rear arc             3+       4+", 12.5, TEXT));
                    for line in [
                        "Dice = Strength (Shaken: half, round up) · range 3 · fire only into your own front arc · LOS blocked by any squadron.",
                        "Move: 3 MP; 1 per hex forward, 1 per 60° turn, or all 3 to back up one hex (facing kept). In command (≤4 of flagship): move AND fire, in either order; outside: move OR fire.",
                        "Morale (d6 ≥ 4) on: taking losses · friend routs/dies within 2 · flagship lost. Mods: +1 steady friend adjacent, +1 in command, −1 hit from flank/rear, −1 low or critical supply, −1 flagship lost. Fail: Steady → Shaken → Routed.",
                        "Supply: low = −1 morale only · critical = −1 morale and the worse to-hit column.",
                        "Fleet breaks at more than half destroyed/fled (3 of 5 · 5 of 9 · 7 of 12). Draw at turn 15.",
                    ] {
                        c.spawn(text(line, 12.5, DIM));
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
                    max_width: Val::Px(540.0),
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
                    button(r, "View board", UiAction::CloseOverlay, BtnKind::Normal);
                    button(r, "Menu", UiAction::Menu, BtnKind::Normal);
                });
            });
        });
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

pub fn handle_buttons(
    buttons: Query<(&Interaction, &UiAction, &Enabled), Changed<Interaction>>,
    mut settings: ResMut<MatchSettings>,
    mut commands: MessageWriter<BattleCommand>,
    mut pacing: ResMut<AiPacing>,
    mut crib: ResMut<CribOpen>,
    mut overlay_hidden: ResMut<OverlayHidden>,
    model: Res<UiModel>,
) {
    for (interaction, action, enabled) in &buttons {
        if *interaction != Interaction::Pressed || !enabled.0 {
            continue;
        }
        match *action {
            UiAction::Cmd(cmd) => {
                commands.write(cmd);
            }
            UiAction::Menu => {
                pacing.auto = false;
                commands.write(BattleCommand::ToMenu);
            }
            UiAction::Restart => {
                commands.write(BattleCommand::Restart);
            }
            UiAction::Step => pacing.step = true,
            UiAction::Auto => pacing.auto = !pacing.auto,
            UiAction::Copy => {
                if let Some(r) = &model.result {
                    copy_to_clipboard(&r.line);
                }
            }
            UiAction::CloseOverlay => overlay_hidden.0 = true,
            UiAction::ToggleCrib => crib.0 = !crib.0,
            UiAction::Control(c) => settings.control = c,
            UiAction::Size(s) => settings.size = s,
            UiAction::Move(m) => settings.move_mode = m,
            UiAction::Deploy(d) => settings.deploy_mode = d,
            UiAction::Ai(a) => settings.ai = [a; 2],
            UiAction::Scenario(i) => {
                settings.scenario = i;
                pacing.auto = false;
                commands.write(BattleCommand::Restart);
            }
        }
    }
}

/// Which buttons are enabled/selected right now.
pub fn update_button_state(
    model: Res<UiModel>,
    settings: Res<MatchSettings>,
    pacing: Res<AiPacing>,
    crib: Res<CribOpen>,
    mut buttons: Query<(&UiAction, &mut Enabled, &mut Selected, &mut Node, &BtnLabel)>,
    mut texts: Query<&mut Text>,
) {
    let c = &model.can;
    let setup = model.phase == Phase::Deployment;
    for (action, mut enabled, mut selected, mut node, labels) in &mut buttons {
        let (on, sel, visible) = match *action {
            UiAction::Cmd(cmd) => (
                match cmd {
                    BattleCommand::TurnLeft | BattleCommand::TurnRight => if setup { c.setup_turn } else { c.turn },
                    BattleCommand::Forward => c.forward,
                    BattleCommand::Backward => c.back,
                    BattleCommand::FireMode => c.fire,
                    BattleCommand::EndActivation => c.end,
                    BattleCommand::SetFlagship => c.setup_flag,
                    BattleCommand::RemoveDeployed => c.setup_remove,
                    BattleCommand::ConfirmDeployment => c.setup_confirm,
                    _ => true,
                },
                false,
                true,
            ),
            UiAction::Step => (model.phase == Phase::Combat && !pacing.auto, false, model.spectate),
            UiAction::Auto => {
                let label = if pacing.auto { "Auto ❚❚" } else { "Auto ▶" };
                if let Some(t) = labels.0.first()
                    && let Ok(mut text) = texts.get_mut(*t)
                    && text.0 != label
                {
                    text.0 = label.to_string();
                }
                (model.phase == Phase::Combat, pacing.auto, model.spectate)
            }
            UiAction::ToggleCrib => {
                let label = if crib.0 { "Rules crib sheet ▾" } else { "Rules crib sheet ▸" };
                if let Some(t) = labels.0.first()
                    && let Ok(mut text) = texts.get_mut(*t)
                    && text.0 != label
                {
                    text.0 = label.to_string();
                }
                (true, false, true)
            }
            UiAction::Copy => (model.result.is_some(), false, true),
            UiAction::Control(v) => (true, settings.control == v, true),
            UiAction::Size(v) => (true, settings.size == v, true),
            UiAction::Move(v) => (true, settings.move_mode == v, true),
            UiAction::Deploy(v) => (true, settings.deploy_mode == v, true),
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
    let setup = model.phase == Phase::Deployment;
    let over = model.phase == Phase::GameOver && model.result.is_some() && !overlay_hidden.0;
    for (mut node, section) in &mut sections {
        let visible = match section {
            Section::Menu => in_menu,
            Section::Panel => !in_menu,
            Section::Overlay => over,
            Section::Controls | Section::KbdHint => !setup,
            Section::SetupControls => setup,
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
                let h = Val::Px(if portrait { 150.0 } else { 300.0 });
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
