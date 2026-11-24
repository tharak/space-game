//! prototype02's motherships: carriers with a population. A mothership
//! carries its ships inside and launches them (its wing) only when a
//! hostile fleet or planet comes near; they dock again when it's calm. Its
//! people run the rebellion model with legitimacy = morale, and the ones
//! working (not rebelling, not in the brig) crew it: speed, building ships
//! aboard, launching and repairs all scale with them. Morale falls with
//! shortages, crowding (more ships than the people can crew), losses and
//! hull damage, and rises with victories; past the revolt share the rebels
//! leave in escape pods, and the population drops. Its ships keep order
//! (cops) whether aboard or out fighting. It is not a Voronoi site.

use bevy::prelude::Entity;

use super::dominion::{Dominion, FOOD, MAINTENANCE_SECS, Owner, PARTS, SHIP_CAP, Stock, UNREST_SMOOTH_SECS, UNREST_STEP_SECS};
use super::rebellion::Rebellion;
use crate::sim::components::{Division, InDivision, MothershipHull, Order, Pos, Ship, Stores, Supply};
use crate::voronoi::Vec2;

/// Seconds escape pods stay on the map.
pub const POD_SECS: f32 = 3.0;
/// Seconds between carrier decisions (launch, target, recall).
pub const CARRIER_SECS: f32 = 0.5;
/// Seconds without a threat before the wing comes home.
pub const RECALL_SECS: f32 = 3.0;

#[derive(Clone, Debug)]
pub struct Mothership {
    pub id: u32,
    pub owner: Owner,
    /// Its hull in the ECS, and the fleet that holds it.
    pub ship: Entity,
    pub fleet: u32,
    pub people: Rebellion,
    /// 0..1; it is also the people's legitimacy.
    pub morale: f32,
    /// Smoothed rebel share.
    pub unrest: f32,
    pub stores: Stock,
    /// Ships aboard.
    pub carried: u32,
    /// Progress toward the next ship built aboard, 0..1.
    pub build: f32,
    /// Progress toward the next launch.
    pub launch: f32,
    /// The fleet of its launched ships.
    pub wing: Option<u32>,
    /// Ships the wing should have (launched minus docked); fewer means losses.
    pub wing_expected: u32,
    /// Seconds without a threat in reach.
    pub calm_for: f32,
    /// People who left in escape pods, in all.
    pub escaped: u32,
    pub alive: bool,
    /// Since the last maintenance phase: ships lost, enemy ships destroyed
    /// near its wing, planets its wing took.
    pub lost: u32,
    pub kills: u32,
    pub captures: u32,
    /// Hull at the last maintenance phase (to see the damage taken).
    pub hull_seen: f32,
    pub cops: usize,
}

impl Mothership {
    /// The share of its people at work: not rebelling, not in the brig.
    pub fn working(&self) -> f32 {
        (1.0 - self.people.active_share() - self.people.jailed_share()).clamp(0.0, 1.0)
    }

    /// People needed to crew the ships aboard.
    pub fn crew_needed(&self, crew_per_ship: f32) -> f32 {
        self.carried as f32 * crew_per_ship
    }

    /// How far the ships aboard outnumber what its people can crew (0 = fine).
    pub fn crowding(&self, crew_per_ship: f32) -> f32 {
        (self.crew_needed(crew_per_ship) / self.people.population().max(1) as f32 - 1.0).max(0.0)
    }
}

impl Dominion {
    /// A new mothership for `owner` at `pos`, carrying `carried` ships.
    pub fn add_mothership(&mut self, owner: Owner, pos: Vec2, carried: u32) -> u32 {
        let t = self.tuning;
        let id = self.motherships.len() as u32 + 1;
        let fleet = self.spawn(owner, pos, 0.0, 0, None);
        let fi = self.fleets.len() - 1;
        let division = self.fleets[fi].division;
        let at = pos / self.unit();
        let ship = crate::sim::add_ship(&mut self.world, division, at).expect("a hull");
        self.world.entity_mut(ship).insert((MothershipHull(id), Ship { hull: t.mothership_hull, max_hull: t.mothership_hull }));
        self.world.entity_mut(ship).remove::<Stores>();
        self.fleets[fi].mothership = Some(id);
        let people = Rebellion::with_population(self.cfg.seed as i64 * 41 + id as i64 * 7, t.start_population as usize);
        self.motherships.push(Mothership {
            id,
            owner,
            ship,
            fleet,
            people,
            morale: 1.0,
            unrest: 0.0,
            stores: t.mothership_cap.map(|v| v * 0.5),
            carried,
            build: 0.0,
            launch: 0.0,
            wing: None,
            wing_expected: 0,
            calm_for: RECALL_SECS,
            escaped: 0,
            alive: true,
            lost: 0,
            kills: 0,
            captures: 0,
            hull_seen: t.mothership_hull,
            cops: 0,
        });
        self.sync();
        id
    }

    pub fn mothership(&self, id: u32) -> Option<&Mothership> {
        self.motherships.iter().find(|m| m.id == id)
    }

    /// The mothership whose hull is this ship.
    pub fn mothership_of_ship(&self, e: Entity) -> Option<&Mothership> {
        self.motherships.iter().find(|m| m.alive && m.ship == e)
    }

    /// Ships taken aboard mothership `id` (they leave the map; their stores
    /// go into its stores).
    pub fn board(&mut self, id: u32, ships: &[Entity]) {
        let Some(mi) = self.motherships.iter().position(|m| m.id == id && m.alive) else { return };
        let cap = self.tuning.mothership_cap;
        let mut by_division: std::collections::HashMap<Entity, Vec<Entity>> = std::collections::HashMap::new();
        for &e in ships {
            if self.world.get::<MothershipHull>(e).is_some() {
                continue;
            }
            let Some(d) = self.world.get::<InDivision>(e).map(|d| d.0) else { continue };
            let m = &mut self.motherships[mi];
            if let Some(s) = self.world.get::<Stores>(e) {
                (0..3).for_each(|k| m.stores[k] = (m.stores[k] + s.0[k]).min(cap[k]));
            }
            if let Some(s) = self.world.get::<Supply>(e) {
                (0..3).for_each(|k| m.stores[k] = (m.stores[k] + s.0[k]).min(cap[k]));
            } else {
                m.carried += 1;
            }
            by_division.entry(d).or_default().push(e);
        }
        for (d, list) in by_division {
            crate::sim::despawn_ships(&mut self.world, d, &list);
        }
        self.sync();
    }

    /// Every tick: notice destroyed motherships; every half second, carrier
    /// decisions (launch, target, recall, dock) and the people's step.
    pub(crate) fn run_motherships(&mut self, dt: f32) {
        if self.motherships.is_empty() {
            return;
        }
        for mi in 0..self.motherships.len() {
            let m = &self.motherships[mi];
            if m.alive && self.world.get_entity(m.ship).is_err() {
                // Destroyed with everyone aboard; the wing fights on alone.
                let wing = m.wing;
                self.motherships[mi].alive = false;
                if let Some(f) = wing.and_then(|w| self.fleet_index(w)) {
                    self.fleets[f].wing_of = None;
                }
            }
        }
        self.mother_timer += dt;
        if self.mother_timer < CARRIER_SECS - 1e-4 {
            return;
        }
        self.mother_timer = 0.0;
        for mi in 0..self.motherships.len() {
            if self.motherships[mi].alive {
                self.carrier(mi, CARRIER_SECS);
            }
        }
        // The people, every UNREST_STEP_SECS (the same cadence as planets).
        self.mother_people_timer += CARRIER_SECS;
        if self.mother_people_timer >= UNREST_STEP_SECS - 1e-4 {
            self.mother_people_timer = 0.0;
            let t = self.tuning;
            for mi in 0..self.motherships.len() {
                let m = &mut self.motherships[mi];
                if !m.alive {
                    continue;
                }
                let out = m.wing_expected;
                let m = &mut self.motherships[mi];
                // Its ships keep order aboard even when they are out fighting.
                m.cops = (((m.carried + out) as f32 * t.cops_per_ship / 3.0).round() as usize).min(super::dominion::MAX_COPS);
                let (legit, cops) = (m.morale, m.cops);
                m.people.step(legit, 0.0, cops, &self.rebellion);
                let share = m.people.active_share();
                m.unrest += (share - m.unrest) * UNREST_STEP_SECS / UNREST_SMOOTH_SECS;
                if m.unrest > t.revolt_share {
                    self.escape(mi);
                }
            }
        }
    }

    /// The nearest hostile in reach of mothership `mi`: a fleet (its
    /// division) or a planet (its position).
    fn threat(&self, mi: usize) -> Option<(Vec2, Option<Entity>)> {
        let m = &self.motherships[mi];
        let at = self.fleet(m.fleet)?.pos;
        let reach = self.range() * self.tuning.deploy_ranges;
        let fleets = self
            .fleets
            .iter()
            .filter(|f| f.armed() && f.owner.hostile(m.owner) && f.morale != crate::sim::components::Morale::Routed)
            .map(|f| (f.pos, Some(f.division)));
        let planets = self
            .planets
            .iter()
            .filter(|p| p.owner != m.owner && p.value > 0)
            .map(|p| (p.pos, None));
        fleets.chain(planets).filter(|(p, _)| p.distance(at) <= reach).min_by(|a, b| a.0.distance(at).total_cmp(&b.0.distance(at)))
    }

    /// Launch while there is a threat (and ships aboard), send the wing at
    /// it; with none for a while, bring the wing home and dock it.
    fn carrier(&mut self, mi: usize, dt: f32) {
        let t = self.tuning;
        let unit = self.unit();
        let Some(home) = self.fleet(self.motherships[mi].fleet).map(|f| f.pos) else { return };
        // Losses: the wing has fewer ships than launched minus docked.
        let wing_now = self.motherships[mi].wing.and_then(|w| self.fleet(w)).map(|f| f.ships);
        if wing_now.is_none() {
            self.motherships[mi].wing = None;
        }
        let have = wing_now.unwrap_or(0);
        let m = &mut self.motherships[mi];
        if have < m.wing_expected {
            m.lost += m.wing_expected - have;
        }
        m.wing_expected = have;
        // Kills near the wing since last time.
        let wing_pos = m.wing.and_then(|w| self.fleets.iter().find(|f| f.id == w)).map(|f| f.pos);
        let owner = m.owner;
        if let Some(wp) = wing_pos {
            let reach = self.range() * 2.0;
            let kills = self.recent_kills.iter().filter(|(p, side)| *side != owner.side() && p.distance(wp) <= reach).count();
            self.motherships[mi].kills += kills as u32;
        }
        match self.threat(mi) {
            Some((target, division)) => {
                let m = &mut self.motherships[mi];
                m.calm_for = 0.0;
                m.launch += dt * t.launch_rate * m.working();
                while self.motherships[mi].launch >= 1.0 && self.motherships[mi].carried > 0 {
                    self.motherships[mi].launch -= 1.0;
                    self.launch_one(mi, home);
                }
                let m = &mut self.motherships[mi];
                if m.carried == 0 {
                    m.launch = m.launch.min(1.0);
                }
                if let Some(w) = self.motherships[mi].wing.and_then(|w| self.fleet(w)).map(|f| (f.division, f.pos)) {
                    let order = match division {
                        Some(e) => Order::Engage(e),
                        None => Order::MoveTo { to: target / unit, facing: (target - w.1).to_angle() },
                    };
                    if let Some(mut d) = self.world.get_mut::<Division>(w.0) {
                        d.order = order;
                    }
                }
            }
            None => {
                self.motherships[mi].calm_for += dt;
                if self.motherships[mi].calm_for >= RECALL_SECS {
                    self.recall(mi, home);
                }
            }
        }
    }

    /// One ship out of mothership `mi` into its wing, with what the stores
    /// can give it.
    fn launch_one(&mut self, mi: usize, at: Vec2) {
        let unit = self.unit();
        let owner = self.motherships[mi].owner;
        let wing = self.motherships[mi].wing.and_then(|w| self.fleet_index(w));
        let fi = match wing {
            Some(fi) => fi,
            None => {
                let id = self.spawn(owner, at, 0.0, 0, None);
                let fi = self.fleets.len() - 1;
                self.fleets[fi].wing_of = Some(self.motherships[mi].id);
                self.motherships[mi].wing = Some(id);
                fi
            }
        };
        let division = self.fleets[fi].division;
        let Some(e) = crate::sim::add_ship(&mut self.world, division, at / unit) else { return };
        let m = &mut self.motherships[mi];
        let stores: Stock = std::array::from_fn(|k| SHIP_CAP[k].min(m.stores[k]));
        (0..3).for_each(|k| m.stores[k] -= stores[k]);
        m.carried -= 1;
        m.wing_expected += 1;
        self.world.entity_mut(e).insert(Stores(stores));
        self.fleets[fi].ships += 1;
        self.sync();
    }

    /// The wing heads back; ships close enough dock (their stores return).
    fn recall(&mut self, mi: usize, home: Vec2) {
        let unit = self.unit();
        let Some(w) = self.motherships[mi].wing.and_then(|w| self.fleet_index(w)) else { return };
        let division = self.fleets[w].division;
        if let Some(mut d) = self.world.get_mut::<Division>(division) {
            let facing = d.facing;
            d.order = Order::MoveTo { to: home / unit, facing };
        }
        let dock = 15.0 * self.scale();
        let close: Vec<Entity> = self
            .world
            .query::<(Entity, &Pos, &InDivision)>()
            .iter(&self.world)
            .filter(|(_, p, d)| d.0 == division && (p.0 * unit).distance(home) <= dock)
            .map(|(e, _, _)| e)
            .collect();
        if close.is_empty() {
            return;
        }
        let cap = self.tuning.mothership_cap;
        for &e in &close {
            let m = &mut self.motherships[mi];
            if let Some(s) = self.world.get::<Stores>(e) {
                (0..3).for_each(|k| m.stores[k] = (m.stores[k] + s.0[k]).min(cap[k]));
            }
            m.carried += 1;
            m.wing_expected = m.wing_expected.saturating_sub(1);
        }
        crate::sim::despawn_ships(&mut self.world, division, &close);
        self.sync();
    }

    /// Past the revolt share: the rebels leave mothership `mi` in escape
    /// pods (the population drops), and the unrest settles.
    fn escape(&mut self, mi: usize) {
        let m = &mut self.motherships[mi];
        let n = m.people.remove_active();
        m.escaped += n as u32;
        m.unrest = 0.0;
        let Some(at) = self.fleet(self.motherships[mi].fleet).map(|f| f.pos) else { return };
        let pods = (n / 10).clamp(1, 30);
        let speed = 12.0 * self.scale();
        for k in 0..pods {
            let a = self.rng.next_float() * std::f32::consts::TAU + k as f32;
            let v = Vec2::from_angle(a) * speed * (0.6 + self.rng.next_float() * 0.8);
            self.pods.push((at, v, 0.0));
        }
    }

    /// The maintenance phase for motherships: stores from a friendly planet
    /// and from supply ships that joined; the people eat, grow or starve;
    /// hull damage kills people and is repaired with parts; ships are built
    /// aboard; morale moves; speed follows the workers.
    pub(crate) fn maintain_motherships(&mut self) {
        let t = self.tuning;
        for mi in 0..self.motherships.len() {
            if !self.motherships[mi].alive {
                continue;
            }
            let (fleet, owner, ship) = (self.motherships[mi].fleet, self.motherships[mi].owner, self.motherships[mi].ship);
            let Some(fi) = self.fleet_index(fleet) else { continue };
            let pos = self.fleets[fi].pos;
            // Stores: the planet it rests at, then supply ships that joined.
            if let Some(pi) = self.friendly_planet(owner, pos) {
                for k in 0..3 {
                    let take = (t.mothership_cap[k] - self.motherships[mi].stores[k]).max(0.0).min(self.planets[pi].stock[k]);
                    self.planets[pi].stock[k] -= take;
                    self.motherships[mi].stores[k] += take;
                }
            }
            let supply: Vec<Entity> =
                self.fleets[fi].ship_ids.iter().zip(&self.fleets[fi].is_supply).filter(|(_, s)| **s).map(|(e, _)| *e).collect();
            if !supply.is_empty() {
                let id = self.motherships[mi].id;
                self.board(id, &supply);
            }
            let m = &mut self.motherships[mi];
            let working = m.working();
            // Food.
            let pop = m.people.population();
            let need = pop as f32 * t.food_per_person;
            let starving = m.stores[FOOD] < need;
            m.stores[FOOD] = (m.stores[FOOD] - need).max(0.0);
            if starving {
                m.people.remove_people(((pop as f32 * 0.03).ceil() as usize).min(pop));
            } else if m.stores[FOOD] > t.mothership_cap[FOOD] * 0.5 && pop < t.max_population as usize {
                let grow = ((pop as f32 * 0.02).ceil() as usize).min(t.max_population as usize - pop);
                m.people.add_people(grow);
            }
            // Hull: damage kills people and shakes morale; parts repair it.
            let (hull, max) = self.world.get::<Ship>(ship).map_or((0.0, 1.0), |s| (s.hull, s.max_hull));
            let m = &mut self.motherships[mi];
            let damage = ((m.hull_seen - hull) / max).max(0.0);
            if damage > 0.0 {
                let pop = m.people.population();
                m.people.remove_people((pop as f32 * damage).round() as usize);
            }
            let fix = (max - hull).max(0.0).min(m.stores[PARTS] * t.hull_per_part * working);
            m.stores[PARTS] -= if t.hull_per_part > 0.0 { fix / t.hull_per_part } else { 0.0 };
            let hull = hull + fix;
            m.hull_seen = hull;
            if let Some(mut s) = self.world.get_mut::<Ship>(ship) {
                s.hull = hull;
            }
            // Ships built aboard.
            let m = &mut self.motherships[mi];
            m.build += MAINTENANCE_SECS / t.mothership_build_secs.max(0.1) * working;
            while m.build >= 1.0 {
                if m.stores[PARTS] < t.ship_cost {
                    m.build = 1.0;
                    break;
                }
                m.stores[PARTS] -= t.ship_cost;
                m.carried += 1;
                m.build -= 1.0;
            }
            // Morale.
            let shortage = m.stores.iter().filter(|v| **v < 1.0).count() as f32 / 3.0;
            let crowding = m.crowding(t.crew_per_ship);
            let mut morale = m.morale - shortage * t.morale_shortage - crowding * t.morale_crowding - m.lost as f32 * t.morale_loss
                - damage * t.morale_damage
                + m.captures as f32 * t.morale_capture
                + m.kills as f32 * t.morale_kill;
            if shortage == 0.0 && crowding == 0.0 {
                morale += t.morale_recover;
            }
            m.morale = morale.clamp(0.0, 1.0);
            m.lost = 0;
            m.kills = 0;
            m.captures = 0;
            // Speed follows the workers.
            let pace = t.mothership_pace * working;
            if let Some(f) = self.fleet(fleet)
                && let Some(mut d) = self.world.get_mut::<Division>(f.division)
            {
                d.pace = pace.max(0.05);
            }
        }
        self.recent_kills.clear();
    }

    /// Mothership `mi`'s wing took planet `p` (called on a capture).
    pub(crate) fn credit_capture(&mut self, by: Owner, at: Vec2) {
        let r = self.capture_radius() * 2.0;
        for mi in 0..self.motherships.len() {
            let m = &self.motherships[mi];
            if !m.alive || m.owner != by {
                continue;
            }
            if m.wing.and_then(|w| self.fleet(w)).is_some_and(|f| f.pos.distance(at) <= r) {
                self.motherships[mi].captures += 1;
            }
        }
    }
}
