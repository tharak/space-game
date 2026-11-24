//! Calibration for planet rebellion: the NetLogo model alone, over
//! legitimacy (1 − tax − occupation), cops and war hardship. Prints the mean
//! and peak active share and when (in game seconds, 2 steps a second) the
//! share first stays above `revolt` for 5 s.
//!
//! `cargo run --release --example rebellion_sweep [revolt share]`, or
//! `... rebellion_sweep ema` for the peak smoothed share after the fleets leave.

use space_game::prototypes::rebellion::{Rebellion, RebellionParams};

fn main() {
    if std::env::args().nth(1).as_deref() == Some("ema") {
        return ema();
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
