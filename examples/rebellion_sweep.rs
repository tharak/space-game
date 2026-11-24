//! Calibration for planet rebellion: the NetLogo model alone, over
//! legitimacy (1 − tax − occupation), cops and war hardship. Prints the mean
//! and peak active share and when (in game seconds, 2 steps a second) the
//! share first stays above `revolt` for 5 s.
//!
//! `cargo run --release --example rebellion_sweep [revolt share]`, or
//! `... rebellion_sweep ema` for the peak smoothed share after the fleets leave,
//! or `... rebellion_sweep table [path]` to regenerate `src/prototypes/calm.rs`.

use space_game::prototypes::rebellion::{Rebellion, RebellionParams};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("ema") {
        return ema();
    }
    if std::env::args().nth(1).as_deref() == Some("table") {
        return table(std::env::args().nth(2).unwrap_or_else(|| "src/prototypes/calm.rs".into()));
    }
    let revolt: f32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0.25);
    let p = RebellionParams::default();
    let steps = 600; // 5 minutes of game time
    println!("revolt share {revolt}: legit, war, cops -> mean, peak, first revolt (s) over 3 seeds");
    for war in [0.0, 0.3] {
        for legit in [0.95, 0.9, 0.85, 0.8, 0.75, 0.7, 0.6, 0.5, 0.4] {
            for cops in [0, 10, 20, 40, 64, 100, 150] {
                let (mut mean, mut peak, mut first) = (0.0, 0.0f32, Vec::new());
                for seed in 1..=3 {
                    let mut r = Rebellion::new(seed);
                    let mut above = 0;
                    let mut when = None;
                    for s in 0..steps {
                        r.step(legit, war, cops, &p);
                        let a = r.active_share();
                        mean += a / (steps * 3) as f32;
                        peak = peak.max(a);
                        above = if a > revolt { above + 1 } else { 0 };
                        if above >= 10 && when.is_none() {
                            when = Some((s + 1) as f32 / 2.0);
                        }
                    }
                    first.push(when.map_or("-".to_string(), |w| format!("{w:.0}")));
                }
                println!("L {legit:.2} war {war:.1} cops {cops:3}: mean {mean:.2} peak {peak:.2} revolt {}", first.join("/"));
            }
        }
    }
}

/// Max smoothed rebel share (10 s time constant) after the people settle
/// under 200 cops and then the fleets leave `cops` behind.
fn ema() {
    let p = RebellionParams::default();
    for legit in [0.8, 0.75, 0.7, 0.6, 0.5] {
        for cops in [20, 40, 60, 80, 120] {
            let mut peaks = Vec::new();
            for seed in 1..=5 {
                let mut r = Rebellion::new(seed);
                for _ in 0..40 {
                    r.step(legit, 0.0, 200, &p);
                }
                let (mut ema, mut peak) = (0.0f32, 0.0f32);
                for _ in 0..600 {
                    r.step(legit, 0.0, cops, &p);
                    ema += (r.active_share() - ema) * 0.05;
                    peak = peak.max(ema);
                }
                peaks.push(format!("{peak:.2}"));
            }
            println!("L {legit:.2} cops {cops:3}: peak smoothed {}", peaks.join(" "));
        }
    }
}

/// Whether `cops` keep people with this legitimacy and hardship bonus calm:
/// after a 40-step warm-up, the smoothed rebel share (10 s time constant at
/// 2 steps a second) stays below 0.1 for 300 steps, for two seeds.
fn calm(legit: f32, bonus: f32, cops: usize) -> bool {
    let p = RebellionParams::default();
    (1..=2).all(|seed| {
        let mut r = Rebellion::new(seed);
        for _ in 0..40 {
            r.step(legit, bonus, cops, &p);
        }
        let mut ema = 0.0f32;
        for _ in 0..300 {
            r.step(legit, bonus, cops, &p);
            ema += (r.active_share() - ema) * 0.05;
            if ema >= 0.1 {
                return false;
            }
        }
        true
    })
}

/// The fewest cops (0..=200, 255 = not even 200) that keep it calm.
fn fewest(legit: f32, bonus: f32) -> u8 {
    if calm(legit, bonus, 0) {
        return 0;
    }
    if !calm(legit, bonus, 200) {
        return 255;
    }
    let (mut lo, mut hi) = (0usize, 200usize);
    while hi - lo > 2 {
        let mid = (lo + hi) / 2;
        if calm(legit, bonus, mid) { hi = mid } else { lo = mid }
    }
    hi as u8
}

/// Writes the CALM_COPS table: legitimacy 0..1 by 0.05 (rows), hardship
/// bonus 0..1 by 0.1 (columns).
fn table(path: String) {
    let cells: Vec<(usize, usize)> = (0..21).flat_map(|l| (0..11).map(move |b| (l, b))).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let out = std::sync::Mutex::new([[0u8; 11]; 21]);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(l, b)) = cells.get(i) else { break };
                let v = fewest(l as f32 * 0.05, b as f32 * 0.1);
                out.lock().unwrap()[l][b] = v;
                eprintln!("L {:.2} bonus {:.1}: {v}", l as f32 * 0.05, b as f32 * 0.1);
            });
        }
    });
    let t = out.into_inner().unwrap();
    let mut src = String::from(
        "//! Generated by `cargo run --release --example rebellion_sweep -- table`.\n\
         //! The fewest cops that keep a planet's people calm (smoothed rebel share\n\
         //! under 0.1), by legitimacy (rows, 0..1 by 0.05) and hardship bonus\n\
         //! (columns, 0..1 by 0.1); 255: not even 200 cops.\n\n\
         pub const CALM_COPS: [[u8; 11]; 21] = [\n",
    );
    for row in t {
        src.push_str(&format!("    {:?},\n", row));
    }
    src.push_str("];\n");
    std::fs::write(&path, src).expect("write table");
    println!("wrote {path}");
}
