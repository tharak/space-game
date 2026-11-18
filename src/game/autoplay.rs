//! Scripted smoke run for development: `SG_AUTOPLAY=<dir> cargo run`
//! drives the real app through the menu, a battle, a human activation,
//! manual deployment and a spectated AI game, saving screenshots to <dir>
//! and exiting. Not compiled for the web.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use super::board::{BoardView, pixel_to_hex, to_world};
use super::flow::AiPacing;
use crate::hex::Hex;
use crate::rules::battle::{Activation, MatchSettings, Phase};
use crate::rules::commands::BattleCommand;
use crate::rules::config::{ControlMode, DeployMode};

#[derive(Resource)]
pub struct Autoplay {
    dir: String,
    frame: u32,
    spectate_started: Option<u32>,
}

impl Autoplay {
    pub fn from_env() -> Option<Self> {
        std::env::var("SG_AUTOPLAY").ok().map(|dir| {
            std::fs::create_dir_all(&dir).ok();
            Self { dir, frame: 0, spectate_started: None }
        })
    }
}

/// Clicks a hex the way a player would: hex -> screen pixel -> board pick.
fn click(view: &BoardView, h: Hex, out: &mut MessageWriter<BattleCommand>) {
    let screen = view.world_to_screen(to_world(h));
    let picked = pixel_to_hex(view.screen_to_world(screen));
    assert_eq!(picked, Some(h), "screen/board mapping round-trip failed for {h:?}");
    out.write(BattleCommand::SelectHex(h));
}

#[allow(clippy::too_many_arguments)]
pub fn autoplay(
    mut commands: Commands,
    mut auto: ResMut<Autoplay>,
    mut settings: ResMut<MatchSettings>,
    mut out: MessageWriter<BattleCommand>,
    mut pacing: ResMut<AiPacing>,
    mut exit: MessageWriter<AppExit>,
    view: Res<BoardView>,
    phase: Res<Phase>,
    act: Res<Activation>,
) {
    auto.frame += 1;
    let f = auto.frame;
    let dir = auto.dir.clone();
    let shot = |commands: &mut Commands, name: &str| {
        let path = format!("{dir}/{name}.png");
        info!("autoplay screenshot {path}");
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    };
    match f {
        40 => shot(&mut commands, "01-menu"),
        45 => {
            *settings = MatchSettings { scenario: 0, deploy_mode: DeployMode::Fixed, ..default() };
            out.write(BattleCommand::Restart);
        }
        90 => shot(&mut commands, "02-battle-start"),
        95 => {
            assert!(act.0.is_some_and(|a| a.side == 0 && a.unit.is_none()), "Blue should be picking a squadron");
            click(&view, Hex::new(9, 13), &mut out); // spindle tip
        }
        110 => shot(&mut commands, "03-selected"),
        115 => {
            out.write(BattleCommand::Forward);
            out.write(BattleCommand::Forward);
            out.write(BattleCommand::Forward);
        }
        130 => shot(&mut commands, "04-moved"),
        135 => {
            out.write(BattleCommand::EndActivation);
        }
        700 => shot(&mut commands, "05-midgame"),
        705 => {
            settings.deploy_mode = DeployMode::Manual;
            settings.control = ControlMode::PlayBlue;
            out.write(BattleCommand::Restart);
        }
        720 => {
            for r in [10, 12, 14] {
                click(&view, Hex::new(6, r), &mut out);
            }
            out.write(BattleCommand::TurnRight);
        }
        740 => shot(&mut commands, "06-deployment"),
        745 => {
            settings.control = ControlMode::Spectate;
            settings.size = 12;
            out.write(BattleCommand::Restart);
            pacing.auto = true;
            auto.spectate_started = Some(f);
        }
        900 => shot(&mut commands, "07-spectate"),
        _ => {}
    }
    if let Some(start) = auto.spectate_started {
        if *phase == Phase::GameOver && f > 900 && f < 100_000 {
            shot(&mut commands, "08-game-over");
            auto.spectate_started = None;
            auto.frame = 100_000;
        } else if f - start > 20_000 {
            panic!("spectated battle did not finish");
        }
    }
    if f == 100_040 {
        exit.write(AppExit::Success);
    }
}
