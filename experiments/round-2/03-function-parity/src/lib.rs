//! # function_parity — SP3 function-parity audit (coverage + correctness vs Excel)
//!
//! Two reproducible measurement tools plus the data they consume:
//!
//! - [`coverage`] — diffs IronCalc's **registered** builtins (`data/ironcalc_functions.csv`,
//!   extracted from the pinned 0.7.1 source) against a **committed canonical Excel list**
//!   (`data/excel_functions_canonical.csv`, Microsoft catalog with documented
//!   provenance). Reports overall / per-category / common-vs-obscure coverage.
//! - [`golden`] + [`cases_io`] — a data-driven golden-file correctness harness. Cases
//!   (`data/golden_cases.csv`) are `formula + inputs → expected value OR expected typed
//!   error`; each runs through the frozen `round2_harness` IronCalc adapter and errors
//!   are compared as **typed errors, not strings** ([`typed_error`]).
//! - [`probe`] — a runtime cross-check that empirically confirms which functions IronCalc
//!   actually recognizes (guards against "registered ≠ working").
//!
//! The binaries (`coverage`, `golden`, `probe`) write env-context'd artifacts into
//! `results/`. All runs are foreground; see `findings.md` for the verdict.

pub mod cases_io;
pub mod coverage;
pub mod golden;
pub mod probe;
pub mod typed_error;
pub mod util;

pub use coverage::{Coverage, CoverageSummary};
pub use golden::{Case, CaseResult, Expected, Outcome};
pub use typed_error::TypedError;
