//! Scripted smoke run for development: `SG_AUTOPLAY=<dir> cargo run`
//! drives the real app through the menu, a commanded battle and a
//! spectated one, saving screenshots to <dir> and exiting. Not compiled for
//! the web.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use super::camera::CameraRig;
use super::flow::{AppRequest, Phase, SeedSource, Speed};
use super::input::Selection;
use crate::sim::Command;
use crate::sim::battle::{Battle, Control, SimSettings};
use crate::sim::components::{Division, Order};
use crate::sim::spatial::SpatialHash;

#[derive(Resource)]
pub struct Autoplay {
    dir: String,
    frame: u32,
    step: u32,
    wait_from: u32,
}

impl Autoplay {
    pub fn from_env() -> Option<Self> {
        std::env::var("SG_AUTOPLAY").ok().map(|dir| {
            std::fs::create_dir_all(&dir).ok();
            Self { dir, frame: 0, step: 0, wait_from: 0 }
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub fn autoplay(
    mut commands: Commands,
    mut auto: ResMut<Autoplay>,
    mut settings: ResMut<SimSettings>,
    mut seed: ResMut<SeedSource>,
    mut requests: MessageWriter<AppRequest>,
    mut orders: MessageWriter<Command>,
    mut speed: ResMut<Speed>,
    mut sel: ResMut<Selection>,
    mut exit: MessageWriter<AppExit>,
    rig: Res<CameraRig>,
    hash: Res<SpatialHash>,
    phase: Res<Phase>,
    battle: Res<Battle>,
    divs: Query<(Entity, &Division)>,
) {
    auto.frame += 1;
    let f = auto.frame;
    let dir = auto.dir.clone();
    let shot = |commands: &mut Commands, name: &str| {
        let path = format!("{dir}/{name}.png");
        info!("autoplay screenshot {path}");
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    };
    let find = |side: usize, number: usize| divs.iter().find(|(_, d)| d.side == side && d.number == number).map(|(e, _)| e);
    let since = f - auto.wait_from;
    let next = |auto: &mut Autoplay| {
        auto.step += 1;
        auto.wait_from = f;
    };
    match auto.step {
        0 if f == 40 => {
            shot(&mut commands, "01-menu");
            next(&mut auto);
        }
        1 if since == 5 => {
            *settings = SimSettings { scenario: 0, ships: 240, control: Control::PlayBlue, ..default() };
            seed.0 = 7;
            requests.write(AppRequest::Start);
            next(&mut auto);
        }
        2 if since == 40 => {
            shot(&mut commands, "02-battle-start");
            // Click the spearhead's ship the way a player would.
            let tip = find(0, 3).expect("B3 exists");
            let ship = hash.divisions.get(&tip).and_then(|d| d.members.first()).map(|i| hash.entries[*i as usize].pos);
            let screen = rig.world_to_screen(ship.expect("B3 has ships"));
            let world = rig.screen_to_world(screen);
            let picked = super::input::pick_division(&hash, world, rig.scale);
            assert_eq!(picked.map(|p| p.0), Some(tip), "screen/world pick round-trip failed");
            sel.division = Some(tip);
            next(&mut auto);
        }
        3 if since == 20 => {
            shot(&mut commands, "03-selected");
            // Swing the reserve out to the left as a flank guard, then go.
            let reserve = find(0, 6).expect("B6 exists");
            orders.write(Command::Order { division: reserve, order: Order::MoveTo { to: Vec2::new(-700.0, 500.0), facing: 0.0 } });
            speed.paused = false;
            speed.mult = 8;
            next(&mut auto);
        }
        4 if battle.time > 52.0 => {
            shot(&mut commands, "04-contact");
            next(&mut auto);
        }
        5 if battle.time > 95.0 => {
            shot(&mut commands, "05-breakthrough");
            next(&mut auto);
        }
        6 if *phase == Phase::Over => {
            shot(&mut commands, "06-result");
            next(&mut auto);
        }
        7 if since == 20 => {
            *settings = SimSettings { scenario: 2, ships: 480, control: Control::Spectate, ..default() };
            requests.write(AppRequest::Start);
            next(&mut auto);
        }
        8 if since >= 5 => {
            speed.mult = 8;
            next(&mut auto);
        }
        9 if battle.time > 80.0 => {
            shot(&mut commands, "07-spectate-480");
            next(&mut auto);
        }
        10 if *phase == Phase::Over => {
            shot(&mut commands, "08-spectate-result");
            next(&mut auto);
        }
        11 if since == 30 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
    if f > 60 * 60 * 10 {
        panic!("autoplay timed out at step {}", auto.step);
    }
}
