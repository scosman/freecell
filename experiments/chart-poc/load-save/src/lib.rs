//! `load-save` — Experiment 1 (functional_spec §5): load/save chart **data stitching** beside
//! IronCalc, which reads and writes no chart data (`research/ironcalc-chart-exposure.md`).
//!
//! - [`load`] parses chart definitions **out of** an `.xlsx` zip (worksheet → drawing → chart
//!   relationship chain, cached `numCache`/`strCache`, **no** formula eval) **into**
//!   [`chart_model::Chart`]. This layer is gpui-free and IronCalc-free — the same `zip` +
//!   `roxmltree` second pass the app's `open_fixups.rs` already does.
//! - [`save`] re-injects the original chart parts into IronCalc's regenerated zip so a chart
//!   **survives** the chart-dropping writer (byte-preservation, §10 #2).
//! - [`authoring`] programmatically writes the example `.xlsx` fixtures (§10 #4).
//! - [`xlsx`] holds the shared zip + OPC-relationship helpers.
//!
//! The render proof (loading a fixture and rendering it through `chart-render` to a PNG) lives
//! in the `render_loaded`/`capture_loaded` bins behind the optional `render` feature, so this
//! library stays gpui-free.

pub mod authoring;
pub mod load;
pub mod save;
pub mod xlsx;

pub use load::{discover, load_charts_from_xlsx, parse_chart_xml, SheetDrawing};
pub use save::{reinject, save_with_charts, SaveReport};
