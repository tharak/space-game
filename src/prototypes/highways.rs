//! Highways: the Delaunay triangulation of the planets. Fleets travel only
//! along its edges; an order anywhere on the map snaps to the nearest point
//! on a highway and the route is the shortest way there along the edges.

use crate::voronoi::Vec2;
use crate::voronoi::delaunay::Delaunay;

#[derive(Clone, Debug, Default)]
pub struct Highways {
    pub nodes: Vec<Vec2>,
    pub neighbors: Vec<Vec<usize>>,
    /// Each edge once, `a < b`.
    pub edges: Vec<(usize, usize)>,
}

/// A point on a highway: the edge's ends and the point itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Spot {
    pub a: usize,
    pub b: usize,
    pub point: Vec2,
}

impl Highways {
    pub fn new(points: &[Vec2]) -> Self {
        let mut neighbors = if points.len() >= 3 { Delaunay::build(points).neighbors } else { vec![Vec::new(); points.len()] };
        // Too few or collinear points give no triangles: chain them by x.
        if neighbors.iter().all(Vec::is_empty) && points.len() >= 2 {
            let mut order: Vec<usize> = (0..points.len()).collect();
            order.sort_by(|a, b| points[*a].x.total_cmp(&points[*b].x).then(points[*a].y.total_cmp(&points[*b].y)));
            for w in order.windows(2) {
                neighbors[w[0]].push(w[1]);
                neighbors[w[1]].push(w[0]);
            }
        }
        let mut edges = Vec::new();
        for (a, ns) in neighbors.iter().enumerate() {
            for &b in ns {
                if a < b {
                    edges.push((a, b));
                }
            }
        }
        Self { nodes: points.to_vec(), neighbors, edges }
    }

    /// The nearest point on any highway.
    pub fn snap(&self, p: Vec2) -> Option<Spot> {
        if self.edges.is_empty() {
            return self.nodes.first().map(|n| Spot { a: 0, b: 0, point: *n });
        }
        self.edges
            .iter()
            .map(|&(a, b)| Spot { a, b, point: closest_on_segment(self.nodes[a], self.nodes[b], p) })
            .min_by(|x, y| x.point.distance_squared(p).total_cmp(&y.point.distance_squared(p)))
    }

    /// Waypoints from `from` to `to` along highways: the snapped start
    /// (when `from` is off the highway), the planets passed, and the
    /// snapped end. Empty when there are no highways.
    pub fn route(&self, from: Vec2, to: Vec2) -> Vec<Vec2> {
        let (Some(s), Some(e)) = (self.snap(from), self.snap(to)) else { return Vec::new() };
        let mut out = Vec::new();
        if s.point.distance(from) > 1.0 {
            out.push(s.point);
        }
        let (_, path) = self.shortest(&s, &e);
        out.extend(path.iter().map(|&n| self.nodes[n]));
        out.push(e.point);
        out.dedup_by(|a, b| a.distance(*b) < 0.5);
        out
    }

    /// Length along highways from `from` to `to` (both snapped first).
    pub fn distance(&self, from: Vec2, to: Vec2) -> f32 {
        let (Some(s), Some(e)) = (self.snap(from), self.snap(to)) else { return f32::INFINITY };
        from.distance(s.point) + self.shortest(&s, &e).0
    }

    /// Dijkstra from a spot to a spot: the length and the planets passed.
    fn shortest(&self, s: &Spot, e: &Spot) -> (f32, Vec<usize>) {
        let same = (s.a == e.a && s.b == e.b) || (s.a == e.b && s.b == e.a);
        let direct = if same { s.point.distance(e.point) } else { f32::INFINITY };
        let n = self.nodes.len();
        let mut dist = vec![f32::INFINITY; n];
        let mut prev = vec![usize::MAX; n];
        let mut done = vec![false; n];
        for k in [s.a, s.b] {
            let d = s.point.distance(self.nodes[k]);
            if d < dist[k] {
                dist[k] = d;
            }
        }
        while let Some(u) = (0..n).filter(|i| !done[*i] && dist[*i].is_finite()).min_by(|a, b| dist[*a].total_cmp(&dist[*b])) {
            done[u] = true;
            for &v in &self.neighbors[u] {
                let d = dist[u] + self.nodes[u].distance(self.nodes[v]);
                if d < dist[v] {
                    dist[v] = d;
                    prev[v] = u;
                }
            }
        }
        let best = [e.a, e.b]
            .into_iter()
            .map(|k| (dist[k] + self.nodes[k].distance(e.point), k))
            .min_by(|x, y| x.0.total_cmp(&y.0));
        match best {
            Some((d, k)) if d < direct => {
                let mut path = vec![k];
                while prev[*path.last().unwrap()] != usize::MAX {
                    path.push(prev[*path.last().unwrap()]);
                }
                path.reverse();
                (d, path)
            }
            _ => (direct, Vec::new()),
        }
    }
}

pub fn closest_on_segment(a: Vec2, b: Vec2, p: Vec2) -> Vec2 {
    let ab = b - a;
    let len2 = ab.length_squared();
    if len2 < 1e-9 {
        return a;
    }
    a + ab * ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square() -> Highways {
        // A square with a centre: the centre links to every corner.
        Highways::new(&[Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0), Vec2::new(100.0, 100.0), Vec2::new(0.0, 100.0), Vec2::new(50.0, 50.0)])
    }

    fn on_some_edge(h: &Highways, p: Vec2) -> bool {
        h.edges.iter().any(|&(a, b)| closest_on_segment(h.nodes[a], h.nodes[b], p).distance(p) < 1e-3)
    }

    #[test]
    fn snapping_lands_on_a_highway() {
        let h = square();
        for p in [Vec2::new(10.0, 3.0), Vec2::new(70.0, 40.0), Vec2::new(-20.0, 140.0)] {
            let s = h.snap(p).unwrap();
            assert!(on_some_edge(&h, s.point), "{p:?} -> {:?}", s.point);
        }
    }

    #[test]
    fn routes_follow_the_edges() {
        let h = square();
        let r = h.route(Vec2::new(5.0, -10.0), Vec2::new(95.0, 105.0));
        assert!(r.len() >= 2);
        // Every leg runs along one edge.
        for w in r.windows(2) {
            let mid = (w[0] + w[1]) * 0.5;
            assert!(on_some_edge(&h, mid), "leg {w:?} is off the highways");
        }
        assert!(r.last().unwrap().distance(h.snap(Vec2::new(95.0, 105.0)).unwrap().point) < 1e-3);
    }

    #[test]
    fn the_same_edge_goes_straight() {
        let h = square();
        let r = h.route(Vec2::new(10.0, 0.0), Vec2::new(90.0, 0.0));
        assert_eq!(r, vec![Vec2::new(90.0, 0.0)]);
    }

    #[test]
    fn two_planets_make_one_highway() {
        let h = Highways::new(&[Vec2::new(0.0, 0.0), Vec2::new(10.0, 0.0)]);
        assert_eq!(h.edges, vec![(0, 1)]);
    }
}
