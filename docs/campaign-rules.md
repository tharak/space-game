# Campaign rules (v0.1)

You command the **Meridian Compact** (blue, west) against the **Hesper Throne** (red, east, AI). The galaxy is split by the **Shroud**, a nebula no fleet can cross except through two corridors:

* **Vigil Gate** (north) is a fortress. Its garrison starts at 150 ships and rebuilds by 10 a turn. Its guns give the garrison +25% fire and steadier morale.
* **Tollmarch** (south) is a neutral trade world worth +3 ships a turn. It is lightly held, with 40 ships.

Each side starts with 9 systems, including its capital, and four fleets of 100–120 ships. **Take the enemy capital to win.** After 120 turns, the side with more systems wins.

## A turn

1. **Orders.** Select a fleet, then click a system to send it there by the shortest route. A one-lane hop takes a turn; crossing the Shroud takes two. A system holds at most 3 fleets per side.
2. **Movement.** All fleets move at once. Enemy fleets that cross on the same lane, or meet inside a corridor, fight in the lane.
3. **Battles.** A battle happens wherever both sides meet, or where you enter a system that still has a hostile garrison. For each battle involving you:
   * **Fight it** on the battle screen, choosing your formation first.
   * **Auto-resolve** it with the same simulation, unseen.
   * **Withdraw**: lose 10% of the fleet as stragglers and fall back.

   The losing side falls back to where it came from, or to an adjacent friendly system. With nowhere to go, the fleet and its admiral are lost; a replacement admiral raises a 40-ship fleet at the capital 3 turns later.
4. **Capture.** A system with only your fleets in it and no garrison left becomes yours.
5. **Supply.** A system is supplied if it connects to its owner's capital through the owner's own systems. A fleet is in supply in, or next to, a supplied friendly system. Out of supply:

   | Turns out | State | In battle |
   | --- | --- | --- |
   | 1–2 | Low | −8% fire rate, shakier morale |
   | 3+ | Critical | −20% fire rate, much shakier morale |

6. **Income and repair.** Each supplied system adds its value (1–3) in new ships. These go to your fleets standing there, up to 160 ships each, and otherwise to the system's garrison. Fleets in supplied friendly systems repair a quarter of their damaged ships each turn.

## Fleets and admirals

* **Persistence.** Fleet sizes carry over between battles. Losses are spread over the fleets that fought, in proportion to their size. Ships left below 60% hull come out *damaged*, and damaged ships fight at half hull until repaired.
* **Raise fleet.** Turns a system's garrison into a new fleet. A capital keeps 40 ships back. A side can have up to 8 fleets.
* **Absorb garrison.** Tops a fleet up from the garrison where it stands.
* **Formation.** Each fleet has a preferred formation (Line, Arrow, Crescent, Echelon, Columns), which you can change before each battle. The AI picks the counter to yours.
* **Admirals.** An admiral gains experience for each battle won and is promoted at 2, 5 and 9 wins. Each promotion brings a trait, which applies when that admiral is the senior one in a battle:

  | Trait | Effect |
  | --- | --- |
  | Gunnery | +8% fire rate |
  | Steadfast | Steadier morale |
  | Maneuver | +20% speed and wheeling |
  | Logistician | Supply penalties start a turn later |

## Intel

You see enemy fleets only in, or next to, your own systems and fleets.

The campaign saves itself every turn: to browser storage on the web, and to `~/.local/share/successor-stars/` natively. Use **Continue** on the menu to resume.
