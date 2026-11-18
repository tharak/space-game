//! Headless battle rules. Nothing in here renders or reads input.

pub mod ai;
pub mod battle;
pub mod commands;
pub mod components;
pub mod config;
pub mod dice;
pub mod formations;

#[cfg(test)]
mod tests;
