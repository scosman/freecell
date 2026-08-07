//! The eval worker seam (`components/engine_worker.md`, `architecture.md §2`).
//!
//! One worker per window owns the IronCalc `UserModel` (via [`WorkbookDocument`]) on a
//! dedicated 64 MiB-stack thread and implements the validated SP1 seam carried to
//! `UserModel` by round-3 A: drain-coalesce commands → apply one recompute → **stage every shared
//! surface** (publication, style cache, chart snapshot, CF map) → bump the generation → notify the
//! UI (E1, `functional_spec.md F4` — it used to publish, bump, notify, and only then write the other
//! three surfaces). The UI holds only a
//! [`DocumentClient`] and reads published snapshots + the resident cache; **no IronCalc type
//! crosses this boundary** (`architecture.md §2`).
//!
//! - [`protocol`] — the engine-free `Command` / `WorkerEvent` contract.
//! - [`client`] — [`DocumentClient`] + the shared read-surfaces + [`WorkerEventReceiver`].
//! - `run` — the worker's loop (coalescing, the one `commit` point + its stage-then-bump ordering,
//!   catch_unwind + degraded policy, dirty-op accounting), the publication, and the save.
//! - [`charts`] — the chart half: the published [`ChartSnapshot`], the authored-chart store, the
//!   chart undo timeline, and every chart command (F1 — extracted from `run.rs`, which carried
//!   1,048 production lines of chart machinery beside this module's 39; see `charts.rs`'s own
//!   header for the provenance of those figures).
//!
//! [`WorkbookDocument`]: crate::WorkbookDocument

pub mod charts;
pub mod client;
pub mod protocol;
mod run;

pub use charts::ChartSnapshot;
pub use client::{join_worker, DocumentClient, WorkerEventReceiver, WorkerExit, WORKER_STACK_SIZE};
pub use freecell_chart_model::{ChartId, ChartInsertKind};
pub use protocol::{
    BorderLine, BorderPreset, ChartAxisKind, ChartChromeEdit, Command, DataLabelToggles,
    EditRejectedReason, FrozenAxis, PasteError, SheetMeta, StyleAttr, StylePath, WorkerEvent,
};
/// The frozen-pane band caps (B2, `functional_spec.md F1`). Crate-visible so
/// [`crate::cache::build_sheet_cache`] clamps the counts it copies out of a worksheet to the same
/// bound the publish loop enforces — the cache is what both the worker and the grid read.
pub(crate) use run::{MAX_FROZEN_COLS, MAX_FROZEN_ROWS};
