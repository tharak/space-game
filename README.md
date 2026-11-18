# Successor Stars — battle board (Rust · Bevy · ECS)

A rebuild of the [BattleStars](https://github.com/tharak/BattleStars) `battle.html` prototype, the hex fleet-battle board that passed its playtest, in Rust with [Bevy 0.19](https://bevy.org) and its ECS. It plays the same **Fleet Battle Rules v0.2.1** ([docs/battle-rules.md](docs/battle-rules.md)) and adds a new opponent AI built from the playtest findings.

**Play:** <https://tharak.github.io/space-game/> (needs a browser with WebGL2)

## How to play

Pick a control mode, fleet size, movement mode, deployment mode and opponent AI on the menu, then a scenario.

| Input | Action |
| --- | --- |
| Click/tap your squadron | Activate it (you can switch until you move or fire) |
| `Q` / `E` or **Turn left / Turn right** | Turn 60° (1 MP) |
| `W` or **Forward** | Move one hex ahead (1 MP) |
| `S` or **Back** | One hex astern, keeping facing (all 3 MP) |
| Click a red-outlined enemy (`F` highlights) | Fire |
| `Space` or **End activation** | Done with this squadron |

* Squadrons within 4 hexes of their flagship are **in command**: move *and* fire, in either order. Out of command they move *or* fire.
* Hits need 5+ / 4+ / 3+ into the target's front / flank / rear arc. Morale fails go Steady → Shaken → Routed, and routs spread.
* A fleet breaks when more than half its squadrons are destroyed or have fled.

The full rules are in the in-game crib sheet and in [docs/battle-rules.md](docs/battle-rules.md).

## What was ported

The scenario menu (9 scenarios), the control modes (play Blue/Red, hotseat, spectate with Step/Auto), 5/9/12-squadron fleets, interleaved and all-at-once activation, manual and fixed deployment, the hexagonal 27×27 board, fire-zone and command-reach shading, legal-target outlines, laser beams, the battle log, the rules crib sheet, and the result overlay with **Copy result line**. The result line now also records which AI played (`ai=doctrine`).

## Doctrine AI (new)

The playtest found that the old AI's flagship "charges recklessly" and dies, so every battle turned into flagship sniping. The **doctrine** AI is the default opponent. For each squadron it searches every reachable pose this activation and scores each plan by:

* **Attack:** expected hits by arc, with bonuses for likely kills, flank/rear shots, shaken targets and the enemy flagship.
* **Exposure:** how much fire the pose invites next turn, weighting flank/rear exposure higher.
* **Formation:** staying in command range and keeping adjacent support and cohesion.
* **Pressure:** a will to close distance that grows every turn.

The flagship keeps station behind the friendly line and inside command range of its squadrons, and moves last unless it is threatened. Squadrons in command can **shoot and scoot** (fire, then move away). The original advance-and-shoot logic is still selectable as **Baseline**, for comparisons with the old sim numbers.

## Balance runs

`cargo run --release --bin sim -- 200 9` plays every scenario AI-vs-AI, with each formation taking each seat half the time, as the original Python sim did. Cells are the first-named side's win % (400 games each):

| Scenario | Baseline mirror | Doctrine mirror | Doctrine A vs baseline B | Baseline A vs doctrine B |
| --- | ---: | ---: | ---: | ---: |
| Spindle vs Wide Line | 54 | 54 | 92 | 4 |
| Wide Line vs Crescent | 35 | 59 | 84 | 6 |
| Crescent vs Spindle | 62 | 52 | 98 | 22 |
| Echelon vs Spindle | 81 | 44 | 98 | 5 |
| Sphere vs Crescent | 19 | 28 | 79 | 2 |
| Wide Line vs Column | 86 | 75 | 100 | 24 |
| Low-supply mirror (normal side) | 62 | 70 | 97 | 18 |
| Flagship hunt (mirror) | 48 | 48 | 94 | 8 |
| Critical-supply mirror (normal side) | 97 | 98 | 100 | 77 |

* **Does the port match the old sim?** The baseline AI matches the old Python sim wherever the board shape doesn't matter: supply 62 vs 66, critical 97 vs 96, line vs column 86 vs 88, line vs crescent 35 vs 36. The spindle matchups differ because this board (like the old web page) is a hexagon, while the Python sim used a rectangle.
* **Doctrine beats baseline** 79–100% from either seat. Doctrine mirrors resolve in about 5–9 turns, and low supply lands at 70/30, inside the 60–75% target band.
* **First-mover effect:** Blue always opens turn 1. Without mirrored seats, the baseline line-vs-line mirror goes 23/77 to Red, because whoever walks into range first gets shot first. That's worth a rules decision; the paper rules roll off for first activation.

## Architecture

```
src/
  hex.rs            pure hex math (odd-r offset grid, arcs, LOS — exact port incl. JS rounding)
  rules/            headless battle rules; no rendering, no input
    components.rs   Squadron, HexPos, Facing, Side, Strength, Morale, Flagship, Alive, Activated, ...
    battle.rs       resources (MatchSettings, Battle, Activation, Deployment, Phase),
                    fire / morale / rout / flagship loss / turn order / victory over a Bevy World
    commands.rs     BattleCommand message: the player-intent vocabulary
    ai.rs           Baseline (port) and Doctrine AIs
    dice.rs         seeded or scripted d6 resource (Mulberry32, as in the original)
    formations.rs   formation templates (positions + facings only)
  game/             the Bevy app
    flow.rs         applies commands, paces AI activations, turns BattleEvents into log lines
    model.rs        per-frame UiModel read model built from the rules world
    board.rs        camera fit, hex tiles, squadron visuals as child entities, gizmo outlines, lasers, picking
    ui.rs           menu, side panel, log, crib sheet, result overlay (bevy_ui)
    autoplay.rs     scripted native smoke run that saves screenshots (dev only)
  bin/sim.rs        Monte-Carlo balance runner
```

Each squadron is one ECS entity. The rules mutate components and resources and publish `BattleEvent` messages. Presentation systems observe those events (the log and lasers) and read components (the tokens). Because the rules take only a `World`, the same code runs in the browser, in tests and in the sim.

## Develop

```bash
cargo test --lib                          # rules tests (arcs, LOS, morale, rout, deployment, every scenario terminates)
cargo run                                 # native window
cargo run --release --bin sim -- 200 9    # balance table
SG_AUTOPLAY=/tmp/shots cargo run          # scripted run: menu → battle → deployment → spectate, saves PNGs
./scripts/build-web.sh                    # wasm bundle into dist/ (python3 -m http.server -d dist)
```

GitHub Actions runs the tests, builds the WebAssembly bundle and deploys `dist/` to GitHub Pages on every push to `main`.

The UI font is Noto Sans Mono (subset), © The Noto Project Authors, under the SIL Open Font License 1.1 ([assets/fonts/OFL.txt](assets/fonts/OFL.txt)).
