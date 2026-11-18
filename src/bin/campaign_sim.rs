//! Plays AI-vs-AI campaigns and reports who wins, how long wars last and
//! how many battles they take.
//!
//! ```sh
//! cargo run --release --bin campaign_sim -- [campaigns]
//! ```

use space_game::campaign::run_ai_campaign;

fn main() {
    let n: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let seeds: Vec<u32> = (1..=n).collect();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = seeds
            .chunks(seeds.len().div_ceil(threads).max(1))
            .map(|chunk| scope.spawn(move || chunk.iter().map(|s| (*s, run_ai_campaign(*s, 120))).collect::<Vec<_>>()))
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    let mut wins = [0; 3];
    for (seed, r) in &results {
        println!("seed {seed:>3}: winner {:<6} after {:>3} turns, {:>3} battles", r.winner.map_or("none", |w| ["Blue", "Red"][w]), r.turns, r.battles);
        wins[r.winner.unwrap_or(2)] += 1;
    }
    let turns: f32 = results.iter().map(|(_, r)| r.turns as f32).sum::<f32>() / results.len() as f32;
    let battles: f32 = results.iter().map(|(_, r)| r.battles as f32).sum::<f32>() / results.len() as f32;
    println!("\nBlue {} · Red {} · none {} — mean {turns:.1} turns, {battles:.1} battles", wins[0], wins[1], wins[2]);
}
