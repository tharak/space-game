//! Exact cells and the neighbour graph from a Delaunay triangulation
//! (tutorial lab 2). Every triangle edge joins two sites whose cells share
//! a border; a cell is the map rectangle cut by the perpendicular bisector
//! to each of those neighbours.

use delaunator::{Point, next_halfedge, triangulate};

use super::Vec2;

#[derive(Clone, Debug)]
pub struct Delaunay {
    pub points: Vec<Vec2>,
    /// Site indices, three per triangle.
    pub triangles: Vec<usize>,
    pub neighbors: Vec<Vec<usize>>,
}

impl Delaunay {
    pub fn build(sites: &[Vec2]) -> Self {
        let pts: Vec<Point> = sites.iter().map(|s| Point { x: s.x as f64, y: s.y as f64 }).collect();
        let t = triangulate(&pts);
        let mut neighbors = vec![Vec::new(); sites.len()];
        for e in 0..t.triangles.len() {
            let (a, b) = (t.triangles[e], t.triangles[next_halfedge(e)]);
            neighbors[a].push(b);
            neighbors[b].push(a);
        }
        for n in &mut neighbors {
            n.sort_unstable();
            n.dedup();
        }
        Self { points: sites.to_vec(), triangles: t.triangles, neighbors }
    }

    pub fn triangle_count(&self) -> usize {
        self.triangles.len() / 3
    }

    pub fn triangle(&self, t: usize) -> [Vec2; 3] {
        [self.points[self.triangles[3 * t]], self.points[self.triangles[3 * t + 1]], self.points[self.triangles[3 * t + 2]]]
    }

    /// Centre and radius of a triangle's circumcircle.
    pub fn circumcircle(&self, t: usize) -> Option<(Vec2, f32)> {
        let [a, b, c] = self.triangle(t);
        let d = 2.0 * (a.x * (b.y - c.y) + b.x * (c.y - a.y) + c.x * (a.y - b.y));
        if d.abs() < 1e-6 {
            return None;
        }
        let (a2, b2, c2) = (a.length_squared(), b.length_squared(), c.length_squared());
        let ux = (a2 * (b.y - c.y) + b2 * (c.y - a.y) + c2 * (a.y - b.y)) / d;
        let uy = (a2 * (c.x - b.x) + b2 * (a.x - c.x) + c2 * (b.x - a.x)) / d;
        let centre = Vec2::new(ux, uy);
        Some((centre, centre.distance(a)))
    }

    /// The exact cell of site `i`, clipped to `[0, w] × [0, h]`.
    pub fn cell_polygon(&self, i: usize, w: f32, h: f32) -> Vec<Vec2> {
        let mut poly = vec![Vec2::new(0.0, 0.0), Vec2::new(w, 0.0), Vec2::new(w, h), Vec2::new(0.0, h)];
        let s = self.points[i];
        // Too few points to triangulate: cut against every other site.
        let all: Vec<usize>;
        let others: &[usize] = if self.triangles.is_empty() {
            all = (0..self.points.len()).filter(|j| *j != i).collect();
            &all
        } else {
            &self.neighbors[i]
        };
        for &j in others {
            let o = self.points[j];
            if o == s {
                continue;
            }
            poly = clip_half_plane(&poly, (s + o) * 0.5, o - s);
            if poly.is_empty() {
                break;
            }
        }
        poly
    }
}

/// Keeps the part of `poly` where `(p - origin) · normal <= 0`.
pub fn clip_half_plane(poly: &[Vec2], origin: Vec2, normal: Vec2) -> Vec<Vec2> {
    let side = |p: Vec2| (p - origin).dot(normal);
    let mut out = Vec::with_capacity(poly.len() + 1);
    for k in 0..poly.len() {
        let (a, b) = (poly[k], poly[(k + 1) % poly.len()]);
        let (da, db) = (side(a), side(b));
        if da <= 0.0 {
            out.push(a);
        }
        if (da < 0.0 && db > 0.0) || (da > 0.0 && db < 0.0) {
            out.push(a + (b - a) * (da / (da - db)));
        }
    }
    out
}

/// Shoelace area.
pub fn polygon_area(poly: &[Vec2]) -> f32 {
    let mut a = 0.0;
    for k in 0..poly.len() {
        let (p, q) = (poly[k], poly[(k + 1) % poly.len()]);
        a += p.x * q.y - q.x * p.y;
    }
    a.abs() * 0.5
}
