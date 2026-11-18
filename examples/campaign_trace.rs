//! Prints an AI-vs-AI campaign turn by turn: `cargo run --release --example campaign_trace -- [seed] [every]`
use space_game::campaign::map::galaxy;
use space_game::campaign::state::Location;
use space_game::campaign::{Campaign, ai, auto_resolve_all, turn};

fn main() {
    let seed = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let every: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let g = galaxy();
    let mut c = Campaign::new(seed);
    ai::orders(&mut c, 0);
    ai::orders(&mut c, 1);
    while c.winner.is_none() && c.turn <= 120 {
        turn::end_turn(&mut c);
        auto_resolve_all(&mut c);
        turn::finish_turn(&mut c, [true, true]);
        for l in &c.log {
            println!("  t{} {l}", c.turn - 1);
        }
        if c.turn.is_multiple_of(every) {
            let own: Vec<String> = (0..c.systems.len())
                .map(|s| format!("{}{}:{}", g.systems[s].name.chars().take(4).collect::<String>(), ["B", "R"].get(c.systems[s].owner.unwrap_or(9)).unwrap_or(&"N"), c.systems[s].garrison))
                .collect();
            println!("== turn {} systems {}", c.turn, own.join(" "));
            for f in &c.fleets {
                let loc = match f.loc {
                    Location::At(s) => g.systems[s].name.to_string(),
                    Location::Lane { from, to, .. } => format!("{}->{}", g.systems[from].name, g.systems[to].name),
                };
                println!("   {} {} {} ships {} dmg {} unsup {} path {:?}", ["B", "R"][f.side], f.admiral.name, loc, f.ships, f.damaged, f.unsupplied, f.path);
            }
        }
    }
    println!("winner {:?} turn {}", c.winner, c.turn);
}
