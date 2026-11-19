//! Scripted smoke run for development: `SG_AUTOPLAY=<dir> cargo run`
//! drives the real app through the start page, two Voronoi labs, the menu, one spectated battle per fleet
//! shape (each must end with a winner) and the campaign map, saving
//! screenshots to <dir> and exiting. Not compiled for the web.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

use super::camera::CameraRig;
use super::flow::{AppRequest, Phase, SeedSource, Speed};
use crate::sim::battle::{Battle, Control, SimSettings, Skirmish};
use crate::voronoi::labs::LabId;
use crate::sim::components::Division;
use crate::sim::scenario::SHOWCASE;
use crate::sim::spatial::SpatialHash;

#[derive(Resource)]
pub struct Autoplay {
    dir: String,
    frame: u32,
    step: u32,
    wait_from: u32,
    /// Which `SHOWCASE` battle is on.
    battle: usize,
    shot_fight: bool,
}

impl Autoplay {
    pub fn from_env() -> Option<Self> {
        std::env::var("SG_AUTOPLAY").ok().map(|dir| {
            std::fs::create_dir_all(&dir).ok();
            Self { dir, frame: 0, step: 0, wait_from: 0, battle: 0, shot_fight: false }
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
    mut speed: ResMut<Speed>,
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
    let since = f - auto.wait_from;
    let next = |auto: &mut Autoplay| {
        auto.step += 1;
        auto.wait_from = f;
    };
    let shapes = SHOWCASE[auto.battle.min(SHOWCASE.len() - 1)];
    let name = format!("{:02}-{}-vs-{}", auto.battle + 2, shapes[0].name(), shapes[1].name()).to_lowercase();
    match auto.step {
        0 if f == 40 => {
            shot(&mut commands, "00-start");
            auto.step = 99;
            auto.wait_from = f;
        }
        // Screenshots land a frame late, so each page change waits a beat.
        99 if since == 10 => {
            requests.write(AppRequest::OpenLab(LabId::Delaunay));
            next(&mut auto);
        }
        100 if since == 20 => {
            shot(&mut commands, "00-lab-delaunay");
            next(&mut auto);
        }
        101 if since == 10 => {
            requests.write(AppRequest::OpenLab(LabId::Island));
            next(&mut auto);
        }
        102 if since == 20 => {
            shot(&mut commands, "00-lab-island");
            next(&mut auto);
        }
        103 if since == 10 => {
            requests.write(AppRequest::Menu);
            next(&mut auto);
        }
        104 if since == 20 => {
            shot(&mut commands, "01-menu");
            auto.step = 1;
            auto.wait_from = f;
        }
        1 if since == 5 => {
            let k = Skirmish { shapes, ..default() };
            *settings = SimSettings { skirmish: Some(k), ships: k.ships(), control: Control::Spectate, ..default() };
            seed.0 = 7;
            auto.shot_fight = false;
            requests.write(AppRequest::Start);
            next(&mut auto);
        }
        2 if since == 30 => {
            shot(&mut commands, &format!("{name}-a-start"));
            // A ship picked on screen must map back to its division.
            if let Some((e, _)) = divs.iter().find(|(_, d)| d.side == 0 && d.number == 1) {
                let ship = hash.divisions.get(&e).and_then(|d| d.members.first()).map(|i| hash.entries[*i as usize].pos);
                let world = rig.screen_to_world(rig.world_to_screen(ship.expect("B1 has ships")));
                let picked = super::input::pick_division(&hash, world, rig.scale);
                assert_eq!(picked.map(|p| p.0), Some(e), "screen/world pick round-trip failed");
            }
            speed.paused = false;
            speed.mult = 8;
            next(&mut auto);
        }
        3 => {
            if battle.time > 70.0 && !auto.shot_fight {
                auto.shot_fight = true;
                shot(&mut commands, &format!("{name}-b-fight"));
            }
            if *phase == Phase::Over {
                let winner = battle.result.as_ref().and_then(|r| r.winner);
                assert!(winner.is_some(), "{} vs {} ended in a draw", shapes[0].name(), shapes[1].name());
                next(&mut auto);
            }
        }
        4 if since == 10 => {
            shot(&mut commands, &format!("{name}-c-result"));
            auto.battle += 1;
            if auto.battle < SHOWCASE.len() {
                auto.step = 1;
                auto.wait_from = f;
            } else {
                next(&mut auto);
            }
        }
        5 if since == 20 => {
            requests.write(AppRequest::NewCampaign);
            next(&mut auto);
        }
        6 if since == 40 => {
            shot(&mut commands, "07-campaign-map");
            next(&mut auto);
        }
        7 if since == 20 => {
            exit.write(AppExit::Success);
        }
        _ => {}
    }
    if f > 60 * 60 * 15 {
        panic!("autoplay timed out at step {}", auto.step);
    }
}
