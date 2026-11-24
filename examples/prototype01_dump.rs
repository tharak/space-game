//! Runs prototype01 for three minutes and writes frames as PPMs. Each side's
//! fleets head for the nearest independent planet, so there's fighting:
//! `cargo run --release --example prototype01_dump -- <dir> [planets] [fleets]`.

use space_game::prototypes::dominion::{DrawOptions, Dominion, Owner, Tuning, VIEW_H, VIEW_W, View, render, render_background, status};
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
        let view = View::fit(&d.cfg, VIEW_W, VIEW_H);
        let t = std::time::Instant::now();
        let bg = render_background(d, &view, true, 1);
        let t_bg = t.elapsed();
        // Blue's most restless planet shows its people.
        let planet = (0..d.planets.len())
            .filter(|i| d.planets[*i].owner == Owner::Side(0))
            .max_by(|a, b| d.planets[*a].unrest.total_cmp(&d.planets[*b].unrest));
        let opts =
            DrawOptions { selected: d.fleets.first().map(|f| f.ship_ids.clone()).unwrap_or_default(), planet, ..Default::default() };
        let t = std::time::Instant::now();
        let frame = render(d, &bg, &view, &opts);
        println!("{name}: background {:.1} ms, frame {:.1} ms", t_bg.as_secs_f32() * 1e3, t.elapsed().as_secs_f32() * 1e3);
        std::fs::write(format!("{dir}/{name}.ppm"), frame.to_ppm()).expect("write ppm");
        // And a 4× close-up of Blue's first fleet.
        let mut close = view;
        if let Some(f) = d.fleets.iter().find(|f| f.owner == Owner::Side(0)) {
            close.zoom_at(view.to_screen(f.pos), 4.0, &d.cfg);
        }
        let bg = render_background(d, &close, true, 1);
        std::fs::write(format!("{dir}/{name}-zoom.ppm"), render(d, &bg, &close, &opts).to_ppm()).expect("write ppm");
        println!("{name}: {}", status(d, &[], planet).replace('\n', " | "));
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
    for t in [0, 20, 40, 60, 90, 120, 180] {
        d.step(t as f32 - d.time);
        write(&d, &format!("t{t:02}"));
    }
}
