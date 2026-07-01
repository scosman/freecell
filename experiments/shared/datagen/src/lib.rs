//! # datagen — deterministic synthetic data for FreeCell Phase 1 experiments
//!
//! This crate generates the **inputs** the Phase 1 sub-projects benchmark against:
//! a deterministic, seedable synthetic-sheet model (a proxy for "a big, difficult
//! spreadsheet"), engine-neutral formula-pattern generators for cascade
//! benchmarks, and CSV sample-file output. It exists so that every input is
//! produced by committed code and is trivially reproducible (functional_spec §5.3,
//! architecture §3).
//!
//! ## Design constraints
//!
//! - **Engine-neutral.** The spreadsheet engine (Formualizer or a pivot) is chosen
//!   at the Sub-project A gate, so this crate must not depend on any engine or
//!   xlsx-writer. It emits an abstract [`CellData`] model, `(address, formula)`
//!   pairs, and CSV. `.xlsx` sample generation is deferred to Sub-project B, which
//!   owns whatever writer the gate selects (architecture §6, §9).
//! - **Deterministic.** Every generator is a pure function of its `(seed, row,
//!   col)` inputs — no RNG state, no globals, no clock. The same inputs always
//!   yield the same output, so results are reproducible and generation is
//!   thread-safe.
//! - **Frozen after scaffolding.** Parallel sub-projects consume this crate
//!   **read-only** (architecture §1). If a phase needs a change here, it escalates
//!   rather than editing shared code.
//!
//! ## What's here
//!
//! - [`cell`] — the [`CellData`] / [`CellValue`] / [`CellFormat`] model, the
//!   [`CellAddress`] with Excel "A1" rendering, and the [`CellSource`] provider
//!   trait the UI PoC renders against.
//! - [`synthetic`] — [`SyntheticSheet`], a deterministic [`CellSource`] with
//!   varied text/numbers, ~10–20% highlighted cells, scattered bold/italic, and
//!   variable row/column sizes.
//! - [`formula`] — [`linear_chain`](formula::linear_chain) (the `=PREV+1` cascade)
//!   and [`wide_fanout`](formula::wide_fanout) shapes for propagation benchmarks.
//! - [`csv`] — CSV sample-file writers over any [`CellSource`].
//!
//! ## Example
//!
//! ```
//! use datagen::{csv_string, CellSource, SyntheticSheet};
//!
//! let sheet = SyntheticSheet::new(42, 1_000, 100);
//! let top_left = sheet.cell(0, 0); // deterministic CellData
//! assert_eq!(sheet.cell(0, 0), top_left);
//!
//! let csv = csv_string(&sheet, 5, 5); // 5x5 sample as CSV text
//! assert_eq!(csv.lines().count(), 5);
//! ```

pub mod cell;
pub mod csv;
pub mod formula;
pub mod synthetic;

pub use cell::{
    CellAddress, CellData, CellFormat, CellSource, CellValue, EXCEL_MAX_COLS, EXCEL_MAX_ROWS,
    HAlign, Rgb, column_label,
};
pub use csv::{csv_string, write_csv};
pub use formula::{FormulaCell, linear_chain, wide_fanout};
pub use synthetic::SyntheticSheet;
