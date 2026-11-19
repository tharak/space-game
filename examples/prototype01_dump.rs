//! Runs prototype01 for a minute and writes frames as PPMs. Each side's
//! fleets head for the nearest independent planet, so there's fighting:
//! `cargo run --release --example prototype01_dump -- <dir> [planets] [fleets]`.

use space_game::prototypes::dominion::{DrawOptions, Dominion, Owner, Tuning, render, render_background, status};
use space_game::voronoi::sites::VoronoiConfig;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).cloned().unwrap_or_else(|| "prototype01-out".into());
    let planets = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(20);
    let fleets = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(3);
    std::fs::create_dir_all(&dir).expect("create output dir");
    let cfg = VoronoiConfig { w: 1080, h: 600, ..Default::default() };
    let mut d = Dominion::new(&cfg, planets, fleets, Tuning::default());
    let write = |d: &Dominion, name: &str| {
        let bg = render_background(d, true);
        let opts = DrawOptions { selected: d.fleets.first().map(|f| f.id), ..Default::default() };
        std::fs::write(format!("{dir}/{name}.ppm"), render(d, &bg, opts).to_ppm()).expect("write ppm");
        println!("{name}: {}", status(d, None).replace('\n', " | "));
    };
    // Every commandable fleet goes for the nearest independent planet.
    let orders: Vec<(u32, _)> = d
        .fleets
        .iter()
        .filter(|f| f.controllable())
        .filter_map(|f| {
            d.planets
                .iter()
                .filter(|p| matches!(p.owner, Owner::Independent(_)))
                .min_by(|a, b| a.pos.distance(f.pos).total_cmp(&b.pos.distance(f.pos)))
                .map(|p| (f.id, p.pos))
        })
        .collect();
    for (id, to) in orders {
        d.order_move(id, to);
    }
    for t in [0, 20, 40, 60] {
        d.step(t as f32 - d.time);
        write(&d, &format!("t{t:02}"));
    }
}
