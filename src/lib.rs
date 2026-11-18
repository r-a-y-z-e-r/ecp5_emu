//! Digital ECP5 configuration, routing and simulation, independent of any UI.
pub mod board;
pub mod chip;
pub mod compile;
pub mod config;
pub mod database;
pub mod decode;
mod design_cache;
pub mod fabric;
pub mod fabric_overlay;
pub mod fast;
pub mod flow;
pub mod image;
pub mod kernel;
pub mod layout;
pub mod live;
pub mod logic;
pub mod model;
pub mod names;
pub mod peripherals;
pub mod prepared;
pub mod routing;
pub mod sim;
mod snapshot;
pub mod sources;
pub mod state;
pub mod timing_path;
pub mod uart;

#[cfg(test)]
mod test_designs;
