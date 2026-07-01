//! FreeCell Round-3 Investigation A (the crux) — style/geometry cache sync + structural
//! editing. Library crate exposing the probe, the resident-cache prototype, and the
//! IronCalc↔cache sync harness so both `main.rs` (the runnable report + cost harness) and
//! `tests/` (the GATE correctness/undo/agreement assertions) share one implementation.
//!
//! See `specs/projects/freecell-phase-3/{functional_spec.md §6-A, architecture.md §4}`
//! and this crate's `findings.md`.

pub mod cache;
pub mod harness;
pub mod probe;
