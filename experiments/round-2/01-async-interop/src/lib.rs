//! # SP1 — Non-blocking recompute & the engine↔render interop seam
//!
//! FreeCell Round-2 Phase-2, the crux experiment (functional_spec §6 SP1,
//! architecture §4). IronCalc has **no incremental recalc**: every edit needs a
//! full-workbook `Model::evaluate()` (O(all cells), ~2 s at 10⁶). SP1 discovers what
//! IronCalc's 0.7.1 API permits and **locks the interop-seam design** around it so
//! recompute never blocks the render loop.
//!
//! ## Modules
//! - [`shapes`] — the five DAG-shape builders for the `evaluate()` latency matrix
//!   (sparse, wide fan-out, deep-serial chain, cross-sheet, volatile) + force+assert
//!   tail metadata.
//! - [`matrix`] — [`matrix::time_evaluate`], which times a single full `evaluate()` and
//!   force+asserts the tail changed (p50/p99).
//! - [`seam`] — the **locked** [`seam::EvalWorker`] (owns the `Send` `Model` on a worker
//!   thread; coalesces edits into a single eval; publishes the visible viewport after
//!   each eval) and the compile-time [`seam::assert_model_send`] proof.
//! - [`probes`] — runtime API findings: the `UserModel` diff-list records only
//!   edit-sites (no evaluated-cell change stream), and the `to_bytes` snapshot route
//!   round-trips.
//!
//! Depends **read-only** by relative path on the frozen `../harness` (IronCalc adapter,
//! `SpreadsheetEngine` trait, `Viewport`) and `../../shared/*` (datagen, bench_util).

pub mod matrix;
pub mod probes;
pub mod seam;
pub mod shapes;

pub use matrix::{time_evaluate, MatrixCell};
pub use probes::{diff_list_is_edit_sites_only, snapshot_roundtrip, DiffListFinding};
pub use seam::{assert_model_send, CellSnapshot, Edit, EvalWorker, PublishedViewport};
pub use shapes::{build, BuiltShape, ReArm, Shape};
