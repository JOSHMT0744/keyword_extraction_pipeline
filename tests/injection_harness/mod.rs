//! Shared harness for instrument 3 (identifier injection), used by both `tests/injection.rs`
//! (the gates that run in CI) and `examples/sweep_stage1.rs` (the parameter sweep, run on
//! demand). Each consumer includes this directory with `#[path]` rather than the crate
//! exposing it as a public module: this is test infrastructure, not part of the library's
//! contract, and a `pub mod` here would grow the public API for something no consumer of
//! the crate ever calls.
//!
//! `#![allow(dead_code)]` below is deliberate and load-bearing, not a smell to silence and
//! forget: `tests/injection.rs` and `examples/sweep_stage1.rs` each use a different subset
//! of this module (the sweep needs the resource-rebuilding path the gates don't, the gates
//! need assertions the sweep doesn't), so whichever one is compiling will always see the
//! other's exclusive items as unused.
#![allow(dead_code)]

pub mod baseline;
pub mod carrier;
pub mod data;
pub mod rng;
pub mod score;
pub mod tiers;

// Same reasoning as the `#![allow(dead_code)]` above: whichever consumer is compiling
// this only uses some of these re-exports (tests/injection.rs uses tier_of directly;
// examples/sweep_stage1.rs reaches TierResources via `tiers::TierResources` instead).
#[allow(unused_imports)]
pub use tiers::{tier_of, Tier, TierResources};
