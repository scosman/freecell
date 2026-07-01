//! # binding_common — engine-abstraction trait + shared scenarios for the bake-off
//!
//! Sub-project C (functional_spec §6.C, architecture §5) is a **two-engine bake-off**:
//! the *same* binding designs and the *same* benchmark scenarios run against both
//! Formualizer and IronCalc so the numbers compare directly. This crate holds the
//! parts that must be identical across both engines:
//!
//! - [`engine`] — the [`SpreadsheetEngine`] trait: the "binding surface" FreeCell
//!   needs (build/load, writes single+batched, reads/eval single+viewport, recompute,
//!   edit→dirty/changelog), plus the neutral [`EngineValue`] / [`Viewport`] /
//!   [`EngineCaps`] types the scenarios speak.
//! - [`binding`] — the three candidate binding **designs** D1 (naive per-cell),
//!   D2 (bulk/range), D3 (cached + changelog), written generically over the trait.
//! - [`scenario`] — the five benchmark **scenarios** built on the frozen `datagen`
//!   generators, with a [`scenario::Profile`] carrying `dev`/`full` sizes.
//! - [`report`] — env-stamped results recording over `bench_util` (`results/` JSON +
//!   `summary.md`).
//!
//! This crate is **engine-neutral**: it depends only on the frozen shared crates
//! (`datagen`, `bench_util`) and never on a spreadsheet engine. Its own tests run
//! against a tiny in-crate [`fake::FakeEngine`] so the scenarios/bindings/report are
//! validated without pulling in Formualizer or IronCalc. The real recorded numbers
//! come from the per-engine crates that `impl SpreadsheetEngine`.

pub mod binding;
pub mod engine;
pub mod fake;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod sysinfo;

pub use binding::{BindingCache, Design};
pub use engine::{CellInput, EngineCaps, EngineValue, SpreadsheetEngine, Viewport};
pub use report::{rebuild_summary, write_all, ScenarioResult};
pub use runner::{run_memory_only, run_suite};
pub use scenario::{targets, Profile};
