//! Battle state and the ship rules: movement, fire, morale, rout, flagship
//! loss, turn order and victory. Every function here operates on a Bevy
//! `World` (squadrons are entities, battle state lives in resources) and
//! never touches rendering or UI, so the same code drives the browser game,
//! the headless tests and the Monte-Carlo `sim` binary.

use std::collections::HashSet;

use bevy::ecs::message::Messages;
use bevy::prelude::*;

use super::components::*;
use super::config::*;
use super::dice::Dice;
use super::formations;
use crate::hex::{self, Arc, Hex};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Options picked on the menu.
#[derive(Resource, Clone, Copy, Debug)]
pub struct MatchSettings {
    pub scenario: usize,
    pub control: ControlMode,
    pub size: usize,
    pub move_mode: MoveMode,
    pub deploy_mode: DeployMode,
    /// AI driving each side when it is not human-controlled.
    pub ai: [AiKind; 2],
    /// Swap the scenario's sides (sim only: lets balance runs average out
    /// the first-mover effect, as the original Python sim did).
    pub mirror: bool,
}

impl Default for MatchSettings {
    fn default() -> Self {
        Self {
            scenario: 0,
            control: ControlMode::PlayBlue,
            size: 9,
            move_mode: MoveMode::Interleaved,
            deploy_mode: DeployMode::Manual,
            ai: [AiKind::Doctrine; 2],
            mirror: false,
        }
    }
}

/// Lifecycle of a battle session. The app mirrors this into its UI state.
#[derive(Resource, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Menu,
    Deployment,
    Combat,
    GameOver,
}

#[derive(Clone, Debug)]
pub struct FleetState {
    pub formation: Formation,
    pub supply: Supply,
    pub flag_lost: bool,
    pub roster: Vec<Entity>,
}

impl FleetState {
    fn new(supply: Supply) -> Self {
        Self { formation: Formation::Custom, supply, flag_lost: false, roster: Vec::new() }
    }
}

#[derive(Clone, Debug)]
pub struct BattleResult {
    pub winner: Option<usize>,
    pub title: String,
    pub body: String,
    pub line: String,
}

#[derive(Resource, Clone, Debug)]
pub struct Battle {
    pub turn: u32,
    pub last_acted: usize,
    pub round_first: usize,
    pub break_at: usize,
    pub fleets: [FleetState; 2],
    pub result: Option<BattleResult>,
    next_order: u32,
}

impl Default for Battle {
    fn default() -> Self {
        Self {
            turn: 0,
            last_acted: 1,
            round_first: 0,
            break_at: 5,
            fleets: [FleetState::new(Supply::Ok), FleetState::new(Supply::Ok)],
            result: None,
            next_order: 0,
        }
    }
}

impl Battle {
    pub fn over(&self) -> bool {
        self.result.is_some()
    }
}

/// The squadron a human is currently activating.
#[derive(Clone, Copy, Debug)]
pub struct Act {
    pub side: usize,
    pub unit: Option<Entity>,
    pub mp: u8,
    pub moved: bool,
    pub fired: bool,
    pub fire_mode: bool,
    /// In command radius when activated: may move AND fire.
    pub cmd: bool,
}

#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Activation(pub Option<Act>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placed {
    pub hex: Hex,
    pub facing: u8,
}

#[derive(Clone, Debug)]
pub struct SetupState {
    pub side: usize,
    pub placed: Vec<Placed>,
    pub selected: Option<usize>,
    pub flagship: Option<usize>,
}

/// Manual deployment in progress (one human side at a time).
#[derive(Resource, Default, Clone, Debug)]
pub struct Deployment {
    pub current: Option<SetupState>,
    pub queue: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Plain,
    Title,
    Bad,
    Good,
    Side(usize),
}

/// Semantic facts published by the rules. Presentation (log, lasers) and
/// tests observe these; the rules never format UI themselves beyond `Note`.
#[derive(Message, Clone, Debug)]
pub enum BattleEvent {
    Note { text: String, tone: Tone },
    MoraleChecked { label: String, roll: u8, modifiers: Vec<&'static str>, total: i32, passed: bool },
    Routed { label: String },
    Destroyed { label: String, side: usize, flagship: bool },
    FlagshipLost { side: usize },
    Shot { attacker: String, target: String, side: usize, arc: Arc, need: u8, rolls: Vec<u8>, hits: u8, from: Hex, to: Hex },
    Fled { label: String },
    Rallied { label: String, roll: u8, bonus: bool },
    RallyFailed { label: String, roll: u8, bonus: bool },
    ShakenRefused { label: String },
}

pub fn emit(world: &mut World, event: BattleEvent) {
    if let Some(mut messages) = world.get_resource_mut::<Messages<BattleEvent>>() {
        messages.write(event);
    }
}

pub fn note(world: &mut World, text: impl Into<String>, tone: Tone) {
    emit(world, BattleEvent::Note { text: text.into(), tone });
}

/// Inserts every resource the rules need into a bare world (tests, sim).
pub fn init_world(world: &mut World, settings: MatchSettings, dice: Dice) {
    world.insert_resource(settings);
    world.insert_resource(dice);
    world.init_resource::<Phase>();
    world.init_resource::<Battle>();
    world.init_resource::<Activation>();
    world.init_resource::<Deployment>();
    world.init_resource::<Messages<BattleEvent>>();
}

// ---------------------------------------------------------------------------
// Read model: a sorted snapshot of every squadron
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub struct Ship {
    pub e: Entity,
    pub order: u32,
    pub hex: Hex,
    pub facing: u8,
    pub side: usize,
    pub strength: u8,
    pub morale: Morale,
    pub flagship: bool,
    pub alive: bool,
    pub activated: bool,
}

/// Every squadron (alive or lost), in spawn order.
pub fn snapshot(world: &mut World) -> Vec<Ship> {
    let mut q = world.query::<(
        Entity,
        &Squadron,
        &HexPos,
        &Facing,
        &Side,
        &Strength,
        &Morale,
        Has<Flagship>,
        Has<Alive>,
        Has<Activated>,
    )>();
    let mut ships: Vec<Ship> = q
        .iter(world)
        .map(|(e, sq, pos, facing, side, strength, morale, flagship, alive, activated)| Ship {
            e,
            order: sq.order,
            hex: pos.0,
            facing: facing.0,
            side: side.0,
            strength: strength.0,
            morale: *morale,
            flagship,
            alive,
            activated,
        })
        .collect();
    ships.sort_by_key(|s| s.order);
    ships
}

pub fn find(ships: &[Ship], e: Entity) -> Option<&Ship> {
    ships.iter().find(|s| s.e == e)
}

pub fn alive(ships: &[Ship]) -> impl Iterator<Item = &Ship> {
    ships.iter().filter(|s| s.alive)
}

pub fn occupied(ships: &[Ship]) -> HashSet<Hex> {
    alive(ships).map(|s| s.hex).collect()
}

pub fn flagship_of(ships: &[Ship], side: usize) -> Option<&Ship> {
    alive(ships).find(|s| s.side == side && s.flagship)
}

pub fn in_command_at(ships: &[Ship], side: usize, at: Hex) -> bool {
    flagship_of(ships, side).is_some_and(|f| hex::dist(at, f.hex) <= CMD_R)
}

pub fn in_command(ships: &[Ship], ship: &Ship) -> bool {
    in_command_at(ships, ship.side, ship.hex)
}

pub fn nearest_enemy(ships: &[Ship], ship: &Ship) -> Option<Ship> {
    let mut best: Option<(i32, Ship)> = None;
    for s in alive(ships).filter(|s| s.side != ship.side) {
        let d = hex::dist(ship.hex, s.hex);
        if best.is_none_or(|(bd, _)| d < bd) {
            best = Some((d, *s));
        }
    }
    best.map(|(_, s)| s)
}

/// Can a squadron at `from` facing `facing` fire at `to` given `occ`?
pub fn can_shoot(from: Hex, facing: u8, to: Hex, occ: &HashSet<Hex>) -> bool {
    hex::dist(from, to) <= RANGE && hex::in_fire_arc(facing, from, to) && hex::los_clear(from, to, |h| occ.contains(&h))
}

pub fn legal_targets(ships: &[Ship], ship: &Ship) -> Vec<Ship> {
    let occ = occupied(ships);
    alive(ships)
        .filter(|t| t.side != ship.side && can_shoot(ship.hex, ship.facing, t.hex, &occ))
        .copied()
        .collect()
}

pub fn label_of(world: &World, e: Entity) -> String {
    world.get::<Callsign>(e).map(|l| l.0.clone()).unwrap_or_default()
}

pub fn losses(world: &mut World, side: usize) -> usize {
    let roster = world.resource::<Battle>().fleets[side].roster.clone();
    roster.iter().filter(|e| !world.entity(**e).contains::<Alive>()).count()
}

pub fn alive_of_side(world: &mut World, side: usize) -> Vec<Entity> {
    let roster = world.resource::<Battle>().fleets[side].roster.clone();
    roster.into_iter().filter(|e| world.entity(*e).contains::<Alive>()).collect()
}

pub fn unactivated_of_side(world: &mut World, side: usize) -> Vec<Entity> {
    alive_of_side(world, side).into_iter().filter(|e| !world.entity(*e).contains::<Activated>()).collect()
}

pub fn surviving_strength(world: &mut World, side: usize) -> u32 {
    alive_of_side(world, side).iter().map(|e| world.get::<Strength>(*e).map_or(0, |s| s.0 as u32)).sum()
}

fn d6(world: &mut World) -> u8 {
    world.resource_mut::<Dice>().d6()
}

fn side_of(world: &World, e: Entity) -> usize {
    world.get::<Side>(e).map_or(0, |s| s.0)
}

/// Fleet-wide morale penalty: supply trouble and a lost flagship.
pub fn fleet_modifier(world: &World, side: usize) -> i32 {
    let fleet = &world.resource::<Battle>().fleets[side];
    (if fleet.supply != Supply::Ok { -1 } else { 0 }) + (if fleet.flag_lost { -1 } else { 0 })
}

// ---------------------------------------------------------------------------
// Morale, destruction, fire
// ---------------------------------------------------------------------------

/// d6 + modifiers >= 4 holds. Failing steps Steady -> Shaken -> Routed; a
/// rout forces the unit to face its own edge and spreads to friends within 2.
/// Returns `None` if no check happened (lost, or already routed).
pub fn morale_check(world: &mut World, e: Entity, flanked: bool, extra: i32) -> Option<bool> {
    let ships = snapshot(world);
    let me = *find(&ships, e)?;
    if !me.alive || me.morale == Morale::Routed {
        return None;
    }
    let support = alive(&ships)
        .any(|v| v.side == me.side && v.e != e && v.morale == Morale::Steady && hex::dist(me.hex, v.hex) == 1);
    let command = in_command(&ships, &me);
    let roll = d6(world);
    let total = roll as i32 + support as i32 + command as i32 - flanked as i32 + extra;
    let passed = total >= 4;
    let fleet = world.resource::<Battle>().fleets[me.side].clone();
    let mut modifiers = Vec::new();
    if support {
        modifiers.push("+1 support");
    }
    if command {
        modifiers.push("+1 command");
    }
    if flanked {
        modifiers.push("−1 flanked");
    }
    if fleet.supply != Supply::Ok {
        modifiers.push("−1 supply");
    }
    if fleet.flag_lost {
        modifiers.push("−1 flagship");
    }
    let label = label_of(world, e);
    emit(world, BattleEvent::MoraleChecked { label: label.clone(), roll, modifiers, total, passed });
    if !passed {
        if me.morale == Morale::Steady {
            world.entity_mut(e).insert(Morale::Shaken);
        } else {
            world.entity_mut(e).insert((Morale::Routed, Facing(home_edge_dir(me.side))));
            emit(world, BattleEvent::Routed { label });
            contagion(world, e, extra);
        }
    }
    Some(passed)
}

/// Friends within 2 hexes of a routed or destroyed squadron check morale.
pub fn contagion(world: &mut World, src: Entity, extra: i32) {
    let ships = snapshot(world);
    let Some(source) = find(&ships, src).copied() else { return };
    let friends: Vec<Entity> =
        alive(&ships).filter(|v| v.side == source.side && v.e != src).map(|v| v.e).collect();
    for v in friends {
        let now = snapshot(world);
        let Some(ship) = find(&now, v) else { continue };
        let src_hex = find(&now, src).map_or(source.hex, |s| s.hex);
        if ship.alive && ship.morale != Morale::Routed && hex::dist(ship.hex, src_hex) <= 2 {
            morale_check(world, v, false, extra);
        }
    }
}

pub fn destroy(world: &mut World, e: Entity, extra: i32) {
    world.entity_mut(e).remove::<Alive>();
    let flagship = world.entity(e).contains::<Flagship>();
    let side = side_of(world, e);
    let label = label_of(world, e);
    emit(world, BattleEvent::Destroyed { label, side, flagship });
    contagion(world, e, extra);
    if flagship {
        flagship_lost(world, side);
    }
}

fn flagship_lost(world: &mut World, side: usize) {
    world.resource_mut::<Battle>().fleets[side].flag_lost = true;
    emit(world, BattleEvent::FlagshipLost { side });
    for v in alive_of_side(world, side) {
        let m = fleet_modifier(world, side);
        morale_check(world, v, false, m);
    }
}

#[derive(Clone, Debug)]
pub struct ShotResult {
    pub hits: u8,
    pub arc: Arc,
    pub need: u8,
}

/// Roll one d6 per strength point (Shaken: half, rounded up). The number
/// needed depends on which arc of the *target* the firer sits in.
pub fn fire(world: &mut World, e: Entity, tgt: Entity) -> ShotResult {
    let ships = snapshot(world);
    let me = *find(&ships, e).expect("firer exists");
    let t = *find(&ships, tgt).expect("target exists");
    let extra = fleet_modifier(world, t.side);
    let critical = world.resource::<Battle>().fleets[me.side].supply == Supply::Critical;
    let dice = if me.morale == Morale::Steady { me.strength } else { me.strength.div_ceil(2) };
    let arc = hex::incoming_arc(t.hex, t.facing, me.hex);
    let need = match arc {
        Arc::Front => 5,
        Arc::Flank => 4,
        Arc::Rear => 3,
    } + critical as u8;
    let rolls: Vec<u8> = (0..dice).map(|_| d6(world)).collect();
    let hits = rolls.iter().filter(|r| **r >= need).count() as u8;
    let (attacker, target) = (label_of(world, e), label_of(world, tgt));
    emit(world, BattleEvent::Shot { attacker, target, side: me.side, arc, need, rolls, hits, from: me.hex, to: t.hex });
    if hits > 0 {
        let left = t.strength.saturating_sub(hits);
        world.entity_mut(tgt).insert((HitSinceAct, Strength(left)));
        if left == 0 {
            destroy(world, tgt, extra);
        } else {
            morale_check(world, tgt, arc != Arc::Front, extra);
        }
    }
    ShotResult { hits, arc, need }
}

// ---------------------------------------------------------------------------
// Movement
// ---------------------------------------------------------------------------

pub fn set_facing(world: &mut World, e: Entity, dir: u8) {
    world.entity_mut(e).insert(Facing(dir % 6));
}

pub fn set_hex(world: &mut World, e: Entity, h: Hex) {
    world.entity_mut(e).insert(HexPos(h));
}

#[derive(Debug, PartialEq, Eq)]
pub enum StepError {
    Blocked,
    /// Shaken squadrons may not end a step closer to the nearest enemy.
    Shaken,
}

pub fn step_into(world: &mut World, e: Entity, dir: u8) -> Result<(), StepError> {
    let ships = snapshot(world);
    let me = *find(&ships, e).expect("ship exists");
    let next = hex::neighbor(me.hex, dir);
    if !in_bounds(next) || occupied(&ships).contains(&next) {
        return Err(StepError::Blocked);
    }
    if me.morale == Morale::Shaken
        && let Some(ne) = nearest_enemy(&ships, &me)
        && hex::dist(next, ne.hex) < hex::dist(me.hex, ne.hex)
    {
        return Err(StepError::Shaken);
    }
    set_hex(world, e, next);
    Ok(())
}

/// A routed squadron spends its whole move fleeing toward its own edge.
pub fn flee(world: &mut World, e: Entity) {
    let side = side_of(world, e);
    let d = home_edge_dir(side);
    for _ in 0..MP_MAX {
        let ships = snapshot(world);
        let me = *find(&ships, e).expect("ship exists");
        if me.facing != d {
            set_facing(world, e, hex::turn_toward(me.facing, d));
            continue;
        }
        let next = hex::neighbor(me.hex, d);
        if !in_bounds(next) {
            world.entity_mut(e).remove::<Alive>();
            let label = label_of(world, e);
            emit(world, BattleEvent::Fled { label });
            return;
        }
        if !occupied(&ships).contains(&next) {
            set_hex(world, e, next);
        }
    }
}

/// A routed squadron that took no hits since it last acted may rally
/// (4+, +1 in command) back to Shaken; otherwise it keeps fleeing.
pub fn routed_activation(world: &mut World, e: Entity) {
    let label = label_of(world, e);
    if !world.entity(e).contains::<HitSinceAct>() {
        let ships = snapshot(world);
        let bonus = find(&ships, e).is_some_and(|s| in_command(&ships, s));
        let roll = d6(world);
        if roll + bonus as u8 >= 4 {
            world.entity_mut(e).insert(Morale::Shaken).remove::<HitSinceAct>();
            emit(world, BattleEvent::Rallied { label, roll, bonus });
            return;
        }
        emit(world, BattleEvent::RallyFailed { label: label.clone(), roll, bonus });
    }
    world.entity_mut(e).remove::<HitSinceAct>();
    flee(world, e);
}

// ---------------------------------------------------------------------------
// Battle lifecycle and turn order
// ---------------------------------------------------------------------------

pub fn spawn_unit(world: &mut World, side: usize, h: Hex, facing: u8, flagship: bool) -> Entity {
    let (order, index) = {
        let mut battle = world.resource_mut::<Battle>();
        battle.next_order += 1;
        (battle.next_order, battle.fleets[side].roster.len())
    };
    let label = format!("{}{}", if side == 0 { 'B' } else { 'R' }, index + 1);
    let mut entity = world.spawn((
        Squadron { order },
        HexPos(h),
        Facing(facing),
        Side(side),
        Strength(MAX_STRENGTH),
        Morale::Steady,
        Callsign(label),
        Alive,
    ));
    if flagship {
        entity.insert(Flagship);
    }
    let e = entity.id();
    world.resource_mut::<Battle>().fleets[side].roster.push(e);
    e
}

pub fn deploy_formation(world: &mut World, formation: Formation, side: usize) -> Vec<Entity> {
    let size = world.resource::<MatchSettings>().size;
    let layout = formations::layout(formation, size);
    world.resource_mut::<Battle>().fleets[side].formation = formation;
    let straight = if side == 0 { 0 } else { 3 };
    let to_pos = if side == 0 { 5 } else { 4 };
    let to_neg = if side == 0 { 1 } else { 2 };
    let entities: Vec<Entity> = layout
        .slots
        .iter()
        .enumerate()
        .map(|(i, &(fwd, lat, df))| {
            let c = if side == 0 { DEPLOY_ANCHOR[0] + fwd } else { DEPLOY_ANCHOR[1] - fwd };
            let facing = match df.signum() {
                0 => straight,
                1 => to_pos,
                _ => to_neg,
            };
            spawn_unit(world, side, Hex::new(c, DEPLOY_ROW_CENTER + lat), facing, i == layout.flag)
        })
        .collect();
    if formation == Formation::Sphere {
        // Ring faces outward from its centre.
        let centre = world.get::<HexPos>(entities[0]).unwrap().0;
        for e in &entities[1..] {
            let p = world.get::<HexPos>(*e).unwrap().0;
            let d = hex::desired_dir(centre, p);
            set_facing(world, *e, d);
        }
    }
    entities
}

/// Clears the board and starts the scenario in `MatchSettings`.
pub fn new_battle(world: &mut World) {
    let old: Vec<Entity> = world.query_filtered::<Entity, With<Squadron>>().iter(world).collect();
    for e in old {
        world.despawn(e);
    }
    let settings = *world.resource::<MatchSettings>();
    let scenario = &SCENARIOS[settings.scenario];
    let (form, supply) = if settings.mirror {
        ([scenario.b, scenario.a], [scenario.supply_b, scenario.supply_a])
    } else {
        ([scenario.a, scenario.b], [scenario.supply_a, scenario.supply_b])
    };
    let mut battle = Battle {
        break_at: settings.size / 2 + 1,
        fleets: [FleetState::new(supply[0]), FleetState::new(supply[1])],
        ..default()
    };
    battle.next_order = world.resource::<Battle>().next_order;
    world.insert_resource(battle);
    world.insert_resource(Activation(None));
    world.insert_resource(Deployment::default());
    let break_at = settings.size / 2 + 1;
    note(
        world,
        format!("Scenario: {} — {} squadrons a side, breaks at {}", scenario.title, settings.size, break_at),
        Tone::Title,
    );
    if settings.control == ControlMode::Spectate || settings.deploy_mode == DeployMode::Fixed {
        deploy_formation(world, form[0], 0);
        deploy_formation(world, form[1], 1);
        start_combat(world);
        return;
    }
    let humans: Vec<usize> = (0..2).filter(|s| settings.control.human_controls(*s)).collect();
    world.resource_mut::<Deployment>().queue = humans[1..].to_vec();
    begin_setup(world, humans[0]);
}

fn begin_setup(world: &mut World, side: usize) {
    *world.resource_mut::<Phase>() = Phase::Deployment;
    world.resource_mut::<Deployment>().current =
        Some(SetupState { side, placed: Vec::new(), selected: None, flagship: None });
    world.insert_resource(Activation(None));
    note(world, format!("{}: deploy your squadrons — click your shaded zone.", side_name(side)), Tone::Title);
}

pub fn start_combat(world: &mut World) {
    *world.resource_mut::<Phase>() = Phase::Combat;
    let fleets = world.resource::<Battle>().fleets.clone();
    let tag = |f: &FleetState| {
        let sup = if f.supply == Supply::Ok { String::new() } else { format!(" ({} SUPPLY)", f.supply.name().to_uppercase()) };
        format!("{}{}", f.formation.name(), sup)
    };
    note(world, format!("Blue: {} — Red: {}", tag(&fleets[0]), tag(&fleets[1])), Tone::Plain);
    new_turn(world);
}

pub fn new_turn(world: &mut World) {
    let activated: Vec<Entity> = world.query_filtered::<Entity, With<Activated>>().iter(world).collect();
    for e in activated {
        world.entity_mut(e).remove::<Activated>();
    }
    let control = world.resource::<MatchSettings>().control;
    let move_mode = world.resource::<MatchSettings>().move_mode;
    let mut battle = world.resource_mut::<Battle>();
    battle.turn += 1;
    let turn = battle.turn;
    // Side (turn + 1) % 2 acts first, mirroring the sim.
    battle.last_acted = (turn % 2) as usize;
    if move_mode == MoveMode::AllAtOnce {
        let humans: Vec<usize> = (0..2).filter(|s| control.human_controls(*s)).collect();
        battle.round_first = if humans.len() == 1 { humans[0] } else { ((turn + 1) % 2) as usize };
    }
    note(world, format!("— Turn {turn} —"), Tone::Title);
}

/// Ends the battle if a fleet has broken. Returns true if it is over.
pub fn check_break(world: &mut World) -> bool {
    if world.resource::<Battle>().over() {
        return true;
    }
    let break_at = world.resource::<Battle>().break_at;
    for s in 0..2 {
        if losses(world, s) >= break_at {
            game_over(world, Some(1 - s), true);
            return true;
        }
    }
    false
}

/// Break check, then the turn-limit draw, then rolls over to a new turn
/// once every squadron on both sides has activated.
pub fn check_end(world: &mut World) -> bool {
    if check_break(world) {
        return true;
    }
    if unactivated_of_side(world, 0).is_empty() && unactivated_of_side(world, 1).is_empty() {
        if world.resource::<Battle>().turn >= MAX_TURNS {
            let (s0, s1) = (surviving_strength(world, 0), surviving_strength(world, 1));
            let winner = if s0 == s1 { None } else if s0 > s1 { Some(0) } else { Some(1) };
            game_over(world, winner, false);
            return true;
        }
        new_turn(world);
    }
    false
}

pub fn next_side(world: &mut World) -> Option<usize> {
    let mode = world.resource::<MatchSettings>().move_mode;
    let battle = world.resource::<Battle>();
    let (first, second) = if mode == MoveMode::AllAtOnce {
        (battle.round_first, 1 - battle.round_first)
    } else {
        (1 - battle.last_acted, battle.last_acted)
    };
    if !unactivated_of_side(world, first).is_empty() {
        Some(first)
    } else if !unactivated_of_side(world, second).is_empty() {
        Some(second)
    } else {
        None
    }
}

fn game_over(world: &mut World, winner: Option<usize>, by_break: bool) {
    let (s0, s1) = (surviving_strength(world, 0), surviving_strength(world, 1));
    let (l0, l1) = (losses(world, 0), losses(world, 1));
    let settings = *world.resource::<MatchSettings>();
    let battle = world.resource::<Battle>().clone();
    let turn = battle.turn;
    let (title, mut body) = match winner {
        None => ("Draw".to_string(), format!("Both fleets stand at turn {turn}.")),
        Some(w) => (
            format!("{} wins", side_name(w)),
            if by_break {
                let lost = if w == 0 { l1 } else { l0 };
                format!("{}'s fleet breaks on turn {turn} ({lost} squadrons destroyed or fled).", side_name(1 - w))
            } else {
                format!("On time at turn {turn}: surviving strength {s0}–{s1}.")
            },
        ),
    };
    if battle.fleets.iter().any(|f| f.formation == Formation::Sphere) {
        body.push_str(&format!(" (Sphere survival score: {turn} turns.)"));
    }
    let winner_tag = match winner {
        None => "draw".to_string(),
        Some(w) => format!("{} ({})", side_name(w), battle.fleets[w].formation.name()),
    };
    let line = format!(
        "{} | ctrl={} | size={} | ai={} | winner={} | turn={} | strength {}-{} | losses {}-{}",
        SCENARIOS[settings.scenario].title,
        settings.control.tag(),
        settings.size,
        if settings.ai[0] == settings.ai[1] { settings.ai[0].name().to_string() } else { format!("{}/{}", settings.ai[0].name(), settings.ai[1].name()) },
        winner_tag,
        turn,
        s0,
        s1,
        l0,
        l1
    );
    note(world, format!("BATTLE OVER — {title}. {body}"), Tone::Title);
    world.resource_mut::<Battle>().result = Some(BattleResult { winner, title, body, line });
    world.insert_resource(Activation(None));
    *world.resource_mut::<Phase>() = Phase::GameOver;
}

/// What the battle is waiting for next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    Over,
    /// A human side must pick (or is activating) a squadron.
    Human(usize),
    /// An AI side should activate one squadron.
    Ai(usize),
}

/// Advances turn bookkeeping until someone has to act.
pub fn advance(world: &mut World) -> Next {
    if *world.resource::<Phase>() != Phase::Combat {
        return Next::Over;
    }
    if let Some(act) = world.resource::<Activation>().0 {
        return Next::Human(act.side);
    }
    for _ in 0..4 {
        if check_end(world) {
            return Next::Over;
        }
        let Some(side) = next_side(world) else { continue };
        let control = world.resource::<MatchSettings>().control;
        if control.human_controls(side) {
            world.insert_resource(Activation(Some(Act {
                side,
                unit: None,
                mp: 0,
                moved: false,
                fired: false,
                fire_mode: false,
                cmd: false,
            })));
            return Next::Human(side);
        }
        return Next::Ai(side);
    }
    Next::Over
}

// ---------------------------------------------------------------------------
// Human activation predicates
// ---------------------------------------------------------------------------

pub fn can_move(act: &Act) -> bool {
    act.unit.is_some() && act.mp > 0 && (act.cmd || !act.fired)
}

pub fn can_back(act: &Act) -> bool {
    can_move(act) && act.mp >= MP_MAX
}

pub fn can_fire(world: &mut World, act: &Act) -> bool {
    let Some(u) = act.unit else { return false };
    if act.fired || !(act.cmd || !act.moved) {
        return false;
    }
    let ships = snapshot(world);
    find(&ships, u).is_some_and(|me| !legal_targets(&ships, me).is_empty())
}

pub fn can_switch_selection(act: &Act) -> bool {
    act.unit.is_some() && !act.moved && !act.fired
}
