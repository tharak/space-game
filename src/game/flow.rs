//! Drives the battle each frame: applies queued commands, paces AI
//! activations, and turns rule events into log lines.

use bevy::ecs::message::Messages;
use bevy::prelude::*;

use crate::rules::ai;
use crate::rules::battle::*;
use crate::rules::commands::{BattleCommand, apply};
use crate::rules::config::ControlMode;

/// Seconds between AI activations when playing against it.
const AI_DELAY: f32 = 0.25;
/// Seconds between activations in spectate auto mode.
const AUTO_DELAY: f32 = 0.22;

#[derive(Resource, Default)]
pub struct AiPacing {
    pub timer: f32,
    pub auto: bool,
    pub step: bool,
}

#[derive(Resource, Default)]
pub struct OverlayHidden(pub bool);

#[derive(Resource, Default)]
pub struct LogBook {
    pub lines: Vec<(String, Tone)>,
    /// Bumped when the log is cleared, so the view can reset.
    pub generation: u64,
}

pub fn run_battle(world: &mut World) {
    let commands: Vec<BattleCommand> = world.resource_mut::<Messages<BattleCommand>>().drain().collect();
    for cmd in commands {
        if cmd == BattleCommand::Restart {
            let mut log = world.resource_mut::<LogBook>();
            log.lines.clear();
            log.generation += 1;
            world.resource_mut::<OverlayHidden>().0 = false;
            world.resource_mut::<AiPacing>().timer = 0.0;
        }
        apply(world, cmd);
    }

    let dt = world.resource::<Time>().delta_secs();
    let spectate = world.resource::<MatchSettings>().control == ControlMode::Spectate;
    if let Next::Ai(side) = advance(world) {
        let mut pacing = world.resource_mut::<AiPacing>();
        pacing.timer += dt;
        let go = if spectate {
            let auto_ready = pacing.auto && pacing.timer >= AUTO_DELAY;
            pacing.step || auto_ready
        } else {
            pacing.timer >= AI_DELAY
        };
        if go {
            pacing.timer = 0.0;
            pacing.step = false;
            ai::activate_next(world, side);
            advance(world);
        }
    } else {
        let mut pacing = world.resource_mut::<AiPacing>();
        pacing.step = false;
        if *world.resource::<Phase>() == Phase::GameOver {
            world.resource_mut::<AiPacing>().auto = false;
        }
    }
}

pub fn log_events(mut events: MessageReader<BattleEvent>, mut log: ResMut<LogBook>) {
    for event in events.read() {
        let (text, tone) = match event {
            BattleEvent::Note { text, tone } => (text.clone(), *tone),
            BattleEvent::MoraleChecked { label, roll, modifiers, total, passed } => (
                format!(
                    "  {label} morale: {roll}{} = {total} → {}",
                    if modifiers.is_empty() { String::new() } else { format!(" {}", modifiers.join(" ")) },
                    if *passed { "holds" } else { "FAILS" }
                ),
                if *passed { Tone::Plain } else { Tone::Bad },
            ),
            BattleEvent::Routed { label } => (format!("  {label} ROUTS!"), Tone::Bad),
            BattleEvent::Destroyed { label, .. } => (format!("  {label} is DESTROYED"), Tone::Bad),
            BattleEvent::FlagshipLost { side } => (
                format!("  {} FLAGSHIP LOST — fleet-wide morale check, command net down", crate::rules::config::side_name(*side)),
                Tone::Bad,
            ),
            BattleEvent::Shot { attacker, target, side, arc, need, rolls, hits, .. } => (
                format!(
                    "{attacker} fires at {target} ({} arc, {need}+): [{}] → {hits} hit{}",
                    arc.name(),
                    rolls.iter().map(|r| r.to_string()).collect::<Vec<_>>().join(" "),
                    if *hits == 1 { "" } else { "s" }
                ),
                if *hits > 0 { Tone::Side(*side) } else { Tone::Plain },
            ),
            BattleEvent::Fled { label } => (format!("  {label} flees off the map"), Tone::Bad),
            BattleEvent::Rallied { label, roll, bonus } => {
                (format!("{label} RALLIES ({roll}{}) — now Shaken", if *bonus { "+1" } else { "" }), Tone::Good)
            }
            BattleEvent::RallyFailed { label, roll, bonus } => (
                format!("{label} fails to rally ({roll}{}) and keeps running", if *bonus { "+1" } else { "" }),
                Tone::Plain,
            ),
            BattleEvent::ShakenRefused { label } => {
                (format!("{label} is Shaken — it refuses to move toward the enemy"), Tone::Bad)
            }
        };
        log.lines.push((text, tone));
    }
}

pub fn keyboard(keys: Res<ButtonInput<KeyCode>>, phase: Res<Phase>, mut commands: MessageWriter<BattleCommand>) {
    if !matches!(*phase, Phase::Deployment | Phase::Combat) {
        return;
    }
    for (key, cmd) in [
        (KeyCode::KeyQ, BattleCommand::TurnLeft),
        (KeyCode::KeyE, BattleCommand::TurnRight),
        (KeyCode::KeyW, BattleCommand::Forward),
        (KeyCode::KeyS, BattleCommand::Backward),
        (KeyCode::KeyF, BattleCommand::FireMode),
        (KeyCode::Space, BattleCommand::EndActivation),
    ] {
        if keys.just_pressed(key) {
            commands.write(cmd);
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod clipboard {
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(inline_js = "export function copy_text(t) { if (navigator.clipboard) { navigator.clipboard.writeText(t).catch(() => {}); } }")]
    extern "C" {
        pub fn copy_text(text: &str);
    }
}

pub fn copy_to_clipboard(text: &str) {
    #[cfg(target_arch = "wasm32")]
    clipboard::copy_text(text);
    #[cfg(not(target_arch = "wasm32"))]
    println!("{text}");
}
