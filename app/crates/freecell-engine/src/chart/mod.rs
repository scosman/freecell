//! `freecell-engine::chart` — the chart **file layer** (charts/architecture §2, §4.1):
//! load/save chart data stitching beside IronCalc, which reads and writes no chart data.
//!
//! - [`load`] parses chart definitions **out of** an `.xlsx` zip (worksheet → drawing → chart
//!   relationship chain, cached `numCache`/`strCache`, **no** formula eval) **into**
//!   [`freecell_chart_model::Chart`]. gpui-free and IronCalc-free — a `zip` + `roxmltree`
//!   second pass over the OPC package.
//! - [`save`] re-injects the chart parts into IronCalc's regenerated zip so a chart **survives**
//!   the chart-dropping writer: an unedited chart is byte-preserved; an **edited-loaded** chart has
//!   its retained source **patched** (its `numCache`/`strCache` reflowed to current values, keeping
//!   `c:f` + styling — [`save::patch_chart_source`]); worksheets are mapped by
//!   name across IronCalc's regenerated parts (multi-sheet), failing loudly on a missing part (P10).
//! - [`binding`] (P9) turns the retained `c:f` refs into structured ranges + a range→chart index,
//!   and re-resolves a dirty chart's values from the current model — the live-binding machinery the
//!   worker drives (charts/architecture §4.1). gpui-free and IronCalc-free (it reads through
//!   closures).
//! - [`write`] is the **write-from-model** path (P16): it *synthesizes* chart XML (+ drawing / rels /
//!   content-types) from an **authored** [`freecell_chart_model::Chart`] into a workbook — the inverse
//!   of [`load`], and the third save write-mode beside [`save`]'s byte-preserve + edit-patch.
//! - [`range`] (P19) turns a picked data-range block into the per-series `c:f` refs an authored
//!   chart binds against — the structural half of shaping a near-empty chart into a real one.
//! - [`authoring`] programmatically writes example `.xlsx` fixtures (used by the tests).
//! - [`xlsx`] holds the shared zip + OPC-relationship helpers.
//!
//! Lifted from the chart PoC (`experiments/chart-poc/load-save`), extended through live binding
//! (P9), source-first save — byte-preserve + edit-reflow patch + multi-sheet part map (P10) — and
//! the write-from-model authoring path (P16).

pub mod authoring;
pub mod binding;
pub mod load;
pub mod range;
pub mod save;
pub mod write;
pub mod xlsx;

pub use binding::{parse_cf, CellData, ChartBinding, ChartBindings};
pub use load::{
    discover, discover_and_parse, discover_and_parse_by_sheet, discover_and_parse_for_part,
    discover_and_parse_for_sheet, load_charts_from_xlsx, parse_chart_xml, workbook_sheet_parts,
    DiscoveredChart, SheetDrawing,
};
pub use range::series_refs_from_block;
pub use save::{
    patch_chart_source, reinject, reinject_live_charts, save_with_charts, LiveChart, SaveReport,
};
pub use write::{
    serialize_chart_xml, synthesize_drawing_xml, write_authored_charts, AuthoredChart,
    AuthoredWriteReport, SeriesRefs,
};
