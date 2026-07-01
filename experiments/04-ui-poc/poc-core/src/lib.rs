//! # poc_core — engine-neutral, GPUI-free core for the FreeCell UI PoC (Sub-project E)
//!
//! The UI proof-of-concept builds **one macOS/Metal app in two variants** — a custom
//! grid on raw `gpui` (`raw-gpui/`) and one on `gpui-component`'s `Table`
//! (`gpui-component/`) — to answer whether GPUI can render an **Excel-max spreadsheet
//! grid** at the functional_spec §5.4 performance bar, and which variant is better
//! (functional_spec §6.E, architecture §7).
//!
//! GPUI targets macOS/Metal and cannot build in the headless Linux container, so all
//! the **load-bearing, testable logic** lives here, GPUI-free, and both shells are thin:
//!
//! - [`config`] — [`PocConfig`] (Excel-max grid, viewport, overscan) + the §5.4
//!   frame-time / cell-load thresholds.
//! - [`layout`] — [`Axis`]: variable-size virtualization via **segment-summed prefix
//!   sums + binary search**, sized for 1M+ rows without O(n) memory.
//! - [`style`] — [`RenderCell`]: GPUI-free conversion of a `datagen::CellData` into
//!   render-ready text + `0xRRGGBB` colours + font flags, so both shells look identical.
//! - [`harness`] — [`Harness`]: the scripted "Run Test" viewport sequence (scroll /
//!   fast-scroll / horizontal / jump / random-jump), advanced frame-by-frame.
//! - [`report`] — [`RunReport`] / [`finalize`]: p50/p99/max via `bench_util`, PASS/FAIL
//!   gates vs §5.4, and a recorded `BenchResult` JSON in `results/`.
//!
//! The static datamodel provider itself is the frozen `datagen::SyntheticSheet`
//! (`trait CellSource`); this crate never generates cells, only lays them out, styles
//! them, scripts the viewport, and gates the numbers.

pub mod config;
pub mod harness;
pub mod layout;
pub mod report;
pub mod style;

pub use config::{
    PocConfig, CELL_LOAD_TARGET_NS, FRAME_TARGET_NS, FRAME_WORST_NS,
};
pub use harness::{newly_visible, FrameSample, Harness, Move, Viewport};
pub use layout::Axis;
pub use report::{build_report, finalize, RunReport};
pub use style::{rgb_hex, Align, RenderCell};

// Re-export the provider surface the shells render against, so a shell can depend on
// just poc_core for the whole engine-neutral model.
pub use datagen::{
    CellData, CellSource, CellValue, HAlign, Rgb, SyntheticSheet, EXCEL_MAX_COLS,
    EXCEL_MAX_ROWS,
};
