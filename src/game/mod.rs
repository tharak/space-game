//! The Bevy app layer: fleet view, camera, orders UI and input on top of
//! the headless [`crate::sim`].

#[cfg(not(target_arch = "wasm32"))]
pub mod autoplay;
pub mod board_page;
pub mod camera;
pub mod campaign_ui;
pub mod ecs_lab;
pub mod flow;
pub mod input;
pub mod lab_ui;
pub mod model;
pub mod theme;
pub mod ui;
pub mod view;

use bevy::asset::AssetId;
use bevy::prelude::*;

use crate::sim::Command;
use crate::sim::battle::{Battle, DT, SimEvent, SimSettings, Tuning};
use crate::sim::rng::Rng;
use crate::sim::spatial::SpatialHash;
use crate::sim::systems::build_schedule;

/// Noto Sans Mono, subset to Latin + arrows/box symbols (SIL OFL 1.1).
const FONT: &[u8] = include_bytes!("../../assets/fonts/NotoSansMono-subset.ttf");

pub struct GamePlugin;

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
enum Stage {
    Input,
    Apply,
    Model,
    Present,
}

impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        // Replace Bevy's ASCII-only default font with one that has — → ▶ etc.
        let _ = app
            .world_mut()
            .resource_mut::<Assets<Font>>()
            .insert(AssetId::default(), Font::from_bytes(FONT.to_vec()));

        app.insert_resource(ClearColor(theme::BG))
            .insert_resource(Time::<Fixed>::from_seconds(DT as f64))
            .insert_resource(flow::SeedSource(random_seed()))
            .init_resource::<SimSettings>()
            .init_resource::<Tuning>()
            .init_resource::<Battle>()
            .init_resource::<SpatialHash>()
            .insert_resource(Rng::seeded(1))
            .init_resource::<flow::Phase>()
            .init_resource::<flow::Speed>()
            .init_resource::<flow::LogBook>()
            .init_resource::<flow::OverlayHidden>()
            .init_resource::<model::UiModel>()
            .init_resource::<camera::CameraRig>()
            .init_resource::<ui::SkirmishSetup>()
            .init_resource::<lab_ui::LabsConfig>()
            .init_resource::<lab_ui::ActiveSlider>()
            .init_resource::<input::Selection>()
            .init_resource::<input::MoveDrag>()
            .init_resource::<view::Fx>()
            .init_resource::<ui::CribOpen>()
            .init_resource::<campaign_ui::CampaignState>()
            .insert_resource(ui::SaveExists(crate::campaign::save::load().is_some()))
            .add_message::<SimEvent>()
            .add_message::<Command>()
            .add_message::<flow::AppRequest>()
            .add_schedule(build_schedule())
            .init_gizmo_group::<view::ThinGizmos>()
            .configure_sets(Update, (Stage::Input, Stage::Apply, Stage::Model, Stage::Present).chain())
            .add_systems(
                Startup,
                (camera::setup_camera, view::setup_view, ui::setup_ui, lab_ui::setup_labs, ecs_lab::setup_ecs_labs, board_page::setup_board_page, ui::setup_campaign_ui, campaign_ui::setup_campaign_map),
            )
            .add_systems(FixedUpdate, flow::run_sim)
            .add_systems(
                Update,
                (
                    (ui::handle_buttons, input::keyboard, input::pointer, campaign_ui::campaign_pointer, ui::scroll).in_set(Stage::Input),
                    (flow::apply_requests, campaign_ui::process_turn, campaign_ui::tick_resolving).chain().in_set(Stage::Apply),
                    model::build_model.in_set(Stage::Model),
                    (lab_ui::handle_lab_buttons, lab_ui::drag_sliders, lab_ui::lab_pointer, lab_ui::proto_pointer, lab_ui::scroll_labs).chain().in_set(Stage::Input),
                    (lab_ui::animate_labs, lab_ui::step_proto).in_set(Stage::Apply),
                    (ecs_lab::handle_ecs_buttons, ecs_lab::scroll_ecs_labs).in_set(Stage::Input),
                    ecs_lab::step_ecs_labs.in_set(Stage::Apply),
                    (board_page::handle_board_buttons, board_page::board_input, board_page::scroll_board_page).chain().in_set(Stage::Input),
                    board_page::step_board.in_set(Stage::Apply),
                    board_page::render_board.in_set(Stage::Present).before(ui::style_buttons),
                    (ecs_lab::render_ecs_labs, ecs_lab::ecs_button_state).chain().in_set(Stage::Present).before(ui::style_buttons),
                    (lab_ui::render_labs, lab_ui::update_lab_ui, lab_ui::lab_button_state).chain().in_set(Stage::Present).before(ui::style_buttons),
                    (
                        flow::log_events,
                        view::collect_fx,
                        ui::sync_log,
                        ui::update_texts,
                        ui::update_layout,
                        ui::update_button_state,
                        ui::style_buttons,
                        camera::apply_camera,
                        view::attach_ship_visuals,
                        view::sync_ships,
                        view::sync_labels,
                        view::draw_fx,
                        view::draw_overlays,
                        campaign_ui::sync_campaign_visuals,
                        campaign_ui::draw_campaign,
                    )
                        .chain()
                        .in_set(Stage::Present),
                ),
            );
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(script) = autoplay::Autoplay::from_env() {
            app.insert_resource(script).add_systems(Update, autoplay::autoplay.in_set(Stage::Input));
        }
    }
}

/// A little entropy for battle seeds: wall-clock time where available.
pub(crate) fn random_seed() -> u32 {
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
