# Successor Stars — fleet battle (Rust · Bevy · ECS)

A real-time fleet battle with hundreds of ships a side, in Rust with [Bevy 0.19](https://bevy.org) and its ECS. You command divisions, and formation decides the fight: a deep arrow can tear a hole in a thin line, a crescent can close on an arrow's flanks, and shots into a ship's flank or rear land far more often than shots into its front.

**Play:** <https://tharak.github.io/space-game/> (needs a browser with WebGL2)

This started as a port of the [BattleStars](https://github.com/tharak/BattleStars) hex board (rules v0.2.1, still in this repo's history). Playing it showed that 9 squadrons a side, each destroyed in 2–3 shots, could not give a formation time to matter. So the game is now a continuous 2D simulation of every ship ([docs/fleet-rules.md](docs/fleet-rules.md)).

## How to play

Pick a side, a fleet size (120/240/480 ships a side), an opponent and a scenario. When you command a fleet, the battle starts paused so you can give orders first.

| Input | Action |
| --- | --- |
| Click a ship, or a row in the division list | Select its division (`Tab` cycles your divisions) |
| Right-click the map (or **Move…**, then click/tap) | Move there; drag while holding the button to set the facing |
| Click an enemy ship with your division selected | Engage that enemy division |
| **Hold / Advance / Charge / Withdraw** (`H` `G` `C` `X`) | Orders |
| Formation buttons (`F` cycles) | Line, Wedge, Crescent, Echelon L/R, Column |
| Drag / arrows / WASD, wheel / pinch, `+` `-` | Pan and zoom |
| `Space`, `1`–`4` | Pause, and speed 1× 2× 4× 8× |

* Ships stop short of enemy ships (a quarter of weapon range) instead of colliding. To get through a line you must destroy the ships in the way.
* A division that loses ships quickly turns Shaken, then Routs. Staying inside the flagship's command circle steadies it.
* A fleet breaks when more than half its ships are destroyed, fled or routed.

## How an arrow breaks a line

The breakthrough is not a scripted rule. It comes out of the simulation:

1. **Short range.** Guns have a short range and fire only in a forward cone. Only the part of a line facing the arrow's tip can shoot at it; the rest of the line is out of the fight until it wheels.
2. **Depth.** A wedge closes ranks as it loses ships, so it always has a full front. A two-rank line keeps its holes.
3. **Blocking.** Ships stop a quarter of weapon range short of any enemy and never push through, so the arrow grinds into the line by destroying the ships in its way.
4. **Getting behind.** Once through, the arrow fires into rear arcs (40% to hit, against 12% from the front). The line's ships cannot turn far enough to answer until their division wheels.
5. **Morale.** Being broken through, plus the sudden losses, shakes and routs the broken divisions.

The counter is to put flanks on the arrow, for example with a crescent or by committing the reserve.

## Balance runs

`cargo run --release --bin sim -- 25 240` plays every scenario AI-vs-AI, with each fleet taking each seat half the time. Each cell gives the first-named fleet's win %, and the battle length in minutes with contact at about 0:48. 50 games per cell:

| Scenario | Commander AI both sides | Formation only (opening orders) |
| --- | ---: | ---: |
| Arrow vs Line | **70%**, 3.2 min | 100%, on time |
| Line vs Line | 58%, 3.9 min | 44%, 3.6 min |
| Crescent vs Arrow | 50%, 3.0 min | 12%, 6.7 min |
| Echelon vs Line | 44%, 3.8 min | 16%, 3.9 min |
| Columns vs Line | 80%, 2.8 min | 96%, 5.0 min |
| Low supply (Red fires 8% slower) | 80%, 4.0 min | 94%, 3.5 min |

* With commanders on both sides, the arrow beats the line and the even matchups come out roughly even.
* "Formation only" means no division ever changes its opening order. That shows the raw shapes. The arrow wins on time there because it punches through and the line never turns to face it. Columns win for the same reason.
* Hundreds of ships average out the dice, so a small edge (like 8% slower fire) decides most battles.
* **Known weaknesses:**
  * With weapon range 150 and a 37.5 stopping distance, the crescent no longer reliably beats the arrow (50%), and deep columns beat lines (80%).
  * The commander AI loses Line vs Line to a formation-only line (34%). Its flanking moves cost it more than they gain.
  * Both need another tuning pass.

## Architecture

```
src/
  sim/                headless simulation; no rendering, no input
    components.rs     Ship, Pos, Vel, Heading, Side, InDivision, Slot, Weapon, FleetFlagship, Division
    formation.rs      slot layouts per formation
    spatial.rs        uniform-grid spatial hash (neighbours, targeting, centroids)
    systems.rs        the SimTick schedule: steering, blocking, fire, close ranks, morale, breakthroughs, victory
    ai.rs             the division-level commander AI
    scenario.rs       fleet shapes and scenarios
    battle.rs         settings, Tuning (every number), Battle state, SimEvent messages
    rng.rs            seeded Mulberry32
  game/               the Bevy app
    flow.rs           start/restart, applies orders, steps the sim at the chosen speed, log lines
    view.rs           ship meshes (shared mesh and materials, batched), labels, lasers, explosions, order overlays
    camera.rs         pan and zoom around the side panel
    input.rs          mouse, touch and keyboard
    model.rs          per-frame UiModel read model
    ui.rs             menu, side panel, division list, orders, log, rules, result overlay
    autoplay.rs       scripted native smoke run that saves screenshots (dev only)
  bin/sim.rs          balance runner
examples/trace.rs     battle timeline, with optional PPM snapshots (TRACE_PPM=dir)
```

Every ship is an entity, and so is every division. The simulation is ordinary ECS systems in a custom `SimTick` schedule with a fixed step. The game runs that schedule from `FixedUpdate` 0–8 times per step, and the tests and balance runner call it directly on a bare `World`. A seed always replays the same battle.

## Develop

```bash
cargo test --lib                          # sim tests (formations, spatial hash, arcs, morale, blocking, breakthrough, determinism)
cargo run                                 # native window
cargo run --release --bin sim -- 25 240   # balance table
cargo run --release --example trace -- 0 c 1   # timeline of Arrow vs Line, commander AI, seed 1
SG_AUTOPLAY=/tmp/shots cargo run          # scripted run, saves PNGs
./scripts/build-web.sh                    # wasm bundle into dist/ (python3 -m http.server -d dist)
```

GitHub Actions runs the tests, builds the WebAssembly bundle and deploys `dist/` to GitHub Pages on every push to `main`.

The UI font is Noto Sans Mono (subset), © The Noto Project Authors, under the SIL Open Font License 1.1 ([assets/fonts/OFL.txt](assets/fonts/OFL.txt)).
