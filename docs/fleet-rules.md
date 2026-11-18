# Fleet battle rules (v0.3, real time)

These rules replace the hex-board rules v0.2.1 (still in git history). Under v0.2.1 a battle was over in a few dice rolls, with only 9 squadrons a side: too few, and too fast, for a formation to decide anything. In v0.3 every ship is simulated, and formations work through geometry rather than modifiers.

## Scale and time

* Each fleet has 120, 240 or 480 ships in six divisions: five fighting divisions and a flagship reserve.
* The battle runs in real time at 20 steps a second. You can pause, or run it at 1×, 2×, 4× or 8×.
* The battle area is about 3,600 × 2,600 units. Ships keep about 14 units apart in formation.

## Divisions and orders

You command divisions, not ships. Each ship steers for its slot in its division's formation.

| Order | Effect |
| --- | --- |
| Move | Go to a point; drag to set the facing on arrival |
| Engage | Close to firing range of an enemy division and face it; never backs away from it |
| Hold | Stay put |
| Advance | Go straight ahead until in firing range, or level with the nearest enemy |
| Charge | Go straight ahead and do not stop at firing range. Once clear through the enemy, wheel about and engage the nearest division |
| Withdraw | Fall back toward your own edge, still facing the enemy |

Formations are Line (2 ranks), Wedge, Crescent, Echelon left/right, and Column. A division wheels at about 13° a second, so turning a wide line takes time.

## Fire

* Range is 110. Guns fire in a 60° cone ahead of the ship.
* A ship can turn up to about 34° away from its division's facing to aim. So a division attacked from behind cannot answer until it wheels.
* Each ship fires about every 4 s at the nearest enemy in its cone.
* The hit chance depends on which arc of the target the shot comes from:

| Into the target's | front (±60°) | flank | rear |
| --- | ---: | ---: | ---: |
| Hit chance | 12% | 25% | 40% |

  At maximum range the hit chance is halved. A hit does 1 damage; ships have 5 hull and the flagship 10.

## Contact

Enemy ships never pass through or shove each other. A ship simply stops instead of moving into an enemy's space. To get through a line you must destroy the ships in the way. A defending line holds its ground.

Deep formations (Wedge, Column) close ranks every 3 s, so rear ships step into the gaps at the front. A Line does not, so its holes stay open. This is why a wedge can tear a line open: it keeps a fresh front while the line thins at one point.

## Morale

Each division has a stress level, checked every second:

* **Raised by:** shock from recent losses and hits (with a 15 s half-life, and flank and rear hits count more), total losses, the fleet's flagship being lost, and being broken through.
* **Lowered by:** being within 600 of the flagship (in command).

Stress moves a division from Steady → Shaken, then Shaken → Routed. A Shaken division can rally back to Steady.

* **Shaken:** fires 1.5× slower and will not advance.
* **Routed:** runs for its own edge, does not fire, and counts as lost.

A **breakthrough** is when enough of a division's ships get behind an enemy division's rear rank. It is announced, and it shocks the broken division.

## Victory

A fleet breaks when more than half its ships are destroyed, fled or in routed divisions. After 10 minutes, the fleet with more hull still fighting wins.

All numbers live in `Tuning` (`src/sim/battle.rs`), and `cargo run --release --bin sim` measures what a change does.
