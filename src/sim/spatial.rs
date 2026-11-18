//! A uniform-grid spatial hash over every ship, rebuilt each tick. It answers
//! "who is within r of here" without comparing every pair, and also groups
//! ships by division for centroids and breakthrough checks.

use std::collections::HashMap;

use bevy::prelude::*;

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    pub entity: Entity,
    pub pos: Vec2,
    pub heading: f32,
    pub side: usize,
    pub division: Entity,
}

#[derive(Clone, Debug, Default)]
pub struct DivisionShips {
    pub members: Vec<u32>,
    pub centroid: Vec2,
}

#[derive(Resource, Debug)]
pub struct SpatialHash {
    pub cell: f32,
    pub entries: Vec<Entry>,
    cells: HashMap<(i32, i32), Vec<u32>>,
    pub divisions: HashMap<Entity, DivisionShips>,
}

impl Default for SpatialHash {
    fn default() -> Self {
        Self::new(110.0)
    }
}

impl SpatialHash {
    pub fn new(cell: f32) -> Self {
        Self { cell, entries: Vec::new(), cells: HashMap::new(), divisions: HashMap::new() }
    }

    fn key(&self, p: Vec2) -> (i32, i32) {
        ((p.x / self.cell).floor() as i32, (p.y / self.cell).floor() as i32)
    }

    pub fn rebuild(&mut self, entries: impl Iterator<Item = Entry>) {
        self.entries.clear();
        self.entries.extend(entries);
        // Stable order regardless of query iteration order, so a seed
        // always gives the same battle.
        self.entries.sort_by_key(|e| e.entity);
        for list in self.cells.values_mut() {
            list.clear();
        }
        for div in self.divisions.values_mut() {
            div.members.clear();
            div.centroid = Vec2::ZERO;
        }
        for i in 0..self.entries.len() {
            let e = self.entries[i];
            let key = self.key(e.pos);
            self.cells.entry(key).or_default().push(i as u32);
            let div = self.divisions.entry(e.division).or_default();
            div.members.push(i as u32);
            div.centroid += e.pos;
        }
        self.divisions.retain(|_, d| !d.members.is_empty());
        for div in self.divisions.values_mut() {
            div.centroid /= div.members.len() as f32;
        }
    }

    /// Calls `f` with every entry within `r` of `p` (and its squared distance).
    pub fn for_each_within(&self, p: Vec2, r: f32, mut f: impl FnMut(&Entry, f32)) {
        let (x0, y0) = self.key(p - Vec2::splat(r));
        let (x1, y1) = self.key(p + Vec2::splat(r));
        let r2 = r * r;
        for cx in x0..=x1 {
            for cy in y0..=y1 {
                let Some(list) = self.cells.get(&(cx, cy)) else { continue };
                for &i in list {
                    let e = &self.entries[i as usize];
                    let d2 = e.pos.distance_squared(p);
                    if d2 <= r2 {
                        f(e, d2);
                    }
                }
            }
        }
    }

    pub fn centroid(&self, division: Entity) -> Option<Vec2> {
        self.divisions.get(&division).map(|d| d.centroid)
    }
}
