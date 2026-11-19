//! Writes prototype01 as PPMs: the opening, then after a few turns with
//! the first Blue fleet reinforced:
//! `cargo run --release --example prototype01_dump -- <dir> [planets] [fleets]`.

use space_game::prototypes::dominion::{DrawOptions, Dominion, Tuning, render, status};
use space_game::voronoi::sites::VoronoiConfig;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).cloned().unwrap_or_else(|| "prototype01-out".into());
    let planets = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let fleets = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    std::fs::create_dir_all(&dir).expect("create output dir");
    let mut d = Dominion::new(&VoronoiConfig::default(), planets, fleets, Tuning::default());
    let write = |d: &Dominion, name: &str| {
        std::fs::write(format!("{dir}/{name}.ppm"), render(d, DrawOptions::default()).to_ppm()).expect("write ppm");
        println!("{name}: {}", status(d, None).replace('\n', " | "));
    };
    write(&d, "turn1");
    d.fleets[0].ships = 120;
    for _ in 0..3 {
        d.end_turn();
    }
    write(&d, "turn4");
}
