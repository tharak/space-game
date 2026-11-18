//! The Bevy app layer: board rendering, UI and input on top of `rules`.

pub mod board;
pub mod flow;
pub mod model;
pub mod theme;
pub mod ui;

use bevy::asset::AssetId;
use bevy::prelude::*;

use crate::rules::battle::{Activation, Battle, BattleEvent, Deployment, MatchSettings, Phase};
use crate::rules::commands::BattleCommand;
use crate::rules::dice::Dice;

/// Noto Sans Mono, subset to Latin + arrows/box symbols (SIL OFL 1.1).
const FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSansMono-subset.ttf");

pub struct GamePlugin;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum Stage {
    Input,
    Rules,
    Model,
    Present,
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        // Replace Bevy's ASCII-only default font with one that has — → ↶ ▶ etc.
        let _ = app
            .world_mut()
            .resource_mut::<Assets<Font>>()
            .insert(AssetId::default(), Font::from_bytes(FONT.to_vec()));

        app.insert_resource(ClearColor(theme::BG))
            .insert_resource(Dice::seeded(random_seed() ^ 0x9e37_79b9))
            .init_resource::<MatchSettings>()
            .init_resource::<Phase>()
            .init_resource::<Battle>()
            .init_resource::<Activation>()
            .init_resource::<Deployment>()
            .init_resource::<model::UiModel>()
            .init_resource::<board::BoardView>()
            .init_resource::<flow::AiPacing>()
            .init_resource::<flow::LogBook>()
            .init_resource::<flow::OverlayHidden>()
            .init_resource::<ui::CribOpen>()
            .add_message::<BattleCommand>()
            .add_message::<BattleEvent>()
            .init_gizmo_group::<board::HaloGizmos>()
            .init_gizmo_group::<board::ThinGizmos>()
            .configure_sets(Update, (Stage::Input, Stage::Rules, Stage::Model, Stage::Present).chain())
            .add_systems(Startup, (board::setup_board, ui::setup_ui))
            .add_systems(
                Update,
                (
                    (ui::handle_buttons, flow::keyboard, board::board_input, ui::scroll).in_set(Stage::Input),
                    flow::run_battle.in_set(Stage::Rules),
                    model::build_model.in_set(Stage::Model),
                    (
                        flow::log_events,
                        ui::sync_log,
                        ui::update_texts,
                        ui::update_layout,
                        ui::update_button_state,
                        ui::style_buttons,
                        board::fit_camera,
                        board::attach_ship_visuals,
                        board::sync_ships,
                        board::sync_setup_ghosts,
                        board::paint_tiles,
                        board::spawn_lasers,
                        board::draw_lasers,
                        board::draw_outlines,
                    )
                        .chain()
                        .in_set(Stage::Present),
                ),
            );
    }
}

/// A little entropy for the dice: wall-clock nanoseconds where available.
fn random_seed() -> u32 {
    #[cfg(target_arch = "wasm32")]
    {
        (js_now() * 1000.0) as u64 as u32
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos() ^ d.as_secs() as u32).unwrap_or(1)
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = "export function js_now() { return Date.now() + performance.now(); }")]
extern "C" {
    fn js_now() -> f64;
}
