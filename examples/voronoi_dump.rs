//! Writes one PPM per Voronoi lab, the tutorial's "Show it" step:
//! `cargo run --release --example voronoi_dump -- <dir> [seed] [sites]`.

use space_game::voronoi::labs::{LabId, LabState, render};
use space_game::voronoi::sites::VoronoiConfig;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = args.get(1).cloned().unwrap_or_else(|| "voronoi-out".into());
    let mut cfg = VoronoiConfig::default();
    if let Some(s) = args.get(2).and_then(|s| s.parse().ok()) {
        cfg.seed = s;
    }
    if let Some(n) = args.get(3).and_then(|s| s.parse().ok()) {
        cfg.sites = n;
    }
    std::fs::create_dir_all(&dir).expect("create output dir");
    let mut st = LabState::new(&cfg);
    st.show_triangles = true;
    st.selected = Some(0);
    st.grow_radius = 30.0;
    for (i, lab) in LabId::ALL.iter().enumerate() {
        let t = std::time::Instant::now();
        let c = render(&cfg, *lab, &st, 1);
        let name = format!("{dir}/{}-{}.ppm", i + 1, lab.title().to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-"));
        std::fs::write(&name, c.to_ppm()).expect("write ppm");
        println!("{name}  {:.0} ms", t.elapsed().as_secs_f32() * 1000.0);
    }
}
