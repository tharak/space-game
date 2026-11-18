//! Monte-Carlo balance runner for the fleet battle. Every scenario is played
//! AI-vs-AI with the seats mirrored (half the games with each fleet as Blue)
//! and reports the first-named fleet's results.
//!
//! ```sh
//! cargo run --release --bin sim -- [games-per-seating] [ships-per-side] [scenario]
//! ```

use space_game::sim::battle::{AiKind, SimSettings};
use space_game::sim::scenario::SCENARIOS;
use space_game::sim::{Report, simulate};

struct Row {
    win: f32,
    draw: f32,
    minutes: f32,
    contact: f32,
    breakthrough: f32,
}

fn run(games: u32, ships: usize, scenario: usize, ai_a: AiKind, ai_b: AiKind) -> Row {
    let jobs: Vec<(bool, u32)> = [false, true].into_iter().flat_map(|m| (0..games).map(move |g| (m, g))).collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let results: Vec<(bool, Report)> = std::thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .chunks(jobs.len().div_ceil(threads).max(1))
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|&(mirror, g)| {
                            let ai = if mirror { [ai_b, ai_a] } else { [ai_a, ai_b] };
                            let settings = SimSettings { scenario, ships, ai, mirror, ..Default::default() };
                            (mirror, simulate(settings, 1000 + g * 7919 + mirror as u32, None))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let n = results.len() as f32;
    let mut row = Row { win: 0.0, draw: 0.0, minutes: 0.0, contact: 0.0, breakthrough: 0.0 };
    for (mirror, r) in &results {
        let a = if *mirror { 1 } else { 0 };
        match r.winner {
            Some(w) if w == a => row.win += 1.0,
            None => row.draw += 1.0,
            _ => {}
        }
        row.minutes += r.time / 60.0;
        row.contact += r.first_contact.unwrap_or(r.time) / 60.0;
        if r.first_breakthrough[a].is_some() {
            row.breakthrough += 1.0;
        }
    }
    Row {
        win: row.win / n * 100.0,
        draw: row.draw / n * 100.0,
        minutes: row.minutes / n,
        contact: row.contact / n,
        breakthrough: row.breakthrough / n * 100.0,
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let games: u32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(50);
    let ships: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(240);
    let only: Option<usize> = args.next().and_then(|s| s.parse().ok());
    println!("{} games per matchup (seats mirrored), {ships} ships a side", games * 2);
    println!("Cells: first-named fleet's win% / draw% / battle minutes (contact at) / its breakthrough%\n");
    use AiKind::*;
    let cols = [("advance mirror", Advance, Advance), ("commander mirror", Commander, Commander), ("A commander, B advance", Commander, Advance)];
    print!("{:<20}", "scenario");
    for (name, _, _) in cols {
        print!(" {name:>30}");
    }
    println!();
    for (i, s) in SCENARIOS.iter().enumerate() {
        if only.is_some_and(|o| o != i) {
            continue;
        }
        print!("{:<20}", s.title);
        for (_, a, b) in cols {
            let r = run(games, ships, i, a, b);
            let cell = format!("{:.0}/{:.0}/{:.1}m ({:.1})/{:.0}", r.win, r.draw, r.minutes, r.contact, r.breakthrough);
            print!(" {cell:>30}");
        }
        println!();
    }
}
