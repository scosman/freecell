//! # bench_util — timing, stats, gating, and results recording for FreeCell Phase 1
//!
//! Shared measurement infrastructure for the Phase 1 engine benchmarks
//! (architecture §3, functional_spec §5.3). It complements Criterion (used for
//! micro/throughput numbers) with helpers for the end-to-end latencies Criterion
//! doesn't fit, and standardizes how results are gated and recorded.
//!
//! ## What's here
//!
//! - [`timing`] — a [`Stopwatch`] and `time_*` helpers over the monotonic clock,
//!   used for **measurement** only.
//! - [`stats`] — [`percentile_ns`] and [`LatencyStats`] (min/max/mean/p50/p99);
//!   report distributions, not just means.
//! - [`gate`] — [`GateResult`] / [`Verdict`]: compare a measured latency against a
//!   §5.4 target and print `PASS`/`FAIL` with the number.
//! - [`record`] — [`Environment`] and [`BenchResult`]: serializable, env-stamped
//!   results written as JSON to a phase's `results/`.
//!
//! ## Determinism boundary
//!
//! Timing uses the real clock, but **recording never does**: [`BenchResult`] takes
//! its report date as a parameter and [`Environment::detect`] reads no clock
//! (architecture §3). This keeps recorded artifacts reproducible and free of
//! wall-clock noise in otherwise-deterministic code.
//!
//! ## Example
//!
//! ```
//! use bench_util::{time_iters, BenchResult, Environment, GateResult, LatencyStats};
//!
//! // Measure something repeatedly.
//! let samples = time_iters(100, || {
//!     let _ = (0..1_000).sum::<u64>();
//! });
//! let stats = LatencyStats::from_durations(&samples).unwrap();
//!
//! // Gate p99 against a 2 ms target and record it (date passed in, not read).
//! let gate = GateResult::max("scrolling-read", stats.p99_ns, 2_000_000);
//! let result = BenchResult::new("scrolling-read", 1_000, "2026-06-30", Environment::detect("HEAD"))
//!     .with_stats(stats)
//!     .with_gate(gate);
//! assert!(result.to_json_pretty().contains("scrolling-read"));
//! ```

pub mod gate;
pub mod record;
pub mod stats;
pub mod timing;

pub use gate::{GateResult, Verdict, fmt_ns};
pub use record::{BenchResult, Environment};
pub use stats::{LatencyStats, percentile_ns};
pub use timing::{Stopwatch, time_iters, time_once};
