//! A planet's people: a port of NetLogo's "Rebellion" sample model
//! (Epstein 2002, "Modeling civil violence"). Agents and cops live on a
//! 40×40 torus of patches. Each agent has a perceived hardship H and a risk
//! aversion R, both uniform in 0..1. Its grievance is G = H·(1−L), where L
//! is the government's legitimacy. It turns active (rebels) when
//! G − R·P > threshold, where P = 1 − exp(−k·floor(C / (1+A))) is its
//! estimated arrest probability from the cops C and active agents A in
//! vision. Every step, in random order, free agents and cops move to a
//! random empty patch in vision, free agents decide, and each cop jails one
//! random active agent in vision for up to `max_jail` steps.
//!
//! The game decides the cops (ships nearby), the legitimacy (tax and
//! occupation) and adds war to the hardship.

use crate::voronoi::rng::ParkMiller;

/// Patches per side (NetLogo's world is 40×40, wrapping).
pub const SIDE: usize = 40;
pub const PATCHES: usize = SIDE * SIDE;
/// Initial agent density (NetLogo default 70 %).
pub const AGENT_DENSITY: f32 = 0.70;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RebellionParams {
    /// Patches an agent or cop sees (radius).
    pub vision: f32,
    /// Arrest-probability constant.
    pub k: f32,
    /// How much net grievance it takes to rebel.
    pub threshold: f32,
    /// Jail terms are uniform in 0..max_jail steps.
    pub max_jail: u32,
}

impl Default for RebellionParams {
    fn default() -> Self {
        Self { vision: 7.0, k: 2.3, threshold: 0.1, max_jail: 30 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Agent {
    pub patch: u16,
    pub hardship: f32,
    pub risk_aversion: f32,
    pub active: bool,
    pub jail: u32,
}

/// What a patch shows, for drawing (the most telling occupant wins).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cell {
    Empty,
    Quiet,
    Active,
    Jailed,
    Cop,
}

#[derive(Clone, Debug)]
pub struct Rebellion {
    pub agents: Vec<Agent>,
    /// The patch each cop stands on.
    pub cops: Vec<u16>,
    /// Agents (free or jailed) on each patch.
    at: Vec<Vec<u16>>,
    /// Cops, free (unjailed) agents and active agents on each patch.
    cops_at: Vec<u16>,
    free_at: Vec<u16>,
    active_at: Vec<u16>,
    /// Patch offsets within vision, and the vision they were made for.
    reach: Vec<(i32, i32)>,
    reach_for: f32,
    rng: ParkMiller,
}

impl Rebellion {
    /// 70 % of the patches get an agent; no cops yet.
    pub fn new(seed: i64) -> Self {
        Self::with_population(seed, (AGENT_DENSITY * PATCHES as f32).round() as usize)
    }

    /// `n` agents (at most the patches) on random patches; no cops yet.
    pub fn with_population(seed: i64, n: usize) -> Self {
        let mut rng = ParkMiller::new(seed);
        let mut patches: Vec<u16> = (0..PATCHES as u16).collect();
        shuffle(&mut patches, &mut rng);
        let n = n.min(PATCHES);
        let mut at = vec![Vec::new(); PATCHES];
        let mut free_at = vec![0; PATCHES];
        let agents = patches[..n]
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                at[p as usize].push(i as u16);
                free_at[p as usize] += 1;
                Agent { patch: p, risk_aversion: rng.next_float(), hardship: rng.next_float(), active: false, jail: 0 }
            })
            .collect();
        Self {
            agents,
            cops: Vec::new(),
            at,
            cops_at: vec![0; PATCHES],
            free_at,
            active_at: vec![0; PATCHES],
            reach: Vec::new(),
            reach_for: -1.0,
            rng,
        }
    }

    /// Everyone released and quiet (after a revolt); cops stay.
    pub fn reset(&mut self) {
        for a in &mut self.agents {
            a.active = false;
            a.jail = 0;
        }
        self.recount();
    }

    pub fn population(&self) -> usize {
        self.agents.len()
    }

    /// `k` newcomers (quiet, free) on empty patches, while any are left.
    pub fn add_people(&mut self, k: usize) {
        for _ in 0..k {
            let empty: Vec<usize> = (0..PATCHES).filter(|p| self.empty(*p)).collect();
            if empty.is_empty() {
                break;
            }
            let p = empty[((self.rng.next_float() * empty.len() as f32) as usize).min(empty.len() - 1)];
            let (hardship, risk_aversion) = (self.rng.next_float(), self.rng.next_float());
            self.agents.push(Agent { patch: p as u16, hardship, risk_aversion, active: false, jail: 0 });
        }
        self.reindex();
    }

    /// `k` random people gone (deaths).
    pub fn remove_people(&mut self, k: usize) {
        for _ in 0..k.min(self.agents.len()) {
            let i = ((self.rng.next_float() * self.agents.len() as f32) as usize).min(self.agents.len() - 1);
            self.agents.swap_remove(i);
        }
        self.reindex();
    }

    /// Rebuilds who stands where after agents come or go.
    fn reindex(&mut self) {
        for list in &mut self.at {
            list.clear();
        }
        for (i, a) in self.agents.iter().enumerate() {
            self.at[a.patch as usize].push(i as u16);
        }
        self.recount();
    }

    fn recount(&mut self) {
        self.free_at.fill(0);
        self.active_at.fill(0);
        for a in &self.agents {
            if a.jail == 0 {
                self.free_at[a.patch as usize] += 1;
            }
            if a.active {
                self.active_at[a.patch as usize] += 1;
            }
        }
    }

    pub fn active_share(&self) -> f32 {
        self.agents.iter().filter(|a| a.active).count() as f32 / self.agents.len().max(1) as f32
    }

    pub fn jailed_share(&self) -> f32 {
        self.agents.iter().filter(|a| a.jail > 0).count() as f32 / self.agents.len().max(1) as f32
    }

    /// One NetLogo tick. `hardship_bonus` is added to every agent's
    /// hardship (capped at 1); `cops_wanted` cops are added or removed first.
    pub fn step(&mut self, legitimacy: f32, hardship_bonus: f32, cops_wanted: usize, p: &RebellionParams) {
        if self.reach_for != p.vision {
            self.reach = offsets(p.vision);
            self.reach_for = p.vision;
        }
        self.set_cops(cops_wanted.min(PATCHES / 2));
        // `ask turtles`: agents and cops in one random order.
        let n_agents = self.agents.len();
        let mut order: Vec<u32> = (0..(n_agents + self.cops.len()) as u32).collect();
        shuffle(&mut order, &mut self.rng);
        let legit = legitimacy.clamp(0.0, 1.0);
        for t in order {
            let t = t as usize;
            if t < n_agents {
                if self.agents[t].jail > 0 {
                    continue;
                }
                self.move_agent(t);
                let a = self.agents[t];
                let grievance = (a.hardship + hardship_bonus).min(1.0) * (1.0 - legit);
                let (c, active) = self.seen(a.patch);
                let arrest = 1.0 - (-p.k * (c as f32 / (1 + active) as f32).floor()).exp();
                let now = grievance - a.risk_aversion * arrest > p.threshold;
                if now != a.active {
                    let q = a.patch as usize;
                    if now {
                        self.active_at[q] += 1;
                    } else {
                        self.active_at[q] -= 1;
                    }
                    self.agents[t].active = now;
                }
            } else {
                let c = t - n_agents;
                self.move_cop(c);
                self.enforce(c, p.max_jail);
            }
        }
        for a in &mut self.agents {
            if a.jail > 0 {
                a.jail -= 1;
                if a.jail == 0 {
                    self.free_at[a.patch as usize] += 1;
                }
            }
        }
    }

    /// Each patch's look, row by row.
    pub fn cells(&self) -> Vec<Cell> {
        let mut cells = vec![Cell::Empty; PATCHES];
        for a in &self.agents {
            let c = &mut cells[a.patch as usize];
            let mine = if a.jail > 0 {
                Cell::Jailed
            } else if a.active {
                Cell::Active
            } else {
                Cell::Quiet
            };
            if rank(mine) > rank(*c) {
                *c = mine;
            }
        }
        for &p in &self.cops {
            cells[p as usize] = Cell::Cop;
        }
        cells
    }

    fn set_cops(&mut self, n: usize) {
        while self.cops.len() > n {
            let i = (self.rng.next_float() * self.cops.len() as f32) as usize;
            let p = self.cops.swap_remove(i.min(self.cops.len() - 1));
            self.cops_at[p as usize] -= 1;
        }
        let mut tries = 0;
        while self.cops.len() < n {
            let p = ((self.rng.next_float() * PATCHES as f32) as usize).min(PATCHES - 1);
            // An empty patch if one turns up soon, else any patch.
            tries += 1;
            if !self.empty(p) && tries < 200 {
                continue;
            }
            self.cops.push(p as u16);
            self.cops_at[p] += 1;
        }
    }

    /// No cops and no free agents (jailed agents don't count).
    fn empty(&self, p: usize) -> bool {
        self.cops_at[p] == 0 && self.free_at[p] == 0
    }

    fn around(&self, patch: u16) -> impl Iterator<Item = usize> + '_ {
        let (x, y) = ((patch as usize % SIDE) as i32, (patch as usize / SIDE) as i32);
        let s = SIDE as i32;
        self.reach.iter().map(move |(dx, dy)| ((y + dy).rem_euclid(s) * s + (x + dx).rem_euclid(s)) as usize)
    }

    /// A random empty patch in vision, if any (reservoir sampling).
    fn spot(&mut self, patch: u16) -> Option<usize> {
        let (x, y) = ((patch as usize % SIDE) as i32, (patch as usize / SIDE) as i32);
        let s = SIDE as i32;
        let (mut seen, mut pick) = (0u32, None);
        for &(dx, dy) in &self.reach {
            let q = ((y + dy).rem_euclid(s) * s + (x + dx).rem_euclid(s)) as usize;
            if self.cops_at[q] == 0 && self.free_at[q] == 0 {
                seen += 1;
                if self.rng.next_float() * seen as f32 <= 1.0 {
                    pick = Some(q);
                }
            }
        }
        pick
    }

    fn move_agent(&mut self, a: usize) {
        let from = self.agents[a].patch;
        if let Some(to) = self.spot(from) {
            let f = from as usize;
            self.at[f].retain(|&x| x as usize != a);
            self.at[to].push(a as u16);
            self.free_at[f] -= 1;
            self.free_at[to] += 1;
            if self.agents[a].active {
                self.active_at[f] -= 1;
                self.active_at[to] += 1;
            }
            self.agents[a].patch = to as u16;
        }
    }

    fn move_cop(&mut self, c: usize) {
        let from = self.cops[c];
        if let Some(to) = self.spot(from) {
            self.cops_at[from as usize] -= 1;
            self.cops_at[to] += 1;
            self.cops[c] = to as u16;
        }
    }

    /// Cops and active agents in vision.
    fn seen(&self, patch: u16) -> (u32, u32) {
        let (mut c, mut a) = (0, 0);
        for q in self.around(patch) {
            c += self.cops_at[q] as u32;
            a += self.active_at[q] as u32;
        }
        (c, a)
    }

    /// Jails one random active agent in vision and steps onto its patch.
    fn enforce(&mut self, c: usize, max_jail: u32) {
        let n: u32 = self.around(self.cops[c]).map(|q| self.active_at[q] as u32).sum();
        if n == 0 {
            return;
        }
        // The i-th active agent in vision.
        let mut i = ((self.rng.next_float() * n as f32) as u32).min(n - 1);
        let mut s = usize::MAX;
        for q in self.around(self.cops[c]) {
            let k = self.active_at[q] as u32;
            if i < k {
                s = self.at[q].iter().map(|&x| x as usize).filter(|&x| self.agents[x].active).nth(i as usize).unwrap();
                break;
            }
            i -= k;
        }
        let to = self.agents[s].patch;
        self.cops_at[self.cops[c] as usize] -= 1;
        self.cops_at[to as usize] += 1;
        self.cops[c] = to;
        let term = ((self.rng.next_float() * max_jail as f32) as u32).min(max_jail.saturating_sub(1));
        self.active_at[to as usize] -= 1;
        if term > 0 {
            self.free_at[to as usize] -= 1;
        }
        let a = &mut self.agents[s];
        a.active = false;
        a.jail = term;
    }
}

/// The fewest cops that keep people with this legitimacy and hardship
/// bonus calm, from the precomputed [`super::calm::CALM_COPS`] table,
/// interpolated. Never fewer for lower legitimacy or more hardship (the
/// table is noisy, and very unhappy people sit calm only because they are
/// in jail). `None`: not even the most cops a planet can get.
pub fn calm_cops(legitimacy: f32, bonus: f32) -> Option<f32> {
    use super::calm::CALM_COPS;
    let at = |l: usize, b: usize| -> f32 {
        let mut most = 0u8;
        for row in CALM_COPS.iter().skip(l) {
            for &v in row.iter().take(b + 1) {
                most = most.max(v);
            }
        }
        if most == 255 { f32::INFINITY } else { most as f32 }
    };
    let l = (legitimacy.clamp(0.0, 1.0) / 0.05).min(20.0);
    let b = (bonus.clamp(0.0, 1.0) / 0.1).min(10.0);
    let (l0, b0) = (l.floor() as usize, b.floor() as usize);
    let (l1, b1) = ((l0 + 1).min(20), (b0 + 1).min(10));
    let (fl, fb) = (l - l0 as f32, b - b0 as f32);
    let v = (at(l0, b0) * (1.0 - fb) + at(l0, b1) * fb) * (1.0 - fl) + (at(l1, b0) * (1.0 - fb) + at(l1, b1) * fb) * fl;
    v.is_finite().then_some(v)
}

fn rank(c: Cell) -> u8 {
    match c {
        Cell::Empty => 0,
        Cell::Jailed => 1,
        Cell::Quiet => 2,
        Cell::Active => 3,
        Cell::Cop => 4,
    }
}

/// Offsets of the patches whose centres lie within `vision` (NetLogo's
/// `in-radius`, which includes the patch itself).
fn offsets(vision: f32) -> Vec<(i32, i32)> {
    let r = vision.floor() as i32;
    let mut v = Vec::new();
    for dy in -r..=r {
        for dx in -r..=r {
            if ((dx * dx + dy * dy) as f32) <= vision * vision {
                v.push((dx, dy));
            }
        }
    }
    v
}

fn shuffle<T>(v: &mut [T], rng: &mut ParkMiller) {
    for i in (1..v.len()).rev() {
        let j = (rng.next_float() * (i + 1) as f32) as usize;
        v.swap(i, j.min(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(legit: f32, cops: usize, steps: usize, seed: i64) -> Rebellion {
        let mut r = Rebellion::new(seed);
        let p = RebellionParams::default();
        for _ in 0..steps {
            r.step(legit, 0.0, cops, &p);
        }
        r
    }

    #[test]
    fn the_same_seed_gives_the_same_people() {
        let (a, b) = (run(0.7, 40, 30, 5), run(0.7, 40, 30, 5));
        assert_eq!(a.agents, b.agents);
        assert_eq!(a.cops, b.cops);
    }

    #[test]
    fn full_legitimacy_means_nobody_rebels() {
        let r = run(1.0, 0, 30, 1);
        assert_eq!(r.active_share(), 0.0);
    }

    #[test]
    fn no_cops_and_no_legitimacy_means_most_rebel() {
        // Active when H > 0.1 with nobody to arrest them: about 90 %.
        let r = run(0.0, 0, 5, 2);
        assert!(r.active_share() > 0.8, "{}", r.active_share());
    }

    #[test]
    fn many_cops_keep_the_peace() {
        let few = run(0.6, 0, 60, 3).active_share();
        let many = run(0.6, 300, 60, 3).active_share();
        assert!(few > 0.3, "{few}");
        assert!(many < 0.05, "{many}");
    }

    #[test]
    fn patch_counts_stay_in_step() {
        let mut r = run(0.5, 60, 25, 9);
        let (free, active) = (r.free_at.clone(), r.active_at.clone());
        r.recount();
        assert_eq!(free, r.free_at);
        assert_eq!(active, r.active_at);
    }

    #[test]
    fn the_calm_table_never_asks_less_of_unhappier_people() {
        assert_eq!(calm_cops(1.0, 0.0), Some(0.0));
        assert_eq!(calm_cops(0.9, 0.0), Some(0.0));
        let mut last = 0.0;
        for k in (0..=20).rev() {
            let v = calm_cops(k as f32 * 0.05, 0.3).unwrap_or(f32::INFINITY);
            assert!(v >= last, "{k}: {v} < {last}");
            last = v;
        }
        assert!(calm_cops(0.8, 0.0).unwrap() < calm_cops(0.8, 0.5).unwrap());
    }

    #[test]
    fn people_come_and_go_and_the_counts_keep_up() {
        let mut r = Rebellion::with_population(3, 300);
        assert_eq!(r.population(), 300);
        let p = RebellionParams::default();
        for _ in 0..10 {
            r.step(0.4, 0.0, 20, &p);
        }
        r.add_people(50);
        assert_eq!(r.population(), 350);
        r.remove_people(120);
        assert_eq!(r.population(), 230);
        for _ in 0..10 {
            r.step(0.4, 0.0, 20, &p);
        }
        let (free, active) = (r.free_at.clone(), r.active_at.clone());
        r.recount();
        assert_eq!((free, active), (r.free_at.clone(), r.active_at.clone()));
        let total: usize = r.at.iter().map(Vec::len).sum();
        assert_eq!(total, 230);
    }

    #[test]
    fn cops_jail_rebels() {
        let r = run(0.3, 100, 20, 4);
        assert!(r.jailed_share() > 0.0);
        assert_eq!(r.cops.len(), 100);
    }
}
