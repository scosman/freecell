//! # round2_harness — the FROZEN shared harness for FreeCell Round-2 experiments
//!
//! ⚠️ **FROZEN / READ-ONLY DOWNSTREAM.** Round-2 experiments (SP1–SP5) **consume**
//! this crate by relative path and **never edit it** (architecture §1, §3). It is
//! created and frozen at scaffolding so every experiment shares one stable engine seam
//! and its numbers stay directly comparable to the Phase-1 baselines. If an experiment
//! genuinely needs a change here, **escalate** — do not modify this crate in place.
//!
//! ## What's inside (copied verbatim from the frozen Phase-1 crates)
//!
//! - [`engine`] — the [`SpreadsheetEngine`] trait (the "binding surface" FreeCell's UI
//!   needs) plus the neutral [`EngineValue`] / [`CellInput`] / [`Viewport`] /
//!   [`EngineCaps`] types the scenarios speak. Copied from `02/common/src/engine.rs`.
//! - [`binding`] — the three candidate binding designs (D1/D2/D3) the scenarios drive.
//!   Copied from `02/common/src/binding.rs`.
//! - [`scenario`] — the five benchmark scenarios + [`scenario::Profile`] /
//!   [`scenario::targets`]. Copied from `02/common/src/scenario.rs`.
//! - [`report`] / [`runner`] — env-stamped results recording and the shared benchmark
//!   driver. Copied from `02/common/src/{report,runner}.rs`.
//! - [`fake`] — a tiny in-crate [`fake::FakeEngine`] used only by this crate's own
//!   tests, so the scenarios/bindings/report validate without a real engine.
//! - [`sysinfo`] — Phase-1 platform helpers (`peak_rss_bytes` VmHWM + `cpu_model`).
//! - [`ironcalc`] — the IronCalc adapter ([`IronCalcEngine`], `impl SpreadsheetEngine`)
//!   copied verbatim from `02/ironcalc/src/lib.rs`, pinned to the same IronCalc 0.7.x
//!   version. The one mechanical change is its import path (the trait now lives in this
//!   same crate).
//! - [`mod@peak_rss`] — **new here** (not in the frozen `shared/bench_util`): a
//!   fresh-process peak-RSS helper ([`peak_rss::peak_rss`], VmHWM with a `getrusage`
//!   fallback, returns bytes) for SP2's child-process peak-memory measurement.
//!
//! This crate depends only on the frozen shared crates (`datagen`, `bench_util`, by
//! relative path) plus IronCalc; the copied names/behavior are identical to Phase 1 so
//! Round-2 numbers stay comparable.

pub mod binding;
pub mod engine;
pub mod fake;
pub mod ironcalc;
pub mod peak_rss;
pub mod report;
pub mod runner;
pub mod scenario;
pub mod sysinfo;

pub use binding::{BindingCache, Design};
pub use engine::{CellInput, EngineCaps, EngineValue, SpreadsheetEngine, Viewport};
pub use ironcalc::IronCalcEngine;
pub use peak_rss::peak_rss;
pub use report::{rebuild_summary, write_all, ScenarioResult};
pub use runner::{run_memory_only, run_suite};
pub use scenario::{targets, Profile};
// `peak_rss::peak_rss` (re-exported above) is the CANONICAL Round-2 peak-RSS entry
// point. `sysinfo::peak_rss_bytes` is the copied Phase-1 variant kept for provenance;
// it returns `0` off-Linux / on `/proc` failure, so prefer `peak_rss` for SP2.
pub use sysinfo::{cpu_model, peak_rss_bytes};
