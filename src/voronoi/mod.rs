//! Voronoi toolkit after the gamegeniuslab tutorial "Voronoi diagrams in
//! game development": seeded sites, nearest-site metrics, owner maps,
//! Delaunay cells, Lloyd relaxation, biome islands, weighted faction
//! territories, cell noise and a painted island.
//!
//! Pure functions over plain data (no ECS), like [`crate::sim`]: tests,
//! `examples/voronoi_dump.rs` and the labs in [`crate::game`] all call the
//! same code. [`labs::render`] turns the shared [`sites::VoronoiConfig`]
//! and a lab's state into an RGB image.

pub mod biomes;
pub mod delaunay;
pub mod grid;
pub mod influence;
pub mod labs;
pub mod metric;
pub mod noise;
pub mod paint;
pub mod raster;
pub mod relax;
pub mod rng;
pub mod sites;


#[cfg(test)]
mod tests;

pub use bevy::math::Vec2;
