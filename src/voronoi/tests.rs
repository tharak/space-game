use super::Vec2;
use super::biomes::{Biome, assign_biomes};
use super::delaunay::{Delaunay, polygon_area};
use super::grid::voronoi_grid;
use super::influence::weighted_owner;
use super::labs::{LabId, LabState, render};
use super::metric::{Metric, SiteIndex, distance, nearest_site, nearest_two};
use super::relax::relax_step;
use super::rng::ParkMiller;
use super::sites::{VoronoiConfig, make_sites};

#[test]
fn park_miller_matches_the_minimal_standard() {
    let mut r = ParkMiller::new(1);
    assert_eq!(r.next_raw(), 16_807);
    assert_eq!(r.next_raw(), 282_475_249);
    // Seeds at or below zero are moved into range rather than sticking at 0.
    let mut z = ParkMiller::new(0);
    assert!(z.next_raw() > 0);
    let f = ParkMiller::new(123_456).next_float();
    assert!((0.0..=1.0).contains(&f));
}

#[test]
fn sites_are_repeatable_and_inside_the_map() {
    let a = make_sites(20, 720.0, 400.0, 123_456);
    assert_eq!(a, make_sites(20, 720.0, 400.0, 123_456));
    assert_ne!(a, make_sites(20, 720.0, 400.0, 123_457));
    assert!(a.iter().all(|s| (0.0..=720.0).contains(&s.x) && (0.0..=400.0).contains(&s.y)));
}

#[test]
fn ties_go_to_the_lower_index() {
    let sites = [Vec2::new(10.0, 0.0), Vec2::new(-10.0, 0.0)];
    for m in Metric::ALL {
        assert_eq!(nearest_site(&sites, Vec2::ZERO, m), Some(0));
        assert_eq!(SiteIndex::new(&sites, m).nearest(Vec2::ZERO), Some(0));
    }
}

#[test]
fn equally_far_shapes_per_metric() {
    let o = Vec2::ZERO;
    let (diag, axis) = (Vec2::new(10.0, 10.0), Vec2::new(10.0, 0.0));
    assert!(distance(o, diag, Metric::Euclidean) > distance(o, axis, Metric::Euclidean)); // circle
    assert_eq!(distance(o, diag, Metric::Manhattan), 20.0); // diamond
    assert_eq!(distance(o, diag, Metric::Chebyshev), distance(o, axis, Metric::Chebyshev)); // square
}

#[test]
fn site_index_agrees_with_brute_force() {
    let sites = make_sites(60, 300.0, 200.0, 99);
    for m in Metric::ALL {
        let index = SiteIndex::new(&sites, m);
        for y in (0..200).step_by(7) {
            for x in (0..300).step_by(7) {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let (a, b) = (index.nearest_two(p), nearest_two(&sites, p, m));
                assert_eq!((a.nearest, a.d1), (b.nearest, b.d1), "{m:?} at {p}");
                assert_eq!(a.d2, b.d2, "{m:?} at {p}");
            }
        }
    }
}

#[test]
fn delaunay_neighbours_match_the_owner_map() {
    let (w, h) = (360, 220);
    let sites = make_sites(30, w as f32, h as f32, 5);
    let d = Delaunay::build(&sites);
    let grid = voronoi_grid(&sites, w, h, Metric::Euclidean);
    // Every pair of cells touching in the pixel map is a Delaunay edge.
    for y in 0..h {
        for x in 0..w - 1 {
            let (a, b) = (grid.at(x, y), grid.at(x + 1, y));
            if a != b {
                assert!(d.neighbors[a as usize].contains(&(b as usize)), "{a} and {b} touch at ({x},{y})");
            }
        }
    }
}

#[test]
fn exact_cells_tile_the_map() {
    let (w, h) = (720.0, 400.0);
    for n in [1, 2, 3, 40] {
        let sites = make_sites(n, w, h, 11);
        let d = Delaunay::build(&sites);
        let total: f32 = (0..n).map(|i| polygon_area(&d.cell_polygon(i, w, h))).sum();
        assert!((total - w * h).abs() < w * h * 1e-3, "{n} sites cover {total}");
    }
}

#[test]
fn lloyd_evens_out_cells() {
    let (w, h) = (300, 200);
    let spread = |sites: &[Vec2]| {
        let grid = voronoi_grid(sites, w, h, Metric::Euclidean);
        let mut count = vec![0f32; sites.len()];
        for o in &grid.owner {
            count[*o as usize] += 1.0;
        }
        let mean = count.iter().sum::<f32>() / count.len() as f32;
        (count.iter().map(|c| (c - mean).powi(2)).sum::<f32>() / count.len() as f32).sqrt()
    };
    let mut sites = make_sites(40, w as f32, h as f32, 3);
    let before = spread(&sites);
    for _ in 0..3 {
        sites = relax_step(&sites, w, h, Metric::Euclidean).0;
    }
    assert!(spread(&sites) < before * 0.7, "{} → {}", before, spread(&sites));
}

#[test]
fn the_map_edge_is_always_ocean() {
    let (w, h) = (360, 200);
    let sites = make_sites(50, w as f32, h as f32, 8);
    let grid = voronoi_grid(&sites, w, h, Metric::Euclidean);
    let biomes = assign_biomes(&sites, &grid, 8);
    for o in grid.edge_owners() {
        assert_eq!(biomes[o as usize], Biome::Ocean);
    }
    assert!(biomes.iter().any(|b| *b != Biome::Ocean));
}

#[test]
fn weighted_territories() {
    let bases = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
    let p = Vec2::new(60.0, 0.0);
    assert_eq!(weighted_owner(&bases, &[1.0, 1.0], p, Metric::Euclidean), Some(1));
    // Twice the weight reaches twice as far.
    assert_eq!(weighted_owner(&bases, &[2.0, 1.0], p, Metric::Euclidean), Some(0));
    assert_eq!(weighted_owner(&bases, &[0.0, 1.0], Vec2::ZERO, Metric::Euclidean), Some(1));
    assert_eq!(weighted_owner(&bases, &[0.0, 0.0], p, Metric::Euclidean), None);
}

#[test]
fn f2_minus_f1_vanishes_on_borders() {
    let sites = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 0.0)];
    let on = nearest_two(&sites, Vec2::new(50.0, 30.0), Metric::Euclidean);
    assert!((on.d2 - on.d1).abs() < 1e-3);
    let off = nearest_two(&sites, Vec2::new(10.0, 0.0), Metric::Euclidean);
    assert!(off.d2 - off.d1 > 50.0);
}

#[test]
fn every_lab_renders_for_every_metric() {
    for metric in Metric::ALL {
        let cfg = VoronoiConfig { metric, sites: 12, w: 160, h: 100, ..Default::default() };
        let st = LabState::new(&cfg);
        for lab in LabId::ALL {
            let c = render(&cfg, lab, &st, 1);
            assert_eq!((c.w, c.h, c.px.len()), (160, 100, 160 * 100 * 3), "{lab:?}");
            assert_eq!(render(&cfg, lab, &st, 1), c, "{lab:?} is deterministic");
        }
    }
}
