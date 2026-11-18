//! Monte-Carlo balance runner. Every scenario is played AI-vs-AI with the
//! sides mirrored (half the games with each formation as Blue), like the
//! original Python sim, and reports the first-named formation's win rate.
//!
//! ```sh
//! cargo run --release --bin sim -- [games-per-seating] [size]
//! ```

use space_game::rules::battle::MatchSettings;
use space_game::rules::config::{AiKind, SCENARIOS};
use space_game::simulate;

/// Win %, draw % and mean turns for scenario side A, where `ai_a`/`ai_b`
/// drive A and B respectively.
fn rate(games: u32, size: usize, scenario: usize, ai_a: AiKind, ai_b: AiKind) -> (f32, f32, f32) {
    let (mut wins, mut draws, mut turns) = (0u32, 0u32, 0u32);
    for mirror in [false, true] {
        let ai = if mirror { [ai_b, ai_a] } else { [ai_a, ai_b] };
        let a_side = if mirror { 1 } else { 0 };
        for seed in 0..games {
            let settings = MatchSettings { scenario, size, ai, mirror, ..Default::default() };
            let (winner, t) = simulate(settings, 1000 + seed * 7919 + mirror as u32);
            turns += t;
            match winner {
                Some(w) if w == a_side => wins += 1,
                None => draws += 1,
                _ => {}
            }
        }
    }
    let n = (games * 2) as f32;
    (wins as f32 / n * 100.0, draws as f32 / n * 100.0, turns as f32 / n)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let games: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(200);
    let size: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(9);
    println!("{} games per matchup (sides mirrored), {size} squadrons a side", games * 2);
    println!("Cells: first-named side's win % / draw % / mean turns\n");
    println!("{:<30} {:>20} {:>20} {:>20} {:>20}", "scenario", "baseline mirror", "doctrine mirror", "A doctrine, B base", "A base, B doctrine");
    use AiKind::*;
    for (i, s) in SCENARIOS.iter().enumerate() {
        let cells = [(Baseline, Baseline), (Doctrine, Doctrine), (Doctrine, Baseline), (Baseline, Doctrine)]
            .map(|(a, b)| rate(games, size, i, a, b))
            .map(|(w, d, t)| format!("{w:>5.0} /{d:>3.0} /{t:>5.1}"));
        println!("{:<30} {:>20} {:>20} {:>20} {:>20}", s.title, cells[0], cells[1], cells[2], cells[3]);
    }
}
