//! The eval worker's main loop (`components/engine_worker.md §Main loop`,
//! `architecture.md §2`).
//!
//! One `Worker` owns one [`WorkbookDocument`] (the IronCalc `UserModel`) on the dedicated
//! 64 MiB-stack thread. Its loop is the SP1 seam carried to `UserModel` (round-3 A):
//!
//! ```text
//! recv() (park when idle) → [first] + try_iter()   // DRAIN = coalescing
//!   → apply the coalesced edit batch under one paused/evaluate() recompute
//!   → publish the viewport snapshot, THEN bump the generation (publish-then-bump)
//!   → handle reads / saves / shutdown
//! ```
//!
//! Robustness (round-3 D): the input cap is **re-checked here** before any formula reaches
//! the recursive parser (the security boundary for the abort class); the apply+eval runs
//! inside `catch_unwind`; a caught panic degrades the worker per the locked policy rather
//! than taking down the window.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::Instant;

use std::collections::{BTreeMap, HashMap, HashSet};

use freecell_core::input_cap::validate_input;
use freecell_core::merge::blocks_fill;
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::tsv::{paste_fits, tsv_dims};
use freecell_core::{
    limits, CellKind, CellRange, CellRef, Direction, Publication, PublishedCell, SelectionStats,
    SheetId,
};

use freecell_chart_model::{
    Anchor, CfRange, Chart, ChartColor, ChartId, ChartInsertKind, ChartKind, ChartSpec, Color,
    Legend,
};

use crate::cache;
use crate::chart::binding::{
    binding_from_refs, binding_is_dirty, build_series_shells, resolve_chart, CellData,
    ChartBindings, RemovedChart, SheetResolver,
};
use crate::chart::write::{AuthoredChart, SeriesRefs};
use crate::document::{DocumentSource, FontFlag, SaveError, WorkbookDocument};
use std::path::Path;

use super::charts::ChartSnapshot;
use super::client::Shared;
use super::protocol::{
    ChartAxisKind, ChartChromeEdit, Command, EditRejectedReason, PasteError, SheetMeta, StyleAttr,
    WorkerEvent,
};

/// Whether the loop should keep running after a batch.
#[derive(Debug, PartialEq, Eq)]
enum Flow {
    Continue,
    Shutdown,
}

/// What an applied edit was, so the batch knows whether to recompute + which follow-up
/// events to emit.
#[derive(Debug, Clone, Copy)]
enum AppliedKind {
    /// A value/undo/redo/clear edit — needs a recompute before republishing values.
    Cell,
    /// A style-only edit — publishes (ships cache deltas) but needs **no** recompute: styles
    /// don't affect values (component-doc command table).
    StyleOnly,
    /// An add/rename/delete sheet — needs a recompute (can affect formulas) and also emits
    /// `SheetsChanged`.
    SheetOp,
    /// A row/column resize — geometry only (no recompute), but the whole active sheet's cache is
    /// rebuilt (the axis geometry changed; `components/grid_structure.md §5.1`).
    GeometryOnly,
    /// An insert/delete rows/columns — content + geometry + formulas shift, so it needs a
    /// recompute **and** a full active-sheet cache rebuild (`components/grid_structure.md §5.3`).
    Structure,
    /// A conditional-formatting mutation (add/update/delete/reorder a rule). Values are unchanged,
    /// but the CF results depend on the new rule set, so IronCalc's `cf_cache` must be refreshed
    /// **before** the `AppliedOp::Rebuild` cache refresh reads it (BUG-1). Under the batch's paused
    /// evaluation the fork's internal `evaluate_if_not_paused()` is a no-op, so this variant sets
    /// `needs_eval = true` to force the coalesced `doc.evaluate()` (which re-evaluates CF) to run —
    /// otherwise the freshly-added rule only shows after a later value edit. Records its op like
    /// `StyleOnly` (maps to `AppliedOp::Rebuild { sheet }`). A full workbook recompute on a CF
    /// mutation is acceptable (user-driven, infrequent; perf follow-up tracked as GAPS CF8). That
    /// forced full-workbook `evaluate()` also re-rolls volatile functions (`RAND`/`NOW`/`TODAY`/
    /// `OFFSET`, etc.) — also acceptable, since it is user-driven/infrequent and Excel likewise
    /// recalculates volatiles broadly.
    CondFmt,
    /// An edit that resolved to a **no change** (e.g. a fill whose selection is an edge/seed-line
    /// no-op, or whose full-line target clamped to an empty used range). It touched no cell, so the
    /// batch must not count it, recompute, republish, or record an undo op for it.
    NoOp,
}

/// The cache **touch-set** of one applied undoable op, recorded so `Undo`/`Redo` can re-read the
/// affected cells (`components/style_cache.md §Lifecycle`: undo/redo re-reads the recorded
/// touch-set). Kept in a parallel worker-side history aligned 1:1 with IronCalc's undo stack.
#[derive(Debug, Clone)]
enum Touch {
    /// A cell/style/clear edit touched `range` on `sheet`; re-read those cells to mirror it.
    Cells { sheet: SheetId, range: CellRange },
    /// A paste touched one or more `(sheet, range)`s in a **single** undo entry (the pasted
    /// destination plus, on a cut, the cleared source — possibly a different sheet). On
    /// undo/redo, every listed range is re-read (`components/clipboard.md §Paste`).
    Ranges(Vec<(SheetId, CellRange)>),
    /// A sheet add/rename/delete; on undo/redo, reconcile the caches map + rebuild the active
    /// sheet (a returning deleted sheet rebuilds lazily on next activation).
    Sheets,
    /// A geometry resize or a structural insert/delete on `sheet`. The touched region is
    /// unbounded (everything at/after the edit shifts) + the axis geometry changed, so on
    /// undo/redo the sheet's cache is **rebuilt** wholesale (`build_and_store_cache`) rather than
    /// re-reading a cell range (`components/grid_structure.md §5.1, §5.3`).
    Rebuild { sheet: SheetId },
}

/// The worker-held clipboard slot (`architecture.md §6`, `components/clipboard.md`): the engine
/// `Clipboard` payload isn't nameable outside ironcalc_base, so `data` is its serialized
/// `ClipboardData`. `sheet` is the **stable** source [`SheetId`] (resolved to an index at paste
/// time, so a copy survives a sheet add/reorder); `range` is the engine's effective 1-based
/// source rectangle; `cut` drives move-vs-copy semantics + single-use clearing.
struct ClipboardSlot {
    sheet: SheetId,
    range: (i32, i32, i32, i32),
    data: serde_json::Value,
    cut: bool,
    /// The copied block's computed values as literal paste tokens (row-major over `range`) — the
    /// Paste Values (⌘⇧V) source. Captured at copy time alongside `data`; see
    /// [`WorkbookDocument::copied_value_tokens`](crate::document::WorkbookDocument).
    values: Vec<Vec<String>>,
}

/// One **authored** chart the worker holds (P17, charts/write-path §1 mode 3). Distinct from the
/// loaded [`ChartBindings`]: an authored chart is **snapshot-but-not-live** — it rides the published
/// [`ChartSnapshot`] so the grid renders it, but it carries no `c:f` binding yet (ranges arrive in
/// P19), so it is **never** touched by the dirty-set re-resolve and is saved by the
/// **write-from-model** path ([`write::write_authored_charts`](crate::chart::write::write_authored_charts)),
/// never the loaded re-inject. `spec.origin` is always [`Authored`](freecell_chart_model::Origin::Authored).
#[derive(Clone)]
struct AuthoredEntry {
    /// The worksheet the chart is anchored on (keys the published snapshot; resolved to the current
    /// worksheet name at save time, so an in-session rename follows and a deleted host drops it).
    anchor_sheet: SheetId,
    /// The stable manipulation handle (P18) the worker stamps onto the published spec, so the app
    /// can name this authored chart back for move/resize/delete.
    id: ChartId,
    /// The authored render envelope (a `ChartSpec::authored`, no retained source).
    spec: ChartSpec,
    /// The per-series `c:f` references once a **data range** is set (P19) — **empty** for a still
    /// near-empty placeholder. This is the source of truth for a bound authored chart: its live
    /// re-resolve derives a `ChartBinding` from it ([`binding_from_refs`]), and the write path
    /// consumes it directly so the saved chart carries `c:f` + caches (not literals). Setting a range
    /// (or switching type on a bound chart) rebuilds these; the chart becomes LIVE the moment it is
    /// non-empty.
    refs: Vec<SeriesRefs>,
}

/// The outcome of a guarded paste (`run_guarded_paste`): applied (with the pasted 0-based
/// rectangle the engine re-selected), a clean engine error, or a caught panic.
enum PasteOutcome {
    Applied(CellRange),
    EngineError(String),
    Panicked,
}

/// What one successfully-applied edit was, for post-eval cache bookkeeping. `Cells`/`Sheets`
/// push a [`Touch`]; `Undo`/`Redo` pop/move one (they consume history, don't create it).
enum AppliedOp {
    Cells {
        sheet: SheetId,
        range: CellRange,
    },
    Sheets,
    /// A resize / insert / delete on `sheet` → the sheet cache is fully rebuilt (see
    /// [`Touch::Rebuild`]).
    Rebuild {
        sheet: SheetId,
    },
    Undo,
    Redo,
}

/// One entry on the **unified undo/redo timeline** (charts feedback item 4). Ctrl+Z pops the single
/// most-recent user action regardless of kind, so IronCalc-backed cell/style/sheet edits and
/// worker-side chart ops share one ordered stack:
/// - `Cell(touch)` — an IronCalc-undoable edit. Its inverse is IronCalc's own `undo()`, and the
///   [`Touch`] is re-read to keep the caches in agreement. These entries stay **1:1** with
///   IronCalc's undo stack (a chart entry never pushes onto it), so the two never desync.
/// - `Chart(cu)` — a chart insert/delete/anchor/range op, inverted from a stashed snapshot
///   ([`ChartUndo`]) entirely worker-side (no IronCalc undo/redo call).
///
/// This **reverses** the earlier P18 decision (chart ops off the Ctrl+Z stack): a half-integration
/// would desync ordering on an interleave (cellEdit → insertChart → deleteChart, then Undo×2 must
/// restore-then-remove the chart, not restore-then-undo-the-cell), so all four chart ops ride here.
enum UndoEntry {
    Cell(Touch),
    Chart(ChartUndo),
}

/// The stashed inverse (and redo) of one chart op — enough whole worker state to both revert it and
/// re-apply it. Deliberately snapshot-based (clone the affected entry) rather than delta-based:
/// simple + obviously correct over lean. The heavy snapshots ([`AuthoredEntry`] / [`RemovedChart`])
/// are boxed so the enum stays small (an undo stack is cold, so the indirection is free).
enum ChartUndo {
    /// Insert of an **authored** chart at list `index` (the born-live `entry` stashed whole, so a
    /// redo re-inserts it with its Batch-3 range binding intact — no re-derivation). Undo removes
    /// index; redo re-inserts `entry` at index.
    InsertAuthored {
        index: usize,
        entry: Box<AuthoredEntry>,
    },
    /// Delete of an **authored** chart. Undo re-inserts `entry` at `index`; redo removes index.
    DeleteAuthored {
        index: usize,
        entry: Box<AuthoredEntry>,
    },
    /// Delete of a **loaded** chart. `removed` is the whole binding (so undo re-binds it exactly);
    /// `chart_part` is the save-drop key the delete added to `loaded_deletes`; `prior_anchor_edit`
    /// is the `loaded_anchor_edits` value the delete evicted (restored on undo). Undo re-binds +
    /// clears the save-set bookkeeping; redo re-runs the delete effects.
    DeleteLoaded {
        removed: Box<RemovedChart>,
        chart_part: String,
        prior_anchor_edit: Option<Anchor>,
    },
    /// Anchor move/resize of an **authored** chart: swap `prior` and `applied` on the chart's model
    /// anchor.
    SetAnchorAuthored {
        id: ChartId,
        prior: Anchor,
        applied: Anchor,
    },
    /// Anchor move/resize of a **loaded** chart: swap the render anchor AND the `loaded_anchor_edits`
    /// entry. `prior_render` is the render anchor before the move; `prior_edit` is the
    /// `loaded_anchor_edits` value the move replaced (restored on undo).
    SetAnchorLoaded {
        id: ChartId,
        chart_part: String,
        prior_render: Anchor,
        prior_edit: Option<Anchor>,
        applied: Anchor,
    },
    /// Range bind of an **authored** chart (P19): the whole pre-bind `prior` entry and post-bind
    /// `applied` entry are stashed, so undo/redo just restore the clone (no re-derivation).
    SetRangeAuthored {
        index: usize,
        prior: Box<AuthoredEntry>,
        applied: Box<AuthoredEntry>,
    },
}

/// The per-window worker: owns the document + the shared read-surfaces and drives the loop.
pub(super) struct Worker {
    doc: WorkbookDocument,
    shared: Arc<Shared>,
    event_tx: async_channel::Sender<WorkerEvent>,
    /// The active sheet (stable id) — the one the published viewport covers.
    active_sheet: SheetId,
    /// The stored overscanned viewport (already overscanned UI-side), clamped to sheet bounds.
    /// `None` until the first `SetViewport` (the initial publish is empty).
    viewport: Option<Viewport>,
    /// Committed undoable ops (dirty tracking; mirrored to `Shared::committed_ops`).
    ops_seen: u64,
    /// Number of **worker-initiated** `evaluate()` calls — the test-observable coalescing
    /// metric. This measures worker behavior (one recompute per drained batch), NOT the
    /// engine's internal recompute count; IronCalc's own coalescing was validated in
    /// round-3 A and Phase 12's perf harness catches recompute regressions.
    eval_count: u64,
    /// Set after an unrecoverable panic: keep serving reads/save, refuse edits.
    degraded: bool,
    /// Count of caught panics (a second one, or an unresponsive probe, degrades the worker).
    panic_count: u32,
    /// The **unified undo timeline** ([`UndoEntry`], charts feedback item 4): one ordered stack over
    /// IronCalc-backed cell edits AND worker-side chart ops, so Ctrl+Z pops the single most-recent
    /// action regardless of kind. A new undoable action (edit OR chart op) pushes here + clears
    /// `redo_stack`; `Undo` pops here → `redo_stack`; `Redo` the reverse. The `Cell(Touch)` entries
    /// stay 1:1 with IronCalc's undo stack (a `Chart` entry never touches it) and re-read the popped
    /// touch-set to keep the caches in agreement; `Chart` entries invert from their stashed snapshot.
    undo_stack: Vec<UndoEntry>,
    /// The redo side of the unified timeline (mirrors IronCalc's redo stack for its `Cell` entries).
    redo_stack: Vec<UndoEntry>,
    /// The range clipboard slot (`architecture.md §6`): `Some` after a copy/cut, replaced by the
    /// next copy/cut, and cleared after a cut is pasted (single-use).
    clipboard: Option<ClipboardSlot>,
    /// The live-bound charts this workbook owns (P9, charts/architecture §4.1) — the range→chart
    /// index the worker re-resolves on edit. Empty for a new/unopened or chart-less workbook.
    charts: ChartBindings,
    /// The **authored** (in-app inserted) charts this workbook owns (P17), held separately from the
    /// loaded [`charts`](Self::charts): they ride the published snapshot but are never re-resolved
    /// (no binding yet) and are saved via the write-from-model path, not the loaded re-inject.
    authored_charts: Vec<AuthoredEntry>,
    /// Monotonic source of stable [`ChartId`]s (P18), shared across loaded + authored charts so a
    /// manipulation id names exactly one chart. Starts at 1 ([`ChartId::NONE`] = 0 is unassigned).
    next_chart_id: u64,
    /// Loaded charts moved/resized in-session (P18): `chart_part → new twoCellAnchor`, accumulated
    /// **relative to the current [`chart_source_path`](Self::chart_source_path)**. The save patches
    /// each into the retained drawing part; a save that advances the source (bakes them in) clears
    /// this. An authored-charts-present save keeps the source (and this map) put.
    loaded_anchor_edits: HashMap<String, Anchor>,
    /// Loaded charts deleted in-session (P18): the `chart_part`s the save must drop from the
    /// package (their `twoCellAnchor` + part chain), also relative to `chart_source_path`. Deleted
    /// parts are additionally skipped by the save-time discovery sweep so they can't be re-bound.
    loaded_deletes: HashSet<String>,
    /// The published [`ChartSnapshot`] version — bumped on load (when charts exist) and on each
    /// dirty re-resolve, so the UI installs charts only when they actually change.
    chart_version: u64,
    /// The file whose chart machinery (drawings, chart parts, content-type overrides) a
    /// chart-preserving save re-injects into the model body (P10, charts/architecture §4.1/§5):
    /// the opened path on load, then the last path successfully saved (a chart-preserving save
    /// writes a self-contained superset, so the just-saved file is a valid source for the next
    /// save — surviving a Save-As away from a since-deleted original). `None` for a workbook never
    /// opened from a file; then save falls through to the plain (chart-less) writer.
    chart_source_path: Option<PathBuf>,
    /// The sheets whose chart drawings have already been **walked** (P11 lazy discovery,
    /// charts/architecture §5 challenge 5). A sheet is inserted the first time it is painted so its
    /// zip is walked **at most once** — even if it carries no charts (so we don't re-parse on every
    /// scroll). Correctness (never double-binding a chart) is `ChartBindings::add_missing`'s job;
    /// this set is purely the "walk each sheet once" guard.
    discovered_chart_sheets: HashSet<SheetId>,
    /// Set once every sheet's charts have been discovered — after the save-time full sweep
    /// (`ensure_all_charts_discovered`), or for a workbook that was never opened from a file. Short-
    /// circuits all further lazy per-sheet walks.
    charts_fully_discovered: bool,
    /// The **stable** `SheetId → file worksheet part` map (e.g. `xl/worksheets/sheet2.xml`),
    /// captured **once at open** by joining the model's at-open sheet names with the file's
    /// `workbook.xml.rels` name→part map (P11 CR fix). Keying lazy discovery + the save sweep on
    /// this — rather than the *current* sheet name — is what keeps them **rename-safe**: a sheet
    /// renamed in-session keeps its `SheetId`, so its charts still resolve to their file part, and
    /// the chart follows the rename on save (`live_sheet_targets` resolves `SheetId → current
    /// name`). Empty for a workbook never opened from a file, or if the map couldn't be read.
    chart_sheet_parts: HashMap<SheetId, String>,
    /// Per-sheet **manually-resized** 0-based rows (`functional_spec.md §3.3`). A row enters when a
    /// **user** [`Command::SetRowHeights`] commits, or is seeded at first cache build from a loaded
    /// `custom_height` row. Wrap-driven auto-grow ([`Command::AutoGrowRowHeights`]) never touches a
    /// manual row (neither grows nor shrinks it) and never adds to this set. **Session-scoped** — not
    /// persisted to xlsx (a reloaded file's non-custom-height rows start auto).
    manual_rows: HashMap<SheetId, HashSet<u32>>,
    /// Per-sheet **wrap-driven** row heights (device px) the UI measured on the render thread — the
    /// auto-grow **contribution** kept separate from IronCalc's own row heights (font/newline
    /// auto-fit / user resize). The resident cache's height for such a row is
    /// `max(base_ironcalc, wrap)`; holding the wrap part here lets it **survive a full cache rebuild**
    /// (resize / insert / delete / band edit) — re-projected in [`build_and_store_cache`] — instead of
    /// being wiped back to the IronCalc base. Only ever holds **auto** rows (manual rows are dropped).
    wrap_heights: HashMap<SheetId, BTreeMap<u32, f32>>,
}

/// A clamped, half-open viewport window on the active sheet.
#[derive(Debug, Clone)]
struct Viewport {
    rows: std::ops::Range<u32>,
    cols: std::ops::Range<u32>,
}

impl Worker {
    /// The thread entry point: builds the document (real I/O, on this thread), emits
    /// `Loaded` / `LoadFailed`, then runs the loop until shutdown or the command channel
    /// closes.
    pub(super) fn load_and_run(
        source: DocumentSource,
        shared: Arc<Shared>,
        event_tx: async_channel::Sender<WorkerEvent>,
        cmd_rx: Receiver<Command>,
    ) {
        let doc = match WorkbookDocument::from_source(&source) {
            Ok(doc) => doc,
            Err(error) => {
                let _ = event_tx.try_send(WorkerEvent::LoadFailed { error });
                return;
            }
        };

        let mut worker = Worker {
            doc,
            shared,
            event_tx,
            active_sheet: SheetId(0),
            viewport: None,
            ops_seen: 0,
            eval_count: 0,
            degraded: false,
            panic_count: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            clipboard: None,
            charts: ChartBindings::default(),
            authored_charts: Vec::new(),
            next_chart_id: 1,
            loaded_anchor_edits: HashMap::new(),
            loaded_deletes: HashSet::new(),
            chart_version: 0,
            chart_source_path: match &source {
                DocumentSource::OpenFile(path) => Some(path.clone()),
                // A CSV import builds a fresh workbook — no source file carries charts.
                DocumentSource::NewWorkbook | DocumentSource::ImportCsv(_) => None,
            },
            discovered_chart_sheets: HashSet::new(),
            // A workbook not opened from an `.xlsx` (new, or CSV-imported) has no charts to
            // discover — start "fully discovered" so save takes the plain path without a wasted walk.
            charts_fully_discovered: matches!(
                source,
                DocumentSource::NewWorkbook | DocumentSource::ImportCsv(_)
            ),
            chart_sheet_parts: HashMap::new(),
            manual_rows: HashMap::new(),
            wrap_heights: HashMap::new(),
        };

        // Point the active sheet at the first sheet's real stable id, and seed an empty
        // publication for it (first paint uses the file's cached values once a viewport
        // arrives — no eval on open, SP2).
        let sheets = worker.sheet_metas();
        if let Some(first) = sheets.first() {
            worker.active_sheet = first.id;
        }
        worker
            .shared
            .publication
            .store(Arc::new(Publication::empty(worker.active_sheet, 0)));

        // Build the active sheet's style & geometry cache on open, so first paint has geometry +
        // styles resident (values follow on the first `SetViewport`). Non-active sheets build on
        // first activation (`components/style_cache.md §Lifecycle`).
        worker.build_and_store_cache(worker.active_sheet);
        // Publish the CF rule list for every sheet that already carries rules (an opened file), so
        // the window can build a CF sidebar synchronously without waiting for a mutation.
        worker.publish_all_cond_fmt_on_open();
        worker.emit(WorkerEvent::Loaded { sheets });
        worker.emit(WorkerEvent::StyleCacheUpdated {
            sheet: worker.active_sheet,
        });

        // Chart **XML parsing** is lazy + off open's critical path (P11, charts/architecture §5
        // challenge 5): no chart part is parsed here — that would block the first
        // `SetViewport → Published` (first cell-value paint) behind a zip walk. Each sheet's charts
        // are parsed the first time that sheet is painted (`ensure_sheet_charts_discovered`, run
        // **after** the viewport publish), and a save forces a full sweep so a never-painted chart
        // sheet is still preserved. What we DO capture eagerly is the tiny, rename-safe
        // `SheetId → file worksheet part` map — no chart XML, just `workbook.xml.rels` — joined while
        // the model's sheet names still match the file. Both discovery paths key off this stable
        // part (not the mutable live name), so a sheet renamed before it is painted still resolves to
        // its charts and follows the rename on save (P11 CR fix).
        if let DocumentSource::OpenFile(path) = &source {
            worker.chart_sheet_parts = worker.build_chart_sheet_part_map(path);
        }

        worker.run(cmd_rx);
    }

    /// Block for a command, drain the rest of the queue (coalescing), process the batch.
    fn run(&mut self, cmd_rx: Receiver<Command>) {
        while let Ok(first) = cmd_rx.recv() {
            let mut batch = vec![first];
            batch.extend(cmd_rx.try_iter()); // DRAIN — the whole queue collapses into one batch
            if self.process_batch(batch) == Flow::Shutdown {
                break;
            }
        }
    }

    /// Split a drained batch into edits / viewport / reads / saves / shutdown, then apply the
    /// edits under a single coalesced eval + publish, then service the control commands.
    fn process_batch(&mut self, batch: Vec<Command>) -> Flow {
        let mut edits: Vec<Command> = Vec::new();
        let mut reads: Vec<(SheetId, CellRef, u64)> = Vec::new();
        // Selection-stats queries (`Command::SelectionStats`) are pure reads (no eval/publish),
        // computed after the edit batch so they observe every mutation in this batch.
        let mut stats_ops: Vec<(SheetId, CellRange, u64)> = Vec::new();
        // Edge-of-data resolves (`Command::ResolveEdge`) are pure reads too (⌘+arrow target lookup),
        // run after the edit batch so a jump observes this batch's mutations.
        let mut edge_ops: Vec<(SheetId, CellRef, Direction, u64)> = Vec::new();
        let mut saves: Vec<(PathBuf, u64)> = Vec::new();
        // CSV exports (`Command::ExportCsv`) are pure reads (used-range → raw values → file); run
        // after the edit batch so an export observes this batch's mutations, and never touch dirty.
        let mut exports: Vec<(SheetId, PathBuf, u64)> = Vec::new();
        // Clipboard ops (copy/cut/paste) run one-by-one after the edit batch — a paste is one
        // undo entry, and running it after the batch keeps the undo/touch-set stacks aligned.
        let mut clipboard_ops: Vec<Command> = Vec::new();
        // Font ops (`SetFont`) also run one-by-one: each emits a *variable* number of engine
        // diff-lists (one style paste + K row-height runs), so the touch-set must stay 1:1 with
        // the undo stack — they can't ride the generic coalesced edit path.
        let mut font_ops: Vec<Command> = Vec::new();
        // Chart ops (`InsertChart` / `SetChartAnchor` / `DeleteChart` / `SetChartRange` / type /
        // chrome) run one-by-one after the edit batch: each mutates the authored/loaded chart set +
        // republishes the chart snapshot. Insert/delete/anchor/range now push onto the unified undo
        // timeline (charts feedback item 4); type/chrome stay immediate (they only invalidate redo).
        let mut chart_ops: Vec<Command> = Vec::new();
        // Find scans (`Command::Find`) are pure reads (no eval/publish); replace ops
        // (`ReplaceOne`/`ReplaceAll`) mutate one-by-one after the edit batch, each carrying its own
        // eval + publish + undo touch entry (like clipboard ops), so the undo/touch stacks stay
        // aligned (`functional_spec.md §4`).
        let mut find_ops: Vec<(SheetId, String, bool, bool)> = Vec::new();
        let mut replace_ops: Vec<Command> = Vec::new();
        // Wrap-driven auto-grow (`Command::AutoGrowRowHeights`): a cache-only geometry update
        // applied after the edit batch (so it sees the batch's fresh IronCalc row heights as its
        // `base`). Never an IronCalc edit → rides no undo/touch stack (§3.4).
        let mut autogrow_ops: Vec<(SheetId, Vec<(u32, f32)>)> = Vec::new();
        // Undo / Redo run one-by-one AFTER the edit + chart ops (a coalesced batch applies its
        // forward ops first, then peels them back in most-recent-first order). Each dispatches on the
        // unified timeline's top entry: a `Cell` top drives IronCalc's undo/redo (via the single-
        // command edit path), a `Chart` top is inverted worker-side (charts feedback item 4).
        let mut undo_ops: Vec<Command> = Vec::new();
        let mut viewport_changed = false;
        // Every sheet activated in this drained batch (in order), so lazy chart discovery walks
        // EACH one — not just the batch's final active sheet (a batch that activates A then B must
        // still discover A's charts, P11 CR Mild #1).
        let mut activated_sheets: Vec<SheetId> = Vec::new();
        let mut shutdown = false;

        for cmd in batch {
            // Exhaustive routing (no catch-all): a newly added Command variant must be
            // explicitly classified as control or edit here — it can never silently fall
            // through to the apply path.
            match cmd {
                Command::SetViewport { sheet, rows, cols } => {
                    self.active_sheet = sheet;
                    self.viewport = Some(clamp_viewport(rows, cols));
                    viewport_changed = true;
                    if !activated_sheets.contains(&sheet) {
                        activated_sheets.push(sheet);
                    }
                }
                Command::GetCellContent {
                    sheet,
                    cell,
                    req_id,
                } => reads.push((sheet, cell, req_id)),
                Command::SelectionStats {
                    sheet,
                    range,
                    req_id,
                } => stats_ops.push((sheet, range, req_id)),
                Command::ResolveEdge {
                    sheet,
                    from,
                    dir,
                    req_id,
                } => edge_ops.push((sheet, from, dir, req_id)),
                Command::Find {
                    sheet,
                    query,
                    match_case,
                    whole_cell,
                } => find_ops.push((sheet, query, match_case, whole_cell)),
                replace @ (Command::ReplaceOne { .. } | Command::ReplaceAll { .. }) => {
                    replace_ops.push(replace)
                }
                Command::AutoGrowRowHeights { sheet, heights } => {
                    autogrow_ops.push((sheet, heights))
                }
                Command::Save { path, req_id } => saves.push((path, req_id)),
                Command::ExportCsv {
                    sheet,
                    path,
                    req_id,
                } => exports.push((sheet, path, req_id)),
                Command::Shutdown => shutdown = true,
                clip @ (Command::CopySelection { .. }
                | Command::PasteInternal { .. }
                | Command::PasteValues { .. }
                | Command::PasteTsv { .. }) => clipboard_ops.push(clip),
                font @ Command::SetFont { .. } => font_ops.push(font),
                chart @ (Command::InsertChart { .. }
                | Command::SetChartAnchor { .. }
                | Command::DeleteChart { .. }
                | Command::SetChartRange { .. }
                | Command::SetChartType { .. }
                | Command::SetChartChrome { .. }) => chart_ops.push(chart),
                edit @ (Command::SetCellInput { .. }
                | Command::ClearCells { .. }
                | Command::FillDown { .. }
                | Command::FillRight { .. }
                | Command::FillDrag { .. }
                | Command::SetStyleAttr { .. }
                | Command::SetStylePath { .. }
                | Command::SetBorders { .. }
                // CF mutations bucket with the style edits: each is style-only (no recompute) and
                // undoable, so it rides the coalesced-eval + publish + undo/redo machinery. The
                // CF-map republish is folded into `apply_edit_batch` (§components/engine_cf.md §5).
                | Command::AddCondFmt { .. }
                | Command::UpdateCondFmt { .. }
                | Command::DeleteCondFmt { .. }
                | Command::RaiseCondFmtPriority { .. }
                | Command::LowerCondFmtPriority { .. }
                | Command::SetColumnWidths { .. }
                | Command::SetRowHeights { .. }
                | Command::SetRowsHidden { .. }
                | Command::SetColumnsHidden { .. }
                | Command::InsertRows { .. }
                | Command::InsertColumns { .. }
                | Command::DeleteRows { .. }
                | Command::DeleteColumns { .. }
                // Merge/unmerge ride the coalesced-eval + publish + undo/redo machinery like a
                // structural insert/delete (they clear covered content + rebuild the sheet cache,
                // which re-reads `merged_regions`), so they bucket with the edits.
                | Command::MergeCells { .. }
                | Command::UnmergeCells { .. }
                | Command::AddSheet
                | Command::RenameSheet { .. }
                | Command::DeleteSheet { .. }
                | Command::MoveSheet { .. }) => edits.push(edit),
                undo @ (Command::Undo | Command::Redo) => undo_ops.push(undo),
                #[cfg(test)]
                edit @ Command::TestPanic => edits.push(edit),
            }
        }

        // Edits first (they carry the coalesced eval + a fresh publish). The publish uses the
        // viewport already updated above, so a batch of {scroll, edit} publishes once.
        let published = if edits.is_empty() {
            false
        } else {
            self.apply_edit_batch(edits)
        };

        // A pure viewport change (no edit) still republishes current values (no eval).
        if !published && viewport_changed {
            self.publish();
            self.emit(WorkerEvent::Published);
        }

        // Activating a sheet (a viewport switch to it) builds its style/geometry cache on first
        // visit, then stays resident (`components/style_cache.md §Lifecycle`).
        if viewport_changed && self.ensure_active_cache_built() {
            self.emit(WorkerEvent::StyleCacheUpdated {
                sheet: self.active_sheet,
            });
        }

        // Lazy chart discovery (P11, charts/architecture §5 challenge 5): the first time a sheet is
        // painted, walk + bind its charts — AFTER the viewport publish above, so the cells paint
        // first and the parse is off the first-paint critical path. Walks EACH sheet the batch
        // activated (not just the final one). A no-op on every later frame.
        for sheet in activated_sheets {
            self.ensure_sheet_charts_discovered(sheet);
        }

        // Font ops after the edit batch (each is standalone: its own style paste + auto-grow +
        // publish + touch-set entries).
        for font in font_ops {
            if let Command::SetFont {
                sheet,
                range,
                family,
                size_pt,
            } = font
            {
                self.apply_set_font(sheet, range, family, size_pt);
            }
        }

        // Wrap-driven auto-grow after the edit + font batches (so the font auto-grow already set
        // its IronCalc base height): a cache-only geometry update per sheet, riding no undo/touch
        // stack (`functional_spec.md §3.4`).
        for (sheet, heights) in autogrow_ops {
            self.apply_auto_grow(sheet, heights);
        }

        // Chart ops after the edit batch (each is standalone: it mutates the authored/loaded chart
        // set + republishes the chart snapshot). Insert/delete/anchor/range push a `Chart` entry onto
        // the unified undo timeline (charts feedback item 4); none is an IronCalc edit, so none
        // touches IronCalc's own undo stack — see the `UndoEntry` doc + `push_chart_undo`.
        for op in chart_ops {
            match op {
                Command::InsertChart {
                    sheet,
                    kind,
                    anchor,
                    data,
                } => self.insert_authored_chart(sheet, kind, anchor, data),
                Command::SetChartAnchor { sheet, id, anchor } => {
                    self.set_chart_anchor(sheet, id, anchor)
                }
                Command::DeleteChart { sheet, id } => self.delete_chart(sheet, id),
                Command::SetChartRange { sheet, id, data } => self.set_chart_range(sheet, id, data),
                Command::SetChartType { sheet, id, kind } => self.set_chart_type(sheet, id, kind),
                Command::SetChartChrome { sheet, id, edit } => {
                    self.set_chart_chrome(sheet, id, edit)
                }
                _ => unreachable!("only chart ops are bucketed here"),
            }
        }

        // Undo / Redo after the forward ops (edits + charts), so a coalesced batch peels back its
        // most-recent action first. Each dispatches on the unified timeline (charts feedback item 4).
        for op in undo_ops {
            match op {
                Command::Undo => self.apply_undo(),
                Command::Redo => self.apply_redo(),
                _ => unreachable!("only Undo/Redo are bucketed here"),
            }
        }

        // Clipboard ops after the edit batch (each is standalone; a paste carries its own eval +
        // publish + one undo entry).
        for clip in clipboard_ops {
            self.apply_clipboard_op(clip);
        }

        // Replace ops after the edit batch (each mutates the model: its own guarded eval + publish +
        // undo touch entries), so a find run below sees the replaced state.
        for replace in replace_ops {
            self.apply_replace_op(replace);
        }

        // Find scans are pure reads (no eval/publish) — run them last so they observe every mutation
        // in this batch.
        for (sheet, query, match_case, whole_cell) in find_ops {
            let matches = match self.resolve(sheet) {
                Some(idx) => self
                    .doc
                    .find_matches(idx, &query, match_case, whole_cell)
                    .unwrap_or_default(),
                None => Vec::new(),
            };
            self.emit(WorkerEvent::FindResults { matches });
        }

        for (sheet, cell, req_id) in reads {
            let raw = match self.resolve(sheet) {
                Some(idx) => self.doc.cell_content(idx, cell).unwrap_or_default(),
                None => String::new(),
            };
            self.emit(WorkerEvent::CellContent { req_id, raw });
        }

        // Selection-stats reads (pure, like `reads`): aggregate the selection's populated cells and
        // reply. An unresolvable sheet (deleted mid-flight) replies the empty aggregate.
        for (sheet, range, req_id) in stats_ops {
            let stats = match self.resolve(sheet) {
                Some(idx) => self.doc.selection_stats(idx, range),
                None => SelectionStats::EMPTY,
            };
            self.emit(WorkerEvent::SelectionStats { req_id, stats });
        }

        // Edge-of-data resolves (pure, like `reads`/`stats_ops`): walk the active line's populated
        // cells and reply the ⌘+arrow target. An unresolvable sheet (deleted mid-flight) replies the
        // origin cell (no move).
        for (sheet, from, dir, req_id) in edge_ops {
            let target = match self.resolve(sheet) {
                Some(idx) => self.doc.resolve_edge(idx, from, dir),
                None => from,
            };
            self.emit(WorkerEvent::EdgeResolved { req_id, target });
        }

        for (path, req_id) in saves {
            match self.save_workbook(&path) {
                Ok(()) => self.emit(WorkerEvent::Saved {
                    req_id,
                    ops_seen: self.ops_seen,
                }),
                Err(error) => self.emit(WorkerEvent::SaveFailed { req_id, error }),
            }
        }

        // CSV exports (pure reads — no dirty change). An unresolvable sheet (deleted mid-flight)
        // fails cleanly rather than writing a wrong sheet.
        for (sheet, path, req_id) in exports {
            let result = match self.resolve(sheet) {
                Some(idx) => self.doc.export_csv(idx, &path),
                None => Err(crate::document::SaveError::Io(
                    "the sheet no longer exists".to_string(),
                )),
            };
            match result {
                Ok(()) => self.emit(WorkerEvent::CsvExported { req_id }),
                Err(error) => self.emit(WorkerEvent::CsvExportFailed { req_id, error }),
            }
        }

        if shutdown {
            Flow::Shutdown
        } else {
            Flow::Continue
        }
    }

    /// Apply a coalesced edit batch: pre-validate (cap / name) outside the panic guard, then
    /// apply the survivors under one `catch_unwind`-guarded paused recompute, then
    /// publish-then-bump. Returns whether it published. Emits the SP1 observable timings
    /// (apply / eval / publish) at `debug` — Phase 12's perf harness reads these.
    fn apply_edit_batch(&mut self, edits: Vec<Command>) -> bool {
        // Clean rejects (no panic risk): input cap + sheet-name re-check + fill merge-guard.
        let mut valid: Vec<Command> = Vec::new();
        for edit in edits {
            // Data-loss confirm gate (`architecture.md §8`): an unconfirmed merge that would discard
            // covered content is not applied — the UI must confirm first. Emit `MergeNeedsConfirm`
            // and drop the command (no mutation, no undo step). A confirmed merge, or one with no
            // covered content, falls through and applies. Realized here (a pre-apply read like the
            // fill merge-guard) rather than in the emit-free `apply_one` — same behavior as §3.
            if let Command::MergeCells {
                sheet,
                area,
                confirmed: false,
            } = &edit
            {
                if self.merge_would_lose_data(*sheet, *area) {
                    self.emit(WorkerEvent::MergeNeedsConfirm {
                        sheet: *sheet,
                        area: *area,
                    });
                    continue;
                }
            }
            match self.pre_validate(&edit) {
                Ok(()) => valid.push(edit),
                Err(reason) => self.emit(WorkerEvent::EditRejected { reason }),
            }
        }
        if valid.is_empty() {
            return false;
        }

        // A degraded worker refuses edits (but still serves reads/save above/below).
        if self.degraded {
            for _ in &valid {
                self.emit(WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded,
                });
            }
            return false;
        }

        // Snapshot the sheet list so a change (add/rename/delete — including via undo/redo) is
        // detected by comparison after the batch, driving both `SheetsChanged` and the cache-map
        // reconcile without threading a flag out of every undo path.
        let sheets_before = self.sheet_metas();

        // A user row-resize (`SetRowHeights`) marks its rows **manual** (exempt from wrap auto-grow,
        // §3.3). Collected before the apply closure moves `valid`; applied only after a successful
        // apply so the marks land before this batch's cache rebuild re-projects wrap heights.
        let resized_rows: Vec<(SheetId, u32, u32)> = valid
            .iter()
            .filter_map(|e| match e {
                Command::SetRowHeights {
                    sheet,
                    row_start,
                    row_end,
                    ..
                } => Some((*sheet, *row_start, *row_end)),
                _ => None,
            })
            .collect();

        self.emit(WorkerEvent::EvalStarted);

        // The IronCalc apply+eval is the only panic-prone region (round-3 D belt-and-braces).
        let started = Instant::now();
        let outcome = {
            let doc = &mut self.doc;
            catch_unwind(AssertUnwindSafe(move || {
                doc.pause_evaluation();
                let mut applied = 0u64;
                let mut needs_eval = false;
                let mut engine_errors: Vec<String> = Vec::new();
                // One `AppliedOp` per successfully-applied edit, in order, for post-eval cache
                // bookkeeping (touch-set stacks + mirror refresh).
                let mut applied_ops: Vec<AppliedOp> = Vec::new();
                for edit in &valid {
                    match apply_one(doc, edit) {
                        Ok(AppliedKind::Cell) => {
                            applied += 1;
                            needs_eval = true;
                            applied_ops.push(op_of(edit));
                        }
                        // Style-only edits don't affect values → skip the recompute.
                        Ok(AppliedKind::StyleOnly) => {
                            applied += 1;
                            applied_ops.push(op_of(edit));
                        }
                        Ok(AppliedKind::SheetOp) => {
                            applied += 1;
                            needs_eval = true;
                            applied_ops.push(op_of(edit));
                        }
                        // A resize is geometry-only (no recompute); a structural insert/delete
                        // shifts formulas (recompute). Both fully rebuild the sheet cache.
                        Ok(AppliedKind::GeometryOnly) => {
                            applied += 1;
                            applied_ops.push(op_of(edit));
                        }
                        Ok(AppliedKind::Structure) => {
                            applied += 1;
                            needs_eval = true;
                            applied_ops.push(op_of(edit));
                        }
                        // A CF mutation records its op like a style edit, but MUST force the eval
                        // (BUG-1): under paused evaluation nothing refreshes IronCalc's `cf_cache`,
                        // so the coalesced `doc.evaluate()` has to run before the sheet-cache rebuild
                        // reads it — otherwise the rule only applies after a later value change.
                        Ok(AppliedKind::CondFmt) => {
                            applied += 1;
                            needs_eval = true;
                            applied_ops.push(op_of(edit));
                        }
                        // A no-op edit changed nothing: don't count it, recompute, or record an op.
                        Ok(AppliedKind::NoOp) => {}
                        Err(msg) => engine_errors.push(msg),
                    }
                }
                let apply_done = Instant::now();
                doc.resume_evaluation();
                if needs_eval {
                    doc.evaluate(); // the ONE coalesced recompute
                }
                let eval_done = Instant::now();
                tracing::debug!(
                    edits = applied,
                    needs_eval,
                    apply_us = apply_done.duration_since(started).as_micros() as u64,
                    eval_us = eval_done.duration_since(apply_done).as_micros() as u64,
                    "worker: applied coalesced batch"
                );
                (applied, needs_eval, engine_errors, applied_ops)
            }))
        };

        self.emit(WorkerEvent::EvalFinished);

        match outcome {
            Ok((applied, needs_eval, engine_errors, applied_ops)) => {
                for msg in engine_errors {
                    self.emit(WorkerEvent::EditRejected {
                        reason: EditRejectedReason::Engine(msg),
                    });
                }
                if applied == 0 {
                    return false;
                }
                // Mark user-resized rows manual BEFORE the cache rebuild below re-projects wrap
                // heights, so the resize wins over any prior auto-grow contribution.
                for (sheet, start, end) in &resized_rows {
                    self.mark_rows_manual(*sheet, *start, *end);
                }
                if needs_eval {
                    self.eval_count += 1;
                }
                self.ops_seen += applied;
                self.shared
                    .committed_ops
                    .store(self.ops_seen, Ordering::Release);

                // Record the batch's edited-cell set (this pops/pushes the undo-redo touch stacks)
                // so both the chart re-resolve and the style-cache mirror below read the same ranges.
                let (refresh, rebuild) = self.collect_edited_ranges(applied_ops);
                // The rebuilt-sheet set is the CF-relevant one: a CF mutation, a structural CF-range
                // shift, and the undo/redo of either land here (all map to `AppliedOp::Rebuild` /
                // `Touch::Rebuild`). Captured before `rebuild` is consumed by `apply_cache_refresh`.
                let mut cf_sheets = rebuild.clone();
                // Re-resolve any charts whose source ranges the edit touched, BEFORE publishing, so
                // the edit's single `Published` carries fresh cells AND fresh charts (P9,
                // charts/architecture §4.1). Only intersecting charts recompute.
                self.reresolve_charts(&refresh, &rebuild);

                self.publish();
                self.emit(WorkerEvent::Published);

                // Mirror the applied ops into the style/geometry cache (re-read touched cells) and
                // ship `StyleCacheUpdated` deltas. Ordered after `Published` (unchanged event order).
                self.apply_cache_refresh(refresh, rebuild, &sheets_before);

                // Value-dependent conditional formatting: a recompute can change cell values, which
                // flips CF results (a threshold crosses, a Top-N/average cell enters or leaves the
                // set, a color scale re-interpolates) with NO CF command. Rebuild the affected CF
                // sheets' style caches via the extended path — only on a recompute, and short-circuited
                // for a non-CF workbook by the empty-map gate inside. `cf_sheets` (this batch's full
                // rebuilds) is passed so a CF/structural op already rebuilt above isn't rebuilt twice.
                if needs_eval {
                    self.refresh_cf_caches_after_recompute(&cf_sheets);
                }

                // Republish the CF rule list for any rebuilt sheet whose rules actually changed, and
                // notify the window (`components/engine_cf.md §5`). Gated inside so a non-CF sheet is
                // free; deduped so a coalesced multi-CF batch reconciles each sheet once.
                cf_sheets.sort_unstable();
                cf_sheets.dedup();
                self.reconcile_published_cond_fmt(&cf_sheets);

                // A changed sheet list (add/rename/delete, or an undo/redo of one) re-syncs the
                // tab bar. Compared by value so undo/redo of a sheet op is caught too.
                let sheets_after = self.sheet_metas();
                if sheets_after != sheets_before {
                    // Reconcile the CF map with the changed sheet SET. Removed sheets (delete /
                    // undo-of-add) are dropped so the map never outlives its sheet. Sheets that
                    // REAPPEARED (undo-of-delete restores the worksheet + its CF rules) are
                    // reconciled so a returning CF sheet republishes its rules + emits
                    // `CondFmtUpdated` — that undo pushes `Touch::Sheets`, not `Touch::Rebuild`, so
                    // the returning sheet never entered `cf_sheets` above.
                    let ids_before: HashSet<SheetId> = sheets_before.iter().map(|m| m.id).collect();
                    let ids_after: HashSet<SheetId> = sheets_after.iter().map(|m| m.id).collect();
                    self.shared
                        .cond_fmt
                        .write()
                        .retain(|id, _| ids_after.contains(id));
                    let appeared: Vec<SheetId> =
                        ids_after.difference(&ids_before).copied().collect();
                    self.reconcile_published_cond_fmt(&appeared);
                    self.emit(WorkerEvent::SheetsChanged {
                        sheets: sheets_after,
                    });
                }
                true
            }
            Err(_) => {
                // The panic unwound out of the closure before `resume_evaluation` ran. Clear
                // the pause flag so the model isn't stuck — but GUARD it: a poisoned model
                // could panic on that call too, and recovery must never itself unwind out of
                // the loop and kill the thread.
                {
                    let doc = &mut self.doc;
                    let _ = catch_unwind(AssertUnwindSafe(|| doc.resume_evaluation()));
                }
                tracing::debug!("worker: caught panic in apply/eval; entering recovery");
                self.handle_caught_panic();
                false
            }
        }
    }

    /// Dispatch one range-clipboard op (`components/clipboard.md`, `architecture.md §6`). Each is
    /// standalone (never coalesced): copy/cut reply with the TSV; paste applies one undoable
    /// diff + replies with the pasted range or a rejection.
    fn apply_clipboard_op(&mut self, cmd: Command) {
        match cmd {
            Command::CopySelection { sheet, range, cut } => self.apply_copy(sheet, range, cut),
            Command::PasteInternal { sheet, target } => self.apply_paste_internal(sheet, target),
            Command::PasteValues { sheet, target } => self.apply_paste_values(sheet, target),
            Command::PasteTsv {
                sheet,
                anchor,
                text,
            } => self.apply_paste_tsv(sheet, anchor, &text),
            // Only the clipboard commands are bucketed here.
            _ => {}
        }
    }

    /// Dispatch one replace op (`functional_spec.md §4.4`). Each mutates the model standalone: its
    /// own guarded paused-eval + single `evaluate()` + publish + undo touch entry(ies), then a
    /// `ReplacedCount` reply the find bar shows.
    fn apply_replace_op(&mut self, cmd: Command) {
        match cmd {
            Command::ReplaceOne {
                sheet,
                cell,
                query,
                replacement,
                match_case,
                whole_cell,
            } => self.apply_replace_one(sheet, cell, &query, &replacement, match_case, whole_cell),
            Command::ReplaceAll {
                sheet,
                query,
                replacement,
                match_case,
                whole_cell,
            } => self.apply_replace_all(sheet, &query, &replacement, match_case, whole_cell),
            // Only the two replace commands are bucketed here.
            _ => {}
        }
    }

    /// Replace the current match in a single cell (`Command::ReplaceOne`). The worker recomputes the
    /// replacement from the cell's fresh raw content (race-free), commits it (one undo entry), and
    /// replies `ReplacedCount { n }` (`1` if it wrote, else `0`).
    fn apply_replace_one(
        &mut self,
        sheet: SheetId,
        cell: CellRef,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
    ) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            self.emit(WorkerEvent::ReplacedCount { n: 0 });
            return;
        }
        let Some(idx) = self.resolve(sheet) else {
            self.emit(WorkerEvent::ReplacedCount { n: 0 });
            return;
        };
        self.emit(WorkerEvent::EvalStarted);
        let outcome = {
            let doc = &mut self.doc;
            let (q, r) = (query.to_string(), replacement.to_string());
            catch_unwind(AssertUnwindSafe(move || {
                doc.pause_evaluation();
                let wrote = doc.replace_one(idx, cell, &q, &r, match_case, whole_cell);
                doc.resume_evaluation();
                if matches!(wrote, Ok(true)) {
                    doc.evaluate();
                }
                wrote
            }))
        };
        self.emit(WorkerEvent::EvalFinished);
        match outcome {
            Ok(Ok(true)) => {
                let touched = vec![(sheet, CellRange::new(cell, cell))];
                self.commit_replacements(&touched);
                self.emit(WorkerEvent::ReplacedCount { n: 1 });
            }
            Ok(Ok(false)) => self.emit(WorkerEvent::ReplacedCount { n: 0 }),
            Ok(Err(msg)) => {
                self.emit(WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(msg),
                });
                self.emit(WorkerEvent::ReplacedCount { n: 0 });
            }
            Err(_) => {
                {
                    let doc = &mut self.doc;
                    let _ = catch_unwind(AssertUnwindSafe(|| doc.resume_evaluation()));
                }
                tracing::debug!("worker: caught panic in ReplaceOne; entering recovery");
                self.handle_caught_panic();
                self.emit(WorkerEvent::ReplacedCount { n: 0 });
            }
        }
    }

    /// Replace **every** match in the used range (`Command::ReplaceAll`). One guarded paused-eval,
    /// one `evaluate()`, one publish, and — via the fork's batched `set_user_inputs` — **one** engine
    /// undo entry for the whole replace (`phase_plans/phase_9.md`). So it pushes a **single**
    /// `Touch::Ranges` covering every changed cell (the `commit_paste` pattern), keeping the
    /// undo/touch stacks 1:1 aligned so a single Undo reverts the entire replace. Replies
    /// `ReplacedCount { n }`.
    fn apply_replace_all(
        &mut self,
        sheet: SheetId,
        query: &str,
        replacement: &str,
        match_case: bool,
        whole_cell: bool,
    ) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            self.emit(WorkerEvent::ReplacedCount { n: 0 });
            return;
        }
        let Some(idx) = self.resolve(sheet) else {
            self.emit(WorkerEvent::ReplacedCount { n: 0 });
            return;
        };
        self.emit(WorkerEvent::EvalStarted);
        let outcome = {
            let doc = &mut self.doc;
            let (q, r) = (query.to_string(), replacement.to_string());
            catch_unwind(AssertUnwindSafe(move || {
                doc.pause_evaluation();
                let changed = doc.replace_all_matches(idx, &q, &r, match_case, whole_cell);
                doc.resume_evaluation();
                if matches!(&changed, Ok(cells) if !cells.is_empty()) {
                    doc.evaluate(); // the ONE coalesced recompute for the whole replace
                }
                changed
            }))
        };
        self.emit(WorkerEvent::EvalFinished);
        match outcome {
            Ok(Ok(changed)) => {
                let n = changed.len();
                if n > 0 {
                    // The whole replace is ONE batched engine undo entry (`set_user_inputs`), so it
                    // records a SINGLE undo touch covering every changed cell — one later Undo pops
                    // it and reverts the entire replace. A fresh edit clears the redo side.
                    let touched: Vec<(SheetId, CellRange)> = changed
                        .iter()
                        .map(|&c| (sheet, CellRange::new(c, c)))
                        .collect();
                    self.eval_count += 1;
                    self.ops_seen += 1;
                    self.shared
                        .committed_ops
                        .store(self.ops_seen, Ordering::Release);
                    self.reresolve_charts(&touched, &[]);
                    self.publish();
                    self.emit(WorkerEvent::Published);
                    self.undo_stack
                        .push(UndoEntry::Cell(Touch::Ranges(touched.clone())));
                    self.redo_stack.clear();
                    for s in self.refresh_cache_cells(&touched) {
                        self.emit(WorkerEvent::StyleCacheUpdated { sheet: s });
                    }
                    // A replace recomputes values → re-evaluate CF on the resident CF sheets
                    // (empty-map gated, so a non-CF workbook is unaffected).
                    self.refresh_cf_caches_after_recompute(&[]);
                }
                self.emit(WorkerEvent::ReplacedCount { n });
            }
            Ok(Err(msg)) => {
                self.emit(WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(msg),
                });
                self.emit(WorkerEvent::ReplacedCount { n: 0 });
            }
            Err(_) => {
                {
                    let doc = &mut self.doc;
                    let _ = catch_unwind(AssertUnwindSafe(|| doc.resume_evaluation()));
                }
                tracing::debug!("worker: caught panic in ReplaceAll; entering recovery");
                self.handle_caught_panic();
                self.emit(WorkerEvent::ReplacedCount { n: 0 });
            }
        }
    }

    /// Shared post-replace bookkeeping for a single-cell replace (`ReplaceOne`): count the op,
    /// re-resolve any charts the change touched, publish, push one undo touch entry, and refresh the
    /// touched cache cell. (`ReplaceAll` inlines the single-entry, multi-range variant.)
    fn commit_replacements(&mut self, touched: &[(SheetId, CellRange)]) {
        self.eval_count += 1;
        self.ops_seen += 1;
        self.shared
            .committed_ops
            .store(self.ops_seen, Ordering::Release);
        self.reresolve_charts(touched, &[]);
        self.publish();
        self.emit(WorkerEvent::Published);
        for (sheet, range) in touched {
            self.undo_stack.push(UndoEntry::Cell(Touch::Cells {
                sheet: *sheet,
                range: *range,
            }));
        }
        self.redo_stack.clear();
        for sheet in self.refresh_cache_cells(touched) {
            self.emit(WorkerEvent::StyleCacheUpdated { sheet });
        }
        // A replace recomputes values → re-evaluate CF on the resident CF sheets (empty-map gated).
        self.refresh_cf_caches_after_recompute(&[]);
    }

    /// Copy (or cut) `range` to the engine clipboard slot and reply with the system-clipboard
    /// TSV. Sets the engine's view selection first (the hidden-state dance) and stashes the
    /// serialized payload; nothing evaluates and no undo entry is created (a copy is a read).
    fn apply_copy(&mut self, sheet: SheetId, range: CellRange, cut: bool) {
        let idx = match self.resolve(sheet) {
            Some(i) => i,
            None => return, // the sheet vanished — nothing to copy
        };
        // Guarded: the copy reads formatted values + styles; a poisoned model must not kill the
        // thread. A failure just skips the reply (copy is never dialog-worthy).
        let outcome = {
            let doc = &mut self.doc;
            catch_unwind(AssertUnwindSafe(move || doc.copy_range(idx, range)))
        };
        match outcome {
            Ok(Ok(copied)) => {
                self.clipboard = Some(ClipboardSlot {
                    sheet,
                    range: copied.range,
                    data: copied.data,
                    cut,
                    values: copied.values,
                });
                self.emit(WorkerEvent::CopyReady { tsv: copied.tsv });
            }
            Ok(Err(msg)) => {
                tracing::debug!(%msg, "worker: copy_to_clipboard failed (ignored)");
            }
            Err(_) => {
                let doc = &mut self.doc;
                let _ = catch_unwind(AssertUnwindSafe(|| doc.resume_evaluation()));
                self.handle_caught_panic();
            }
        }
    }

    /// Paste the engine clipboard slot at `anchor` on `dest` (`paste_from_clipboard`): full
    /// fidelity, one undo entry, Excel ref-adjustment (copy) / move + source-clear (cut).
    ///
    /// Rejection order mirrors [`apply_paste_tsv`]: degraded → nothing-to-paste → overflow →
    /// unresolved sheet. The slot is `take`n so its JSON is passed to the engine by reference
    /// (no clone); a non-consuming early return / failed paste restores it, a successful copy
    /// keeps it (repeatable), and a successful cut drops it (single-use).
    fn apply_paste_internal(&mut self, dest: SheetId, target: CellRange) {
        let anchor = target.start;
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(slot) = self.clipboard.take() else {
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::NothingToPaste,
            });
            return;
        };
        // A degenerate slot (an inverted range from an out-of-range copy, or no rows) has nothing
        // to paste — reject at the worker rather than trusting the UI, and avoid the `as u32`
        // wrap a `r1 < r0` range would produce below (Mild #3). It is junk, so it is not restored.
        let (r0, c0, r1, c1) = slot.range;
        if r1 < r0 || c1 < c0 || slot.data.as_object().is_none_or(|o| o.is_empty()) {
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::NothingToPaste,
            });
            return;
        }
        // A single-cell / exact-divisor COPY into a larger selection fills the whole target (BUG 4);
        // values + styles fill exactly, formula refs get one uniform (not per-cell) shift (accepted
        // limitation U2 in `GAPS.md`). Cap the fill at the same size guard font edits use so a 1-cell
        // paste into a full-column selection can't materialise a million cells — reject it as
        // Overflow (nothing pasted). The fill target is itself a valid on-sheet selection, so no
        // sheet-edge overflow is possible.
        let fill = if slot.cut {
            None
        } else {
            crate::document::fill_target_dims(slot.range, target)
        };
        if let Some((tw, th)) = fill {
            if tw as u64 * th as u64 > MAX_REFRESH_CELLS {
                self.clipboard = Some(slot); // still valid — the user can retry on a smaller target
                self.emit(WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow,
                });
                return;
            }
        }
        // Overflow pre-check against the slot's effective (dimension-clamped) source dims. When
        // filling, the source is a divisor of the (valid) target, so its top-left tile fits too.
        let (width, height) = ((c1 - c0 + 1) as u32, (r1 - r0 + 1) as u32);
        if !paste_fits(anchor, width, height) {
            self.clipboard = Some(slot); // still valid — the user can retry at a smaller anchor
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::Overflow,
            });
            return;
        }
        let (Some(dest_idx), Some(source_idx)) = (self.resolve(dest), self.resolve(slot.sheet))
        else {
            // The destination or copied-from sheet was deleted — keep the copy (a sheet can
            // return via undo-of-delete); this paste just can't run now.
            self.clipboard = Some(slot);
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::NothingToPaste,
            });
            return;
        };

        let source_range = slot.range;
        let cut = slot.cut;
        // Borrow the slot's JSON into the guarded paste (no clone). The closure's borrow ends when
        // `run_guarded_paste` returns, freeing `slot` for the restore/drop decision below.
        let outcome = {
            let data = &slot.data;
            self.run_guarded_paste(move |doc| {
                doc.paste_clipboard(dest_idx, source_idx, source_range, data, cut, target)
            })
        };
        match outcome {
            PasteOutcome::Applied(pasted) => {
                // The pasted destination, plus (on cut) the cleared source, form ONE undo entry.
                let mut touched = vec![(dest, pasted)];
                if cut {
                    touched.push((slot.sheet, tuple_to_range(source_range)));
                }
                self.commit_paste(dest, pasted, touched);
                if !cut {
                    self.clipboard = Some(slot); // a copy is repeatable; a cut is consumed
                }
            }
            PasteOutcome::EngineError(msg) => {
                self.clipboard = Some(slot); // the paste didn't apply — keep the copy
                self.emit(WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(msg),
                });
            }
            // A caught panic degrades the model; the (now-suspect) slot is dropped.
            PasteOutcome::Panicked => self.handle_caught_panic(),
        }
    }

    /// Paste the internal clipboard slot's **computed values** into `target` — the values-only
    /// sibling of [`apply_paste_internal`](Self::apply_paste_internal) (⌘⇧V, `functional_spec.md
    /// §5`). Reuses that path's sizing/overflow rules (a single-cell / exact-divisor source tiles
    /// to fill the selection; a block pastes from the anchor; oversized → Overflow) and its guarded
    /// one-undo write, but writes each source cell's evaluated value as a **literal** (no formulas,
    /// no formatting — the target keeps its own). A values paste is always repeatable and never
    /// clears the source, so the slot is kept regardless of its `cut` flag.
    fn apply_paste_values(&mut self, dest: SheetId, target: CellRange) {
        let anchor = target.start;
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(slot) = self.clipboard.take() else {
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::NothingToPaste,
            });
            return;
        };
        // A degenerate slot (inverted range, or no captured values) has nothing to paste — reject
        // rather than trust the UI (matches `apply_paste_internal`). Junk, so not restored.
        let (r0, c0, r1, c1) = slot.range;
        if r1 < r0 || c1 < c0 || slot.values.is_empty() {
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::NothingToPaste,
            });
            return;
        }
        // A single-cell / exact-divisor source fills the whole (larger) selection; cap the fill at
        // the same size guard the internal paste uses so a 1-cell fill into a full-column selection
        // can't materialise a million cells. A values paste never "moves", so it always fills.
        let fill = crate::document::fill_target_dims(slot.range, target);
        if let Some((tw, th)) = fill {
            if tw as u64 * th as u64 > MAX_REFRESH_CELLS {
                self.clipboard = Some(slot); // still valid — the user can retry on a smaller target
                self.emit(WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow,
                });
                return;
            }
        }
        // Overflow pre-check against the slot's effective source dims (when filling, the source is a
        // divisor of the valid target, so its top-left tile fits too).
        let (width, height) = ((c1 - c0 + 1) as u32, (r1 - r0 + 1) as u32);
        if !paste_fits(anchor, width, height) {
            self.clipboard = Some(slot); // still valid — the user can retry at a smaller anchor
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::Overflow,
            });
            return;
        }
        let Some(dest_idx) = self.resolve(dest) else {
            self.clipboard = Some(slot); // the destination sheet vanished — keep the copy
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::NothingToPaste,
            });
            return;
        };
        let (paste_w, paste_h) = fill.unwrap_or((width, height));
        // Borrow the slot's values into the guarded paste (no clone); the borrow ends when
        // `run_guarded_paste` returns, freeing `slot` for the restore below.
        let outcome = {
            let values = &slot.values;
            self.run_guarded_paste(move |doc| {
                doc.paste_values(dest_idx, anchor, values, paste_w, paste_h)
            })
        };
        match outcome {
            PasteOutcome::Applied(pasted) => {
                self.commit_paste(dest, pasted, vec![(dest, pasted)]);
                self.clipboard = Some(slot); // a values paste is repeatable
            }
            PasteOutcome::EngineError(msg) => {
                self.clipboard = Some(slot); // the paste didn't apply — keep the copy
                self.emit(WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(msg),
                });
            }
            PasteOutcome::Panicked => self.handle_caught_panic(),
        }
    }

    /// Paste external tab-separated `text` at `anchor` on `dest` (`paste_csv_string`): each token
    /// as user input, one undo entry.
    fn apply_paste_tsv(&mut self, dest: SheetId, anchor: CellRef, text: &str) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let (width, height) = tsv_dims(text);
        if width == 0 || height == 0 {
            return; // empty clipboard text → nothing to paste (no-op)
        }
        if !paste_fits(anchor, width, height) {
            self.emit(WorkerEvent::PasteRejected {
                reason: PasteError::Overflow,
            });
            return;
        }
        let Some(dest_idx) = self.resolve(dest) else {
            return;
        };
        let text = text.to_string();
        match self.run_guarded_paste(move |doc| doc.paste_tsv(dest_idx, anchor, &text)) {
            PasteOutcome::Applied(pasted) => {
                self.commit_paste(dest, pasted, vec![(dest, pasted)]);
            }
            PasteOutcome::EngineError(msg) => self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Engine(msg),
            }),
            PasteOutcome::Panicked => self.handle_caught_panic(),
        }
    }

    /// Run a paste mutation under the same paused-recompute + `catch_unwind` guard the edit batch
    /// uses (round-3 D belt-and-braces: a pasted formula reaches the recursive parser). On
    /// success the engine has re-selected the pasted rectangle; read it back as the outcome.
    fn run_guarded_paste(
        &mut self,
        f: impl FnOnce(&mut WorkbookDocument) -> Result<(), String>,
    ) -> PasteOutcome {
        self.emit(WorkerEvent::EvalStarted);
        let outcome = {
            let doc = &mut self.doc;
            catch_unwind(AssertUnwindSafe(move || {
                doc.pause_evaluation();
                let result = f(doc);
                doc.resume_evaluation();
                if result.is_ok() {
                    doc.evaluate(); // the ONE coalesced recompute for this paste
                }
                result.map(|()| doc.selected_range_0based())
            }))
        };
        self.emit(WorkerEvent::EvalFinished);
        match outcome {
            Ok(Ok(range)) => PasteOutcome::Applied(range),
            Ok(Err(msg)) => PasteOutcome::EngineError(msg),
            Err(_) => {
                // Recover the pause flag (guarded — a poisoned model could panic on it too).
                {
                    let doc = &mut self.doc;
                    let _ = catch_unwind(AssertUnwindSafe(|| doc.resume_evaluation()));
                }
                tracing::debug!("worker: caught panic in paste; entering recovery");
                PasteOutcome::Panicked
            }
        }
    }

    /// Shared post-paste bookkeeping: count the eval + committed op, publish, push the single
    /// undo touch-entry (clearing redo), refresh the touched cache ranges, and reply `Pasted`.
    fn commit_paste(
        &mut self,
        dest: SheetId,
        pasted: CellRange,
        touched: Vec<(SheetId, CellRange)>,
    ) {
        self.eval_count += 1;
        self.ops_seen += 1;
        self.shared
            .committed_ops
            .store(self.ops_seen, Ordering::Release);
        // Re-resolve any charts the pasted values touched, before the publish (P9) — a paste into a
        // source range re-renders the chart on the same `Published`.
        self.reresolve_charts(&touched, &[]);
        self.publish();
        self.emit(WorkerEvent::Published);

        // One paste = one engine undo entry → one touch-entry (possibly multi-range), and a
        // fresh edit invalidates the redo stack.
        self.undo_stack
            .push(UndoEntry::Cell(Touch::Ranges(touched.clone())));
        self.redo_stack.clear();
        for sheet in self.refresh_cache_cells(&touched) {
            self.emit(WorkerEvent::StyleCacheUpdated { sheet });
        }
        // A paste recomputes values → re-evaluate CF on the resident CF sheets (empty-map gated).
        self.refresh_cf_caches_after_recompute(&[]);

        self.emit(WorkerEvent::Pasted {
            sheet: dest,
            range: pasted,
        });
    }

    /// Apply a `SetFont` (`architecture.md §3.3`, `components/style_render.md`): materialise the
    /// font family/size over the (clamped) selection via `on_paste_styles`, auto-grow rows too
    /// small for a larger size, then mirror the touched cells + heights into the cache. Style-only
    /// — no evaluation. Emits a **variable** number of engine diff-lists (one style paste + one per
    /// contiguous grown-row run); it pushes exactly that many touch-set entries so the undo stack
    /// stays 1:1 aligned (so undoing a font change is up to K+1 steps — accepted, DECISIONS).
    fn apply_set_font(
        &mut self,
        sheet: SheetId,
        range: CellRange,
        family: Option<String>,
        size_pt: Option<f64>,
    ) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(idx) = self.resolve(sheet) else {
            return; // the sheet vanished — nothing to do
        };
        // Full row/col/select-all clamps to the used range (no font bands); a bounded selection
        // applies as-is. An empty intersection (band beyond the used range) is a no-op.
        let clamped = match self.doc.clamp_to_used(idx, range) {
            Ok(Some(r)) => r,
            Ok(None) => return,
            Err(msg) => {
                tracing::debug!(%msg, "worker: SetFont clamp failed (ignored)");
                return;
            }
        };
        // Cap: on_paste_styles materialises one style per cell, so a pathological used-range is
        // refused with a dialog-worthy message rather than churning millions of cells.
        if range_area(&clamped) > MAX_REFRESH_CELLS {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Engine(
                    "Selection too large for font changes".to_string(),
                ),
            });
            return;
        }

        let default_name = self.doc.default_font().1;
        self.emit(WorkerEvent::EvalStarted);
        // Guarded (round-3 D belt-and-braces): the style paste + height writes run under
        // catch_unwind on the worker's big stack. Count the diff-lists actually committed (a failed
        // height run commits nothing — `set_rows_height` pushes atomically) so the touch-set stays
        // aligned even under a (near-impossible) partial failure.
        let outcome = {
            let doc = &mut self.doc;
            catch_unwind(AssertUnwindSafe(move || {
                doc.pause_evaluation();
                let set_res = doc.set_font(idx, clamped, family.as_deref(), size_pt, &default_name);
                let mut height_runs = 0u64;
                if set_res.is_ok() {
                    if let Some(pt) = size_pt {
                        // Auto-grow only on a size change (a family swap keeps the size, so the
                        // row already fits). Grow the row PROPORTIONALLY — keep the default
                        // row-height : font-size ratio at every size (`cache::autofit_row_ironcalc_px`)
                        // rather than "line box + fixed padding". The old fixed-padding formula grew
                        // large fonts *less* than their line box, so the overflowing line box
                        // inverted top/bottom vertical alignment; a proportional row always contains
                        // the line box with the same slack the default cell has. Returned in
                        // IronCalc px (get_row_height's storage space) to compare + write directly.
                        let font_px = pt * 96.0 / 72.0;
                        let needed = cache::autofit_row_ironcalc_px(font_px).ceil();
                        let mut grow_rows: Vec<u32> = Vec::new();
                        for row in clamped.rows() {
                            if let Ok(cur) = doc.row_height_px(idx, row) {
                                if needed > cur {
                                    grow_rows.push(row);
                                }
                            }
                        }
                        // Coalesce contiguous rows into runs; one set_rows_height per run.
                        let mut i = 0;
                        while i < grow_rows.len() {
                            let start = grow_rows[i];
                            let mut end = start;
                            while i + 1 < grow_rows.len() && grow_rows[i + 1] == end + 1 {
                                i += 1;
                                end = grow_rows[i];
                            }
                            if doc.set_row_heights_run(idx, start, end, needed).is_ok() {
                                height_runs += 1;
                            }
                            i += 1;
                        }
                    }
                }
                doc.resume_evaluation();
                (set_res, height_runs)
            }))
        };
        self.emit(WorkerEvent::EvalFinished);

        match outcome {
            Ok((Ok(()), height_runs)) => {
                let diff_lists = 1 + height_runs;
                self.ops_seen += diff_lists;
                self.shared
                    .committed_ops
                    .store(self.ops_seen, Ordering::Release);
                self.publish();
                self.emit(WorkerEvent::Published);
                // One touch per committed diff-list (all covering the clamped range — re-reading it
                // syncs both the styles and the row heights), and a fresh edit clears the redo side.
                for _ in 0..diff_lists {
                    self.undo_stack.push(UndoEntry::Cell(Touch::Cells {
                        sheet,
                        range: clamped,
                    }));
                }
                self.redo_stack.clear();
                for s in self.refresh_cache_cells(&[(sheet, clamped)]) {
                    self.emit(WorkerEvent::StyleCacheUpdated { sheet: s });
                }
            }
            // A clean engine error (near-unreachable for valid input): nothing committed → no touch.
            Ok((Err(msg), _)) => self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Engine(msg),
            }),
            Err(_) => {
                // Recover the pause flag (guarded — a poisoned model could panic on it too).
                {
                    let doc = &mut self.doc;
                    let _ = catch_unwind(AssertUnwindSafe(|| doc.resume_evaluation()));
                }
                tracing::debug!("worker: caught panic in SetFont; entering recovery");
                self.handle_caught_panic();
            }
        }
    }

    /// Record a batch's applied ops against the undo/redo touch-set stacks and return the cells the
    /// batch changed: `(refresh_ranges, rebuild_sheets)`. This is the state-mutating half of the
    /// post-eval bookkeeping — it pushes new touches (clearing redo) and pops on undo/redo, so it
    /// runs **exactly once** per batch. Both the chart re-resolve and the style-cache mirror
    /// ([`apply_cache_refresh`](Self::apply_cache_refresh)) consume the returned ranges.
    fn collect_edited_ranges(
        &mut self,
        applied_ops: Vec<AppliedOp>,
    ) -> (Vec<(SheetId, CellRange)>, Vec<SheetId>) {
        let mut refresh: Vec<(SheetId, CellRange)> = Vec::new();
        // Sheets whose whole cache must be rebuilt (a resize / insert / delete, or the undo/redo
        // of one — the region touched is unbounded so a per-cell mirror can't express it).
        let mut rebuild: Vec<SheetId> = Vec::new();
        for op in applied_ops {
            match op {
                AppliedOp::Cells { sheet, range } => {
                    self.undo_stack
                        .push(UndoEntry::Cell(Touch::Cells { sheet, range }));
                    self.redo_stack.clear(); // a fresh edit invalidates the redo stack
                    refresh.push((sheet, range));
                }
                AppliedOp::Sheets => {
                    self.undo_stack.push(UndoEntry::Cell(Touch::Sheets));
                    self.redo_stack.clear();
                }
                AppliedOp::Rebuild { sheet } => {
                    self.undo_stack
                        .push(UndoEntry::Cell(Touch::Rebuild { sheet }));
                    self.redo_stack.clear();
                    rebuild.push(sheet);
                }
                // Only a **cell** undo reaches the IronCalc edit path (a chart undo is applied
                // worker-side by `undo_chart_op`, never routed here), so the popped entry is a
                // `Cell` — pop it, re-read its touch-set, and mirror it onto the redo stack.
                AppliedOp::Undo => match self.undo_stack.pop() {
                    Some(UndoEntry::Cell(touch)) => {
                        refresh.extend(touch_refresh_ranges(&touch));
                        rebuild.extend(touch_rebuild_sheets(&touch));
                        self.redo_stack.push(UndoEntry::Cell(touch));
                    }
                    // Unreachable (routing guarantees a `Cell` top); restore + warn, never panic
                    // out of the worker loop.
                    Some(other) => {
                        tracing::error!("cell-undo path popped a chart entry; restoring");
                        self.undo_stack.push(other);
                    }
                    None => {}
                },
                AppliedOp::Redo => match self.redo_stack.pop() {
                    Some(UndoEntry::Cell(touch)) => {
                        refresh.extend(touch_refresh_ranges(&touch));
                        rebuild.extend(touch_rebuild_sheets(&touch));
                        self.undo_stack.push(UndoEntry::Cell(touch));
                    }
                    Some(other) => {
                        tracing::error!("cell-redo path popped a chart entry; restoring");
                        self.redo_stack.push(other);
                    }
                    None => {}
                },
            }
        }
        (refresh, rebuild)
    }

    /// Mirror a batch's edited cells into the resident cache (`components/style_cache.md
    /// §Lifecycle`): reconcile the caches map when the sheet set changed, re-read the touched cells,
    /// and emit `StyleCacheUpdated` per changed sheet. Consumes the `(refresh, rebuild)` from
    /// [`collect_edited_ranges`](Self::collect_edited_ranges). Runs after the eval + publish (styles
    /// don't depend on the recompute).
    fn apply_cache_refresh(
        &mut self,
        refresh: Vec<(SheetId, CellRange)>,
        mut rebuild: Vec<SheetId>,
        sheets_before: &[SheetMeta],
    ) {
        // When the sheet-id SET changed (delete, or undo-of-add), drop caches for absent sheets.
        // A returning sheet (undo-of-delete) rebuilds lazily on its next activation.
        let ids_before: HashSet<SheetId> = sheets_before.iter().map(|m| m.id).collect();
        let ids_after: HashSet<SheetId> = self.sheet_metas().iter().map(|m| m.id).collect();
        if ids_before != ids_after {
            self.shared
                .caches
                .write()
                .retain(|id| ids_after.contains(&id));
        }

        for sheet in self.refresh_cache_cells(&refresh) {
            self.emit(WorkerEvent::StyleCacheUpdated { sheet });
        }

        // Full rebuilds for resize / insert / delete (and their undo/redo). Only resident sheets
        // rebuild — an absent sheet rebuilds lazily on its next activation. Deduped so a batch that
        // resizes the same sheet twice rebuilds once.
        rebuild.sort_unstable();
        rebuild.dedup();
        for sheet in rebuild {
            if self.shared.caches.read().contains(sheet) && self.build_and_store_cache(sheet) {
                self.emit(WorkerEvent::StyleCacheUpdated { sheet });
            }
        }
    }

    /// Re-evaluate conditional formatting for the resident CF sheets after a recompute may have
    /// changed cell values (`architecture.md §4.4`, `components/engine_cf.md §6`). CF results are
    /// **value-dependent** — a `CellIs` threshold crosses, a Top-N / average cell enters or leaves the
    /// set, a color scale re-interpolates — all with NO CF command. Because a rule can be **global**
    /// (its result at one cell depends on the whole range), the touched-cell mirror is insufficient,
    /// so each affected CF sheet's whole style cache is rebuilt via the extended path
    /// ([`build_and_store_cache`](Self::build_and_store_cache) → `build_sheet_cache(cf = true)`) and a
    /// `StyleCacheUpdated` emitted. This is the one new coupling in FreeCell: value publish → style
    /// refresh.
    ///
    /// **Fast gate (the perf invariant):** the published CF map is empty ⟺ no sheet carries a rule
    /// (P2 maintains it), so a non-CF workbook returns here immediately — no resident scan, no
    /// `has_cond_fmt` reads, no rebuilds. `already_rebuilt` names sheets the caller has just fully
    /// rebuilt this batch (a CF-rule mutation or a structural op), so they are not rebuilt twice.
    fn refresh_cf_caches_after_recompute(&mut self, already_rebuilt: &[SheetId]) {
        if self.shared.cond_fmt.read().is_empty() {
            return; // no CF anywhere → nothing value-dependent to refresh (non-CF fast path)
        }
        // Snapshot the resident ids (bounded by the few activated sheets) so the read lock is
        // released before the per-sheet `&mut` rebuild below.
        let resident = self.shared.caches.read().resident_ids();
        for sheet in resident {
            if already_rebuilt.contains(&sheet) {
                continue;
            }
            let Some(idx) = self.resolve(sheet) else {
                continue; // sheet deleted out from under the snapshot
            };
            if !self.doc.has_cond_fmt(idx) {
                continue; // a non-CF sheet's cache is value-independent → leave it untouched
            }
            if self.build_and_store_cache(sheet) {
                self.emit(WorkerEvent::StyleCacheUpdated { sheet });
            }
        }
    }

    /// Re-read every cell in `refresh` and update its cache entry (the mirror primitive), for the
    /// sheets that are resident. Returns the distinct sheets whose cache changed.
    ///
    /// A **band-creating** range (spanning all columns of a row, or all rows of a column) makes
    /// IronCalc's `update_range_style` set a row/column **band** rather than per-cell styles
    /// (`ironcalc_base/src/user_model/common.rs`, the full-rows / full-columns branches). The
    /// per-cell [`cache::refresh_cell`] can't create a band, so such a range — and any
    /// pathologically large one — falls back to a full (populated-cell-bounded) rebuild that reads
    /// the bands back from the engine. Non-band ranges take the cheap per-cell path, plus a
    /// row-height mirror (a value edit can auto-fit a row taller).
    fn refresh_cache_cells(&mut self, refresh: &[(SheetId, CellRange)]) -> Vec<SheetId> {
        let caches = Arc::clone(&self.shared.caches);
        let mut touched: Vec<SheetId> = Vec::new();
        for (sheet, range) in refresh {
            if !caches.read().contains(*sheet) {
                continue; // not resident → will rebuild correctly on activation
            }
            let idx = match self.resolve(*sheet) {
                Some(i) => i,
                None => continue, // sheet deleted out from under the touch-set
            };
            if is_band_creating(range) || range_area(range) > MAX_REFRESH_CELLS {
                // A failed rebuild drops the (now stale) entry and returns false; don't announce
                // a StyleCacheUpdated for a sheet whose cache is no longer resident.
                if !self.build_and_store_cache(*sheet) {
                    continue;
                }
            } else {
                // Resolve the workbook default font once for the whole range (not per cell).
                let (def_sz, def_name) = self.doc.default_font();
                // CF gate once per range (not per cell): a CF sheet re-reads the effective (base +
                // CF overlay) style so a style edit to a rule-filled cell keeps its overlay; a
                // non-CF sheet takes the unchanged base-style fast path.
                let cf = self.doc.has_cond_fmt(idx);
                let mut guard = caches.write();
                if let Some(cache) = guard.get_mut(*sheet) {
                    for row in range.rows() {
                        for col in range.cols() {
                            let _ = cache::refresh_cell(
                                cache,
                                &self.doc,
                                idx,
                                CellRef::new(row, col),
                                def_sz,
                                &def_name,
                                cf,
                            );
                        }
                    }
                    // Mirror IronCalc's row-height auto-fit over the touched rows (one axis
                    // rebuild). Cheap: a non-band range spans a bounded number of rows. CRITICAL:
                    // fold in the persisted wrap-driven contribution here too — otherwise a cheap
                    // per-cell refresh (a value/style edit to ANY cell in the row, e.g. a short cell
                    // beside a wrapped notes cell) would reset the row to the IronCalc base and
                    // collapse a wrap-grown row, which the render thread would NOT re-measure (the
                    // wrapped cell's inputs didn't change). `manual` rows keep their IronCalc height;
                    // auto rows take `max(base, wrap)`, exactly as `project_wrap_heights` does on a
                    // full rebuild.
                    let manual = self.manual_rows.get(sheet);
                    let wrap = self.wrap_heights.get(sheet);
                    let default_px = freecell_core::cache::DEFAULT_ROW_HEIGHT_PX;
                    let heights: Vec<(u32, Option<f32>)> = range
                        .rows()
                        .map(|row| {
                            let base = cache::row_override_px(&self.doc, idx, row);
                            if manual.is_some_and(|m| m.contains(&row)) {
                                return (row, base);
                            }
                            let wrap_px = wrap.and_then(|w| w.get(&row)).copied().unwrap_or(0.0);
                            let final_px = base.unwrap_or(default_px).max(wrap_px);
                            let target = if (final_px - default_px).abs() < AUTO_GROW_EPS_PX {
                                None
                            } else {
                                Some(final_px)
                            };
                            (row, target)
                        })
                        .collect();
                    cache.set_row_heights(&heights);
                }
            }
            if !touched.contains(sheet) {
                touched.push(*sheet);
            }
        }
        touched
    }

    /// Build `sheet`'s style/geometry cache from the engine's current state and install it under
    /// the write lock (build-on-activation / full-rebuild path). Returns whether the cache is now
    /// resident.
    ///
    /// On **any** failure to produce a cache (build error, or the sheet no longer resolving) the
    /// sheet's entry is **dropped**, never left stale: a rebuild replaces a pre-edit cache, so
    /// leaving the old one in place would make the grid re-read a stale cache — the exact
    /// divergence this phase exists to prevent. Dropping it makes the grid fall back to unstyled
    /// (correct-but-plain) instead. (`build_sheet_cache`'s getters only error on an invalid sheet
    /// index, and callers resolve the index first, so the `Err` path is effectively unreachable
    /// today; the drop keeps the invariant robust regardless.)
    fn build_and_store_cache(&mut self, sheet: SheetId) -> bool {
        let idx = match self.resolve(sheet) {
            Some(i) => i,
            None => {
                self.shared.caches.write().remove(sheet);
                return false;
            }
        };
        // The CF gate (`components/engine_cf.md §6`): a CF sheet builds each populated cell's style
        // from the effective (base + CF overlay) style; a non-CF sheet takes the unchanged base-style
        // fast path. Computed once per build.
        let cf = self.doc.has_cond_fmt(idx);
        match cache::build_sheet_cache(&self.doc, idx, cf) {
            Ok(mut built) => {
                // Seed the manual set on the FIRST build for this sheet from the freshly built
                // cache's height overrides — a loaded `custom_height` row starts **manual** so
                // auto-grow never shrinks a file's intentional heights (§3.3). Only on the first
                // build: a later rebuild must NOT re-derive manual from `custom_height`, because
                // IronCalc's own font/newline auto-fit sets `custom_height` on **auto** rows too.
                self.manual_rows
                    .entry(sheet)
                    .or_insert_with(|| built.row_overrides().keys().copied().collect());
                // Re-project persisted wrap-driven heights (auto rows only) so a rebuild doesn't
                // wipe grown rows back to the IronCalc base.
                self.project_wrap_heights(sheet, &mut built);
                self.shared.caches.write().insert(sheet, built);
                true
            }
            Err(error) => {
                tracing::debug!(
                    sheet = sheet.0,
                    %error,
                    "worker: style-cache build failed; dropping the entry so the grid never reads a stale cache"
                );
                self.shared.caches.write().remove(sheet);
                false
            }
        }
    }

    /// Apply this sheet's persisted wrap-driven heights onto a freshly built cache, so grown rows
    /// survive the rebuild. Each **auto** row's height becomes `max(built base, wrap)` — never
    /// below a font/newline auto-fit already in the built cache. Manual rows are skipped (their
    /// height is the IronCalc value the build already carries).
    fn project_wrap_heights(&self, sheet: SheetId, built: &mut freecell_core::cache::SheetCache) {
        let Some(wh) = self.wrap_heights.get(&sheet) else {
            return;
        };
        let manual = self.manual_rows.get(&sheet);
        let updates: Vec<(u32, Option<f32>)> = wh
            .iter()
            .filter(|(row, _)| manual.is_none_or(|m| !m.contains(*row)))
            .map(|(&row, &wrap_px)| {
                let final_px = built.row_height(row).max(wrap_px);
                let target = if (final_px - freecell_core::cache::DEFAULT_ROW_HEIGHT_PX).abs()
                    < AUTO_GROW_EPS_PX
                {
                    None
                } else {
                    Some(final_px)
                };
                (row, target)
            })
            .collect();
        if !updates.is_empty() {
            built.set_row_heights(&updates);
        }
    }

    /// Mark an inclusive 0-based row run `[start, end]` **manual** (a user resize, §3.3), and drop
    /// any wrap-driven contribution for those rows so the manual height wins outright.
    fn mark_rows_manual(&mut self, sheet: SheetId, start: u32, end: u32) {
        let set = self.manual_rows.entry(sheet).or_default();
        for row in start..=end {
            set.insert(row);
        }
        if let Some(wh) = self.wrap_heights.get_mut(&sheet) {
            for row in start..=end {
                wh.remove(&row);
            }
        }
    }

    /// Apply a wrap-driven row auto-grow ([`Command::AutoGrowRowHeights`]) as a **cache-only**
    /// geometry update (`architecture.md §3`). For each `(row, wrap_px)`: manual rows are skipped;
    /// an auto row's clamped wrap contribution is stored (or dropped when `<= default`), and the
    /// resident cache's row height is set to `max(base IronCalc height, wrap)` — but only for rows
    /// whose height actually changes, so a settled measurement emits no `StyleCacheUpdated` (the
    /// convergence / no-oscillation guard). Touches neither IronCalc, `ops_seen`, nor the undo
    /// stacks, so it adds no user-visible undo step (§3.4).
    fn apply_auto_grow(&mut self, sheet: SheetId, heights: Vec<(u32, f32)>) {
        if self.degraded {
            return; // cosmetic geometry; a degraded worker silently skips it
        }
        let Some(idx) = self.resolve(sheet) else {
            return;
        };
        let default_px = freecell_core::cache::DEFAULT_ROW_HEIGHT_PX;
        let manual = self.manual_rows.get(&sheet).cloned().unwrap_or_default();
        // Precompute the IronCalc base height per row BEFORE borrowing `wrap_heights` mutably.
        let bases: Vec<(u32, f32, f32)> = heights
            .iter()
            .map(|(row, px)| {
                let base = cache::row_override_px(&self.doc, idx, *row).unwrap_or(default_px);
                (*row, base, *px)
            })
            .collect();
        let wh = self.wrap_heights.entry(sheet).or_default();
        let mut targets: Vec<(u32, Option<f32>)> = Vec::new();
        for (row, base, px) in bases {
            if manual.contains(&row) {
                wh.remove(&row); // manual wins — drop any stale wrap contribution
                continue;
            }
            let clamped = px.clamp(default_px, freecell_core::cache::MAX_AUTO_ROW_HEIGHT_PX);
            if clamped > default_px + AUTO_GROW_EPS_PX {
                wh.insert(row, clamped);
            } else {
                wh.remove(&row);
            }
            let wrap = wh.get(&row).copied().unwrap_or(0.0);
            let final_px = base.max(wrap);
            let target = if (final_px - default_px).abs() < AUTO_GROW_EPS_PX {
                None
            } else {
                Some(final_px)
            };
            targets.push((row, target));
        }

        let mut changed = false;
        {
            let caches = Arc::clone(&self.shared.caches);
            let mut guard = caches.write();
            if let Some(cache) = guard.get_mut(sheet) {
                // Only the rows whose committed height actually moves — a settled row is a no-op,
                // so a confirming re-measure produces no command and the loop converges.
                let real: Vec<(u32, Option<f32>)> = targets
                    .into_iter()
                    .filter(|(row, target)| {
                        let want = target.unwrap_or(default_px);
                        (cache.row_height(*row) - want).abs() > AUTO_GROW_EPS_PX
                    })
                    .collect();
                if !real.is_empty() {
                    cache.set_row_heights(&real);
                    changed = true;
                }
            }
        }
        if changed {
            self.emit(WorkerEvent::StyleCacheUpdated { sheet });
        }
    }

    /// Ensure the active sheet has a resident cache, building it if absent. Returns whether this
    /// call built one (so the caller emits `StyleCacheUpdated`) — `false` if it was already
    /// resident or the build failed (in which case the entry stays absent, not stale).
    fn ensure_active_cache_built(&mut self) -> bool {
        if self.shared.caches.read().contains(self.active_sheet) {
            return false;
        }
        self.build_and_store_cache(self.active_sheet)
    }

    /// The locked catch_unwind poisoning policy (`components/engine_worker.md §Main loop`):
    /// probe the model; if it still responds and this is the first panic, reject the edit and
    /// keep serving; on a second panic or an unresponsive probe, degrade and stop taking edits.
    fn handle_caught_panic(&mut self) {
        self.panic_count += 1;
        let responsive = self.probe_model();
        if self.panic_count >= 2 || !responsive {
            self.degraded = true;
            self.emit(WorkerEvent::WorkerDegraded {
                reason: "the calculation engine hit an unrecoverable error".to_string(),
            });
        } else {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::EnginePanic,
            });
        }
    }

    /// A cheap read to check the model is still responsive after a caught panic. Itself
    /// guarded, since a poisoned model could panic on read.
    fn probe_model(&self) -> bool {
        catch_unwind(AssertUnwindSafe(|| {
            self.doc.formatted_value(0, CellRef::new(0, 0)).is_ok()
        }))
        .unwrap_or(false)
    }

    /// Clean, non-panicking validation done before the apply guard.
    fn pre_validate(&self, edit: &Command) -> Result<(), EditRejectedReason> {
        match edit {
            Command::SetCellInput { input, .. } => {
                validate_input(input).map_err(EditRejectedReason::InputCap)
            }
            Command::RenameSheet { sheet, name } => {
                let props = self.doc.sheet_properties();
                let target = props.iter().position(|(id, _)| SheetId(*id) == *sheet);
                let existing: Vec<&str> = props
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| Some(*i) != target)
                    .map(|(_, (_, name))| name.as_str())
                    .collect();
                validate_sheet_name(name, &existing).map_err(EditRejectedReason::InvalidSheetName)
            }
            // Insert/delete rows/columns near a merge is NO LONGER guarded — the engine displaces
            // merges across structural edits (grow/shrink/drop, never split), so the op proceeds and
            // the cache re-reads the new region (merged-cell-ui `architecture.md §5`,
            // `functional_spec.md F6`).
            //
            // Fill into a merged region stays rejected (merged cells aren't a supported fill target,
            // matching the engine's covered-cell write rejection) → the fill-only `MergedCells`
            // dialog (`functional_spec.md F6`). The written rectangle is the selection `range`.
            Command::FillDown { sheet, range } | Command::FillRight { sheet, range } => {
                self.fill_merge_guard(*sheet, |merges| blocks_fill(merges, *range))
            }
            // A drag-fill writes over `target` (⊇ seed); a fill into a merged region is rejected.
            Command::FillDrag { sheet, target, .. } => {
                self.fill_merge_guard(*sheet, |merges| blocks_fill(merges, *target))
            }
            _ => Ok(()),
        }
    }

    /// Runs the fill merge-guard predicate against `sheet`'s live merged regions, returning
    /// `Err(MergedCells)` when `blocked` is true. Reads the engine's normalized `merged_regions`
    /// (live truth — file-loaded + in-app-created). A sheet that doesn't resolve (or whose merges
    /// can't be read) passes the guard — the apply path then surfaces its own error.
    fn fill_merge_guard(
        &self,
        sheet: SheetId,
        blocked: impl Fn(&[CellRange]) -> bool,
    ) -> Result<(), EditRejectedReason> {
        let Some(idx) = self.resolve(sheet) else {
            return Ok(());
        };
        match self.doc.merged_regions(idx) {
            Ok(merges) if blocked(&merges) => Err(EditRejectedReason::MergedCells),
            _ => Ok(()),
        }
    }

    /// Whether an unconfirmed `MergeCells` over `area` on `sheet` would discard covered content —
    /// the data-loss confirm gate (`architecture.md §8`). Fails **closed**: an unresolvable sheet
    /// or a doc read error returns `true` (assume a loss), so the gate errs toward asking the user
    /// to confirm rather than silently discarding content. (The sheet id comes from a live UI
    /// selection, so this backstop rarely triggers in practice.)
    fn merge_would_lose_data(&self, sheet: SheetId, area: CellRange) -> bool {
        let Some(idx) = self.resolve(sheet) else {
            return true;
        };
        self.doc.merge_would_lose_data(idx, area).unwrap_or(true)
    }

    /// Re-resolve the charts whose source ranges the edit touched, and store a fresh
    /// [`ChartSnapshot`] iff any changed (P9, charts/architecture §4.1, §5 challenge 2). The dirty
    /// set is the range→chart index intersected with the edit's `refresh` cells (+ any structurally
    /// `rebuilt` data sheet); only those charts read live values. Runs **before** the `Published`
    /// bump so the fresh charts ride the same event that repaints the cells. Cheap when nothing
    /// intersects — a disjoint edit does no reads and leaves the snapshot untouched.
    fn reresolve_charts(&mut self, refresh: &[(SheetId, CellRange)], rebuilt: &[SheetId]) {
        // A bound authored chart (P19) rides the SAME dirty-set/re-resolve path as a loaded one, so
        // an edit re-renders it too — even in a workbook that has *only* authored charts (no loaded
        // set). Bail only when there is nothing bound to re-resolve.
        let any_authored_bound = self.authored_charts.iter().any(|e| !e.refs.is_empty());
        if self.charts.is_empty() && !any_authored_bound {
            return;
        }
        // A `c:f` sheet name → stable id against the current model. Owned (`move`), so it never
        // borrows `self` while the chart sets are mutated below.
        let props = self.doc.sheet_properties();
        let resolve_sheet = move |name: &str| -> Option<SheetId> {
            props
                .iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| SheetId(*id))
        };
        let mut changed = false;

        // Loaded charts (P9): intersect the range→chart index, re-resolve only the dirty ones.
        if !self.charts.is_empty() {
            let indices = self.charts.dirty_indices(refresh, rebuilt, &resolve_sheet);
            if !indices.is_empty() {
                // Live cell reader over the doc — a disjoint field borrow from `self.charts` below.
                let doc = &self.doc;
                let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
                    match resolve_idx(doc, sheet) {
                        Ok(idx) => doc.cell_value(idx, cell),
                        Err(_) => CellData::Empty,
                    }
                };
                if self.charts.reresolve(&indices, &resolve_sheet, &read_cell) {
                    changed = true;
                }
            }
        }

        // Authored charts (P19): re-resolve any bound authored chart the edit touched, so a range set
        // in the panel behaves exactly like a loaded chart's live binding.
        if any_authored_bound {
            changed |= self.reresolve_authored(refresh, rebuilt, &resolve_sheet);
        }

        if changed {
            self.chart_version += 1;
            self.store_chart_snapshot();
        }
    }

    /// Capture the **stable** `SheetId → file worksheet part` map at open (P11 CR fix): the file's
    /// `workbook.xml.rels` name→part map joined with the model's **at-open** sheet names (which still
    /// match the file). No chart XML is parsed — this is the tiny, eager half of "lazy parse off the
    /// critical path"; the heavy chart XML still defers to first paint. A read failure yields an
    /// empty map (the workbook opens chart-less rather than failing) and is logged.
    ///
    /// **Join assumption:** the map is built by **exact name-equality** between the file's
    /// `workbook.xml` `<sheet name>` and the model's at-open `sheet_properties()` name — i.e. it
    /// assumes IronCalc loads sheet names byte-identical to the file's `<sheets>` (true at open;
    /// both derive from the same `workbook.xml`). A sheet whose name fails to join is filter-mapped
    /// out, so its charts degrade to **chart-less** (never discovered/saved) rather than
    /// mis-anchored — matching the "workbook open never breaks on charts" invariant.
    fn build_chart_sheet_part_map(&self, path: &Path) -> HashMap<SheetId, String> {
        let file_parts = match crate::chart::workbook_sheet_parts(path) {
            Ok(parts) => parts,
            Err(err) => {
                tracing::warn!("chart sheet-part map unreadable; opening chart-less: {err:#}");
                return HashMap::new();
            }
        };
        let props = self.doc.sheet_properties();
        file_parts
            .into_iter()
            .filter_map(|(name, part)| {
                props
                    .iter()
                    .find(|(_, n)| *n == name)
                    .map(|(id, _)| (SheetId(*id), part))
            })
            .collect()
    }

    /// Walk + bind `sheet`'s charts the first time it is painted (P11 lazy discovery,
    /// charts/architecture §5 challenge 5). Runs after the viewport publish, so the parse is off the
    /// first-paint critical path — the cells are already on screen; the charts ride the **next**
    /// `Published`, exactly as a live re-resolve does (P9). Keyed on the sheet's **stable file part**
    /// (via [`chart_sheet_parts`](Self::chart_sheet_parts)), NOT its live name, so a sheet renamed
    /// before it is painted still resolves to its charts (P11 CR fix). A no-op once the sheet has been
    /// walked, once every sheet has been discovered, or for a non-file / in-session-added sheet.
    fn ensure_sheet_charts_discovered(&mut self, sheet: SheetId) {
        if self.charts_fully_discovered {
            return;
        }
        let Some(path) = self.chart_source_path.clone() else {
            return; // never opened from a file → nothing to discover
        };
        if !self.discovered_chart_sheets.insert(sheet) {
            return; // already walked this sheet (walk each at most once)
        }
        let Some(part) = self.chart_sheet_parts.get(&sheet).cloned() else {
            return; // not a file worksheet (added in-session) → no file charts
        };
        match crate::chart::discover_and_parse_for_part(&path, &part) {
            Ok(specs) => {
                if self.bind_discovered(sheet, specs) {
                    self.chart_version += 1;
                    self.store_chart_snapshot();
                    self.emit(WorkerEvent::Published);
                }
            }
            Err(err) => tracing::warn!(%part, "lazy chart discovery failed: {err:#}"),
        }
    }

    /// Bind the charts `specs` discovered on `sheet`, skipping any deleted in-session (P18 — so a
    /// save-time full sweep can't resurrect a deleted loaded chart), and — when new charts were
    /// bound — stamp their stable [`ChartId`]s. Returns whether anything was added.
    fn bind_discovered(&mut self, sheet: SheetId, specs: crate::chart::load::SheetCharts) -> bool {
        let specs: crate::chart::load::SheetCharts = specs
            .into_iter()
            .filter(|(part, _)| !self.loaded_deletes.contains(part))
            .collect();
        if self.charts.add_missing(vec![(sheet, specs)]) {
            self.charts.assign_missing_ids(&mut self.next_chart_id);
            true
        } else {
            false
        }
    }

    /// Discover + bind **every** file worksheet's charts (P11), so a chart-preserving save never
    /// drops a chart whose sheet the user never painted. Runs once at the top of
    /// [`save_workbook`](Self::save_workbook); a no-op after the first full sweep. Iterates the
    /// **stable** `SheetId → file part` map, so each chart binds to its real `SheetId` regardless of
    /// any in-session rename — a renamed host's chart follows the rename, a deleted host's `SheetId`
    /// no longer resolves so `live_sheet_targets` drops it (the P10 delete outcome), and the
    /// active-sheet-fallback mis-anchoring bug is impossible. Merges through
    /// [`add_missing`](ChartBindings::add_missing), so charts already bound lazily (and their
    /// live-resolved values) are kept untouched. A discovery failure is logged (the save then
    /// proceeds with whatever was already bound, rather than aborting the user's save).
    fn ensure_all_charts_discovered(&mut self) {
        if self.charts_fully_discovered {
            return;
        }
        if self.chart_source_path.is_none() {
            self.charts_fully_discovered = true;
            return; // never opened from a file → nothing to discover
        }
        let path = self.chart_source_path.clone().expect("checked Some above");
        // Snapshot the stable map so we don't borrow `self` while binding into `self.charts`.
        let sheet_parts: Vec<(SheetId, String)> = self
            .chart_sheet_parts
            .iter()
            .map(|(id, part)| (*id, part.clone()))
            .collect();
        let mut added = false;
        for (sheet, part) in sheet_parts {
            self.discovered_chart_sheets.insert(sheet);
            match crate::chart::discover_and_parse_for_part(&path, &part) {
                Ok(specs) if !specs.is_empty() => {
                    if self.bind_discovered(sheet, specs) {
                        added = true;
                    }
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(%part, "chart discovery for save failed: {err:#}"),
            }
        }
        self.charts_fully_discovered = true;
        if added {
            self.chart_version += 1;
            self.store_chart_snapshot();
            self.emit(WorkerEvent::Published);
        }
    }

    /// The current name of the worksheet with stable id `sheet` (against the live model), or `None`
    /// if that sheet no longer exists (deleted in-session). The rename-safe key the chart-preserving
    /// save resolves each chart's host worksheet through.
    fn sheet_name_of(&self, sheet: SheetId) -> Option<String> {
        self.doc
            .sheet_properties()
            .into_iter()
            .find(|(id, _)| SheetId(*id) == sheet)
            .map(|(_, name)| name)
    }

    /// Save the workbook to `path` (`Command::Save` / Save-As), **preserving embedded charts**
    /// (P10, charts/architecture §4.1/§5). When the workbook was opened from a file *and* carries
    /// live charts, it re-injects that file's chart machinery into the current model body and
    /// writes the result atomically — an unedited chart byte-for-byte, an edited chart with its
    /// caches reflowed to current values. Otherwise (a new workbook, or one with no charts) it
    /// takes the plain atomic writer, so the **non-chart save path is behaviorally identical to
    /// before**. On success the just-saved file becomes the chart source for the next save (it is a
    /// self-contained superset). A missing target part surfaces as a [`SaveError`] (fail loudly).
    fn save_workbook(&mut self, path: &Path) -> Result<(), SaveError> {
        // Charts are discovered lazily per painted sheet (P11), so before a save force a full sweep
        // — otherwise a chart on a sheet the user never scrolled to would be silently dropped by the
        // chart-less writer.
        self.ensure_all_charts_discovered();

        // Mode 1/2 (loaded re-inject) applies only to a workbook opened from a file that still
        // carries loaded charts; mode 3 (authored write-from-model) applies to any inserted chart.
        let reinject_source = self
            .chart_source_path
            .clone()
            .filter(|_| !self.charts.is_empty());
        let has_authored = !self.authored_charts.is_empty();

        // No charts at all → the plain (chart-less) writer, byte-identical to the pre-chart path.
        if reinject_source.is_none() && !has_authored {
            return self.doc.save(path);
        }

        let mut bytes = self.doc.to_xlsx_bytes()?;

        // Mode 1/2: re-inject the LOADED charts (byte-preserve unedited, patch edited-loaded) from
        // the original file into the current model body. Each chart's host worksheet resolves
        // through its stable anchor `SheetId` → CURRENT name (rename-safe; a deleted host drops).
        if let Some(original) = &reinject_source {
            let live = self.charts.live_charts(|id| self.sheet_name_of(id));
            // P18: moved/resized loaded charts patch their retained `twoCellAnchor`; deleted loaded
            // charts drop from the package. Both are keyed by `chart_part`, relative to `original`.
            let (reinjected, _report) = crate::chart::reinject_live_charts(
                original,
                &bytes,
                &live,
                &self.loaded_anchor_edits,
                &self.loaded_deletes,
            )
            .map_err(|e| SaveError::Serialize(format!("charts couldn't be saved: {e:#}")))?;
            bytes = reinjected;
        }

        // Mode 3: synthesize the AUTHORED charts on top (write-from-model). This runs AFTER the
        // loaded re-inject, so an authored chart on a sheet that already carries a loaded chart's
        // drawing hits `write_authored_charts`' fail-loud precondition (merging into an existing
        // drawing is not yet supported) — surfaced here as a `SaveError`, never a silent drop or a
        // double `<drawing>` (charts/architecture §6). The two save reports (`SaveReport` vs
        // `AuthoredWriteReport`) stay distinct — a written-from-scratch chart is never conflated with
        // a byte-preserved one.
        if has_authored {
            let authored = self.authored_write_list(&bytes);
            if !authored.is_empty() {
                let (written, _report) = crate::chart::write_authored_charts(&bytes, &authored)
                    .map_err(|e| {
                        SaveError::Serialize(format!("authored charts couldn't be saved: {e:#}"))
                    })?;
                bytes = written;
            }
        }

        crate::document::write_xlsx_bytes_atomic(path, &bytes)?;

        // Advance the re-inject source to the just-saved file ONLY when there are no authored charts
        // — the saved file is then a self-contained superset of the LOADED charts, valid to
        // re-inject from on the next save (surviving a Save-As away from a since-deleted original,
        // P10). We must NOT point it at a file that also holds authored drawings: `reinject` carries
        // every `xl/charts/*` + `xl/drawings/*` by prefix, so a resave would carry the authored parts
        // AND re-synthesize them → duplicates. With authored charts present the source stays put, so
        // each save re-synthesizes them fresh from `authored_charts`.
        if !has_authored {
            self.chart_source_path = Some(path.to_path_buf());
            // The just-saved file now bakes in the loaded moves/deletes (they became part of the new
            // source), so the accumulated diffs vs. the old source are spent — clear them (P18). With
            // authored charts present the source stays put, so the diffs must persist to re-apply.
            self.loaded_anchor_edits.clear();
            self.loaded_deletes.clear();
        }
        Ok(())
    }

    /// The authored charts as [`AuthoredChart`]s for the write-from-model save, resolving each one's
    /// host worksheet name (dropping a chart whose host sheet was deleted in-session, like a loaded
    /// chart) and assigning it a **free** `xl/charts/chartN.xml` part — one that collides with
    /// neither an existing part in `package_bytes` (loaded charts already re-injected) nor another
    /// authored chart. A **ranged** chart (P19) carries its per-series `c:f`
    /// [`refs`](AuthoredEntry::refs), so the serializer emits `numRef`/`strRef` + caches (fully
    /// cell-bound, live-binds like a loaded chart on reopen); a still near-empty placeholder carries
    /// empty `refs`, so the serializer emits schema-valid literals.
    fn authored_write_list(&self, package_bytes: &[u8]) -> Vec<AuthoredChart> {
        let mut used = existing_chart_parts(package_bytes);
        let mut out = Vec::new();
        for entry in &self.authored_charts {
            let Some(sheet_name) = self.sheet_name_of(entry.anchor_sheet) else {
                tracing::warn!("dropping an authored chart whose host worksheet was deleted");
                continue;
            };
            let Some(chart) = entry.spec.chart().cloned() else {
                continue; // an authored chart always has a typed Chart; defensive
            };
            out.push(AuthoredChart {
                sheet_name,
                chart_part: next_chart_part(&mut used),
                chart,
                anchor: entry.spec.anchor,
                refs: entry.refs.clone(),
            });
        }
        out
    }

    /// Insert an **authored** chart of `kind` onto `sheet` at `anchor` (P17, charts/ui_design §3.1).
    /// A degraded worker rejects it (like every mutating op). Otherwise it builds the template chart
    /// and holds it as an Authored [`ChartSpec`], marks the document dirty, and republishes the chart
    /// snapshot so the window's `sync_charts` installs it into the grid. When `data` is `Some` (the
    /// action bar captured a real selection — Batch 3 item 8) the chart is **bound at creation** via
    /// [`bind_authored_range_at`](Self::bind_authored_range_at), so it is born LIVE; when `None` it
    /// stays snapshot-but-not-live (no `c:f` binding, so it never enters the dirty-set re-resolve
    /// until a range is set in P19).
    fn insert_authored_chart(
        &mut self,
        sheet: SheetId,
        kind: ChartInsertKind,
        anchor: Anchor,
        data: Option<CellRange>,
    ) {
        // A degraded worker refuses edits (consistent with the edit batch / paste / SetFont).
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        // The host sheet must exist (the UI only ever sends the active sheet; this is a backstop).
        if self.resolve(sheet).is_none() {
            tracing::warn!(sheet = sheet.0, "InsertChart onto a missing sheet ignored");
            return;
        }
        let id = ChartId(self.next_chart_id);
        self.next_chart_id += 1;
        self.authored_charts.push(AuthoredEntry {
            anchor_sheet: sheet,
            id,
            spec: ChartSpec::authored(kind.near_empty_chart(), anchor),
            // A freshly inserted chart carries placeholder literals — no `c:f` binding until a range
            // is set (P19). Empty `refs` keeps it snapshot-but-not-live, saved as literals.
            refs: Vec::new(),
        });
        // Post-v1 Batch 3, item 8: if the action bar captured a real selection at insert time, bind
        // it right now so the chart is born LIVE (real `c:f` refs + resolved values) — same block→
        // series binding as `SetChartRange`, on the id we just assigned. The data lives on the insert
        // `sheet` (the selection's own — anchor — sheet). A `None` selection stays near-empty.
        let pos = self.authored_charts.len() - 1;
        if let Some(data) = data {
            if let Some(data_sheet_name) = self.sheet_name_of(sheet) {
                self.bind_authored_range_at(pos, sheet, &data_sheet_name, data);
            }
        }
        // **Undo timeline (charts feedback item 4, reversing the earlier P18 "charts off Ctrl+Z"
        // decision):** an insert now pushes onto the unified undo stack. We stash the FINAL entry
        // (after any born-live range bind above) so a redo re-inserts it whole — refs/series intact,
        // no re-derivation. A chart entry never touches IronCalc's own undo stack, so the `Cell`
        // entries stay 1:1 with it (no desync). `ops_seen` still counts the op for the dirty flag.
        let entry = Box::new(self.authored_charts[pos].clone());
        self.push_chart_undo(ChartUndo::InsertAuthored { index: pos, entry });
        // Publish the new chart on the same seam the loaded charts ride, so the window installs it.
        self.commit_chart_op();
    }

    /// Move/resize a chart (P18, `Command::SetChartAnchor`): set the chart named by `id` to `anchor`.
    /// Degraded-guarded like every mutating op. An **authored** chart's model anchor is rewritten
    /// (the write-from-model save re-synthesizes its drawing there); a **loaded** chart's render
    /// anchor is updated AND recorded in `loaded_anchor_edits` so the source-first save patches its
    /// retained `twoCellAnchor`. Republishes the chart snapshot so the grid repaints at the new rect.
    fn set_chart_anchor(&mut self, _sheet: SheetId, id: ChartId, anchor: Anchor) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        // Authored first (its ids never collide with loaded ones — one shared counter). Each branch
        // stashes the prior placement onto the undo timeline (charts feedback item 4) before the move.
        if let Some(entry) = self.authored_charts.iter_mut().find(|e| e.id == id) {
            let prior = entry.spec.anchor;
            entry.spec.anchor = anchor;
            self.push_chart_undo(ChartUndo::SetAnchorAuthored {
                id,
                prior,
                applied: anchor,
            });
        } else if let Some(prior_render) = self.charts.anchor_by_id(id) {
            let chart_part = self
                .charts
                .set_anchor_by_id(id, anchor)
                .expect("id resolved by anchor_by_id");
            // `insert` returns the value it replaced — the prior `loaded_anchor_edits` state to
            // restore on undo (a chart moved twice in-session has a prior edit; a first move has None).
            let prior_edit = self.loaded_anchor_edits.insert(chart_part.clone(), anchor);
            self.push_chart_undo(ChartUndo::SetAnchorLoaded {
                id,
                chart_part,
                prior_render,
                prior_edit,
                applied: anchor,
            });
        } else {
            tracing::warn!(id = id.0, "SetChartAnchor for an unknown chart id ignored");
            return;
        }
        self.commit_chart_op();
    }

    /// Delete a chart (P18, `Command::DeleteChart`): drop the chart named by `id`. Degraded-guarded.
    /// An **authored** chart is removed from the authored set; a **loaded** chart is unbound and its
    /// `chart_part` recorded in `loaded_deletes` so the source-first save drops it from the package
    /// (its `twoCellAnchor` + part chain) — and the save-time discovery sweep skips it so it can't be
    /// re-bound. Republishes so the grid drops it. Both provenances stash enough state to **undo** the
    /// delete (charts feedback item 4): the removed authored entry, or the whole removed loaded
    /// binding + the save-set bookkeeping it changed.
    fn delete_chart(&mut self, _sheet: SheetId, id: ChartId) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        if let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) {
            let entry = Box::new(self.authored_charts.remove(pos));
            self.push_chart_undo(ChartUndo::DeleteAuthored { index: pos, entry });
        } else if let Some(removed) = self.charts.take_by_id(id) {
            let chart_part = removed.chart_part().to_string();
            // `remove` returns the anchor-edit the delete evicts — restored on undo.
            let prior_anchor_edit = self.loaded_anchor_edits.remove(&chart_part);
            self.loaded_deletes.insert(chart_part.clone());
            self.push_chart_undo(ChartUndo::DeleteLoaded {
                removed: Box::new(removed),
                chart_part,
                prior_anchor_edit,
            });
        } else {
            tracing::warn!(id = id.0, "DeleteChart for an unknown chart id ignored");
            return;
        }
        self.commit_chart_op();
    }

    /// Set an **authored** chart's data range (P19, `Command::SetChartRange`): give the chart named by
    /// `id` real `c:f` refs derived from the `data` block on `sheet` (the sheet the data lives on —
    /// not necessarily the chart's host sheet; the chart is found by `id`), rebuild its series in the
    /// kind's data shape, and re-resolve their values from the current cells — so it transitions from
    /// P17's snapshot-but-not-live placeholder to a **LIVE** chart (re-renders on edit,
    /// `reresolve_authored`; saves with `c:f` + caches, `authored_write_list`). Degraded-guarded; a
    /// loaded/unknown id is ignored (loaded re-range is P20's source-patch territory).
    fn set_chart_range(&mut self, sheet: SheetId, id: ChartId, data: CellRange) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(data_sheet_name) = self.sheet_name_of(sheet) else {
            tracing::warn!(
                sheet = sheet.0,
                "SetChartRange onto a missing sheet ignored"
            );
            return;
        };
        let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) else {
            tracing::warn!(
                id = id.0,
                "SetChartRange for a non-authored/unknown chart ignored (loaded re-range is P20)"
            );
            return;
        };
        // Stash the whole pre-bind entry (refs/series/source_ranges) and the post-bind entry, so the
        // undo timeline can restore either clone without re-deriving (charts feedback item 4).
        let prior = Box::new(self.authored_charts[pos].clone());
        self.bind_authored_range_at(pos, sheet, &data_sheet_name, data);
        let applied = Box::new(self.authored_charts[pos].clone());
        self.push_chart_undo(ChartUndo::SetRangeAuthored {
            index: pos,
            prior,
            applied,
        });
        self.commit_chart_op();
    }

    /// Bind the authored chart at index `pos` to the `data` block on `data_sheet` (named
    /// `data_sheet_name`): derive its `c:f` refs, rebuild its series shells in the current kind's data
    /// shape, and re-resolve their values from the current cells — turning a near-empty placeholder
    /// into a **LIVE** chart. The shared body of `SetChartRange` (P19) and the range-at-insert path
    /// (Batch 3 item 8). Does **not** publish — the caller commits/publishes once.
    fn bind_authored_range_at(
        &mut self,
        pos: usize,
        data_sheet: SheetId,
        data_sheet_name: &str,
        data: CellRange,
    ) {
        let Some(mut template) = self.authored_charts[pos].spec.chart().cloned() else {
            return; // an authored chart always has a typed Chart; defensive
        };
        let refs = crate::chart::series_refs_from_block(data_sheet_name, data);
        let binding = binding_from_refs(&refs);
        // The range keeps the current type, so the series data shape is unchanged — derive the shape
        // from the current kind (xy for scatter, xy+size for bubble, else category/value) the same
        // way `set_chart_type` does (`ChartInsertKind::series_shape`), not with an ad-hoc `matches!`.
        let shape = ChartInsertKind::from_chart_kind(&template.kind)
            .map(|k| k.series_shape())
            .unwrap_or(freecell_chart_model::SeriesShape::CategoryValue);
        template.series = build_series_shells(refs.len(), shape);
        let resolved = self.resolve_authored_chart(data_sheet, &template, &binding);
        let source_ranges = source_ranges_from_refs(&refs);

        let entry = &mut self.authored_charts[pos];
        if let Some(slot) = entry.spec.chart_mut() {
            *slot = resolved;
        }
        entry.spec.source_ranges = source_ranges;
        entry.refs = refs;
    }

    /// Switch an **authored** chart's type (P19, `Command::SetChartType`): rebuild the chart named by
    /// `id` to `kind`, preserving its title and — if it is already **bound** to a data range — its
    /// `c:f` refs (rebuilding the series in the new kind's data shape and re-resolving live). An
    /// unbound (still near-empty) chart is swapped to that kind's placeholder template, keeping the
    /// title. Degraded-guarded; a loaded/unknown id is ignored.
    fn set_chart_type(&mut self, sheet: SheetId, id: ChartId, kind: ChartInsertKind) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) else {
            tracing::warn!(
                id = id.0,
                "SetChartType for a non-authored/unknown chart ignored"
            );
            return;
        };
        let title = self.authored_charts[pos]
            .spec
            .chart()
            .and_then(|c| c.title.clone());
        let refs = self.authored_charts[pos].refs.clone();

        if refs.is_empty() {
            // Unbound placeholder: swap to the new kind's near-empty template, keeping the title so a
            // pre-range retype doesn't reset the (only) field the user has set.
            let mut chart = kind.near_empty_chart();
            chart.title = title;
            if let Some(slot) = self.authored_charts[pos].spec.chart_mut() {
                *slot = chart;
            }
        } else {
            // Bound: keep the range refs + title/axes/legend, rebuild the series in the new kind's
            // data shape, and re-resolve their values from the current cells.
            let mut template = self.authored_charts[pos]
                .spec
                .chart()
                .cloned()
                .expect("authored chart has a typed Chart");
            template.kind = kind.chart_kind();
            template.series = build_series_shells(refs.len(), kind.series_shape());
            let binding = binding_from_refs(&refs);
            let resolved = self.resolve_authored_chart(sheet, &template, &binding);
            if let Some(slot) = self.authored_charts[pos].spec.chart_mut() {
                *slot = resolved;
            }
        }
        // Type change stays IMMEDIATE (not itself undoable — charts feedback item 4 covers only
        // insert/delete/anchor/range), but it is a new forward action, so it invalidates the redo
        // stack (a pending redo must not resurrect pre-retype state).
        self.redo_stack.clear();
        self.commit_chart_op();
    }

    /// Edit a chart's **chrome** (P20, `Command::SetChartChrome`): apply one chrome attribute change
    /// — title / legend / axis title / series color / data-label toggles — to the chart named by `id`,
    /// on **either** provenance. An **authored** chart's model is mutated (re-serialized on save); a
    /// **loaded** chart's retained render model is mutated (so it re-renders live) and its retained
    /// `chartN.xml` is source-patched on save (only the changed sub-element, preserving unmodeled
    /// styling — the edit contract). Degraded-guarded; an unknown/Unsupported id is ignored.
    fn set_chart_chrome(&mut self, _sheet: SheetId, id: ChartId, edit: ChartChromeEdit) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        // Authored first (its ids never collide with loaded ones — one shared counter). Chrome edits
        // stay IMMEDIATE (not themselves undoable — charts feedback item 4 covers only insert/delete/
        // anchor/range), but each is a new forward action, so it invalidates the redo stack.
        if let Some(pos) = self.authored_charts.iter().position(|e| e.id == id) {
            if let Some(chart) = self.authored_charts[pos].spec.chart_mut() {
                apply_chrome_edit(chart, &edit);
                self.redo_stack.clear();
                self.commit_chart_op();
            }
            return;
        }
        // Loaded: mutate the bound chart's render model in place (its binding is untouched, so it
        // still live-re-resolves; the save patches its retained source).
        if self
            .charts
            .edit_chart_by_id(id, |chart| apply_chrome_edit(chart, &edit))
        {
            self.redo_stack.clear();
            self.commit_chart_op();
            return;
        }
        tracing::warn!(
            id = id.0,
            "SetChartChrome for an unknown/unsupported chart id ignored"
        );
    }

    /// Resolve an authored chart's series values from the **current** cells, given a `template` whose
    /// series shells are already in the right data shape and its `binding` (P19). A `&self` mirror of
    /// the [`reresolve_charts`](Self::reresolve_charts) closure setup, used by the range/type handlers
    /// to fill a freshly-rebuilt chart before it is published.
    fn resolve_authored_chart(
        &self,
        anchor_sheet: SheetId,
        template: &Chart,
        binding: &crate::chart::ChartBinding,
    ) -> Chart {
        let props = self.doc.sheet_properties();
        let resolve_sheet = move |name: &str| -> Option<SheetId> {
            props
                .iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| SheetId(*id))
        };
        let doc = &self.doc;
        let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
            match resolve_idx(doc, sheet) {
                Ok(idx) => doc.cell_value(idx, cell),
                Err(_) => CellData::Empty,
            }
        };
        resolve_chart(template, binding, anchor_sheet, &resolve_sheet, &read_cell)
    }

    /// Re-resolve every **bound** authored chart the edit touched (P19), in place — returns whether
    /// any authored chart's picture changed. Mirrors [`ChartBindings::reresolve`] for the authored
    /// set: an authored chart's `ChartBinding` is derived from its `refs` on demand (their single
    /// source of truth), so a dirty chart refreshes from the current cells through the shared
    /// [`resolve_chart`].
    fn reresolve_authored(
        &mut self,
        refresh: &[(SheetId, CellRange)],
        rebuilt: &[SheetId],
        resolve_sheet: &SheetResolver<'_>,
    ) -> bool {
        let doc = &self.doc;
        let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
            match resolve_idx(doc, sheet) {
                Ok(idx) => doc.cell_value(idx, cell),
                Err(_) => CellData::Empty,
            }
        };
        let mut changed = false;
        for entry in &mut self.authored_charts {
            if entry.refs.is_empty() {
                continue; // a still near-empty placeholder has no binding to re-resolve
            }
            let binding = binding_from_refs(&entry.refs);
            let anchor_sheet = entry.anchor_sheet;
            if !binding_is_dirty(&binding, anchor_sheet, refresh, rebuilt, resolve_sheet) {
                continue;
            }
            let Some(template) = entry.spec.chart() else {
                continue;
            };
            let resolved =
                resolve_chart(template, &binding, anchor_sheet, resolve_sheet, &read_cell);
            if entry.spec.chart() != Some(&resolved) {
                if let Some(slot) = entry.spec.chart_mut() {
                    *slot = resolved;
                }
                changed = true;
            }
        }
        changed
    }

    /// Shared post-mutation bookkeeping for a chart op: count the committed op (dirty + savable),
    /// bump the chart version, re-store the snapshot, and publish. Deliberately does **not** touch
    /// the undo/redo stacks — the caller (a forward op via [`push_chart_undo`](Self::push_chart_undo),
    /// or an undo/redo via [`undo_chart_op`](Self::undo_chart_op)) owns the timeline — so it is reused
    /// verbatim by both the forward chart ops and the chart undo/redo republish.
    fn commit_chart_op(&mut self) {
        self.ops_seen += 1;
        self.shared
            .committed_ops
            .store(self.ops_seen, Ordering::Release);
        self.chart_version += 1;
        self.store_chart_snapshot();
        self.emit(WorkerEvent::Published);
    }

    /// Push a chart op onto the unified undo timeline (charts feedback item 4) and clear the redo
    /// stack — a new forward action always invalidates redo. Called by the four undoable chart ops
    /// (insert / delete / anchor / range) just before [`commit_chart_op`](Self::commit_chart_op).
    fn push_chart_undo(&mut self, cu: ChartUndo) {
        self.undo_stack.push(UndoEntry::Chart(cu));
        self.redo_stack.clear();
    }

    /// Undo the single most-recent action (`Command::Undo`). Dispatches on the unified timeline's top
    /// entry: a `Cell` top drives IronCalc's own undo through the single-command edit path (which
    /// re-reads the popped touch-set); a `Chart` top is inverted worker-side without touching
    /// IronCalc's stack. An empty stack routes to the edit path too, reproducing the old no-op undo.
    fn apply_undo(&mut self) {
        if matches!(self.undo_stack.last(), Some(UndoEntry::Chart(_))) {
            self.undo_chart_op();
        } else {
            // `Cell` top or empty → the IronCalc undo path (degraded-guarded inside; the empty case
            // is a no-op, identical to before charts joined the timeline).
            self.apply_edit_batch(vec![Command::Undo]);
        }
    }

    /// Redo the single most-recent undone action (`Command::Redo`) — the mirror of
    /// [`apply_undo`](Self::apply_undo), dispatching on the redo stack's top entry.
    fn apply_redo(&mut self) {
        if matches!(self.redo_stack.last(), Some(UndoEntry::Chart(_))) {
            self.redo_chart_op();
        } else {
            self.apply_edit_batch(vec![Command::Redo]);
        }
    }

    /// Undo a chart op: pop the `Chart` entry (the caller guaranteed one is on top), apply its
    /// inverse worker-side, push the counterpart onto the redo stack, and republish. Degraded-guarded
    /// like every mutating op — a degraded worker refuses (leaving the stacks untouched). Does NOT
    /// call IronCalc's `undo()`, so the `Cell` entries stay 1:1 with IronCalc's stack.
    fn undo_chart_op(&mut self) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(UndoEntry::Chart(cu)) = self.undo_stack.pop() else {
            return; // caller guarantees a Chart entry is on top
        };
        let counterpart = self.undo_chart_entry(cu);
        self.redo_stack.push(UndoEntry::Chart(counterpart));
        self.commit_chart_op();
    }

    /// Redo a chart op: the mirror of [`undo_chart_op`](Self::undo_chart_op) over the redo stack.
    fn redo_chart_op(&mut self) {
        if self.degraded {
            self.emit(WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded,
            });
            return;
        }
        let Some(UndoEntry::Chart(cu)) = self.redo_stack.pop() else {
            return;
        };
        let counterpart = self.redo_chart_entry(cu);
        self.undo_stack.push(UndoEntry::Chart(counterpart));
        self.commit_chart_op();
    }

    /// Apply the **inverse** of one chart op (revert it), and return the [`ChartUndo`] to push onto
    /// the redo stack (the same variant — [`redo_chart_entry`](Self::redo_chart_entry) re-applies it
    /// forward). Snapshot-based, so each arm just restores or removes stashed whole state.
    fn undo_chart_entry(&mut self, cu: ChartUndo) -> ChartUndo {
        match cu {
            // Undo an insert → remove it (redo re-inserts `entry`).
            ChartUndo::InsertAuthored { index, entry } => {
                if index < self.authored_charts.len() {
                    self.authored_charts.remove(index);
                }
                ChartUndo::InsertAuthored { index, entry }
            }
            // Undo a delete → re-insert `entry` at its old slot (redo removes it again).
            ChartUndo::DeleteAuthored { index, entry } => {
                let at = index.min(self.authored_charts.len());
                self.authored_charts.insert(at, (*entry).clone());
                ChartUndo::DeleteAuthored { index, entry }
            }
            // Undo a loaded delete → re-bind the chart, clear the save-set bookkeeping the delete
            // added, and restore the anchor-edit it evicted.
            ChartUndo::DeleteLoaded {
                removed,
                chart_part,
                prior_anchor_edit,
            } => {
                self.charts.reinsert_removed((*removed).clone());
                self.loaded_deletes.remove(&chart_part);
                match prior_anchor_edit {
                    Some(a) => {
                        self.loaded_anchor_edits.insert(chart_part.clone(), a);
                    }
                    None => {
                        self.loaded_anchor_edits.remove(&chart_part);
                    }
                }
                ChartUndo::DeleteLoaded {
                    removed,
                    chart_part,
                    prior_anchor_edit,
                }
            }
            ChartUndo::SetAnchorAuthored { id, prior, applied } => {
                if let Some(e) = self.authored_charts.iter_mut().find(|e| e.id == id) {
                    e.spec.anchor = prior;
                }
                ChartUndo::SetAnchorAuthored { id, prior, applied }
            }
            ChartUndo::SetAnchorLoaded {
                id,
                chart_part,
                prior_render,
                prior_edit,
                applied,
            } => {
                self.charts.set_anchor_by_id(id, prior_render);
                match prior_edit {
                    Some(a) => {
                        self.loaded_anchor_edits.insert(chart_part.clone(), a);
                    }
                    None => {
                        self.loaded_anchor_edits.remove(&chart_part);
                    }
                }
                ChartUndo::SetAnchorLoaded {
                    id,
                    chart_part,
                    prior_render,
                    prior_edit,
                    applied,
                }
            }
            ChartUndo::SetRangeAuthored {
                index,
                prior,
                applied,
            } => {
                if index < self.authored_charts.len() {
                    self.authored_charts[index] = (*prior).clone();
                }
                ChartUndo::SetRangeAuthored {
                    index,
                    prior,
                    applied,
                }
            }
        }
    }

    /// Apply the **forward** of one chart op (re-apply it), and return the [`ChartUndo`] to push onto
    /// the undo stack — the mirror of [`undo_chart_entry`](Self::undo_chart_entry).
    fn redo_chart_entry(&mut self, cu: ChartUndo) -> ChartUndo {
        match cu {
            // Redo an insert → re-insert `entry` at its slot.
            ChartUndo::InsertAuthored { index, entry } => {
                let at = index.min(self.authored_charts.len());
                self.authored_charts.insert(at, (*entry).clone());
                ChartUndo::InsertAuthored { index, entry }
            }
            // Redo a delete → remove it again.
            ChartUndo::DeleteAuthored { index, entry } => {
                if index < self.authored_charts.len() {
                    self.authored_charts.remove(index);
                }
                ChartUndo::DeleteAuthored { index, entry }
            }
            // Redo a loaded delete → re-run the delete effects: take the chart out again (a fresh
            // whole binding for a later undo), re-add its part to `loaded_deletes`, drop the
            // anchor-edit the undo restored.
            ChartUndo::DeleteLoaded {
                removed,
                chart_part,
                prior_anchor_edit,
            } => {
                let fresh = self.charts.take_by_id(removed.id());
                self.loaded_anchor_edits.remove(&chart_part);
                self.loaded_deletes.insert(chart_part.clone());
                // A fresh whole binding for a later undo (identical to `removed`; the chart wasn't
                // touched while re-bound), falling back to the stash if the take somehow missed.
                let removed = fresh.map(Box::new).unwrap_or(removed);
                ChartUndo::DeleteLoaded {
                    removed,
                    chart_part,
                    prior_anchor_edit,
                }
            }
            ChartUndo::SetAnchorAuthored { id, prior, applied } => {
                if let Some(e) = self.authored_charts.iter_mut().find(|e| e.id == id) {
                    e.spec.anchor = applied;
                }
                ChartUndo::SetAnchorAuthored { id, prior, applied }
            }
            ChartUndo::SetAnchorLoaded {
                id,
                chart_part,
                prior_render,
                prior_edit,
                applied,
            } => {
                self.charts.set_anchor_by_id(id, applied);
                self.loaded_anchor_edits.insert(chart_part.clone(), applied);
                ChartUndo::SetAnchorLoaded {
                    id,
                    chart_part,
                    prior_render,
                    prior_edit,
                    applied,
                }
            }
            ChartUndo::SetRangeAuthored {
                index,
                prior,
                applied,
            } => {
                if index < self.authored_charts.len() {
                    self.authored_charts[index] = (*applied).clone();
                }
                ChartUndo::SetRangeAuthored {
                    index,
                    prior,
                    applied,
                }
            }
        }
    }

    /// Store the current bound charts as the published [`ChartSnapshot`] (charts/architecture §4.1),
    /// riding the same wait-free `arc_swap` container as the cell publication. Merges the loaded
    /// (live-bound) charts with the **authored** ones (P17) into per-sheet groups.
    fn store_chart_snapshot(&self) {
        let sheets = if self.authored_charts.is_empty() {
            // Fast path (no authored charts): share the worker's `Arc<[ChartSpec]>` allocations
            // directly (P11 "off-screen free" — no per-publish deep copy of the loaded specs).
            self.charts.specs_by_sheet()
        } else {
            self.charts_by_sheet_with_authored()
        };
        self.shared.chart_snapshot.store(Arc::new(ChartSnapshot {
            version: self.chart_version,
            sheets,
        }));
    }

    /// The loaded specs (grouped by sheet) with each authored chart appended to its anchor sheet's
    /// group — the snapshot payload when the workbook carries authored charts. Loaded charts keep
    /// their discovery order; authored charts follow in insert order.
    fn charts_by_sheet_with_authored(&self) -> Vec<(SheetId, Arc<[ChartSpec]>)> {
        let mut groups: Vec<(SheetId, Vec<ChartSpec>)> = self
            .charts
            .specs_by_sheet()
            .into_iter()
            .map(|(sheet, specs)| (sheet, specs.to_vec()))
            .collect();
        for entry in &self.authored_charts {
            // Stamp the authored chart's stable id (P18) so the app can manipulate it.
            let spec = entry.spec.clone().with_id(entry.id);
            match groups.iter_mut().find(|(s, _)| *s == entry.anchor_sheet) {
                Some((_, specs)) => specs.push(spec),
                None => groups.push((entry.anchor_sheet, vec![spec])),
            }
        }
        groups
            .into_iter()
            .map(|(sheet, specs)| (sheet, Arc::from(specs)))
            .collect()
    }

    /// Publish the active sheet's viewport snapshot, THEN bump the generation — a bump always
    /// has fresh data behind it (SP1's publish-then-bump; the render loop reads generation
    /// then the publication, safe either order). Logs the publish timing at `debug` (an SP1
    /// observable; both the edit and the pure-scroll republish paths route through here).
    fn publish(&self) {
        let started = Instant::now();
        let generation = self.shared.generation.load(Ordering::Acquire) + 1;
        let publication = self.build_publication(generation);
        let cells = publication.cells.len();
        self.shared.publication.store(Arc::new(publication));
        self.shared.generation.store(generation, Ordering::Release);
        tracing::debug!(
            generation,
            cells,
            publish_us = started.elapsed().as_micros() as u64,
            "worker: published viewport"
        );
    }

    /// Build the active-sheet publication for `generation` by probing every cell in the
    /// clamped overscan window and keeping the non-empty ones (the per-cell probe is the
    /// component doc's accepted approach at ≤ ~20k window cells). Display text is
    /// engine-formatted; empty cells are omitted (the grid defaults them).
    fn build_publication(&self, generation: u64) -> Publication {
        let sheet = self.active_sheet;
        match (self.resolve(sheet), &self.viewport) {
            (Some(idx), Some(vp)) => {
                let mut cells = Vec::new();
                for row in vp.rows.clone() {
                    for col in vp.cols.clone() {
                        let cell = CellRef::new(row, col);
                        if let Ok(text) = self.doc.formatted_value(idx, cell) {
                            if !text.is_empty() {
                                // Classify the cell + resolve its text colour ([Red]-style
                                // number-format colour or explicit font colour, `§1.2`). A
                                // rare read error defaults to plain text (never fails a
                                // publish).
                                let (kind, text_color) = self
                                    .doc
                                    .published_style(idx, cell)
                                    .unwrap_or((CellKind::Text, None));
                                cells.push(PublishedCell {
                                    row,
                                    col,
                                    display_text: text,
                                    kind,
                                    text_color,
                                });
                            }
                        }
                    }
                }
                Publication {
                    sheet,
                    rows: vp.rows.clone(),
                    cols: vp.cols.clone(),
                    generation,
                    cells,
                }
            }
            _ => Publication::empty(sheet, generation),
        }
    }

    /// Reconcile the published CF rule map for each of `sheets` against the document's current
    /// rules, emitting [`WorkerEvent::CondFmtUpdated`] only when a sheet's published list actually
    /// changed (`architecture.md §4.5`, `components/engine_cf.md §5`).
    ///
    /// Gated so a non-CF workbook pays nothing: a sheet with no CF rule **and** no published entry is
    /// skipped before any list read. A rule change (add / update / delete / reorder, or a structural
    /// range shift) or the undo/redo of one flips `has_cond_fmt` / the list contents, so it is
    /// reconciled. The map holds no entry for a sheet with zero rules (so the client reads empty).
    fn reconcile_published_cond_fmt(&self, sheets: &[SheetId]) {
        for &sheet in sheets {
            let Some(idx) = self.resolve(sheet) else {
                continue; // sheet deleted out from under the touch-set — pruned by the caller
            };
            let has_now = self.doc.has_cond_fmt(idx);
            let had_entry = self.shared.cond_fmt.read().contains_key(&sheet);
            if !has_now && !had_entry {
                continue; // non-CF sheet, nothing published → the fast path stays free
            }
            // A read failure degrades to "no rules" (never a panic) — the sheet's entry is dropped.
            // Logged (architecture §6), matching `extended_render_style`'s fallback.
            let rules = match self.doc.cond_fmt_rules(idx) {
                Ok(rules) => rules,
                Err(err) => {
                    tracing::warn!(
                        sheet = idx,
                        error = %err,
                        "cond_fmt_rules read failed; publishing an empty CF list",
                    );
                    Vec::new()
                }
            };
            let changed = {
                let map = self.shared.cond_fmt.read();
                match map.get(&sheet) {
                    Some(published) => published != &rules,
                    None => !rules.is_empty(),
                }
            };
            if !changed {
                continue;
            }
            {
                let mut map = self.shared.cond_fmt.write();
                if rules.is_empty() {
                    map.remove(&sheet);
                } else {
                    map.insert(sheet, rules);
                }
            }
            self.emit(WorkerEvent::CondFmtUpdated { sheet });
        }
    }

    /// Populate the published CF map once on open for every sheet that carries rules
    /// (`components/engine_cf.md §5`). No event is emitted — the window reads the map when it first
    /// builds a panel (the `Loaded` event already drives the initial UI). Cheap: gated on the
    /// `has_cond_fmt` fast check, so a workbook with no CF writes nothing.
    fn publish_all_cond_fmt_on_open(&self) {
        for meta in self.sheet_metas() {
            let Some(idx) = self.resolve(meta.id) else {
                continue;
            };
            if !self.doc.has_cond_fmt(idx) {
                continue;
            }
            if let Ok(rules) = self.doc.cond_fmt_rules(idx) {
                if !rules.is_empty() {
                    self.shared.cond_fmt.write().insert(meta.id, rules);
                }
            }
        }
    }

    /// The sheet list as `SheetMeta` (stable id + current name + `has_content`), in workbook
    /// order. `has_content` gates the UI's delete-confirm modal (`functional_spec.md §3.7`).
    fn sheet_metas(&self) -> Vec<SheetMeta> {
        self.doc
            .sheet_properties_with_content()
            .into_iter()
            .map(|(id, name, has_content)| SheetMeta {
                id: SheetId(id),
                name,
                has_content,
            })
            .collect()
    }

    /// Resolve a stable [`SheetId`] to its current worksheet index (`None` if it was deleted).
    fn resolve(&self, sheet: SheetId) -> Option<u32> {
        resolve_idx(&self.doc, sheet).ok()
    }

    /// Send an event; drops silently if the UI has released the receiver (worker outlives it
    /// only at teardown).
    fn emit(&self, event: WorkerEvent) {
        let _ = self.event_tx.try_send(event);
    }
}

/// Apply one chrome edit to a render [`Chart`] (P20) — the pure mutation shared by the authored and
/// loaded chrome-edit paths (`set_chart_chrome`). A [`DataLabels`](ChartChromeEdit::DataLabels) edit
/// applies the show toggles across **every** series, preserving each series' existing label
/// number-format / separator / position (and any legend-key / series-name already shown), and clears
/// a series' labels to `None` only when nothing at all would show.
fn apply_chrome_edit(chart: &mut Chart, edit: &ChartChromeEdit) {
    match edit {
        ChartChromeEdit::Title(title) => chart.title = title.clone(),
        ChartChromeEdit::Legend(position) => {
            chart.legend = position.map(|position| Legend { position })
        }
        ChartChromeEdit::AxisTitle { axis, title } => {
            let ax = match axis {
                ChartAxisKind::Category => &mut chart.cat_axis,
                ChartAxisKind::Value => &mut chart.val_axis,
            };
            ax.title = title.clone();
        }
        ChartChromeEdit::SeriesColor { series, color } => {
            // A "series color" edit recolors the WHOLE series. Only a LINE or SCATTER series carries
            // its visible color on the `a:ln` stroke, which the renderer prefers over `color`
            // (line.rs / scatter.rs: `stroke.color.or(color)`). Leaving the stroke on its original
            // color is exactly why a loaded LINE chart kept its old color on screen and through save
            // while an authored one (no stroke) honored the edit — charts feedback item 9. FILLED
            // kinds (bar/column/area/pie/bubble) render from the fill (`color`) and treat `a:ln` as
            // a decorative border, so recoloring their stroke would over-reach — mutating a border
            // the user never touched (charts feedback Batch 5). Gate the stroke recolor accordingly.
            let recolor_stroke = matches!(
                chart.kind,
                ChartKind::Line { .. } | ChartKind::Scatter { .. }
            );
            if let Some(s) = chart.series.get_mut(*series) {
                let new = color.map(|rgb| ChartColor::Rgb(Color::from_hex(rgb.to_hex())));
                s.color = new;
                // Override only the stroke's COLOR (keep its width/alpha); clearing the series color
                // (`None`) reverts the stroke to the palette too.
                if recolor_stroke {
                    if let Some(stroke) = &mut s.stroke {
                        stroke.color = new;
                    }
                }
            }
        }
        ChartChromeEdit::DataLabels(toggles) => {
            for s in &mut chart.series {
                let mut labels = s.data_labels.clone().unwrap_or_default();
                labels.show_value = toggles.show_value;
                labels.show_category_name = toggles.show_category_name;
                labels.show_percent = toggles.show_percent;
                s.data_labels = labels.is_shown().then_some(labels);
            }
        }
    }
}

/// Apply one edit command to the model, resolving its stable sheet id to an index. Runs inside
/// the `catch_unwind` guard, so a genuine engine panic here is caught by the caller.
fn apply_one(doc: &mut WorkbookDocument, edit: &Command) -> Result<AppliedKind, String> {
    match edit {
        Command::SetCellInput { sheet, cell, input } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.set_cell_input(idx, *cell, input)?;
            Ok(AppliedKind::Cell)
        }
        Command::ClearCells { sheet, range } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.clear_contents(idx, *range)?;
            Ok(AppliedKind::Cell)
        }
        Command::FillDown { sheet, range } => {
            let idx = resolve_idx(doc, *sheet)?;
            // An edge/seed-line/empty-clamp fill writes nothing → NoOp (skip eval/publish/op).
            let applied = doc.fill_down(idx, *range)?;
            Ok(if applied {
                AppliedKind::Cell
            } else {
                AppliedKind::NoOp
            })
        }
        Command::FillRight { sheet, range } => {
            let idx = resolve_idx(doc, *sheet)?;
            let applied = doc.fill_right(idx, *range)?;
            Ok(if applied {
                AppliedKind::Cell
            } else {
                AppliedKind::NoOp
            })
        }
        Command::FillDrag {
            sheet,
            seed,
            target,
            axis,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            // Overflow guard: reject a drag-fill whose target exceeds the same cell-count cap
            // paste/fill use (`architecture.md §3.3`) → the standard large-op dialog.
            if range_area(target) > MAX_REFRESH_CELLS {
                return Err("Fill target is too large".to_string());
            }
            let applied = doc.fill_drag(idx, *seed, *target, *axis)?;
            Ok(if applied {
                AppliedKind::Cell
            } else {
                AppliedKind::NoOp
            })
        }
        Command::SetStyleAttr { sheet, range, attr } => {
            let idx = resolve_idx(doc, *sheet)?;
            apply_style(doc, idx, *range, *attr)?;
            // Styles don't affect values → no recompute needed (component-doc command table).
            Ok(AppliedKind::StyleOnly)
        }
        Command::SetStylePath {
            sheet,
            range,
            path,
            value,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.update_style_path(idx, *range, path.as_str(), value)?;
            // Text color / alignment / number format never change values → no recompute.
            Ok(AppliedKind::StyleOnly)
        }
        Command::SetBorders {
            sheet,
            range,
            preset,
            line,
            color,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            // `None` ⇒ default black; otherwise the pen's `#RRGGBB` (same form as `font.color`).
            let color_hex = color
                .map(|c| format!("#{:06X}", c.to_hex()))
                .unwrap_or_else(|| "#000000".to_string());
            doc.set_borders(
                idx,
                *range,
                preset.border_type_tag(),
                line.style_tag(),
                &color_hex,
            )?;
            // Borders never change values → no recompute.
            Ok(AppliedKind::StyleOnly)
        }
        Command::SetColumnWidths {
            sheet,
            col_start,
            col_end,
            px,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.set_column_widths(idx, *col_start, *col_end, *px)?;
            Ok(AppliedKind::GeometryOnly)
        }
        Command::SetRowHeights {
            sheet,
            row_start,
            row_end,
            px,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.set_row_heights_px(idx, *row_start, *row_end, *px)?;
            Ok(AppliedKind::GeometryOnly)
        }
        Command::SetRowsHidden {
            sheet,
            start,
            end,
            hidden,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.set_rows_hidden(idx, *start, *end, *hidden)?;
            Ok(AppliedKind::GeometryOnly)
        }
        Command::SetColumnsHidden {
            sheet,
            start,
            end,
            hidden,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.set_columns_hidden(idx, *start, *end, *hidden)?;
            Ok(AppliedKind::GeometryOnly)
        }
        Command::InsertRows { sheet, row, count } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.insert_rows(idx, *row, *count)?;
            Ok(AppliedKind::Structure)
        }
        Command::InsertColumns { sheet, col, count } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.insert_columns(idx, *col, *count)?;
            Ok(AppliedKind::Structure)
        }
        Command::DeleteRows { sheet, row, count } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.delete_rows(idx, *row, *count)?;
            Ok(AppliedKind::Structure)
        }
        Command::DeleteColumns { sheet, col, count } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.delete_columns(idx, *col, *count)?;
            Ok(AppliedKind::Structure)
        }
        // The data-loss confirm gate runs BEFORE apply (`apply_edit_batch`), so a `MergeCells` that
        // reaches here is safe to perform (confirmed, or no covered content). `Structure` triggers
        // the eval + full active-sheet cache rebuild that re-reads `merged_regions` (§3).
        Command::MergeCells { sheet, area, .. } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.merge_cells(idx, *area)?;
            Ok(AppliedKind::Structure)
        }
        Command::UnmergeCells { sheet, anchor } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.unmerge_cells(idx, *anchor)?;
            Ok(AppliedKind::Structure)
        }
        Command::AddSheet => {
            doc.add_sheet()?;
            Ok(AppliedKind::SheetOp)
        }
        Command::RenameSheet { sheet, name } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.rename_sheet(idx, name)?;
            Ok(AppliedKind::SheetOp)
        }
        Command::DeleteSheet { sheet } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.delete_sheet(idx)?;
            Ok(AppliedKind::SheetOp)
        }
        Command::MoveSheet { sheet, to_index } => {
            // Map the stable id → its CURRENT worksheet index, then reorder by index (the fork
            // API is index-based). A `SheetsChanged` republish is driven by the batch's
            // before/after `sheet_metas()` comparison, so tabs rebuild in the new engine order.
            let idx = resolve_idx(doc, *sheet)?;
            doc.move_sheet(idx, *to_index)?;
            Ok(AppliedKind::SheetOp)
        }
        Command::AddCondFmt { sheet, range, spec } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.add_cond_fmt(idx, range, spec)?;
            // A CF rule leaves values unchanged, but its result depends on the new rule → force a
            // CF re-eval so `cf_cache` is fresh before the cache rebuild reads it (BUG-1).
            Ok(AppliedKind::CondFmt)
        }
        Command::UpdateCondFmt {
            sheet,
            index,
            range,
            spec,
        } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.update_cond_fmt(idx, *index, range, spec)?;
            Ok(AppliedKind::CondFmt)
        }
        Command::DeleteCondFmt { sheet, index } => {
            let idx = resolve_idx(doc, *sheet)?;
            doc.delete_cond_fmt(idx, *index)?;
            Ok(AppliedKind::CondFmt)
        }
        // Raise/Lower may be a boundary NO-OP (rule already top/bottom): the engine records no undo
        // diff in that case, so returning anything but `NoOp` would push a phantom worker undo entry
        // and desync the 1:1 stack. Compare the rule list before/after to tell a real reorder apart.
        Command::RaiseCondFmtPriority { sheet, index } => {
            let idx = resolve_idx(doc, *sheet)?;
            let before = doc.cond_fmt_rules(idx)?;
            doc.raise_cond_fmt(idx, *index)?;
            Ok(if doc.cond_fmt_rules(idx)? == before {
                AppliedKind::NoOp
            } else {
                AppliedKind::CondFmt
            })
        }
        Command::LowerCondFmtPriority { sheet, index } => {
            let idx = resolve_idx(doc, *sheet)?;
            let before = doc.cond_fmt_rules(idx)?;
            doc.lower_cond_fmt(idx, *index)?;
            Ok(if doc.cond_fmt_rules(idx)? == before {
                AppliedKind::NoOp
            } else {
                AppliedKind::CondFmt
            })
        }
        Command::Undo => {
            doc.undo()?;
            Ok(AppliedKind::Cell)
        }
        Command::Redo => {
            doc.redo()?;
            Ok(AppliedKind::Cell)
        }
        #[cfg(test)]
        Command::TestPanic => panic!("injected test panic (catch_unwind recovery)"),
        // Control commands are bucketed out before apply — never reached in practice.
        _ => Err("non-edit command reached the apply path".to_string()),
    }
}

/// Apply a style attribute across a range. Bold/italic/underline/strikethrough and wrap-text are
/// toggles resolved from the current state ("any cell lacks it → set the whole range, else clear
/// it"); `Fill` is a direct set/clear.
fn apply_style(
    doc: &mut WorkbookDocument,
    idx: u32,
    range: CellRange,
    attr: StyleAttr,
) -> Result<(), String> {
    let flag = match attr {
        StyleAttr::Fill(fill) => return doc.set_fill(idx, range, fill),
        // Wrap toggles the same "any-lacking → set all, else clear" way, but writes the
        // `alignment.wrap_text` path (it isn't a `font.*` flag).
        StyleAttr::WrapText => {
            let any_lacking = any_cell_lacks(range, |cell| doc.wrap_flag(idx, cell))?;
            let value = if any_lacking { "true" } else { "false" };
            return doc.update_style_path(idx, range, "alignment.wrap_text", value);
        }
        StyleAttr::Bold => FontFlag::Bold,
        StyleAttr::Italic => FontFlag::Italic,
        StyleAttr::Underline => FontFlag::Underline,
        StyleAttr::Strikethrough => FontFlag::Strike,
    };
    // Toggle resolution. P4 reads current state per cell from the engine; P5's resident cache
    // makes this an O(1)-ish map lookup. Ranges are user selections (bounded), not full sheets.
    let any_lacking = any_cell_lacks(range, |cell| doc.font_flag(idx, cell, flag))?;
    doc.set_font_flag(idx, range, flag, any_lacking)
}

/// Whether any cell in `range` fails `is_set` — the toggle scan shared by the font-flag and
/// wrap-text toggles ("any cell lacks the attribute → set the whole range, else clear it").
/// Short-circuits on the first lacking cell. Ranges are bounded user selections.
fn any_cell_lacks(
    range: CellRange,
    mut is_set: impl FnMut(CellRef) -> Result<bool, String>,
) -> Result<bool, String> {
    for row in range.rows() {
        for col in range.cols() {
            if !is_set(CellRef::new(row, col))? {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The `xl/charts/chartN.xml` part names already present in a serialized package (loaded charts
/// re-injected by mode 1/2) — the used set the authored-chart part assignment avoids colliding with.
/// A package that can't be read as a zip yields an empty set (the write path then validates any real
/// collision and fails loudly).
fn existing_chart_parts(package_bytes: &[u8]) -> HashSet<String> {
    let Ok(mut zip) = zip::ZipArchive::new(std::io::Cursor::new(package_bytes)) else {
        return HashSet::new();
    };
    (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with("xl/charts/") && n.ends_with(".xml") && !n.contains("/_rels/"))
        .collect()
}

/// The distinct `c:f` formulas of a ranged authored chart's [`SeriesRefs`] as [`CfRange`]s, in
/// first-seen order (name / categories / values across the series), for the published spec's
/// `source_ranges` (P19). Deduped so a shared category column isn't listed once per series — the
/// value the edit panel reads back to show the chart's current data range.
fn source_ranges_from_refs(refs: &[SeriesRefs]) -> Vec<CfRange> {
    let mut out: Vec<CfRange> = Vec::new();
    for formula in refs
        .iter()
        .flat_map(|r| [&r.name, &r.categories, &r.values, &r.sizes])
        .flatten()
    {
        if !out.iter().any(|r| r.as_str() == formula) {
            out.push(CfRange::new(formula.clone()));
        }
    }
    out
}

/// The next free `xl/charts/chartN.xml` part, marking it used (mirrors `write::next_drawing_part`).
fn next_chart_part(used: &mut HashSet<String>) -> String {
    let mut n = 1;
    loop {
        let candidate = format!("xl/charts/chart{n}.xml");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// Resolve a stable [`SheetId`] to a worksheet index, or an engine-style error message.
fn resolve_idx(doc: &WorkbookDocument, sheet: SheetId) -> Result<u32, String> {
    doc.sheet_properties()
        .iter()
        .position(|(id, _)| *id == sheet.0)
        .map(|i| i as u32)
        .ok_or_else(|| format!("no sheet with id {}", sheet.0))
}

/// Classify a just-applied edit for post-eval cache bookkeeping. Only ever called on
/// successfully-applied edit commands (control commands are bucketed out before apply, and
/// `TestPanic` panics before returning `Ok`), so the non-edit arm is unreachable.
fn op_of(edit: &Command) -> AppliedOp {
    match edit {
        Command::SetCellInput { sheet, cell, .. } => AppliedOp::Cells {
            sheet: *sheet,
            range: CellRange::single(*cell),
        },
        Command::ClearCells { sheet, range } => AppliedOp::Cells {
            sheet: *sheet,
            range: *range,
        },
        // Fill writes within the selection rectangle (the single-cell pull-from-neighbor case only
        // reads the seed neighbor, writes the selected cell) → refresh exactly that range.
        Command::FillDown { sheet, range } | Command::FillRight { sheet, range } => {
            AppliedOp::Cells {
                sheet: *sheet,
                range: *range,
            }
        }
        // Drag-fill writes over the whole `target` rectangle (⊇ seed) → refresh exactly that range.
        Command::FillDrag { sheet, target, .. } => AppliedOp::Cells {
            sheet: *sheet,
            range: *target,
        },
        Command::SetStyleAttr { sheet, range, .. } | Command::SetStylePath { sheet, range, .. } => {
            AppliedOp::Cells {
                sheet: *sheet,
                range: *range,
            }
        }
        // `set_area_with_border` also fixes up the four cells adjacent to the range (heavier-wins
        // sync of the shared edge), so the mirror must re-read a one-cell ring around the target.
        // A full row/col stays band-creating after expansion → the refresh takes the full-rebuild
        // path, which reads bands back correctly.
        Command::SetBorders { sheet, range, .. } => AppliedOp::Cells {
            sheet: *sheet,
            range: expand_by_one_cell(*range),
        },
        // Resize / insert / delete: the touched region is unbounded (geometry + shifted content),
        // so the whole sheet cache is rebuilt on apply and on undo/redo.
        Command::SetColumnWidths { sheet, .. }
        | Command::SetRowHeights { sheet, .. }
        | Command::SetRowsHidden { sheet, .. }
        | Command::SetColumnsHidden { sheet, .. }
        | Command::InsertRows { sheet, .. }
        | Command::InsertColumns { sheet, .. }
        | Command::DeleteRows { sheet, .. }
        | Command::DeleteColumns { sheet, .. }
        // Merge/unmerge shift covered content + the region set → a full active-sheet rebuild that
        // re-reads `merged_regions` (merged-cell-ui `architecture.md §3`).
        | Command::MergeCells { sheet, .. }
        | Command::UnmergeCells { sheet, .. } => AppliedOp::Rebuild { sheet: *sheet },
        Command::AddSheet
        | Command::RenameSheet { .. }
        | Command::DeleteSheet { .. }
        | Command::MoveSheet { .. } => AppliedOp::Sheets,
        // Every CF mutation maps to a full sheet-cache rebuild (not `Cells { range }`): Delete/Raise/
        // Lower carry no range, a rule's range may be a multi-area address, and a rule/reorder can
        // affect its whole (possibly large) range — so a wholesale rebuild is the simple, always-
        // correct refresh (matching the value-publish path, architecture §6). This rebuilt-sheet set
        // is also what drives the CF-map republish in `apply_edit_batch` (incl. structural range
        // shifts + undo/redo of a CF op, whose `Touch::Rebuild` re-lands the sheet here).
        Command::AddCondFmt { sheet, .. }
        | Command::UpdateCondFmt { sheet, .. }
        | Command::DeleteCondFmt { sheet, .. }
        | Command::RaiseCondFmtPriority { sheet, .. }
        | Command::LowerCondFmtPriority { sheet, .. } => AppliedOp::Rebuild { sheet: *sheet },
        Command::Undo => AppliedOp::Undo,
        Command::Redo => AppliedOp::Redo,
        _ => unreachable!("op_of called on a non-edit command"),
    }
}

/// The `(sheet, range)`s to re-read when a touch-entry is undone/redone (a paste's
/// `Touch::Ranges` fans out to several; `Touch::Sheets` reconciles the map instead of cells).
fn touch_refresh_ranges(touch: &Touch) -> Vec<(SheetId, CellRange)> {
    match touch {
        Touch::Cells { sheet, range } => vec![(*sheet, *range)],
        Touch::Ranges(ranges) => ranges.clone(),
        Touch::Sheets | Touch::Rebuild { .. } => Vec::new(),
    }
}

/// The sheet(s) to fully rebuild when a touch-entry is undone/redone — only a
/// [`Touch::Rebuild`] (a resize / insert / delete), whose region is unbounded.
fn touch_rebuild_sheets(touch: &Touch) -> Vec<SheetId> {
    match touch {
        Touch::Rebuild { sheet } => vec![*sheet],
        Touch::Cells { .. } | Touch::Ranges(_) | Touch::Sheets => Vec::new(),
    }
}

/// Converts a 1-based inclusive engine rectangle `(row0, col0, row1, col1)` to a 0-based
/// [`CellRange`] (the clipboard slot stores the engine's tuple; the cache mirror wants a range).
fn tuple_to_range((r0, c0, r1, c1): (i32, i32, i32, i32)) -> CellRange {
    let cell = |r: i32, c: i32| CellRef::new(r.max(1) as u32 - 1, c.max(1) as u32 - 1);
    CellRange::new(cell(r0, c0), cell(r1, c1))
}

/// The number of cells a [`CellRange`] covers (for the mirror's pathological-range guard).
fn range_area(range: &CellRange) -> u64 {
    let rows = (range.end.row - range.start.row) as u64 + 1;
    let cols = (range.end.col - range.start.col) as u64 + 1;
    rows * cols
}

/// Grows `range` by one cell in every direction, clamped to the sheet bounds — the refresh window
/// for a border edit (the engine's `set_area_with_border` also touches the four adjacent strips).
/// A full-row/col range is unchanged on its spanning axis (already at the bound), so it stays
/// band-creating and the refresh full-rebuilds.
fn expand_by_one_cell(range: CellRange) -> CellRange {
    CellRange::new(
        CellRef::new(
            range.start.row.saturating_sub(1),
            range.start.col.saturating_sub(1),
        ),
        CellRef::new(
            (range.end.row + 1).min(limits::MAX_ROWS - 1),
            (range.end.col + 1).min(limits::MAX_COLS - 1),
        ),
    )
}

/// Whether a style edit over `range` makes IronCalc create a **band** (a row spanning every
/// column, or a column spanning every row) instead of per-cell styles — in which case the
/// per-cell mirror is insufficient and the sheet cache must be rebuilt from the engine. This is
/// the precise trigger (not the cell-count cap): a single full-row band is only 16,384 cells,
/// which sits *below* `MAX_REFRESH_CELLS`, so relying on the cap alone would let bands rot the
/// cache (they'd take the per-cell path and never create the band).
fn is_band_creating(range: &CellRange) -> bool {
    let all_columns = range.start.col == 0 && range.end.col == limits::MAX_COLS - 1;
    let all_rows = range.start.row == 0 && range.end.row == limits::MAX_ROWS - 1;
    all_columns || all_rows
}

/// Above this many cells, a mirror re-read of a range falls back to a full active-sheet rebuild
/// (bounded by populated cells) instead of iterating the selection cell by cell — a guard against
/// a pathologically large selection. Comfortably exceeds any real user selection.
const MAX_REFRESH_CELLS: u64 = 100_000;

/// Epsilon (device px) for wrap-driven row-height comparisons — a settled row (measured height ==
/// committed height within this) is treated as unchanged, so a confirming re-measure emits no
/// command and the feedback loop converges (`architecture.md §3`).
const AUTO_GROW_EPS_PX: f32 = 0.5;

/// The largest overscan window the worker will publish, per axis. These bounds cap the
/// per-cell probe cost at `MAX_PUBLISH_ROWS * MAX_PUBLISH_COLS` = 131,072 cells so a
/// pathological `SetViewport` (e.g. the whole sheet) can't wedge the worker in a billion-cell
/// loop — the worker is the robustness boundary, and this loop is not inside `catch_unwind`.
///
/// They are sized to comfortably exceed a ~3× overscan of the largest supported display (a 4K
/// screen requests on the order of ~300 rows × ~180 cols of overscan), so overscan pre-fetch
/// is never clipped in practice. NOTE (Phase 6/7): once the real grid exists, cross-check that
/// these still keep margin over the actual overscan dimensions the grid requests.
const MAX_PUBLISH_ROWS: u32 = 512;
const MAX_PUBLISH_COLS: u32 = 256;

/// Clamp a requested viewport to the sheet bounds **and** a bounded overscan window. The
/// viewport arrives pre-overscanned UI-side; the worker keeps the top-left anchor and truncates
/// the span so the published window can never exceed `MAX_PUBLISH_ROWS × MAX_PUBLISH_COLS`.
fn clamp_viewport(rows: std::ops::Range<u32>, cols: std::ops::Range<u32>) -> Viewport {
    Viewport {
        rows: clamp_span(rows, limits::MAX_ROWS, MAX_PUBLISH_ROWS),
        cols: clamp_span(cols, limits::MAX_COLS, MAX_PUBLISH_COLS),
    }
}

/// Clamp a half-open span to `[0, sheet_max)` then cap its length to `max_len` (keeping the
/// start). An inverted or empty input yields an empty span.
fn clamp_span(range: std::ops::Range<u32>, sheet_max: u32, max_len: u32) -> std::ops::Range<u32> {
    let start = range.start.min(sheet_max);
    let end = range.end.clamp(start, sheet_max);
    let capped_end = end.min(start.saturating_add(max_len));
    start..capped_end
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::protocol::StylePath;
    use freecell_core::input_cap::{InputRejection, MAX_INPUT_LEN, MAX_NESTING_DEPTH};
    use freecell_core::{CfFormat, CfRuleSpec, CfValueOp, Rgb};

    /// Build a headless worker over a fresh empty workbook plus the event receiver, without a
    /// spawned thread — the deterministic substrate for the coalescing / recovery tests.
    fn test_worker() -> (Worker, async_channel::Receiver<WorkerEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let doc = WorkbookDocument::new_empty().unwrap();
        let shared = Arc::new(Shared::new(SheetId(0)));
        let mut worker = Worker {
            doc,
            shared,
            event_tx: tx,
            active_sheet: SheetId(0),
            viewport: None,
            ops_seen: 0,
            eval_count: 0,
            degraded: false,
            panic_count: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            clipboard: None,
            charts: ChartBindings::default(),
            authored_charts: Vec::new(),
            next_chart_id: 1,
            loaded_anchor_edits: HashMap::new(),
            loaded_deletes: HashSet::new(),
            chart_version: 0,
            chart_source_path: None,
            discovered_chart_sheets: HashSet::new(),
            charts_fully_discovered: true,
            chart_sheet_parts: HashMap::new(),
            manual_rows: HashMap::new(),
            wrap_heights: HashMap::new(),
        };
        if let Some(first) = worker.sheet_metas().first() {
            worker.active_sheet = first.id;
        }
        // Build the active sheet's cache so worker-level tests exercise the same resident-cache
        // state the real `load_and_run` sets up (build-on-open).
        worker.build_and_store_cache(worker.active_sheet);
        (worker, rx)
    }

    /// The only sheet's stable id (what commands must address).
    fn sheet0(worker: &Worker) -> SheetId {
        worker.sheet_metas()[0].id
    }

    fn drain_events(rx: &async_channel::Receiver<WorkerEvent>) -> Vec<WorkerEvent> {
        let mut out = Vec::new();
        while let Ok(ev) = rx.try_recv() {
            out.push(ev);
        }
        out
    }

    fn set_input(sheet: SheetId, row: u32, col: u32, input: &str) -> Command {
        Command::SetCellInput {
            sheet,
            cell: CellRef::new(row, col),
            input: input.to_string(),
        }
    }

    /// Silence the default panic hook while `f` runs, so the injected-panic tests don't spew a
    /// scary (but expected) backtrace into the test log.
    fn quiet_panics<R>(f: impl FnOnce() -> R) -> R {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let r = f();
        std::panic::set_hook(prev);
        r
    }

    #[test]
    fn drain_coalesces_burst_into_one_eval() {
        // 30 edits pushed onto the channel are drained into ONE batch → exactly one eval.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let (tx, cmd_rx) = std::sync::mpsc::channel();
        for i in 0..30u32 {
            tx.send(set_input(sheet, 0, 0, &format!("{}", i + 1)))
                .unwrap();
        }
        tx.send(Command::Shutdown).unwrap();
        drop(tx);

        worker.run(cmd_rx);

        assert_eq!(
            worker.eval_count, 1,
            "30 drained edits must coalesce to 1 eval"
        );
        assert_eq!(worker.ops_seen, 30, "each applied edit is one committed op");
        // The last write wins (A1 == 30).
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "30"
        );
    }

    #[test]
    fn find_command_emits_row_major_results() {
        // Populate a few cells, then a `Find` batch replies with the matching cells in row-major
        // order — a pure read (no publish).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "apple"),     // A1
            set_input(sheet, 1, 1, "APPLE pie"), // B2
            set_input(sheet, 2, 0, "grape"),     // A3 (no match)
        ]);
        let _ = drain_events(&rx);

        worker.process_batch(vec![Command::Find {
            sheet,
            query: "apple".to_string(),
            match_case: false,
            whole_cell: false,
        }]);
        let events = drain_events(&rx);
        let matches = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::FindResults { matches } => Some(matches.clone()),
                _ => None,
            })
            .expect("a Find batch replies FindResults");
        assert_eq!(matches, vec![CellRef::new(0, 0), CellRef::new(1, 1)]);
        // A find is a read — it publishes nothing.
        assert!(!events.iter().any(|e| matches!(e, WorkerEvent::Published)));
    }

    #[test]
    fn selection_stats_command_replies_aggregate() {
        // A `SelectionStats` batch aggregates the selection's populated cells and replies — a pure
        // read (no publish), tagged with the request's `req_id`.
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "10"),   // A1 number
            set_input(sheet, 1, 0, "text"), // A2 text
            set_input(sheet, 2, 0, "20"),   // A3 number
            set_input(sheet, 0, 1, "999"),  // B1 — outside the queried column
        ]);
        let _ = drain_events(&rx);

        worker.process_batch(vec![Command::SelectionStats {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(2, 0)),
            req_id: 7,
        }]);
        let events = drain_events(&rx);
        let stats = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::SelectionStats { req_id: 7, stats } => Some(*stats),
                _ => None,
            })
            .expect("a SelectionStats batch replies SelectionStats with the same req_id");
        assert_eq!(stats.count, 3, "A1:A3 has three non-empty cells");
        assert_eq!(stats.numeric_count, 2);
        assert_eq!(stats.sum, 30.0);
        assert_eq!(stats.min, Some(10.0));
        assert_eq!(stats.max, Some(20.0));
        // A stats query is a read — it publishes nothing.
        assert!(!events.iter().any(|e| matches!(e, WorkerEvent::Published)));
    }

    #[test]
    fn resolve_edge_command_replies_target() {
        // A `ResolveEdge` batch resolves the ⌘+arrow edge-of-data target and replies — a pure read
        // (no publish), tagged with the request's `req_id`.
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "a"), // A1
            set_input(sheet, 1, 0, "b"), // A2
            set_input(sheet, 2, 0, "c"), // A3 — end of the run
        ]);
        let _ = drain_events(&rx);

        worker.process_batch(vec![Command::ResolveEdge {
            sheet,
            from: CellRef::new(0, 0),
            dir: Direction::Down,
            req_id: 42,
        }]);
        let events = drain_events(&rx);
        let target = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::EdgeResolved { req_id: 42, target } => Some(*target),
                _ => None,
            })
            .expect("a ResolveEdge batch replies EdgeResolved with the same req_id");
        // Down from A1 through the run A1:A3 lands on the run's last cell A3 (row 2).
        assert_eq!(target, CellRef::new(2, 0));
        // A resolve is a read — it publishes nothing.
        assert!(!events.iter().any(|e| matches!(e, WorkerEvent::Published)));
    }

    #[test]
    fn replace_all_command_replaces_reports_count_and_publishes() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "cat"),  // A1
            set_input(sheet, 0, 1, "cats"), // B1
            set_input(sheet, 1, 0, "dog"),  // A2 (no match)
        ]);
        let _ = drain_events(&rx);
        let entries_before = worker.undo_stack.len();

        worker.process_batch(vec![Command::ReplaceAll {
            sheet,
            query: "cat".to_string(),
            replacement: "dog".to_string(),
            match_case: true,
            whole_cell: false,
        }]);
        let events = drain_events(&rx);
        let n = events
            .iter()
            .find_map(|e| match e {
                WorkerEvent::ReplacedCount { n } => Some(*n),
                _ => None,
            })
            .expect("a ReplaceAll batch replies ReplacedCount");
        assert_eq!(n, 2);
        assert!(
            events.iter().any(|e| matches!(e, WorkerEvent::Published)),
            "ReplaceAll republishes"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "dog"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 1)).unwrap(),
            "dogs"
        );
        // Single-undo: the whole batch is one engine undo entry → the unified timeline grows by
        // exactly one `UndoEntry::Cell`, however many cells changed.
        assert_eq!(
            worker.undo_stack.len() - entries_before,
            1,
            "the whole Replace All is a single undo entry"
        );
        assert!(
            matches!(worker.undo_stack.last(), Some(UndoEntry::Cell(_))),
            "ReplaceAll records a Cell entry on the unified undo timeline"
        );
    }

    #[test]
    fn replace_all_is_a_single_undo_step() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "cat"),  // A1
            set_input(sheet, 0, 1, "cats"), // B1
            set_input(sheet, 2, 3, "cat"),  // D3 (scattered, non-contiguous)
            set_input(sheet, 1, 0, "dog"),  // A2 (no match)
        ]);
        let _ = drain_events(&rx);

        worker.process_batch(vec![Command::ReplaceAll {
            sheet,
            query: "cat".to_string(),
            replacement: "dog".to_string(),
            match_case: true,
            whole_cell: false,
        }]);
        let _ = drain_events(&rx);
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "dog"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 1)).unwrap(),
            "dogs"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(2, 3)).unwrap(),
            "dog"
        );

        // A SINGLE Undo reverts every replaced cell (the point of Phase 9).
        worker.process_batch(vec![Command::Undo]);
        let _ = drain_events(&rx);
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "cat"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 1)).unwrap(),
            "cats"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(2, 3)).unwrap(),
            "cat"
        );
        // The unmatched cell is untouched throughout.
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(1, 0)).unwrap(),
            "dog"
        );
    }

    #[test]
    fn replace_one_command_rewrites_single_cell() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 0, 0, "foobar")]);
        let _ = drain_events(&rx);

        worker.process_batch(vec![Command::ReplaceOne {
            sheet,
            cell: CellRef::new(0, 0),
            query: "foo".to_string(),
            replacement: "qux".to_string(),
            match_case: true,
            whole_cell: false,
        }]);
        let events = drain_events(&rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, WorkerEvent::ReplacedCount { n: 1 })));
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "quxbar"
        );
    }

    #[test]
    fn negative_control_eval_counter_detects_no_coalesce() {
        // NEGATIVE CONTROL for the coalescing metric: feed the SAME worker 30 single-edit
        // batches (defeating the drain). The eval counter climbs to 30, proving the `== 1`
        // assertion above is discriminating — the counter is not hard-wired to pass.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        for i in 0..30u32 {
            worker.process_batch(vec![set_input(sheet, 0, 0, &format!("{}", i + 1))]);
        }
        assert_eq!(
            worker.eval_count, 30,
            "un-coalesced edits must each cost an eval (the metric can register failure)"
        );
    }

    #[test]
    fn catch_unwind_recovery_keeps_worker_alive() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);

        quiet_panics(|| worker.process_batch(vec![Command::TestPanic]));

        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::EnginePanic
                }
            )),
            "a caught panic emits EditRejected{{EnginePanic}}; got {events:?}"
        );
        assert!(
            !worker.degraded,
            "one caught panic must not degrade the worker"
        );

        // A subsequent real edit still applies (the worker survived).
        worker.process_batch(vec![set_input(sheet, 0, 0, "7")]);
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "7"
        );
    }

    #[test]
    fn second_panic_degrades_and_refuses_edits() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);

        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded, "a second caught panic degrades the worker");
        let events = drain_events(&rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::WorkerDegraded { .. })),
            "the second panic emits WorkerDegraded; got {events:?}"
        );

        // Edits are now refused …
        worker.process_batch(vec![set_input(sheet, 0, 0, "1")]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects edits; got {events:?}"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "",
            "the refused edit never reached the model"
        );

        // … but a Save still works (the escape hatch).
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("degraded.xlsx");
        worker.process_batch(vec![Command::Save {
            path: path.clone(),
            req_id: 9,
        }]);
        let events = drain_events(&rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::Saved { req_id: 9, .. })),
            "a degraded worker can still Save As; got {events:?}"
        );
        assert!(path.exists());
    }

    #[test]
    fn worker_side_cap_rejects_abort_reproducers_without_touching_engine() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);

        // Round-3 D reproducers + exact boundary cases, all as one drained batch. Both must be
        // rejected by the cap *before* the engine — the flat chain is the canonical D length
        // reproducer (11897 terms ⇒ ~23.8k chars > the 8192 length cap; a shorter chain would
        // be *under* the cap and — correctly — allowed through, so it must not appear here).
        let deep = format!("={}1{}", "(".repeat(490), ")".repeat(490)); // depth 490 > 64
        let flat = {
            let mut f = String::from("=1");
            for _ in 1..11_897 {
                f.push_str("+1");
            }
            f
        }; // ~23.8k chars > 8192
        let over_depth = format!(
            "={}1{}",
            "(".repeat(MAX_NESTING_DEPTH + 1),
            ")".repeat(MAX_NESTING_DEPTH + 1)
        );
        let over_len = format!("={}", "1".repeat(MAX_INPUT_LEN)); // total MAX+1

        worker.process_batch(vec![
            set_input(sheet, 0, 0, &deep),
            set_input(sheet, 1, 0, &flat),
            set_input(sheet, 2, 0, &over_depth),
            set_input(sheet, 3, 0, &over_len),
        ]);

        let rejects = drain_events(&rx)
            .into_iter()
            .filter(|e| matches!(e, WorkerEvent::EditRejected { .. }))
            .count();
        assert_eq!(rejects, 4, "all four abort-class inputs are rejected");
        assert_eq!(worker.eval_count, 0, "nothing was applied → no eval ran");
        // None of the abort-class formulas reached the engine.
        for row in 0..4 {
            assert_eq!(
                worker.doc.formatted_value(0, CellRef::new(row, 0)).unwrap(),
                ""
            );
        }

        // Exactly-at-cap boundaries (depth 64, length 8192) are accepted, applied, evaluated.
        let at_depth = format!(
            "={}1{}",
            "(".repeat(MAX_NESTING_DEPTH),
            ")".repeat(MAX_NESTING_DEPTH)
        );
        assert!(matches!(validate_input(&at_depth), Ok(())));
        worker.process_batch(vec![set_input(sheet, 5, 0, &at_depth)]);
        assert_eq!(
            worker.eval_count, 1,
            "the at-cap formula is accepted and evaluated"
        );
    }

    #[test]
    fn ops_seen_counts_edits_and_undo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 0, 0, "1")]); // ops 1
        worker.process_batch(vec![set_input(sheet, 0, 0, "2")]); // ops 2
        worker.process_batch(vec![Command::Undo]); // ops 3 (undo counts)
        assert_eq!(worker.ops_seen, 3);
        assert_eq!(worker.shared.committed_ops.load(Ordering::Acquire), 3);
        // Undo reverted A1 back to its first value.
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "1"
        );
    }

    #[test]
    fn cap_rejection_is_typed() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let over = format!("={}", "1".repeat(MAX_INPUT_LEN));
        worker.process_batch(vec![set_input(sheet, 0, 0, &over)]);
        let reason = drain_events(&rx).into_iter().find_map(|e| match e {
            WorkerEvent::EditRejected { reason } => Some(reason),
            _ => None,
        });
        assert!(matches!(
            reason,
            Some(EditRejectedReason::InputCap(InputRejection::TooLong { .. }))
        ));
    }

    #[test]
    fn style_toggle_any_lacking_sets_all_then_clears() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        // A1 already bold, A2 plain → the range "lacks" bold somewhere → toggle sets ALL bold.
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "x"),
            set_input(sheet, 1, 0, "y"),
        ]);
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(0, 0)),
            attr: StyleAttr::Bold,
        }]);
        // Now A1 bold, A2 not; toggle over A1:A2 → any-lacking (A2) → set all bold.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            attr: StyleAttr::Bold,
        }]);
        assert!(worker
            .doc
            .font_flag(0, CellRef::new(0, 0), FontFlag::Bold)
            .unwrap());
        assert!(worker
            .doc
            .font_flag(0, CellRef::new(1, 0), FontFlag::Bold)
            .unwrap());
        // Toggle again: all already bold → clear all.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            attr: StyleAttr::Bold,
        }]);
        assert!(!worker
            .doc
            .font_flag(0, CellRef::new(0, 0), FontFlag::Bold)
            .unwrap());
        assert!(!worker
            .doc
            .font_flag(0, CellRef::new(1, 0), FontFlag::Bold)
            .unwrap());
    }

    #[test]
    fn strikethrough_toggle_sets_all_then_clears() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "x"),
            set_input(sheet, 1, 0, "y"),
        ]);
        // A1 strike, A2 plain → range lacks strike somewhere → toggle sets all.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(0, 0)),
            attr: StyleAttr::Strikethrough,
        }]);
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            attr: StyleAttr::Strikethrough,
        }]);
        assert!(worker
            .doc
            .font_flag(0, CellRef::new(0, 0), FontFlag::Strike)
            .unwrap());
        assert!(worker
            .doc
            .font_flag(0, CellRef::new(1, 0), FontFlag::Strike)
            .unwrap());
        // Toggle again: all set → clear all.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            attr: StyleAttr::Strikethrough,
        }]);
        assert!(!worker
            .doc
            .font_flag(0, CellRef::new(0, 0), FontFlag::Strike)
            .unwrap());
        assert!(!worker
            .doc
            .font_flag(0, CellRef::new(1, 0), FontFlag::Strike)
            .unwrap());
    }

    #[test]
    fn wrap_toggle_sets_all_then_clears() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "x"),
            set_input(sheet, 1, 0, "y"),
        ]);
        // A1 wrapped, A2 plain → range lacks wrap somewhere → toggle sets all.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(0, 0)),
            attr: StyleAttr::WrapText,
        }]);
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            attr: StyleAttr::WrapText,
        }]);
        assert!(worker.doc.wrap_flag(0, CellRef::new(0, 0)).unwrap());
        assert!(worker.doc.wrap_flag(0, CellRef::new(1, 0)).unwrap());
        // Toggle again: all wrapped → clear all.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            attr: StyleAttr::WrapText,
        }]);
        assert!(!worker.doc.wrap_flag(0, CellRef::new(0, 0)).unwrap());
        assert!(!worker.doc.wrap_flag(0, CellRef::new(1, 0)).unwrap());
    }

    #[test]
    fn set_style_path_vertical_align_applies() {
        use freecell_core::VAlign;
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "hi"),
        ]);
        drain_events(&rx);

        worker.process_batch(vec![Command::SetStylePath {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            path: StylePath::AlignVertical,
            value: "top".to_string(),
        }]);

        let rs = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(1, 1)
            .copied()
            .unwrap();
        assert_eq!(rs.v_align, Some(VAlign::Top));
        worker_cache_agrees(&worker, sheet, &rows, &cols);

        // A second set to a different value replaces it (a plain set, like horizontal align).
        worker.process_batch(vec![Command::SetStylePath {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            path: StylePath::AlignVertical,
            value: "center".to_string(),
        }]);
        let rs = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(1, 1)
            .copied()
            .unwrap();
        assert_eq!(rs.v_align, Some(VAlign::Center));
        worker_cache_agrees(&worker, sheet, &rows, &cols);
    }

    #[test]
    fn publication_reflects_edits_and_skips_empties() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..3,
            cols: 0..3,
        }]);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "42"),
            set_input(sheet, 2, 2, "=40+2"),
        ]);
        let publication = worker.shared.publication.load_full();
        assert_eq!(
            publication.generation,
            worker.shared.generation.load(Ordering::Acquire)
        );
        // Two non-empty cells; the rest of the 3×3 window is omitted.
        assert_eq!(publication.cells.len(), 2);
        let a1 = publication
            .cells
            .iter()
            .find(|c| c.row == 0 && c.col == 0)
            .unwrap();
        assert_eq!(a1.display_text, "42");
        let c3 = publication
            .cells
            .iter()
            .find(|c| c.row == 2 && c.col == 2)
            .unwrap();
        assert_eq!(c3.display_text, "42"); // =40+2 evaluated
        assert!(publication.covers(1, 1) && !publication.covers(3, 3));
    }

    #[test]
    fn publish_then_bump_generation_ordering() {
        // Every publish stores the Arc before bumping the counter, so the published
        // generation never lags the counter.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..1,
            cols: 0..1,
        }]);
        for i in 0..5u32 {
            worker.process_batch(vec![set_input(sheet, 0, 0, &format!("{i}"))]);
            let gen = worker.shared.generation.load(Ordering::Acquire);
            let pubn = worker.shared.publication.load_full();
            assert_eq!(pubn.generation, gen, "the published gen equals the counter");
        }
    }

    #[test]
    fn style_edit_publishes_but_does_not_recompute() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..2,
            cols: 0..2,
        }]);
        worker.process_batch(vec![set_input(sheet, 0, 0, "5")]); // eval #1
        let evals_before = worker.eval_count;
        let gen_before = worker.shared.generation.load(Ordering::Acquire);

        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(0, 0)),
            attr: StyleAttr::Bold,
        }]);

        assert_eq!(
            worker.eval_count, evals_before,
            "a style edit needs no recompute (styles don't affect values)"
        );
        assert!(
            worker.shared.generation.load(Ordering::Acquire) > gen_before,
            "but it still publishes (a repaint; P5 ships cache deltas)"
        );
        assert_eq!(worker.ops_seen, 2, "the style edit is a committed op");
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "5",
            "the value is unchanged"
        );
    }

    /// Assert the resident cache for `sheet` agrees with a fresh engine re-read over the probe
    /// grid — the worker-level agreement contract (reads through the real `shared.caches`).
    fn worker_cache_agrees(worker: &Worker, sheet: SheetId, rows: &[u32], cols: &[u32]) {
        let idx = worker.resolve(sheet).expect("sheet resolves");
        let caches = worker.shared.caches.read();
        let cache = caches.get(sheet).expect("sheet cache is resident");
        cache::assert_cache_agrees(&worker.doc, cache, idx, rows, cols)
            .expect("cache must agree with a fresh engine re-read");
    }

    fn small_probes() -> (Vec<u32>, Vec<u32>) {
        ((0..6).collect(), (0..6).collect())
    }

    #[test]
    fn load_builds_active_sheet_cache() {
        // `test_worker` mirrors `load_and_run`: the active sheet's cache is resident immediately.
        let (worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        assert!(worker.shared.caches.read().contains(sheet));
        let (rows, cols) = small_probes();
        worker_cache_agrees(&worker, sheet, &rows, &cols);
    }

    #[test]
    fn style_edit_mirrors_cache_and_emits_stylecacheupdated() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "x"),
        ]);
        drain_events(&rx);

        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            attr: StyleAttr::Bold,
        }]);

        // The cache now shows the cell bold, agrees with the engine, and a delta was emitted.
        let bold = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(1, 1)
            .copied();
        assert_eq!(bold.map(|s| s.bold), Some(true));
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        assert!(
            drain_events(&rx)
                .iter()
                .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)),
            "a style edit ships a StyleCacheUpdated delta"
        );
    }

    #[test]
    fn set_style_path_num_fmt_applies_and_cache_reflects() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "1234.5"),
        ]);
        drain_events(&rx);

        worker.process_batch(vec![Command::SetStylePath {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            path: StylePath::NumFmt,
            value: "$#,##0.00".to_string(),
        }]);

        // The cache's num-fmt side table now resolves the cell to the Currency code.
        {
            let guard = worker.shared.caches.read();
            let cache = guard.get(sheet).unwrap();
            let rs = cache
                .render_style(1, 1)
                .copied()
                .expect("format-only cell stored");
            assert_eq!(cache.num_fmt_code(rs.num_fmt), "$#,##0.00");
        }
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        // Style-only: it ships a StyleCacheUpdated delta (the coalesced `evaluate()` is skipped
        // for a format change, verified structurally by `AppliedKind::StyleOnly`; the spinner's
        // EvalStarted still fires for the batch, as with any other style edit).
        assert!(drain_events(&rx)
            .iter()
            .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)));
    }

    #[test]
    fn set_style_path_align_and_color_apply() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "hi"),
        ]);
        drain_events(&rx);

        worker.process_batch(vec![
            Command::SetStylePath {
                sheet,
                range: CellRange::single(CellRef::new(1, 1)),
                path: StylePath::AlignHorizontal,
                value: "right".to_string(),
            },
            Command::SetStylePath {
                sheet,
                range: CellRange::single(CellRef::new(1, 1)),
                path: StylePath::FontColor,
                value: "#FF0000".to_string(),
            },
        ]);

        let rs = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(1, 1)
            .copied()
            .unwrap();
        assert_eq!(rs.h_align, Some(freecell_core::Align::Right));
        assert_eq!(rs.font_color, Some(Rgb::from_hex(0xFF0000)));
        worker_cache_agrees(&worker, sheet, &rows, &cols);

        // Re-pressing the alignment clears horizontal only (value "general") → back to default.
        worker.process_batch(vec![Command::SetStylePath {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            path: StylePath::AlignHorizontal,
            value: "general".to_string(),
        }]);
        let rs = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(1, 1)
            .copied()
            .unwrap();
        assert_eq!(
            rs.h_align, None,
            "general clears the explicit horizontal alignment"
        );
        assert_eq!(
            rs.font_color,
            Some(Rgb::from_hex(0xFF0000)),
            "color is untouched"
        );
        worker_cache_agrees(&worker, sheet, &rows, &cols);
    }

    #[test]
    fn set_borders_applies_and_undo() {
        use crate::worker::protocol::{BorderLine, BorderPreset};
        use freecell_core::BorderSpec;
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 1, 1, "x"),
        ]);
        drain_events(&rx);

        // Apply "All" over a bounded 2×2 block B2:C3 (one undoable diff-list).
        worker.process_batch(vec![Command::SetBorders {
            sheet,
            range: CellRange::new(CellRef::new(1, 1), CellRef::new(2, 2)),
            preset: BorderPreset::All,
            line: BorderLine::ThinSolid,
            color: None,
        }]);

        // B2 now carries all four thin edges, and the cache agrees with a fresh engine re-read
        // (which also validates the adjacent-strip fix-up refresh via the expanded range).
        {
            let guard = worker.shared.caches.read();
            let cache = guard.get(sheet).unwrap();
            let rs = cache
                .render_style(1, 1)
                .copied()
                .expect("bordered cell stored");
            let spec = cache.border_spec(rs.border);
            assert!(spec.top.is_some() && spec.right.is_some() && spec.bottom.is_some());
        }
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        assert!(
            drain_events(&rx)
                .iter()
                .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)),
            "a border edit ships a StyleCacheUpdated delta"
        );

        // One undo step reverts the whole border edit → B2 has no border again.
        worker.process_batch(vec![Command::Undo]);
        {
            let guard = worker.shared.caches.read();
            let cache = guard.get(sheet).unwrap();
            let border = cache
                .render_style(1, 1)
                .map(|rs| cache.border_spec(rs.border))
                .unwrap_or(BorderSpec::NONE);
            assert!(border.is_none(), "undo reverts the border to NONE");
        }
        worker_cache_agrees(&worker, sheet, &rows, &cols);
    }

    #[test]
    fn set_borders_carries_line_style_and_color_into_cache() {
        use crate::worker::protocol::{BorderLine, BorderPreset};
        use freecell_core::{LinePattern, Rgb};
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);

        // Paint an "All" dashed red border over B2 — the pen's line + color must survive into the
        // resolved render `Edge` (dashed pattern, medium weight, red).
        worker.process_batch(vec![Command::SetBorders {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            preset: BorderPreset::All,
            line: BorderLine::Dashed,
            color: Some(Rgb::new(0xFF, 0, 0)),
        }]);

        let guard = worker.shared.caches.read();
        let cache = guard.get(sheet).unwrap();
        let rs = cache.render_style(1, 1).copied().expect("bordered cell");
        let top = cache.border_spec(rs.border).top.expect("top edge");
        assert_eq!(top.pattern, LinePattern::Dashed, "dashed line resolves");
        assert_eq!(top.weight, 2, "mediumdashed is weight-2");
        assert_eq!(top.color, Rgb::new(0xFF, 0, 0), "pen colour resolves");
    }

    #[test]
    fn set_borders_double_line_round_trips_into_cache() {
        use crate::worker::protocol::{BorderLine, BorderPreset};
        use freecell_core::LinePattern;
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);

        // Paint an "All" double border over B2 — "double" must round-trip through IronCalc into a
        // resolved `Edge` with pattern Double and weight 3.
        worker.process_batch(vec![Command::SetBorders {
            sheet,
            range: CellRange::single(CellRef::new(1, 1)),
            preset: BorderPreset::All,
            line: BorderLine::Double,
            color: None,
        }]);

        let guard = worker.shared.caches.read();
        let cache = guard.get(sheet).unwrap();
        let rs = cache.render_style(1, 1).copied().expect("bordered cell");
        let top = cache.border_spec(rs.border).top.expect("top edge");
        assert_eq!(top.pattern, LinePattern::Double, "double line resolves");
        assert_eq!(top.weight, 3, "double is weight-3");
    }

    #[test]
    fn set_borders_full_column_is_band() {
        use crate::worker::protocol::{BorderLine, BorderPreset};
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);
        // Full-column "All" over column D → `set_area_with_border` routes to a column band; the
        // mirror must full-rebuild (band-creating, even after the +1 expansion) rather than
        // materialize 1M cells.
        worker.process_batch(vec![Command::SetBorders {
            sheet,
            range: CellRange::new(CellRef::new(0, 3), CellRef::new(limits::MAX_ROWS - 1, 3)),
            preset: BorderPreset::All,
            line: BorderLine::ThinSolid,
            color: None,
        }]);

        let (rows, cols) = wide_probes();
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        // A far, empty cell on the banded column resolves to the border (the band), not default.
        let spec = {
            let guard = worker.shared.caches.read();
            let cache = guard.get(sheet).unwrap();
            cache
                .render_style(5000, 3)
                .map(|rs| cache.border_spec(rs.border))
        };
        assert!(
            matches!(spec, Some(s) if !s.is_none()),
            "a far cell on the banded column carries the border"
        );
    }

    /// Send a `SetFont` and drain — the font op runs standalone after any edit batch.
    fn set_font(
        sheet: SheetId,
        range: CellRange,
        family: Option<&str>,
        size_pt: Option<f64>,
    ) -> Command {
        Command::SetFont {
            sheet,
            range,
            family: family.map(str::to_string),
            size_pt,
        }
    }

    #[test]
    fn set_font_grows_rows_and_reflects_cache() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "Big"),
        ]);
        let before_h = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .row_height(1);
        drain_events(&rx);

        worker.process_batch(vec![set_font(
            sheet,
            CellRange::single(CellRef::new(1, 1)),
            None,
            Some(24.0),
        )]);

        // The cache reflects the 24pt size (96 quarter-points) and the row grew.
        {
            let guard = worker.shared.caches.read();
            let cache = guard.get(sheet).unwrap();
            let rs = cache.render_style(1, 1).copied().expect("font cell stored");
            assert_eq!(rs.font_size_q, 96);
            assert!(
                cache.row_height(1) > before_h,
                "row 1 auto-grew for the larger font ({} → {})",
                before_h,
                cache.row_height(1)
            );
        }
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        // Style-only: no evaluate; a StyleCacheUpdated delta ships.
        assert!(drain_events(&rx)
            .iter()
            .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)));
    }

    #[test]
    fn set_font_undo_reverts_size_and_height() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "x"),
        ]);
        let base_h = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .row_height(1);
        // `ops_seen` is a monotonic dirty counter (undo increments it too), so capture how many
        // diff-lists the SetFont committed and undo exactly that many (K+1 = style + row runs).
        let ops_before = worker.ops_seen; // = 1 (the input)
        worker.process_batch(vec![set_font(
            sheet,
            CellRange::single(CellRef::new(1, 1)),
            Some("Arial"),
            Some(28.0),
        )]);
        drain_events(&rx);
        let font_diff_lists = worker.ops_seen - ops_before;
        assert!(
            font_diff_lists >= 2,
            "SetFont committed a style + a height diff-list, got {font_diff_lists}"
        );

        // Undo every committed diff-list; the cache re-reads and agrees with the engine each step.
        for _ in 0..font_diff_lists {
            worker.process_batch(vec![Command::Undo]);
            worker_cache_agrees(&worker, sheet, &rows, &cols);
        }
        let guard = worker.shared.caches.read();
        let cache = guard.get(sheet).unwrap();
        assert_eq!(
            cache.render_style(1, 1).map(|s| s.font_size_q).unwrap_or(0),
            0,
            "undo restored the default size"
        );
        assert!(
            (cache.row_height(1) - base_h).abs() < 1e-3,
            "undo restored the original row height"
        );
    }

    #[test]
    fn set_font_full_column_clamps_to_used() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "1"),
            set_input(sheet, 2, 0, "2"),
        ]);
        // A full-column SetFont clamps to the used rows (does NOT materialise 1M cells).
        let full_col = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(freecell_core::limits::MAX_ROWS - 1, 0),
        );
        worker.process_batch(vec![set_font(sheet, full_col, Some("Arial"), None)]);
        let guard = worker.shared.caches.read();
        let cache = guard.get(sheet).unwrap();
        assert_eq!(
            cache.font_family_name(cache.render_style(0, 0).unwrap().font_family),
            "Arial"
        );
        assert_eq!(
            cache.font_family_name(cache.render_style(2, 0).unwrap().font_family),
            "Arial"
        );
        // A row past the used range was not materialised.
        assert!(cache.render_style(100, 0).is_none());
    }

    #[test]
    fn set_font_too_large_selection_rejects() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Populate a corner so the used range spans a huge rectangle (>100k cells).
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "a"),
            set_input(sheet, 999, 199, "b"), // used range 1000 × 200 = 200k cells
        ]);
        drain_events(&rx);
        // Select-all clamps to the used rectangle (1000 × 200 = 200k cells) → over the 100k cap.
        let select_all = CellRange::new(
            CellRef::new(0, 0),
            CellRef::new(
                freecell_core::limits::MAX_ROWS - 1,
                freecell_core::limits::MAX_COLS - 1,
            ),
        );
        worker.process_batch(vec![set_font(sheet, select_all, None, Some(20.0))]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(m)
                } if m.contains("too large")
            )),
            "a >100k clamped font selection is rejected with a dialog-worthy message"
        );
    }

    #[test]
    fn set_font_degraded_rejected() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);
        worker.process_batch(vec![set_font(
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            Some("Arial"),
            Some(20.0),
        )]);
        assert!(drain_events(&rx).iter().any(|e| matches!(
            e,
            WorkerEvent::EditRejected {
                reason: EditRejectedReason::Degraded
            }
        )));
    }

    /// A default authored-chart anchor (8 cols × 15 rows from A1), matching the chrome's insert.
    fn test_anchor() -> Anchor {
        Anchor::new(
            freecell_chart_model::AnchorCell::new(0, 0),
            freecell_chart_model::AnchorCell::new(8, 15),
        )
    }

    #[test]
    fn insert_chart_publishes_authored_snapshot() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let before = worker.chart_version;
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);

        // The insert holds one authored chart, bumps the version, and publishes.
        assert_eq!(worker.authored_charts.len(), 1);
        assert!(
            worker.chart_version > before,
            "the insert bumps the chart version"
        );
        assert!(drain_events(&rx)
            .iter()
            .any(|e| matches!(e, WorkerEvent::Published)));

        // The published snapshot carries the authored (snapshot-but-not-live) line chart.
        let snap = worker.shared.chart_snapshot.load_full();
        let (snap_sheet, specs) = &snap.sheets[0];
        assert_eq!(*snap_sheet, sheet);
        assert_eq!(specs.len(), 1);
        assert!(
            specs[0].is_authored(),
            "the inserted chart is Authored (no retained source, no live binding)"
        );
        assert!(matches!(
            specs[0].chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        ));
        // Dirty tracking: one committed op is recorded so the chart can be saved.
        assert_eq!(worker.shared.committed_ops.load(Ordering::Acquire), 1);
    }

    /// Batch 3 item 8: inserting a chart with a `data` range binds it **at creation** — the published
    /// chart is born LIVE (real `c:f` refs + values resolved from the current cells), exactly as if a
    /// `SetChartRange` had followed the insert, but in one op. A `data: None` insert (asserted by
    /// `insert_chart_publishes_authored_snapshot`) stays the near-empty placeholder.
    #[test]
    fn insert_chart_with_data_binds_range_at_creation() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet); // A1:B3 = Widgets / Q1,Q2 / 10,20
        let before = worker.chart_version;
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: Some(CellRange::from_a1("A1:B3").unwrap()),
        }]);

        // Exactly one authored chart, bound at creation (non-empty refs) — no follow-up SetChartRange.
        assert_eq!(worker.authored_charts.len(), 1);
        let entry = &worker.authored_charts[0];
        assert!(
            !entry.refs.is_empty(),
            "inserting with a data range binds `c:f` refs at creation"
        );
        assert!(
            entry
                .spec
                .source_ranges
                .iter()
                .any(|r| r.as_str().contains("$B$2:$B$3")),
            "the value range is published on the spec (the chart is LIVE, not near-empty)"
        );
        // The series resolved LIVE from B2:B3 (10, 20), not the placeholder literals.
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
        assert_eq!(
            entry.spec.chart().unwrap().series[0].name.as_deref(),
            Some("Widgets"),
            "the series name resolved from B1 at creation"
        );
        assert!(
            worker.chart_version > before,
            "the create-and-bind publishes once"
        );
    }

    /// Criterion #3: a degraded worker MUST reject `InsertChart` (consistent with the edit batch /
    /// paste / SetFont), pushing no authored chart and bumping no version.
    #[test]
    fn insert_chart_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);
        let version_before = worker.chart_version;

        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);

        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects InsertChart"
        );
        assert!(
            worker.authored_charts.is_empty(),
            "no authored chart is held when degraded"
        );
        assert_eq!(
            worker.chart_version, version_before,
            "no publish / version bump when degraded"
        );
    }

    /// P18: `SetChartAnchor` moves/resizes an authored chart's model anchor + bumps the version.
    #[test]
    fn set_chart_anchor_updates_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let version_before = worker.chart_version;
        let moved = Anchor::new(
            freecell_chart_model::AnchorCell::new(3, 3),
            freecell_chart_model::AnchorCell::new(11, 18),
        );
        worker.process_batch(vec![Command::SetChartAnchor {
            sheet,
            id,
            anchor: moved,
        }]);
        assert_eq!(worker.authored_charts[0].spec.anchor, moved);
        assert!(worker.chart_version > version_before, "a move republishes");
    }

    /// P18: `DeleteChart` removes the named authored chart (leaving the rest).
    #[test]
    fn delete_chart_removes_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::InsertChart {
                sheet,
                kind: ChartInsertKind::Line,
                anchor: test_anchor(),
                data: None,
            },
            Command::InsertChart {
                sheet,
                kind: ChartInsertKind::Bar,
                anchor: test_anchor(),
                data: None,
            },
        ]);
        assert_eq!(worker.authored_charts.len(), 2);
        let first = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::DeleteChart { sheet, id: first }]);
        assert_eq!(worker.authored_charts.len(), 1);
        assert_ne!(
            worker.authored_charts[0].id, first,
            "the other chart survives"
        );
    }

    /// P18 degraded guard: a degraded worker rejects `SetChartAnchor` + `DeleteChart` (like insert).
    #[test]
    fn set_anchor_and_delete_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Insert a chart BEFORE degrading (so there's an id), then degrade.
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![
            Command::SetChartAnchor {
                sheet,
                id,
                anchor: test_anchor(),
            },
            Command::DeleteChart { sheet, id },
        ]);
        let rejects = drain_events(&rx)
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    WorkerEvent::EditRejected {
                        reason: EditRejectedReason::Degraded
                    }
                )
            })
            .count();
        assert_eq!(rejects, 2, "both chart ops are rejected when degraded");
        // The chart is untouched (still present).
        assert_eq!(worker.authored_charts.len(), 1);
    }

    /// Unified undo timeline (charts feedback item 4): with a chart present but the **cell edit** the
    /// most-recent action, Ctrl+Z targets the cell (not the chart), and the chart is left untouched.
    /// The `Cell` entries stay 1:1 with IronCalc's stack even with a `Chart` entry beneath them, so a
    /// chart's presence never breaks cell undo/redo.
    #[test]
    fn cell_undo_redo_correct_with_authored_chart_present() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let cell00 = |w: &Worker| w.doc.formatted_value(0, CellRef::new(0, 0)).unwrap();
        worker.process_batch(vec![set_input(sheet, 0, 0, "hello")]);
        assert_eq!(cell00(&worker), "hello");
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(cell00(&worker), "", "cell undo works with a chart present");
        worker.process_batch(vec![Command::Redo]);
        assert_eq!(cell00(&worker), "hello", "cell redo works too");
        // The authored chart is untouched by the cell undo/redo.
        assert_eq!(worker.authored_charts.len(), 1);
    }

    // --- Charts feedback item 4: chart ops on the unified undo timeline ----------------------

    /// Build + install a single **loaded** chart on `sheet` with the given `chart_part`, returning
    /// its assigned [`ChartId`]. Mirrors a discovered file-loaded chart so its delete/anchor undo can
    /// be exercised at the worker level (no real xlsx needed).
    fn install_loaded_chart(worker: &mut Worker, sheet: SheetId, chart_part: &str) -> ChartId {
        use freecell_chart_model::{
            Axis, Category, Chart, ChartKind, Grouping, Legend, Series, SourceXml,
        };
        let chart = Chart {
            title: None,
            kind: ChartKind::Line {
                grouping: Grouping::Standard,
                smooth: false,
            },
            series: vec![Series::category_value(
                Some("Loaded"),
                vec![Category::Text("Q1".into())],
                vec![1.0],
            )],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: Some(Legend::default()),
        };
        let spec = ChartSpec::loaded(
            chart,
            SourceXml::new("<c:chartSpace/>"),
            Vec::new(),
            test_anchor(),
        );
        worker.charts =
            ChartBindings::from_specs_by_sheet(vec![(sheet, vec![(chart_part.to_string(), spec)])]);
        let id = ChartId(worker.next_chart_id);
        worker.charts.assign_missing_ids(&mut worker.next_chart_id);
        id
    }

    /// Delete an authored chart → Undo restores it (same id/kind/anchor) → Redo removes it again.
    #[test]
    fn delete_authored_chart_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let anchor = worker.authored_charts[0].spec.anchor;
        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        assert!(
            worker.authored_charts.is_empty(),
            "delete removes the chart"
        );

        worker.process_batch(vec![Command::Undo]);
        assert_eq!(worker.authored_charts.len(), 1, "undo brings it back");
        assert_eq!(worker.authored_charts[0].id, id, "same id restored");
        assert_eq!(worker.authored_charts[0].spec.anchor, anchor, "same anchor");
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        ));

        worker.process_batch(vec![Command::Redo]);
        assert!(worker.authored_charts.is_empty(), "redo re-deletes it");
    }

    /// Delete a **loaded** chart → Undo re-binds it AND drops its part from `loaded_deletes` → Redo
    /// re-deletes it AND restores the part to `loaded_deletes` (so a later save writes the right set).
    #[test]
    fn delete_loaded_chart_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let part = "xl/charts/chart1.xml";
        let id = install_loaded_chart(&mut worker, sheet, part);
        assert!(!worker.charts.is_empty());

        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        assert!(worker.charts.is_empty(), "delete unbinds the loaded chart");
        assert!(
            worker.loaded_deletes.contains(part),
            "delete records the part in loaded_deletes"
        );

        worker.process_batch(vec![Command::Undo]);
        assert!(!worker.charts.is_empty(), "undo re-binds the loaded chart");
        assert_eq!(
            worker.charts.anchor_by_id(id),
            Some(test_anchor()),
            "the re-bound chart keeps its id + anchor"
        );
        assert!(
            !worker.loaded_deletes.contains(part),
            "undo clears the loaded-delete record"
        );

        worker.process_batch(vec![Command::Redo]);
        assert!(worker.charts.is_empty(), "redo re-deletes it");
        assert!(
            worker.loaded_deletes.contains(part),
            "redo restores the loaded-delete record"
        );
    }

    /// Insert a chart → Undo removes it → Redo re-inserts it.
    #[test]
    fn insert_chart_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Bar,
            anchor: test_anchor(),
            data: None,
        }]);
        assert_eq!(worker.authored_charts.len(), 1);

        worker.process_batch(vec![Command::Undo]);
        assert!(worker.authored_charts.is_empty(), "undo removes the insert");

        worker.process_batch(vec![Command::Redo]);
        assert_eq!(worker.authored_charts.len(), 1, "redo re-inserts it");
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Bar { .. }
        ));
    }

    /// SetAnchor (authored) → Undo restores the prior anchor → Redo re-applies the new one.
    #[test]
    fn set_anchor_authored_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let prior = worker.authored_charts[0].spec.anchor;
        let moved = Anchor::new(
            freecell_chart_model::AnchorCell::new(3, 3),
            freecell_chart_model::AnchorCell::new(11, 18),
        );
        worker.process_batch(vec![Command::SetChartAnchor {
            sheet,
            id,
            anchor: moved,
        }]);
        assert_eq!(worker.authored_charts[0].spec.anchor, moved);

        worker.process_batch(vec![Command::Undo]);
        assert_eq!(
            worker.authored_charts[0].spec.anchor, prior,
            "undo restores"
        );

        worker.process_batch(vec![Command::Redo]);
        assert_eq!(
            worker.authored_charts[0].spec.anchor, moved,
            "redo re-applies"
        );
    }

    /// SetAnchor (loaded) → Undo restores the prior render anchor AND clears the `loaded_anchor_edits`
    /// entry the move added → Redo re-applies both.
    #[test]
    fn set_anchor_loaded_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let part = "xl/charts/chart1.xml";
        let id = install_loaded_chart(&mut worker, sheet, part);
        let moved = Anchor::new(
            freecell_chart_model::AnchorCell::new(3, 3),
            freecell_chart_model::AnchorCell::new(11, 18),
        );
        worker.process_batch(vec![Command::SetChartAnchor {
            sheet,
            id,
            anchor: moved,
        }]);
        assert_eq!(worker.charts.anchor_by_id(id), Some(moved));
        assert_eq!(worker.loaded_anchor_edits.get(part), Some(&moved));

        worker.process_batch(vec![Command::Undo]);
        assert_eq!(
            worker.charts.anchor_by_id(id),
            Some(test_anchor()),
            "undo restores the render anchor"
        );
        assert!(
            !worker.loaded_anchor_edits.contains_key(part),
            "undo clears the anchor-edit the move added"
        );

        worker.process_batch(vec![Command::Redo]);
        assert_eq!(worker.charts.anchor_by_id(id), Some(moved));
        assert_eq!(worker.loaded_anchor_edits.get(part), Some(&moved));
    }

    /// SetRange (authored, born-live) → Undo restores the pre-bind (near-empty) state → Redo re-binds.
    #[test]
    fn set_range_authored_undo_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet); // A1:B3 = Widgets / Q1,Q2 / 10,20
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        assert!(
            worker.authored_charts[0].refs.is_empty(),
            "inserted near-empty (no range yet)"
        );
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        assert!(
            !worker.authored_charts[0].refs.is_empty(),
            "range bound it live"
        );
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);

        worker.process_batch(vec![Command::Undo]);
        assert!(
            worker.authored_charts[0].refs.is_empty(),
            "undo restores the pre-bind (unbound) state"
        );

        worker.process_batch(vec![Command::Redo]);
        assert!(
            !worker.authored_charts[0].refs.is_empty(),
            "redo re-binds the range"
        );
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
    }

    /// The key correctness test: an INTERLEAVE of cell edit + chart ops undoes/redoes in exact
    /// most-recent-first order across both op families (cellEdit → InsertChart → DeleteChart, then
    /// Undo×3 restores-then-removes-the-chart-then-undoes-the-cell, and Redo×3 replays forward).
    #[test]
    fn interleaved_cell_and_chart_undo_redo_ordering() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let cell00 = |w: &Worker| w.doc.formatted_value(0, CellRef::new(0, 0)).unwrap();

        worker.process_batch(vec![set_input(sheet, 0, 0, "hello")]);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        assert!(worker.authored_charts.is_empty());
        assert_eq!(cell00(&worker), "hello");

        // Undo #1 → most recent = DeleteChart → the chart comes back.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(worker.authored_charts.len(), 1, "undo1 restores the chart");
        assert_eq!(cell00(&worker), "hello", "the cell edit is untouched");
        // Undo #2 → InsertChart → the chart goes away (NOT the cell edit).
        worker.process_batch(vec![Command::Undo]);
        assert!(worker.authored_charts.is_empty(), "undo2 removes the chart");
        assert_eq!(cell00(&worker), "hello", "the cell edit is still untouched");
        // Undo #3 → the cell edit.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(cell00(&worker), "", "undo3 undoes the cell edit");

        // Redo replays forward: cell → insert → delete.
        worker.process_batch(vec![Command::Redo]);
        assert_eq!(cell00(&worker), "hello", "redo1 re-applies the cell edit");
        assert!(
            worker.authored_charts.is_empty(),
            "chart still gone after redo1"
        );
        worker.process_batch(vec![Command::Redo]);
        assert_eq!(
            worker.authored_charts.len(),
            1,
            "redo2 re-inserts the chart"
        );
        worker.process_batch(vec![Command::Redo]);
        assert!(
            worker.authored_charts.is_empty(),
            "redo3 re-deletes the chart"
        );
    }

    /// A new action after an undo clears the redo stack: chart op → Undo → cell edit → the pending
    /// chart redo is discarded, so Redo is a no-op (the chart is not resurrected).
    #[test]
    fn new_action_after_undo_clears_chart_redo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        worker.process_batch(vec![Command::Undo]);
        assert!(worker.authored_charts.is_empty());
        assert_eq!(worker.redo_stack.len(), 1, "the insert is redoable");

        // A new (cell) action must invalidate the pending chart redo.
        worker.process_batch(vec![set_input(sheet, 0, 0, "x")]);
        assert!(worker.redo_stack.is_empty(), "a new action clears redo");

        worker.process_batch(vec![Command::Redo]);
        assert!(
            worker.authored_charts.is_empty(),
            "redo is a no-op — the chart is not resurrected"
        );
    }

    /// A degraded worker refuses a chart Undo/Redo (like every mutating op), leaving the timeline +
    /// chart state untouched.
    #[test]
    fn chart_undo_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::DeleteChart { sheet, id }]);
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![Command::Undo]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects a chart undo"
        );
        assert!(
            worker.authored_charts.is_empty(),
            "the delete is not undone while degraded"
        );
    }

    // --- P19: edit panel range/type ---------------------------------------------------------

    /// A small data grid an authored chart can bind to: B1 header, A2:A3 categories, B2:B3 values.
    fn seed_chart_data(worker: &mut Worker, sheet: SheetId) {
        worker.process_batch(vec![
            set_input(sheet, 0, 1, "Widgets"), // B1 (series name)
            set_input(sheet, 1, 0, "Q1"),      // A2 (category)
            set_input(sheet, 2, 0, "Q2"),      // A3
            set_input(sheet, 1, 1, "10"),      // B2 (value)
            set_input(sheet, 2, 1, "20"),      // B3
        ]);
    }

    fn first_series_values(worker: &Worker, chart_idx: usize) -> Vec<f64> {
        match &worker.authored_charts[chart_idx]
            .spec
            .chart()
            .unwrap()
            .series[0]
            .data
        {
            freecell_chart_model::SeriesData::CategoryValue { values, .. } => values.clone(),
            freecell_chart_model::SeriesData::Xy { y, .. } => y.clone(),
        }
    }

    /// P19: setting a data range binds an authored chart to real cells — its published spec gains
    /// `source_ranges` (`c:f`) AND its values re-resolve LIVE from the current cells (not the
    /// placeholder literals).
    #[test]
    fn set_chart_range_binds_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let version_before = worker.chart_version;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);

        let entry = &worker.authored_charts[0];
        assert!(!entry.refs.is_empty(), "the range binds `c:f` refs");
        assert!(
            entry
                .spec
                .source_ranges
                .iter()
                .any(|r| r.as_str().contains("$B$2:$B$3")),
            "the value range is published on the spec"
        );
        // The first series re-resolved from B2:B3 (10, 20), replacing the (4,6,5,8) placeholder.
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
        // And its category + name came from the cells.
        match &entry.spec.chart().unwrap().series[0].data {
            freecell_chart_model::SeriesData::CategoryValue { categories, .. } => {
                assert_eq!(
                    categories[0],
                    freecell_chart_model::Category::Text("Q1".into())
                );
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        assert_eq!(
            worker.authored_charts[0].spec.chart().unwrap().series[0]
                .name
                .as_deref(),
            Some("Widgets"),
        );
        assert!(
            worker.chart_version > version_before,
            "the range republishes"
        );
    }

    /// P19: once ranged, an authored chart re-resolves on a source-cell edit — it rides the SAME
    /// dirty-set/publish path as a loaded chart, even though the workbook has no loaded charts.
    #[test]
    fn edit_reresolves_ranged_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);

        let version_before = worker.chart_version;
        worker.process_batch(vec![set_input(sheet, 1, 1, "999")]); // edit B2
        assert_eq!(
            first_series_values(&worker, 0),
            vec![999.0, 20.0],
            "editing a bound cell re-resolves the authored chart"
        );
        assert!(
            worker.chart_version > version_before,
            "the live re-resolve bumped the chart version"
        );
    }

    /// A disjoint edit (outside every bound authored range) does NOT re-resolve the chart — the
    /// authored dirty-set intersection is honored just like the loaded one.
    #[test]
    fn disjoint_edit_leaves_ranged_authored_chart_untouched() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        let version_before = worker.chart_version;
        worker.process_batch(vec![set_input(sheet, 20, 20, "42")]); // far outside A1:B3
        assert_eq!(
            worker.chart_version, version_before,
            "a disjoint edit re-resolves nothing"
        );
    }

    /// P19: switching an authored chart's type rebuilds it to the new kind, preserving the title.
    #[test]
    fn set_chart_type_switches_kind_and_preserves_title() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let version_before = worker.chart_version;
        worker.process_batch(vec![Command::SetChartType {
            sheet,
            id,
            kind: ChartInsertKind::Column,
        }]);
        let chart = worker.authored_charts[0].spec.chart().unwrap();
        assert!(
            matches!(
                chart.kind,
                freecell_chart_model::ChartKind::Bar {
                    dir: freecell_chart_model::BarDir::Col,
                    ..
                }
            ),
            "the chart is now a column chart"
        );
        assert_eq!(
            chart.title.as_deref(),
            Some("Chart"),
            "the title is preserved across a retype"
        );
        assert!(
            worker.chart_version > version_before,
            "a retype republishes"
        );
    }

    /// P19: retyping a chart that already has a data range keeps the range binding (its `c:f` refs +
    /// live values) — only the kind changes.
    #[test]
    fn set_chart_type_preserves_the_range_binding() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        seed_chart_data(&mut worker, sheet);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        worker.process_batch(vec![Command::SetChartRange {
            sheet,
            id,
            data: CellRange::from_a1("A1:B3").unwrap(),
        }]);
        worker.process_batch(vec![Command::SetChartType {
            sheet,
            id,
            kind: ChartInsertKind::Column,
        }]);
        // Still bound to the same cells → still the live values, now on a column chart.
        assert!(!worker.authored_charts[0].refs.is_empty(), "refs preserved");
        assert_eq!(first_series_values(&worker, 0), vec![10.0, 20.0]);
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Bar { .. }
        ));
    }

    /// P19 degraded guard: a degraded worker rejects `SetChartRange` + `SetChartType` (like every
    /// mutating op), leaving the chart untouched.
    #[test]
    fn set_chart_range_and_type_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![
            Command::SetChartRange {
                sheet,
                id,
                data: CellRange::from_a1("A1:B3").unwrap(),
            },
            Command::SetChartType {
                sheet,
                id,
                kind: ChartInsertKind::Bar,
            },
        ]);
        let rejects = drain_events(&rx)
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    WorkerEvent::EditRejected {
                        reason: EditRejectedReason::Degraded
                    }
                )
            })
            .count();
        assert_eq!(rejects, 2, "both chart edits are rejected when degraded");
        // Untouched: still an unbound line chart.
        assert!(worker.authored_charts[0].refs.is_empty());
        assert!(matches!(
            worker.authored_charts[0].spec.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        ));
    }

    // --- P20: chrome editing ----------------------------------------------------------------

    /// The published authored chart's typed [`Chart`] (for chrome assertions).
    fn authored_chart(worker: &Worker, idx: usize) -> Chart {
        worker.authored_charts[idx].spec.chart().unwrap().clone()
    }

    fn chrome(sheet: SheetId, id: ChartId, edit: ChartChromeEdit) -> Command {
        Command::SetChartChrome { sheet, id, edit }
    }

    /// P20: each chrome edit mutates an **authored** chart's model + republishes.
    #[test]
    fn set_chart_chrome_edits_an_authored_chart() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;

        // Title.
        let v0 = worker.chart_version;
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::Title(Some("Sales".into())),
        )]);
        assert_eq!(authored_chart(&worker, 0).title.as_deref(), Some("Sales"));
        assert!(worker.chart_version > v0, "a chrome edit republishes");

        // Legend off, then on-at-bottom.
        worker.process_batch(vec![chrome(sheet, id, ChartChromeEdit::Legend(None))]);
        assert_eq!(authored_chart(&worker, 0).legend, None);
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::Legend(Some(freecell_chart_model::LegendPosition::Bottom)),
        )]);
        assert_eq!(
            authored_chart(&worker, 0).legend.map(|l| l.position),
            Some(freecell_chart_model::LegendPosition::Bottom)
        );

        // Axis titles.
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::AxisTitle {
                axis: ChartAxisKind::Category,
                title: Some("Quarter".into()),
            },
        )]);
        assert_eq!(
            authored_chart(&worker, 0).cat_axis.title.as_deref(),
            Some("Quarter")
        );

        // Series color.
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::SeriesColor {
                series: 0,
                color: Some(Rgb::from_hex(0x70AD47)),
            },
        )]);
        assert_eq!(
            authored_chart(&worker, 0).series[0].color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x70AD47)
            )),
        );

        // Data-label toggles apply to every series; clearing all turns labels off.
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::DataLabels(crate::worker::protocol::DataLabelToggles {
                show_value: true,
                show_category_name: false,
                show_percent: true,
            }),
        )]);
        let dl = authored_chart(&worker, 0).series[0]
            .data_labels
            .clone()
            .expect("labels set");
        assert!(dl.show_value && dl.show_percent && !dl.show_category_name);
        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::DataLabels(crate::worker::protocol::DataLabelToggles::default()),
        )]);
        assert!(
            authored_chart(&worker, 0).series[0].data_labels.is_none(),
            "clearing every toggle removes the labels"
        );
    }

    /// **Batch 5 gate — a `SeriesColor` edit recolors the `a:ln` STROKE only for LINE / SCATTER.**
    /// Those two kinds paint their visible color on the stroke (renderer prefers `stroke.color` over
    /// `color`), so the edit must override it. FILLED kinds (bar/column/area/pie/bubble) render from
    /// the fill and treat `a:ln` as a decorative border, so recoloring their imported stroke would
    /// over-reach — mutating a border the user never touched. Pins the live-side gate.
    #[test]
    fn series_color_recolors_stroke_only_for_line_and_scatter() {
        use freecell_chart_model::{
            Axis, BarDir, BarLayout, Category, Chart, ChartColor, ChartKind, Color, Grouping,
            LineStroke, ScatterStyle, Series,
        };

        // A series carrying BOTH a fill color (4472C4) and an imported `a:ln` stroke on a DISTINCT
        // color (203040) — the shape that distinguishes a fill recolor from a stroke recolor.
        let stroked = || {
            let mut s =
                Series::category_value(Some("S"), vec![Category::Text("Q1".into())], vec![1.0])
                    .with_color(Color::from_hex(0x4472C4));
            s.stroke = Some(
                LineStroke::new()
                    .with_width_pt(1.0)
                    .with_color(Color::from_hex(0x203040)),
            );
            s
        };
        let chart_of = |kind: ChartKind| Chart {
            title: None,
            kind,
            series: vec![stroked()],
            cat_axis: Axis::untitled(),
            val_axis: Axis::untitled(),
            legend: None,
        };
        let edit = ChartChromeEdit::SeriesColor {
            series: 0,
            color: Some(Rgb::from_hex(0x70AD47)),
        };
        let new_cc = Some(ChartColor::Rgb(Color::from_hex(0x70AD47)));
        let orig_stroke = Some(ChartColor::Rgb(Color::from_hex(0x203040)));

        // FILLED (column): the fill recolors, the imported border stroke is LEFT on its color.
        let mut bar = chart_of(ChartKind::Bar {
            dir: BarDir::Col,
            grouping: Grouping::Clustered,
            layout: BarLayout::default(),
        });
        apply_chrome_edit(&mut bar, &edit);
        assert_eq!(bar.series[0].color, new_cc, "the filled fill recolors");
        assert_eq!(
            bar.series[0].stroke.and_then(|s| s.color),
            orig_stroke,
            "a filled series' imported border stroke is left untouched (Batch 5 gate)",
        );

        // LINE: both fill and stroke recolor (the stroke is the visible line — feedback item 9).
        let mut line = chart_of(ChartKind::Line {
            grouping: Grouping::Standard,
            smooth: false,
        });
        apply_chrome_edit(&mut line, &edit);
        assert_eq!(line.series[0].color, new_cc, "the line fill recolors");
        assert_eq!(
            line.series[0].stroke.and_then(|s| s.color),
            new_cc,
            "a line's visible stroke recolors to the new color",
        );

        // SCATTER: same as line — its color also lives on the stroke.
        let mut scatter = chart_of(ChartKind::Scatter {
            style: ScatterStyle::LineMarker,
        });
        apply_chrome_edit(&mut scatter, &edit);
        assert_eq!(
            scatter.series[0].stroke.and_then(|s| s.color),
            new_cc,
            "a scatter's visible stroke recolors to the new color",
        );
    }

    /// P20 degraded guard: a degraded worker rejects `SetChartChrome`, leaving the chart untouched.
    #[test]
    fn set_chart_chrome_rejected_when_degraded() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::InsertChart {
            sheet,
            kind: ChartInsertKind::Line,
            anchor: test_anchor(),
            data: None,
        }]);
        let id = worker.authored_charts[0].id;
        let title_before = authored_chart(&worker, 0).title;
        quiet_panics(|| {
            worker.process_batch(vec![Command::TestPanic]);
            worker.process_batch(vec![Command::TestPanic]);
        });
        assert!(worker.degraded);
        drain_events(&rx);

        worker.process_batch(vec![chrome(
            sheet,
            id,
            ChartChromeEdit::Title(Some("nope".into())),
        )]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Degraded
                }
            )),
            "a degraded worker rejects SetChartChrome"
        );
        assert_eq!(
            authored_chart(&worker, 0).title,
            title_before,
            "the chart is untouched when degraded"
        );
    }

    #[test]
    fn undo_redo_agreement_walk() {
        // A scripted edit/undo/redo walk: the cache must agree with the engine after EVERY step.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        let bold = |r, c| Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(r, c)),
            attr: StyleAttr::Bold,
        };
        let fill = |r, c| Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(r, c)),
            attr: StyleAttr::Fill(Some(Rgb::from_hex(0x00FF00))),
        };

        let steps: Vec<Command> = vec![
            set_input(sheet, 1, 1, "a"),
            bold(1, 1),
            fill(2, 2),
            Command::Undo,               // undo fill(2,2)
            Command::Undo,               // undo bold(1,1)
            Command::Redo,               // redo bold(1,1)
            set_input(sheet, 3, 3, "b"), // new edit clears redo of fill
            bold(3, 3),
            Command::Undo, // undo bold(3,3)
            Command::Undo, // undo set_input(3,3)
            Command::Redo, // redo set_input(3,3)
        ];
        for step in steps {
            worker.process_batch(vec![step]);
            worker_cache_agrees(&worker, sheet, &rows, &cols);
        }
    }

    #[test]
    fn sheet_switch_builds_cache_on_activation() {
        let (mut worker, rx) = test_worker();
        let sheet0_id = sheet0(&worker);
        worker.process_batch(vec![Command::AddSheet]);
        let sheets = worker.sheet_metas();
        let sheet1_id = sheets[1].id;
        assert_ne!(sheet0_id, sheet1_id);
        // The new sheet isn't cached until activated.
        assert!(!worker.shared.caches.read().contains(sheet1_id));
        drain_events(&rx);

        worker.process_batch(vec![Command::SetViewport {
            sheet: sheet1_id,
            rows: 0..4,
            cols: 0..4,
        }]);
        assert!(worker.shared.caches.read().contains(sheet1_id));
        let (rows, cols) = small_probes();
        worker_cache_agrees(&worker, sheet1_id, &rows, &cols);
        assert!(
            drain_events(&rx).iter().any(
                |e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet1_id)
            ),
            "activating a sheet ships its StyleCacheUpdated delta"
        );
    }

    #[test]
    fn delete_sheet_reconciles_cache_map() {
        let (mut worker, _rx) = test_worker();
        worker.process_batch(vec![Command::AddSheet]);
        let sheets = worker.sheet_metas();
        let sheet1_id = sheets[1].id;
        // Activate + cache the second sheet.
        worker.process_batch(vec![Command::SetViewport {
            sheet: sheet1_id,
            rows: 0..4,
            cols: 0..4,
        }]);
        assert!(worker.shared.caches.read().contains(sheet1_id));
        // Deleting it drops its resident cache.
        worker.process_batch(vec![Command::DeleteSheet { sheet: sheet1_id }]);
        assert!(!worker.shared.caches.read().contains(sheet1_id));
    }

    #[test]
    fn move_sheet_reorders_metas_and_undo_restores() {
        let (mut worker, rx) = test_worker();
        // Three sheets: [s0, s1, s2] in workbook order.
        worker.process_batch(vec![Command::AddSheet]);
        worker.process_batch(vec![Command::AddSheet]);
        let before: Vec<SheetId> = worker.sheet_metas().iter().map(|m| m.id).collect();
        assert_eq!(before.len(), 3, "expected three sheets");
        drain_events(&rx);

        // Move the first sheet to the last index → [s1, s2, s0].
        worker.process_batch(vec![Command::MoveSheet {
            sheet: before[0],
            to_index: 2,
        }]);
        let after: Vec<SheetId> = worker.sheet_metas().iter().map(|m| m.id).collect();
        assert_eq!(
            after,
            vec![before[1], before[2], before[0]],
            "MoveSheet reorders the sheet metas"
        );
        assert!(
            drain_events(&rx)
                .iter()
                .any(|e| matches!(e, WorkerEvent::SheetsChanged { .. })),
            "a reorder republishes SheetsChanged so the tab bar rebuilds in engine order"
        );

        // Undo restores the prior order and re-publishes.
        worker.process_batch(vec![Command::Undo]);
        let restored: Vec<SheetId> = worker.sheet_metas().iter().map(|m| m.id).collect();
        assert_eq!(
            restored, before,
            "undo of a reorder restores the prior order"
        );
        assert!(
            drain_events(&rx)
                .iter()
                .any(|e| matches!(e, WorkerEvent::SheetsChanged { .. })),
            "undo of a reorder republishes SheetsChanged"
        );
    }

    /// Probes that include cells FAR out on a row/column, so a band that fills the whole row/column
    /// is actually exercised (an agreement probe confined to 0..6 would miss a rotted band).
    fn wide_probes() -> (Vec<u32>, Vec<u32>) {
        (
            vec![0, 1, 2, 3, 4, 5, 6, 7, 500, 5000],
            vec![0, 1, 2, 3, 4, 5, 6, 7, 500, 5000],
        )
    }

    #[test]
    fn full_row_style_edit_creates_band_and_agrees() {
        // A style edit spanning ALL columns of a row makes IronCalc set a ROW BAND, not per-cell
        // styles. The per-cell mirror can't represent that, so the worker must rebuild — this test
        // FAILS on the pre-fix per-cell path (empty banded cells stay default in the cache).
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 2, 0, "x"), // a value on the row (a mix of styled + empty cells)
        ]);
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(2, 0), CellRef::new(2, limits::MAX_COLS - 1)),
            attr: StyleAttr::Fill(Some(Rgb::from_hex(0xFFFF00))),
        }]);

        let (rows, cols) = wide_probes();
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        // A far, empty cell on the banded row resolves to the fill (the band), not the default.
        let filled = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(2, 500)
            .map(|s| s.fill);
        assert_eq!(filled, Some(Some(Rgb::from_hex(0xFFFF00))));
    }

    #[test]
    fn full_column_style_edit_creates_band_and_agrees() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 3), CellRef::new(limits::MAX_ROWS - 1, 3)),
            attr: StyleAttr::Bold,
        }]);

        let (rows, cols) = wide_probes();
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        let bold = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .render_style(5000, 3)
            .map(|s| s.bold);
        assert_eq!(bold, Some(true), "a far cell on the banded column is bold");
    }

    #[test]
    fn multi_row_block_full_width_band_agrees() {
        // Six full-width rows: 6 × 16,384 = 98,304 cells — BELOW MAX_REFRESH_CELLS, so the
        // cell-count cap alone would miss it. Each row still spans all columns → row bands.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(5, limits::MAX_COLS - 1)),
            attr: StyleAttr::Italic,
        }]);
        let (rows, cols) = wide_probes();
        worker_cache_agrees(&worker, sheet, &rows, &cols);
    }

    #[test]
    fn multiline_input_mirrors_row_height_and_agrees() {
        // A multi-line value grows the engine row height (auto-fit). The mirror must reflect that
        // geometry change (and its undo), or the cache diverges — FAILS without the row-height
        // mirror on the value-edit path.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..6,
            cols: 0..6,
        }]);
        worker.process_batch(vec![set_input(sheet, 1, 1, "line1\nline2\nline3")]);
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        let tall = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .row_height(1);
        assert!(
            tall > 24.0 + 1.0,
            "a 3-line input auto-fits row 1 taller than the 24px default (got {tall})"
        );

        worker.process_batch(vec![Command::Undo]);
        worker_cache_agrees(&worker, sheet, &rows, &cols);
        let reverted = worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .row_height(1);
        assert!(
            (reverted - 24.0).abs() < 1e-3,
            "undo reverts the auto-fit height to the default (got {reverted})"
        );
    }

    #[test]
    fn failed_rebuild_drops_stale_entry_and_reports_failure() {
        // A rebuild that cannot produce a cache (here: a SheetId that no longer resolves — the
        // reachable proxy for a build error) must DROP the entry rather than leave the stale
        // pre-edit cache in place, and report failure so no StyleCacheUpdated is announced.
        let (mut worker, _rx) = test_worker();
        let bogus = SheetId(9999);
        worker
            .shared
            .caches
            .write()
            .insert(bogus, freecell_core::SheetCacheBuilder::new(4, 4).build());
        assert!(worker.shared.caches.read().contains(bogus));

        let rebuilt = worker.build_and_store_cache(bogus);
        assert!(!rebuilt, "an unresolvable sheet reports build failure");
        assert!(
            !worker.shared.caches.read().contains(bogus),
            "the stale entry is dropped (grid falls back to unstyled, never a stale re-read)"
        );
    }

    #[test]
    fn clamp_viewport_bounds_a_pathological_full_sheet_window() {
        // A whole-sheet SetViewport must not wedge the worker in a billions-of-cells probe:
        // the stored window is capped to the overscan bounds.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..limits::MAX_ROWS,
            cols: 0..limits::MAX_COLS,
        }]);
        let vp = worker.viewport.clone().unwrap();
        assert_eq!(vp.rows.end - vp.rows.start, MAX_PUBLISH_ROWS);
        assert_eq!(vp.cols.end - vp.cols.start, MAX_PUBLISH_COLS);
        // And the publish over that bounded window completed (didn't hang) — the generation
        // advanced, and its cell count is within the hard bound.
        assert!(worker.shared.generation.load(Ordering::Acquire) >= 1);
        let cells = worker.shared.publication.load_full().cells.len();
        assert!(cells <= (MAX_PUBLISH_ROWS * MAX_PUBLISH_COLS) as usize);
    }

    // ---- Range clipboard (`components/clipboard.md`) --------------------------------------

    /// The displayed value of a cell on sheet index 0 (the only sheet in these tests).
    fn value_at(worker: &Worker, row: u32, col: u32) -> String {
        worker
            .doc
            .formatted_value(0, CellRef::new(row, col))
            .unwrap_or_default()
    }

    /// Copy `range` on `sheet` (drains the `CopyReady` reply) and return its TSV.
    fn do_copy(
        worker: &mut Worker,
        rx: &async_channel::Receiver<WorkerEvent>,
        sheet: SheetId,
        range: CellRange,
        cut: bool,
    ) -> String {
        worker.process_batch(vec![Command::CopySelection { sheet, range, cut }]);
        drain_events(rx)
            .into_iter()
            .find_map(|e| match e {
                WorkerEvent::CopyReady { tsv } => Some(tsv),
                _ => None,
            })
            .expect("CopySelection must reply CopyReady")
    }

    #[test]
    fn copy_reply_carries_tab_separated_text() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "1"),
            set_input(sheet, 0, 1, "2"),
        ]);
        drain_events(&rx);
        let tsv = do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::new(CellRef::new(0, 0), CellRef::new(0, 1)),
            false,
        );
        assert_eq!(
            tsv, "1\t2",
            "the copy reply is the row's tab-separated values"
        );
    }

    #[test]
    fn copy_then_paste_internal_writes_values_and_selects() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "10"),
            set_input(sheet, 1, 0, "20"),
        ]);
        drain_events(&rx);

        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            false,
        );
        // Paste the A1:A2 payload with its top-left at C1 (row 0, col 2).
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 2)),
        }]);

        assert_eq!(value_at(&worker, 0, 2), "10");
        assert_eq!(value_at(&worker, 1, 2), "20");
        // The reply carries the pasted rectangle (C1:C2) + a repaint + a style-cache delta.
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::Pasted { sheet: s, range }
                    if *s == sheet
                        && *range == CellRange::new(CellRef::new(0, 2), CellRef::new(1, 2))
            )),
            "paste replies with the pasted rectangle; got {events:?}"
        );
        assert!(events
            .iter()
            .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { .. })));
    }

    #[test]
    fn cut_slot_is_single_use() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "7"),
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            true,
        );

        // First paste moves the cut value to C1 and clears the source.
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 2)),
        }]);
        assert_eq!(value_at(&worker, 0, 2), "7");
        assert_eq!(value_at(&worker, 0, 0), "", "the cut source is cleared");
        drain_events(&rx);

        // The slot is consumed → a second paste has nothing to paste.
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 4)),
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::NothingToPaste
                }
            )),
            "a cut is single-use; got {events:?}"
        );
        assert_eq!(
            value_at(&worker, 0, 4),
            "",
            "the second paste wrote nothing"
        );
    }

    #[test]
    fn degenerate_slot_is_nothing_to_paste_not_overflow() {
        // A slot with an inverted (clamped) range must reject as NothingToPaste — not wrap the
        // `(r1 - r0 + 1) as u32` height into a spurious Overflow (Mild #3).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.clipboard = Some(ClipboardSlot {
            sheet,
            range: (5, 1, 2, 1), // r1 (2) < r0 (5): degenerate
            data: serde_json::json!({ "5": { "1": { "text": "x", "style": {} } } }),
            cut: false,
            values: vec![vec!["x".to_string()]],
        });
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 0)),
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::NothingToPaste
                }
            )),
            "an inverted slot range is NothingToPaste; got {events:?}"
        );
        assert!(
            !events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow
                }
            )),
            "it must NOT surface as Overflow"
        );
    }

    #[test]
    fn paste_internal_overflow_is_rejected() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "a"),
            set_input(sheet, 1, 0, "b"),
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
            false,
        );
        let ops_before = worker.ops_seen;

        // A 2-row payload pasted onto the very last row spills past the sheet edge.
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(limits::MAX_ROWS - 1, 0)),
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow
                }
            )),
            "an overflowing paste is rejected; got {events:?}"
        );
        assert_eq!(worker.ops_seen, ops_before, "nothing was applied");
    }

    #[test]
    fn paste_tsv_writes_typed_cells() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..8,
            cols: 0..8,
        }]);
        drain_events(&rx);

        worker.process_batch(vec![Command::PasteTsv {
            sheet,
            anchor: CellRef::new(0, 0),
            text: "1\t2\n=1+2\ttrue\n".to_string(),
        }]);
        assert_eq!(value_at(&worker, 0, 0), "1");
        assert_eq!(value_at(&worker, 0, 1), "2");
        assert_eq!(value_at(&worker, 1, 0), "3", "the =1+2 formula evaluated");
        assert_eq!(value_at(&worker, 1, 1), "TRUE");
        let events = drain_events(&rx);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::Pasted { .. })),
            "a TSV paste replies Pasted; got {events:?}"
        );
    }

    #[test]
    fn paste_tsv_overflow_is_rejected() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let ops_before = worker.ops_seen;
        worker.process_batch(vec![Command::PasteTsv {
            sheet,
            anchor: CellRef::new(limits::MAX_ROWS - 1, 0),
            text: "1\n2\n".to_string(), // two rows onto the last row → overflow
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow
                }
            )),
            "an overflowing TSV paste is rejected; got {events:?}"
        );
        assert_eq!(worker.ops_seen, ops_before);
    }

    #[test]
    fn paste_tsv_quoted_field_width_overflow_is_rejected() {
        // CR Moderate (width): a quoted field with an embedded newline is a 3-wide record; pasted
        // two columns from the right edge it spills — the guard must catch it, no partial write.
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let ops_before = worker.ops_seen;
        worker.process_batch(vec![Command::PasteTsv {
            sheet,
            anchor: CellRef::new(0, limits::MAX_COLS - 2),
            text: "a\t\"x\ny\"\tb".to_string(),
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow
                }
            )),
            "a quoted 3-wide record near the right edge must overflow-reject; got {events:?}"
        );
        assert_eq!(worker.ops_seen, ops_before, "nothing written");
    }

    #[test]
    fn copy_slot_survives_repeated_pastes() {
        // A copy (not cut) is repeatable: the slot stays live across multiple pastes (Mild #4).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "42"),
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );

        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 2)),
        }]);
        assert_eq!(value_at(&worker, 0, 2), "42");
        assert!(
            worker.clipboard.is_some(),
            "a copy stays on the slot after the first paste"
        );
        drain_events(&rx);

        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 4)),
        }]);
        assert_eq!(value_at(&worker, 0, 4), "42", "the second paste also lands");
        assert!(
            worker.clipboard.is_some(),
            "and the slot is still present after the second paste"
        );
    }

    #[test]
    fn engine_error_on_paste_restores_the_slot() {
        // If the paste fails mid-flight (here: a slot whose JSON isn't valid `ClipboardData`, so
        // the engine adapter errors before mutating), the copy is kept for a retry (Mild #4).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.clipboard = Some(ClipboardSlot {
            sheet,
            range: (1, 1, 1, 1), // valid 1×1 — passes the degenerate + overflow guards
            data: serde_json::json!({ "not-an-i32-row": 1 }), // non-empty, but not ClipboardData
            cut: false,
            values: vec![vec!["x".to_string()]],
        });
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 0)),
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(_)
                }
            )),
            "a failed paste surfaces an engine rejection; got {events:?}"
        );
        assert!(
            worker.clipboard.is_some(),
            "the copy is kept after a failed paste (retryable)"
        );
    }

    #[test]
    fn paste_is_a_single_undo_step() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "5"),
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            target: CellRange::single(CellRef::new(0, 2)),
        }]);
        assert_eq!(value_at(&worker, 0, 2), "5");
        drain_events(&rx);

        // One undo removes the whole paste.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(value_at(&worker, 0, 2), "", "one undo reverts the paste");
        assert_eq!(value_at(&worker, 0, 0), "5", "the copy source is untouched");
    }

    #[test]
    fn single_cell_paste_fills_the_whole_target_selection_in_one_undo() {
        // BUG 4: copy one cell, paste onto a 3×3 selection → all nine cells fill, one undo step.
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "7"),
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        // Paste onto C1:E3 (rows 0..=2, cols 2..=4) — a 3×3 target, anchor at C1.
        let target = CellRange::new(CellRef::new(0, 2), CellRef::new(2, 4));
        worker.process_batch(vec![Command::PasteInternal { sheet, target }]);

        for r in 0..3 {
            for c in 2..5 {
                assert_eq!(
                    value_at(&worker, r, c),
                    "7",
                    "cell ({r},{c}) should be filled by the single-cell paste"
                );
            }
        }
        // The reply carries the FULL filled rectangle (C1:E3), and one undo clears all nine cells.
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::Pasted { sheet: s, range } if *s == sheet && *range == target
            )),
            "fill reply carries the whole target; got {events:?}"
        );
        worker.process_batch(vec![Command::Undo]);
        for r in 0..3 {
            for c in 2..5 {
                assert_eq!(
                    value_at(&worker, r, c),
                    "",
                    "one undo must clear the entire fill at ({r},{c})"
                );
            }
        }
        assert_eq!(value_at(&worker, 0, 0), "7", "the copy source is untouched");
    }

    #[test]
    fn single_cell_paste_into_oversized_selection_is_rejected() {
        // A 1-cell paste into a full-column selection would fill > 100k cells — reject as Overflow
        // (the fill cap), nothing pasted, and the copy is preserved for a retry.
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 0, 0, "9")]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        // A whole column A (Excel-max rows) as the target.
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(limits::MAX_ROWS - 1, 0));
        worker.process_batch(vec![Command::PasteInternal { sheet, target }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow
                }
            )),
            "an oversized fill is rejected; got {events:?}"
        );
        assert!(
            worker.clipboard.is_some(),
            "the copy is preserved after a rejected fill"
        );
    }

    // ---- Paste Values (⌘⇧V, `functional_spec.md §5`) --------------------------------------

    #[test]
    fn paste_values_writes_the_evaluated_value_and_keeps_the_slot() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "=1+2"), // a formula → 3
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        worker.process_batch(vec![Command::PasteValues {
            sheet,
            target: CellRange::single(CellRef::new(0, 2)),
        }]);
        // The literal value landed — not the formula.
        assert_eq!(value_at(&worker, 0, 2), "3");
        assert_eq!(
            worker
                .doc
                .cell_content(0, CellRef::new(0, 2))
                .unwrap_or_default(),
            "3",
            "the pasted cell holds the value, not `=1+2`"
        );
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::Pasted { sheet: s, range }
                    if *s == sheet && *range == CellRange::single(CellRef::new(0, 2))
            )),
            "paste-values replies with the pasted rectangle; got {events:?}"
        );
        // A values paste is repeatable → the slot is kept, and one undo reverts it.
        assert!(worker.clipboard.is_some(), "the slot is kept (repeatable)");
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(value_at(&worker, 0, 2), "", "one undo reverts the paste");
        assert_eq!(value_at(&worker, 0, 0), "3", "the source is untouched");
    }

    #[test]
    fn paste_values_forces_a_formula_looking_string_to_a_literal() {
        // The load-bearing edge case: a computed *string* value of `=x` must land as literal text,
        // never re-parsed into a formula (which would evaluate to an error).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "=\"=x\""), // a formula whose *value* is the string "=x"
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        worker.process_batch(vec![Command::PasteValues {
            sheet,
            target: CellRange::single(CellRef::new(0, 2)),
        }]);
        assert_eq!(
            value_at(&worker, 0, 2),
            "=x",
            "the literal text `=x` landed, not a re-interpreted formula"
        );
    }

    #[test]
    fn paste_values_single_cell_fills_the_target_in_one_undo() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..8,
                cols: 0..8,
            },
            set_input(sheet, 0, 0, "7"),
        ]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        // Fill C1:E3 (a 3×3 target) from the single-cell source.
        let target = CellRange::new(CellRef::new(0, 2), CellRef::new(2, 4));
        worker.process_batch(vec![Command::PasteValues { sheet, target }]);
        for r in 0..3 {
            for c in 2..5 {
                assert_eq!(value_at(&worker, r, c), "7", "cell ({r},{c}) filled");
            }
        }
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::Pasted { sheet: s, range } if *s == sheet && *range == target
            )),
            "the fill reply carries the whole target; got {events:?}"
        );
        worker.process_batch(vec![Command::Undo]);
        for r in 0..3 {
            for c in 2..5 {
                assert_eq!(
                    value_at(&worker, r, c),
                    "",
                    "one undo clears the whole fill"
                );
            }
        }
    }

    #[test]
    fn paste_values_oversized_fill_is_rejected() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 0, 0, "9")]);
        drain_events(&rx);
        do_copy(
            &mut worker,
            &rx,
            sheet,
            CellRange::single(CellRef::new(0, 0)),
            false,
        );
        let target = CellRange::new(CellRef::new(0, 0), CellRef::new(limits::MAX_ROWS - 1, 0));
        worker.process_batch(vec![Command::PasteValues { sheet, target }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::Overflow
                }
            )),
            "an oversized values fill is rejected; got {events:?}"
        );
        assert!(
            worker.clipboard.is_some(),
            "the copy is preserved after a rejected fill"
        );
    }

    #[test]
    fn paste_values_with_empty_slot_is_nothing_to_paste() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::PasteValues {
            sheet,
            target: CellRange::single(CellRef::new(0, 0)),
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::PasteRejected {
                    reason: PasteError::NothingToPaste
                }
            )),
            "paste-values with no prior copy is NothingToPaste; got {events:?}"
        );
    }

    // ---- Phase 7: structure (resize, insert/delete, clamp, merge guard) --------------------

    /// A resident cache's device-px column width for `col`.
    fn col_w(worker: &Worker, sheet: SheetId, col: u32) -> f32 {
        worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .col_width(col)
    }
    /// A resident cache's device-px row height for `row`.
    fn row_h(worker: &Worker, sheet: SheetId, row: u32) -> f32 {
        worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .row_height(row)
    }

    #[test]
    fn set_columns_width_range_and_undo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetColumnWidths {
            sheet,
            col_start: 1,
            col_end: 2,
            px: 200.0,
        }]);
        // The resize round-trips device px through the engine (device → IronCalc → device) and the
        // cache is rebuilt to reflect it; the untouched column stays at the default.
        assert!(
            (col_w(&worker, sheet, 1) - 200.0).abs() < 1.0,
            "col 1 = {}",
            col_w(&worker, sheet, 1)
        );
        assert!((col_w(&worker, sheet, 2) - 200.0).abs() < 1.0);
        assert!(
            (col_w(&worker, sheet, 0) - 100.0).abs() < 1.0,
            "col 0 default"
        );
        // Undo is one step and restores the default width.
        worker.process_batch(vec![Command::Undo]);
        assert!(
            (col_w(&worker, sheet, 1) - 100.0).abs() < 1.0,
            "after undo col 1 = {}",
            col_w(&worker, sheet, 1)
        );
    }

    #[test]
    fn set_rows_height_and_undo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetRowHeights {
            sheet,
            row_start: 3,
            row_end: 3,
            px: 60.0,
        }]);
        assert!(
            (row_h(&worker, sheet, 3) - 60.0).abs() < 1.0,
            "row 3 = {}",
            row_h(&worker, sheet, 3)
        );
        assert!(
            (row_h(&worker, sheet, 0) - 24.0).abs() < 1.0,
            "row 0 default"
        );
        worker.process_batch(vec![Command::Undo]);
        assert!(
            (row_h(&worker, sheet, 3) - 24.0).abs() < 1.0,
            "after undo row 3 default"
        );
    }

    /// Whether the resident cache flags `row` hidden.
    fn row_hidden(worker: &Worker, sheet: SheetId, row: u32) -> bool {
        worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .is_row_hidden(row)
    }
    /// Whether the resident cache flags `col` hidden.
    fn col_hidden(worker: &Worker, sheet: SheetId, col: u32) -> bool {
        worker
            .shared
            .caches
            .read()
            .get(sheet)
            .unwrap()
            .is_col_hidden(col)
    }

    #[test]
    fn set_rows_hidden_renders_zero_size_and_undo_restores() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Hide rows 2..=4.
        worker.process_batch(vec![Command::SetRowsHidden {
            sheet,
            start: 2,
            end: 4,
            hidden: true,
        }]);
        assert!(row_hidden(&worker, sheet, 3), "row 3 flagged hidden");
        // A hidden row renders zero-size; its neighbor keeps the default.
        assert_eq!(row_h(&worker, sheet, 3), 0.0, "hidden row is zero-size");
        assert!(
            (row_h(&worker, sheet, 1) - 24.0).abs() < 1.0,
            "row 1 default"
        );
        // Geometry-only: the batch published (StyleCacheUpdated) but ran no eval.
        let events = drain_events(&rx);
        assert!(events
            .iter()
            .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)));

        // One undo step restores full visibility + size.
        worker.process_batch(vec![Command::Undo]);
        assert!(!row_hidden(&worker, sheet, 3), "undo unhides row 3");
        assert!(
            (row_h(&worker, sheet, 3) - 24.0).abs() < 1.0,
            "undo restores row 3 size"
        );
    }

    #[test]
    fn set_columns_hidden_toggle_and_unhide() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetColumnsHidden {
            sheet,
            start: 1,
            end: 1,
            hidden: true,
        }]);
        assert!(col_hidden(&worker, sheet, 1));
        assert_eq!(col_w(&worker, sheet, 1), 0.0, "hidden col is zero-size");
        // Unhide (hidden: false) over the same run restores it — one undo-independent op.
        worker.process_batch(vec![Command::SetColumnsHidden {
            sheet,
            start: 1,
            end: 1,
            hidden: false,
        }]);
        assert!(!col_hidden(&worker, sheet, 1));
        assert!(
            (col_w(&worker, sheet, 1) - 100.0).abs() < 1.0,
            "col restored"
        );
    }

    fn auto_grow(sheet: SheetId, heights: Vec<(u32, f32)>) -> Command {
        Command::AutoGrowRowHeights { sheet, heights }
    }

    #[test]
    fn auto_grow_grows_an_auto_row_without_marking_manual_or_adding_undo() {
        // Wrap-driven auto-grow is a cache-only geometry update (`functional_spec.md §3.4`): it
        // grows the row but does NOT mark it manual, bump the undo counter, or add an undo step.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        let ops_before = worker.ops_seen;
        worker.process_batch(vec![auto_grow(sheet, vec![(2, 96.0)])]);
        assert!(
            (row_h(&worker, sheet, 2) - 96.0).abs() < 1.0,
            "auto row grew to the measured height (got {})",
            row_h(&worker, sheet, 2)
        );
        assert_eq!(
            worker.ops_seen, ops_before,
            "auto-grow must not bump the undo op counter (no separate undo step)"
        );
        assert!(
            !worker
                .manual_rows
                .get(&sheet)
                .is_some_and(|m| m.contains(&2)),
            "auto-grow must NOT mark the row manual"
        );
    }

    #[test]
    fn user_resize_marks_manual_and_auto_grow_skips_it() {
        // A user `SetRowHeights` marks the row manual; a later auto-grow leaves it untouched, while
        // an auto (unmarked) row still grows (§3.3).
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetRowHeights {
            sheet,
            row_start: 3,
            row_end: 3,
            px: 50.0,
        }]);
        assert!(
            worker
                .manual_rows
                .get(&sheet)
                .is_some_and(|m| m.contains(&3)),
            "a user resize marks the row manual"
        );
        // Auto-grow both the manual row 3 and an auto row 4.
        worker.process_batch(vec![auto_grow(sheet, vec![(3, 120.0), (4, 120.0)])]);
        assert!(
            (row_h(&worker, sheet, 3) - 50.0).abs() < 1.0,
            "manual row is not grown by auto-grow (stayed {})",
            row_h(&worker, sheet, 3)
        );
        assert!(
            (row_h(&worker, sheet, 4) - 120.0).abs() < 1.0,
            "auto row grows (got {})",
            row_h(&worker, sheet, 4)
        );
        // A rebuild must NOT re-derive manual from `custom_height` (row 3's resize set it): row 3
        // stays manual, so auto-grow still skips it.
        worker.build_and_store_cache(sheet);
        worker.process_batch(vec![auto_grow(sheet, vec![(3, 200.0)])]);
        assert!(
            (row_h(&worker, sheet, 3) - 50.0).abs() < 1.0,
            "manual row stays manual across a rebuild (got {})",
            row_h(&worker, sheet, 3)
        );
    }

    #[test]
    fn auto_grow_survives_rebuild_and_shrinks_back() {
        // A grown auto height survives a full cache rebuild (a column resize elsewhere) via the
        // persisted wrap-height projection, and a `<= default` height shrinks the row back.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![auto_grow(sheet, vec![(4, 130.0)])]);
        assert!((row_h(&worker, sheet, 4) - 130.0).abs() < 1.0);
        // A resize on a DIFFERENT column triggers a full sheet rebuild.
        worker.process_batch(vec![Command::SetColumnWidths {
            sheet,
            col_start: 6,
            col_end: 6,
            px: 200.0,
        }]);
        assert!(
            (row_h(&worker, sheet, 4) - 130.0).abs() < 1.0,
            "the grown height survives an unrelated rebuild (got {})",
            row_h(&worker, sheet, 4)
        );
        // Shrink: a default-or-smaller measurement drops the wrap contribution.
        worker.process_batch(vec![auto_grow(sheet, vec![(4, 24.0)])]);
        assert!(
            (row_h(&worker, sheet, 4) - 24.0).abs() < 1.0,
            "the row shrinks back to default when the wrap need is gone (got {})",
            row_h(&worker, sheet, 4)
        );
    }

    #[test]
    fn auto_grow_survives_a_neighbor_cell_edit() {
        // The COMMON case: a wrapped notes cell grew its row; editing a SHORT neighbour cell in the
        // same row takes the cheap per-cell cache-refresh path (not a full rebuild). The grown
        // height must be preserved — the render thread won't re-measure (the wrapped cell's inputs
        // didn't change). Regression guard for the per-cell mirror folding in `wrap_heights`.
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..10,
            cols: 0..10,
        }]);
        worker.process_batch(vec![auto_grow(sheet, vec![(4, 130.0)])]);
        assert!((row_h(&worker, sheet, 4) - 130.0).abs() < 1.0);
        // Edit a neighbour cell in the SAME row (a bounded, non-band range → the per-cell mirror).
        worker.process_batch(vec![set_input(sheet, 4, 6, "hi")]);
        assert!(
            (row_h(&worker, sheet, 4) - 130.0).abs() < 1.0,
            "a per-cell edit to a neighbour must NOT collapse the wrap-grown row (got {})",
            row_h(&worker, sheet, 4)
        );
    }

    /// Builds a fixture xlsx whose row 7 carries `ht="15" customHeight="false"` plus two number
    /// cells — the shape Excel/LibreOffice write for an **auto-height** row (the `ht` is the
    /// writer's own calculated height, not a user intent). IronCalc imports it as a `Row` record
    /// with `custom_height: false`, so its `row_height()` (15 pt × 1.5625 = 23.4375 IC px) differs
    /// from the 25 IC px default even though no one ever set a height.
    fn auto_height_row_fixture(dir: &std::path::Path) -> std::path::PathBuf {
        use std::io::{Read, Write};
        let base = dir.join("base.xlsx");
        WorkbookDocument::new_empty().unwrap().save(&base).unwrap();
        let bytes = std::fs::read(&base).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let out = dir.join("auto_height_row.xlsx");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut content = Vec::new();
            f.read_to_end(&mut content).unwrap();
            if name.contains("worksheets/sheet1.xml") {
                let s = String::from_utf8(content).unwrap().replace(
                    "</sheetData>",
                    "<row r=\"7\" customFormat=\"false\" ht=\"15\" hidden=\"false\" \
                     customHeight=\"false\" outlineLevel=\"0\" collapsed=\"false\">\
                     <c r=\"B7\" t=\"n\"><v>44000</v></c>\
                     <c r=\"C7\" t=\"n\"><v>29000</v></c></row></sheetData>",
                );
                content = s.into_bytes();
            }
            writer.start_file(name, opts).unwrap();
            writer.write_all(&content).unwrap();
        }
        writer.finish().unwrap();
        out
    }

    #[test]
    fn value_edit_keeps_auto_height_row_at_default() {
        // Regression (user bug): editing any cell in a file row that carries an Excel-calculated
        // `ht` with `customHeight="false"` shrank the row. The cache build correctly ignores such
        // heights (only `custom_height` rows get an override, so the row renders at the 24 px grid
        // default), but the per-cell edit mirror read the raw engine height (23.4375 IC px → 22.5
        // device px) and "discovered" it as an override, snapping the row from 24 → 22.5 px on the
        // first edit. The mirror must apply the same `custom_height` gate as the build.
        let dir = tempfile::tempdir().unwrap();
        let path = auto_height_row_fixture(dir.path());
        let doc = WorkbookDocument::from_source(&DocumentSource::OpenFile(path)).unwrap();
        let (mut worker, _rx) = worker_over(doc);
        let sheet = sheet0(&worker);
        assert!(
            (row_h(&worker, sheet, 6) - 24.0).abs() < 0.01,
            "an auto-height file row renders at the grid default (got {})",
            row_h(&worker, sheet, 6)
        );

        // Edit a cell in the row (a value-only edit — no font/size change).
        worker.process_batch(vec![set_input(sheet, 6, 2, "123")]);
        assert!(
            (row_h(&worker, sheet, 6) - 24.0).abs() < 0.01,
            "a value edit must not move an auto-height row off the default (got {})",
            row_h(&worker, sheet, 6)
        );
    }

    #[test]
    fn auto_grow_settle_keeps_auto_height_row_at_default() {
        // The same stale-base flaw via the OTHER `row_override_px` consumer: when a wrap
        // contribution on an auto-height file row (`ht` + `customHeight="false"`) is dropped, the
        // row must settle back to the 24 px grid default the build renders — not to the raw engine
        // height (22.5 device px).
        let dir = tempfile::tempdir().unwrap();
        let path = auto_height_row_fixture(dir.path());
        let doc = WorkbookDocument::from_source(&DocumentSource::OpenFile(path)).unwrap();
        let (mut worker, _rx) = worker_over(doc);
        let sheet = sheet0(&worker);
        worker.process_batch(vec![auto_grow(sheet, vec![(6, 130.0)])]);
        assert!((row_h(&worker, sheet, 6) - 130.0).abs() < 1.0);
        // The wrap need goes away (e.g. wrap toggled off) → the row settles at the default.
        worker.process_batch(vec![auto_grow(sheet, vec![(6, 24.0)])]);
        assert!(
            (row_h(&worker, sheet, 6) - 24.0).abs() < 0.01,
            "dropping the wrap contribution must settle the row at the grid default (got {})",
            row_h(&worker, sheet, 6)
        );
    }

    #[test]
    fn insert_rows_shifts_and_undo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..10,
            cols: 0..3,
        }]);
        worker.process_batch(vec![set_input(sheet, 2, 0, "42")]); // A3 = 42
                                                                  // Insert one row at the top → A3's content shifts down to A4.
        worker.process_batch(vec![Command::InsertRows {
            sheet,
            row: 0,
            count: 1,
        }]);
        assert_eq!(
            value_at(&worker, 3, 0),
            "42",
            "content shifted down one row"
        );
        assert_eq!(value_at(&worker, 2, 0), "", "the vacated row is empty");
        // Undo restores the original position.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(
            value_at(&worker, 2, 0),
            "42",
            "undo restores the pre-insert layout"
        );
    }

    #[test]
    fn delete_columns_shifts_and_undo() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::SetViewport {
            sheet,
            rows: 0..3,
            cols: 0..5,
        }]);
        worker.process_batch(vec![set_input(sheet, 0, 2, "z")]); // C1 = z
                                                                 // Delete column A → C1's content shifts left to B1.
        worker.process_batch(vec![Command::DeleteColumns {
            sheet,
            col: 0,
            count: 1,
        }]);
        assert_eq!(
            value_at(&worker, 0, 1),
            "z",
            "content shifted left one column"
        );
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(
            value_at(&worker, 0, 2),
            "z",
            "undo restores the deleted column"
        );
    }

    #[test]
    fn clear_contents_clamps_full_column() {
        let (mut worker, _rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 5, 1, "x")]); // B6
                                                                 // Delete over the WHOLE column B (all 1,048,576 rows). The clamp keeps this from iterating
                                                                 // the full band — it clears only the used cell and returns promptly.
        let full_col_b = CellRange::new(CellRef::new(0, 1), CellRef::new(limits::MAX_ROWS - 1, 1));
        worker.process_batch(vec![Command::ClearCells {
            sheet,
            range: full_col_b,
        }]);
        assert_eq!(
            value_at(&worker, 5, 1),
            "",
            "the used cell in the column was cleared"
        );
    }

    /// Builds a merged-cell fixture xlsx (`K7:L10`) by saving a fresh workbook and injecting a
    /// `<mergeCells>` element into its sheet XML — IronCalc has no merge-creation API at 0.7.1, but
    /// its importer reads `mergeCells` from the file (`import/worksheets.rs:load_merge_cells`).
    fn merged_fixture(dir: &std::path::Path) -> std::path::PathBuf {
        use std::io::{Read, Write};
        let base = dir.join("base.xlsx");
        WorkbookDocument::new_empty().unwrap().save(&base).unwrap();
        let bytes = std::fs::read(&base).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        let out = dir.join("merged.xlsx");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&out).unwrap());
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..archive.len() {
            let mut f = archive.by_index(i).unwrap();
            let name = f.name().to_string();
            let mut content = Vec::new();
            f.read_to_end(&mut content).unwrap();
            if name.contains("worksheets/sheet1.xml") {
                let s = String::from_utf8(content).unwrap().replace(
                    "</worksheet>",
                    "<mergeCells count=\"1\"><mergeCell ref=\"K7:L10\"/></mergeCells></worksheet>",
                );
                content = s.into_bytes();
            }
            writer.start_file(name, opts).unwrap();
            writer.write_all(&content).unwrap();
        }
        writer.finish().unwrap();
        out
    }

    /// A worker over an already-opened document (the merged fixture), with its active-sheet cache
    /// built — mirrors `test_worker` but takes the document.
    fn worker_over(doc: WorkbookDocument) -> (Worker, async_channel::Receiver<WorkerEvent>) {
        let (tx, rx) = async_channel::unbounded();
        let shared = Arc::new(Shared::new(SheetId(0)));
        let mut worker = Worker {
            doc,
            shared,
            event_tx: tx,
            active_sheet: SheetId(0),
            viewport: None,
            ops_seen: 0,
            eval_count: 0,
            degraded: false,
            panic_count: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            clipboard: None,
            charts: ChartBindings::default(),
            authored_charts: Vec::new(),
            next_chart_id: 1,
            loaded_anchor_edits: HashMap::new(),
            loaded_deletes: HashSet::new(),
            chart_version: 0,
            chart_source_path: None,
            discovered_chart_sheets: HashSet::new(),
            charts_fully_discovered: true,
            chart_sheet_parts: HashMap::new(),
            manual_rows: HashMap::new(),
            wrap_heights: HashMap::new(),
        };
        if let Some(first) = worker.sheet_metas().first() {
            worker.active_sheet = first.id;
        }
        worker.build_and_store_cache(worker.active_sheet);
        (worker, rx)
    }

    #[test]
    fn insert_near_merge_displaces_no_longer_blocked() {
        // The interim insert/delete merge guard is retired (merged-cell-ui `architecture.md §5`):
        // the engine now DISPLACES merges across structural edits, so an insert near a merge applies
        // (no `MergedCells` rejection) and the resident cache reflects the shifted region.
        let dir = tempfile::tempdir().unwrap();
        let path = merged_fixture(dir.path());
        let doc = WorkbookDocument::from_source(&DocumentSource::OpenFile(path)).unwrap();
        // The merge reads back 0-based via the normalized engine API (K7:L10 → rows 6..=9,
        // cols 10..=11).
        let merge = CellRange::new(CellRef::new(6, 10), CellRef::new(9, 11));
        assert_eq!(doc.merged_regions(0).unwrap(), vec![merge]);
        let (mut worker, rx) = worker_over(doc);
        let sheet = sheet0(&worker);
        // …and rides into the resident cache for the UI to render/select from.
        assert_eq!(
            worker.shared.caches.read().get(sheet).unwrap().merges(),
            &[merge]
        );

        // Inserting a row ABOVE the merge (0-based row 0) now APPLIES — no rejection — and shifts the
        // whole region down by one (rows 6..=9 → 7..=10, cols unchanged).
        let ops_before = worker.ops_seen;
        worker.process_batch(vec![Command::InsertRows {
            sheet,
            row: 0,
            count: 1,
        }]);
        assert!(
            !drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::MergedCells
                }
            )),
            "insert near a merge must no longer be merge-blocked"
        );
        assert!(
            worker.ops_seen > ops_before,
            "the displacing insert must commit"
        );
        let displaced = CellRange::new(CellRef::new(7, 10), CellRef::new(10, 11));
        assert_eq!(
            worker.shared.caches.read().get(sheet).unwrap().merges(),
            &[displaced],
            "the resident cache reflects the displaced region"
        );
    }

    #[test]
    fn fill_into_merge_is_rejected_disjoint_fill_applies() {
        let dir = tempfile::tempdir().unwrap();
        let path = merged_fixture(dir.path());
        let doc = WorkbookDocument::from_source(&DocumentSource::OpenFile(path)).unwrap();
        // K7:L10 → 0-based rows 6..=9, cols 10..=11.
        let (mut worker, rx) = worker_over(doc);
        let sheet = sheet0(&worker);

        // A ⌘D whose target rectangle overlaps the merge is refused with the same typed dialog the
        // structural ops use, and commits nothing (`functional_spec.md §3` edge case).
        let ops_before = worker.ops_seen;
        worker.process_batch(vec![Command::FillDown {
            sheet,
            range: CellRange::new(CellRef::new(6, 10), CellRef::new(9, 10)), // K7:K10, inside merge
        }]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::MergedCells
                }
            )),
            "a fill overlapping a merge must be refused with MergedCells"
        );
        assert_eq!(
            worker.ops_seen, ops_before,
            "a blocked fill commits nothing"
        );

        // The column path (⌘R) is guarded identically.
        let ops_before_right = worker.ops_seen;
        worker.process_batch(vec![Command::FillRight {
            sheet,
            range: CellRange::new(CellRef::new(6, 10), CellRef::new(6, 11)), // K7:L7, inside merge
        }]);
        assert!(
            drain_events(&rx).iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::MergedCells
                }
            )),
            "a fill-right overlapping a merge must be refused with MergedCells"
        );
        assert_eq!(
            worker.ops_seen, ops_before_right,
            "a blocked fill-right commits nothing"
        );

        // A ⌘D far from any merge (A1:A3) applies normally — the guard is not a blanket block.
        worker.process_batch(vec![set_input(sheet, 0, 0, "1")]);
        let ops_before_fill = worker.ops_seen;
        worker.process_batch(vec![Command::FillDown {
            sheet,
            range: CellRange::new(CellRef::new(0, 0), CellRef::new(2, 0)),
        }]);
        assert!(
            worker.ops_seen > ops_before_fill,
            "a fill disjoint from every merge must apply"
        );
        let events = drain_events(&rx);
        assert!(
            !events.iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::MergedCells
                }
            )),
            "a fill disjoint from every merge must not be merge-blocked"
        );
    }

    #[test]
    fn merge_cells_creates_region_and_clears_covered_content() {
        // A merge over a range with content only in the anchor applies with no confirm, keeps the
        // anchor value, clears covered content, and the resident cache reflects the new region
        // (merged-cell-ui `functional_spec.md F2`, `architecture.md §3`).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 0, 0, "hi")]);
        drain_events(&rx);

        let area = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1)); // A1:B2
        worker.process_batch(vec![Command::MergeCells {
            sheet,
            area,
            confirmed: false,
        }]);
        let events = drain_events(&rx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, WorkerEvent::MergeNeedsConfirm { .. })),
            "an all-anchor merge needs no confirm"
        );
        assert_eq!(
            worker.shared.caches.read().get(sheet).unwrap().merges(),
            &[area],
            "the cache reflects the new region"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(0, 0)).unwrap(),
            "hi"
        );

        // Unmerge (by the anchor) removes the region.
        worker.process_batch(vec![Command::UnmergeCells {
            sheet,
            anchor: CellRef::new(0, 0),
        }]);
        drain_events(&rx);
        assert!(
            worker
                .shared
                .caches
                .read()
                .get(sheet)
                .unwrap()
                .merges()
                .is_empty(),
            "unmerge clears the region from the cache"
        );
    }

    #[test]
    fn merge_data_loss_round_trip() {
        // A merge that would discard covered content asks to confirm first (no mutation); the
        // re-send with `confirmed: true` performs it, discarding the covered value
        // (merged-cell-ui `functional_spec.md F3`, `architecture.md §8`).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "anchor"),
            set_input(sheet, 1, 1, "covered"), // B2 — a covered cell with content
        ]);
        drain_events(&rx);
        let area = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1)); // A1:B2

        // Unconfirmed → MergeNeedsConfirm, nothing applied.
        let ops_before = worker.ops_seen;
        worker.process_batch(vec![Command::MergeCells {
            sheet,
            area,
            confirmed: false,
        }]);
        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::MergeNeedsConfirm { sheet: s, area: a } if *s == sheet && *a == area
            )),
            "a data-losing merge must ask to confirm"
        );
        assert_eq!(
            worker.ops_seen, ops_before,
            "an unconfirmed data-losing merge commits nothing"
        );
        assert!(
            worker
                .shared
                .caches
                .read()
                .get(sheet)
                .unwrap()
                .merges()
                .is_empty(),
            "nothing merged before confirmation"
        );

        // Confirmed → performs the merge (covered content discarded).
        worker.process_batch(vec![Command::MergeCells {
            sheet,
            area,
            confirmed: true,
        }]);
        let events = drain_events(&rx);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, WorkerEvent::MergeNeedsConfirm { .. })),
            "a confirmed merge does not re-ask"
        );
        assert_eq!(
            worker.shared.caches.read().get(sheet).unwrap().merges(),
            &[area]
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(1, 1)).unwrap(),
            "",
            "the covered cell's content was discarded"
        );
    }

    #[test]
    fn merge_unmerge_undo_redo_restores_region_and_content() {
        // Merge/unmerge are single undoable engine steps; undo restores the prior merge state AND
        // any discarded content (merged-cell-ui `functional_spec.md F7`, `architecture.md §4`).
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            set_input(sheet, 0, 0, "anchor"),
            set_input(sheet, 1, 1, "covered"),
        ]);
        drain_events(&rx);
        let area = CellRange::new(CellRef::new(0, 0), CellRef::new(1, 1));

        worker.process_batch(vec![Command::MergeCells {
            sheet,
            area,
            confirmed: true,
        }]);
        drain_events(&rx);
        assert_eq!(
            worker.shared.caches.read().get(sheet).unwrap().merges(),
            &[area]
        );

        // Undo restores both the (absent) region and the discarded covered content.
        worker.process_batch(vec![Command::Undo]);
        drain_events(&rx);
        assert!(
            worker
                .shared
                .caches
                .read()
                .get(sheet)
                .unwrap()
                .merges()
                .is_empty(),
            "undo removes the region"
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(1, 1)).unwrap(),
            "covered",
            "undo restores the discarded covered content"
        );

        // Redo re-applies the merge (content discarded again).
        worker.process_batch(vec![Command::Redo]);
        drain_events(&rx);
        assert_eq!(
            worker.shared.caches.read().get(sheet).unwrap().merges(),
            &[area]
        );
        assert_eq!(
            worker.doc.formatted_value(0, CellRef::new(1, 1)).unwrap(),
            ""
        );
    }

    #[test]
    fn noop_fill_skips_recompute_publish_and_ops_bump() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![set_input(sheet, 0, 0, "5")]);
        let _ = drain_events(&rx); // discard the setup edit's events
        let ops_before = worker.ops_seen;

        // ⌘D on a lone A1 (row 0, no neighbor above) resolves to a no-op: it must not recompute,
        // republish, or bump ops_seen (Mild #1 — an errant fill over zero change stays free).
        worker.process_batch(vec![Command::FillDown {
            sheet,
            range: CellRange::single(CellRef::new(0, 0)),
        }]);
        assert_eq!(worker.ops_seen, ops_before, "a no-op fill commits no op");
        assert!(
            !drain_events(&rx)
                .iter()
                .any(|e| matches!(e, WorkerEvent::Published)),
            "a no-op fill must not republish"
        );
        assert_eq!(value_at(&worker, 0, 0), "5"); // unchanged
    }

    // ---- Conditional formatting (P2 worker seam) ----

    const CF_RED: Rgb = Rgb::new(255, 0, 0);

    /// A "Cell value > operand" highlight rule filling matches red — the common CF fixture.
    fn gt_rule(operand: &str) -> CfRuleSpec {
        CfRuleSpec::CellIs {
            op: CfValueOp::Gt,
            operand: operand.to_string(),
            operand2: None,
            format: CfFormat {
                fill: Some(CF_RED),
                ..Default::default()
            },
            stop_if_true: false,
        }
    }

    fn add_cf(sheet: SheetId, range: &str, operand: &str) -> Command {
        Command::AddCondFmt {
            sheet,
            range: range.to_string(),
            spec: gt_rule(operand),
        }
    }

    /// The published CF rules for `sheet` (a clone of the shared map entry; empty when absent).
    fn published_cf(worker: &Worker, sheet: SheetId) -> Vec<freecell_core::CfRuleView> {
        worker
            .shared
            .cond_fmt
            .read()
            .get(&sheet)
            .cloned()
            .unwrap_or_default()
    }

    fn cond_fmt_updated_for(events: &[WorkerEvent], sheet: SheetId) -> bool {
        events
            .iter()
            .any(|e| matches!(e, WorkerEvent::CondFmtUpdated { sheet: s } if *s == sheet))
    }

    fn any_cond_fmt_updated(events: &[WorkerEvent]) -> bool {
        events
            .iter()
            .any(|e| matches!(e, WorkerEvent::CondFmtUpdated { .. }))
    }

    #[test]
    fn add_cond_fmt_publishes_rules_and_emits_events() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // A resident, viewport-covered sheet so the style-cache mirror ships its delta.
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "150"),
        ]);
        drain_events(&rx);

        worker.process_batch(vec![add_cf(sheet, "A1:A10", "100")]);

        let rules = published_cf(&worker, sheet);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].range, "A1:A10");
        assert_eq!(rules[0].summary, "Cell value > 100");

        let events = drain_events(&rx);
        assert!(
            cond_fmt_updated_for(&events, sheet),
            "an AddCondFmt republishes the rule list",
        );
        assert!(
            events
                .iter()
                .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)),
            "an AddCondFmt ships a StyleCacheUpdated delta (the rule range refreshes)",
        );
    }

    #[test]
    fn update_cond_fmt_republishes() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![add_cf(sheet, "A1:A10", "100")]);
        drain_events(&rx);

        worker.process_batch(vec![Command::UpdateCondFmt {
            sheet,
            index: 0,
            range: "B1:B20".to_string(),
            spec: gt_rule("50"),
        }]);

        let rules = published_cf(&worker, sheet);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].range, "B1:B20");
        assert_eq!(rules[0].summary, "Cell value > 50");
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));
    }

    #[test]
    fn delete_cond_fmt_removes_from_map() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![add_cf(sheet, "A1:A10", "100")]);
        drain_events(&rx);
        assert_eq!(published_cf(&worker, sheet).len(), 1);

        worker.process_batch(vec![Command::DeleteCondFmt { sheet, index: 0 }]);

        assert!(
            published_cf(&worker, sheet).is_empty(),
            "the sheet's rules are gone"
        );
        assert!(
            !worker.shared.cond_fmt.read().contains_key(&sheet),
            "no empty-vec entry lingers (a zero-rule sheet holds no key)"
        );
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));
    }

    #[test]
    fn raise_lower_reorders_published_list() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Rule A (index 0, priority 1) then rule B (index 1, priority 2) → priority-desc: B first.
        worker.process_batch(vec![
            add_cf(sheet, "A1:A1", "1"),
            add_cf(sheet, "B1:B1", "1"),
        ]);
        drain_events(&rx);
        assert_eq!(published_cf(&worker, sheet)[0].range, "B1:B1");

        // Raise A (storage index 0) above B.
        worker.process_batch(vec![Command::RaiseCondFmtPriority { sheet, index: 0 }]);
        assert_eq!(published_cf(&worker, sheet)[0].range, "A1:A1");
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));

        // Lower A back down.
        worker.process_batch(vec![Command::LowerCondFmtPriority { sheet, index: 0 }]);
        assert_eq!(published_cf(&worker, sheet)[0].range, "B1:B1");
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));
    }

    #[test]
    fn raise_at_top_is_noop() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            add_cf(sheet, "A1:A1", "1"),
            add_cf(sheet, "B1:B1", "1"),
        ]);
        drain_events(&rx);
        let ops_before = worker.ops_seen;

        // B (storage index 1) already holds the highest priority → raising it changes nothing: it
        // must record no undo op (the engine pushes no diff) and must not republish. This keeps the
        // worker undo stack 1:1 with the engine's.
        worker.process_batch(vec![Command::RaiseCondFmtPriority { sheet, index: 1 }]);
        assert_eq!(
            worker.ops_seen, ops_before,
            "a boundary raise commits no op"
        );
        assert!(
            !any_cond_fmt_updated(&drain_events(&rx)),
            "a no-op raise does not republish"
        );
    }

    #[test]
    fn undo_redo_restores_and_republishes_cf() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Seed a matching VALUE (A1 = 150 > 100) in its own batch and make the sheet cache resident,
        // so the render-cache fill assertions below are real. The CF add stays value-free — exactly
        // the BUG-1 case — so undo/redo must refresh the render cache on their own. A rule-list-only
        // check (published_cf) would miss a stale-`cf_cache` render regression across undo/redo.
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..10,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "150"),
        ]);
        drain_events(&rx);

        worker.process_batch(vec![add_cf(sheet, "A1:A10", "100")]);
        drain_events(&rx);
        assert_eq!(published_cf(&worker, sheet).len(), 1);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            Some(CF_RED),
            "A1 (150 > 100) is filled in the render cache after the add"
        );

        // Undo the add → the rule is gone and the map republishes empty.
        worker.process_batch(vec![Command::Undo]);
        assert!(
            published_cf(&worker, sheet).is_empty(),
            "undo removes the published rule"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "undo clears the render-cache fill (rule gone, no value edit) — the BUG-1 coverage class"
        );
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));

        // Redo → the rule and its publication come back.
        worker.process_batch(vec![Command::Redo]);
        let rules = published_cf(&worker, sheet);
        assert_eq!(rules.len(), 1, "redo restores the rule");
        assert_eq!(rules[0].range, "A1:A10");
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            Some(CF_RED),
            "redo restores the render-cache fill (no value edit)"
        );
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));
    }

    #[test]
    fn bad_range_add_surfaces_error() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![Command::AddCondFmt {
            sheet,
            range: "not-a-range".to_string(),
            spec: gt_rule("100"),
        }]);

        let events = drain_events(&rx);
        assert!(
            events.iter().any(|e| matches!(
                e,
                WorkerEvent::EditRejected {
                    reason: EditRejectedReason::Engine(_)
                }
            )),
            "a bad-range add surfaces an engine error on the result channel"
        );
        assert!(
            published_cf(&worker, sheet).is_empty(),
            "nothing was added on a rejected op"
        );
        assert!(
            !any_cond_fmt_updated(&events),
            "a rejected add does not republish"
        );
    }

    #[test]
    fn non_cf_sheet_has_empty_published_map() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..4,
                cols: 0..4,
            },
            set_input(sheet, 0, 0, "42"),
        ]);
        // A style edit (AppliedOp::Cells) and a structural edit (AppliedOp::Rebuild) on a CF-free
        // sheet must never populate the map or emit CondFmtUpdated — the non-CF fast path stays free.
        worker.process_batch(vec![Command::SetStyleAttr {
            sheet,
            range: CellRange::single(CellRef::new(0, 0)),
            attr: StyleAttr::Bold,
        }]);
        worker.process_batch(vec![Command::InsertRows {
            sheet,
            row: 0,
            count: 1,
        }]);

        assert!(
            worker.shared.cond_fmt.read().is_empty(),
            "a non-CF workbook keeps the published map empty"
        );
        assert!(!any_cond_fmt_updated(&drain_events(&rx)));
    }

    #[test]
    fn structural_edit_shifts_published_cf_range() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![add_cf(sheet, "A5:A10", "100")]);
        drain_events(&rx);
        assert_eq!(published_cf(&worker, sheet)[0].range, "A5:A10");

        // Inserting a row above the rule's range shifts it down (the engine displaces CF ranges on a
        // structural edit); the published list must reflect the new range + emit CondFmtUpdated.
        worker.process_batch(vec![Command::InsertRows {
            sheet,
            row: 0,
            count: 1,
        }]);
        assert_eq!(
            published_cf(&worker, sheet)[0].range,
            "A6:A11",
            "the published CF range follows the structural shift"
        );
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet));
    }

    // Verifies the FreeCell-side reconcile-of-reappeared-sheets fix end-to-end, but only passes
    // once the engine restores CF on undo-of-delete. That engine fix is committed on the fork's
    // `freecell-fixes` branch (base/src/user_model/undo_redo.rs `Diff::DeleteSheet` arm) but could
    // not be pushed from this container (git-proxy 403 on push to scosman/ironcalc), so the pinned
    // engine still drops CF on undo-of-delete. Remove this `#[ignore]` once `freecell-fixes` carries
    // the fix and `app/Cargo.lock` is re-pinned (validated green against the fixed fork locally).
    #[ignore = "blocked on fork CF-undo fix on freecell-fixes (push 403-denied); re-pin then un-ignore"]
    #[test]
    fn undo_of_sheet_delete_repopulates_cf_map() {
        let (mut worker, rx) = test_worker();
        // Add a second sheet, put a CF rule on it, then delete it — the delete prunes its CF entry.
        worker.process_batch(vec![Command::AddSheet]);
        let sheet1 = worker.sheet_metas().last().unwrap().id;
        worker.process_batch(vec![add_cf(sheet1, "A1:A10", "100")]);
        drain_events(&rx);
        assert_eq!(published_cf(&worker, sheet1).len(), 1);

        worker.process_batch(vec![Command::DeleteSheet { sheet: sheet1 }]);
        assert!(
            published_cf(&worker, sheet1).is_empty(),
            "deleting the sheet prunes its CF entry"
        );
        drain_events(&rx);

        // Undo restores the sheet AND its CF rules — the published map must be repopulated and a
        // CondFmtUpdated emitted (undo-of-delete pushes `Touch::Sheets`, so the returning sheet only
        // reaches the reconcile via the sheet-set-change path).
        worker.process_batch(vec![Command::Undo]);
        let rules = published_cf(&worker, sheet1);
        assert_eq!(rules.len(), 1, "undo-of-delete repopulates the CF map");
        assert_eq!(rules[0].range, "A1:A10");
        assert!(cond_fmt_updated_for(&drain_events(&rx), sheet1));
    }

    // ---- Conditional formatting: value-dependent render cache (P3) ----

    /// The cached render-style fill for a resident sheet's cell (`None` when unstored or unfilled).
    fn cache_fill(worker: &Worker, sheet: SheetId, row: u32, col: u32) -> Option<Rgb> {
        worker
            .shared
            .caches
            .read()
            .get(sheet)
            .expect("sheet cache resident")
            .render_style(row, col)
            .and_then(|rs| rs.fill)
    }

    /// A "Top rank" highlight rule filling matches red. A **global** rule: its result at one cell
    /// depends on the whole range, so a value edit anywhere must re-evaluate every cell.
    fn top_fill_rule(rank: u32) -> CfRuleSpec {
        CfRuleSpec::Top {
            rank,
            percent: false,
            bottom: false,
            format: CfFormat {
                fill: Some(CF_RED),
                ..Default::default()
            },
            stop_if_true: false,
        }
    }

    #[test]
    fn cf_value_change_flips_cached_style_no_cf_command() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // A Top-1 rule over A1:A3 with values 10/20/30 → A3 (row 2) holds the top value.
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "10"),
            set_input(sheet, 1, 0, "20"),
            set_input(sheet, 2, 0, "30"),
            Command::AddCondFmt {
                sheet,
                range: "A1:A3".to_string(),
                spec: top_fill_rule(1),
            },
        ]);
        drain_events(&rx);
        assert_eq!(
            cache_fill(&worker, sheet, 2, 0),
            Some(CF_RED),
            "A3 (the top value) is filled"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "A1 is not the top value"
        );

        // Edit A1 to 100 — the new top — with NO CF command. The whole range must re-evaluate: A1
        // gains the fill and A3 loses it (though A3's own value 30 never changed), proving the
        // global value-publish invalidation (not just a touched-cell refresh).
        worker.process_batch(vec![set_input(sheet, 0, 0, "100")]);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            Some(CF_RED),
            "A1 is now the top value → filled after the value edit"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 2, 0),
            None,
            "A3 lost the fill though its own value never changed"
        );
        assert!(
            drain_events(&rx)
                .iter()
                .any(|e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)),
            "the value edit ships a StyleCacheUpdated for the CF sheet"
        );
    }

    #[test]
    fn cf_threshold_value_change_flips_cached_style() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "50"),
            add_cf(sheet, "A1:A10", "100"),
        ]);
        drain_events(&rx);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "50 is below the > 100 threshold"
        );

        worker.process_batch(vec![set_input(sheet, 0, 0, "150")]);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            Some(CF_RED),
            "150 crosses the threshold → filled, with no CF command"
        );

        worker.process_batch(vec![set_input(sheet, 0, 0, "50")]);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "back below the threshold → fill gone"
        );
    }

    #[test]
    fn cf_color_scale_reflected_in_render_cache() {
        use freecell_core::{CfColorStop, CfThresholdKind};
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let green = Rgb::new(0, 255, 0);
        let red = Rgb::new(255, 0, 0);
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "0"),
            set_input(sheet, 1, 0, "50"),
            set_input(sheet, 2, 0, "100"),
            Command::AddCondFmt {
                sheet,
                range: "A1:A3".to_string(),
                spec: CfRuleSpec::ColorScale {
                    stops: vec![
                        CfColorStop {
                            kind: CfThresholdKind::Min,
                            value: None,
                            color: green,
                        },
                        CfColorStop {
                            kind: CfThresholdKind::Max,
                            value: None,
                            color: red,
                        },
                    ],
                },
            },
        ]);
        drain_events(&rx);
        let mid = cache_fill(&worker, sheet, 1, 0);
        assert!(
            mid.is_some(),
            "the scale's mid cell carries an interpolated fill in the render cache"
        );
        assert_ne!(mid, Some(green), "mid is not the min endpoint");
        assert_ne!(mid, Some(red), "mid is not the max endpoint");
    }

    // BUG-1 regression: a CF mutation must refresh the published render cache immediately, with no
    // subsequent value edit. Pre-fix, a CF-only batch left `needs_eval = false`, so `doc.evaluate()`
    // never ran, IronCalc's `cf_cache` stayed stale, and the sheet-cache rebuild read no CF overlay
    // — the rule only appeared after a later value change. Each of these tests isolates a CF op in
    // its own value-free batch, so it FAILS pre-fix and PASSES once CF forces the coalesced eval.

    #[test]
    fn cond_fmt_applies_on_add_without_value_change() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Seed the values in their OWN batch and let it settle. The CF add below carries NO value
        // edit — exactly the case BUG-1 missed (a value edit would have forced the eval anyway).
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "10"),
            set_input(sheet, 1, 0, "20"),
            set_input(sheet, 2, 0, "30"),
        ]);
        drain_events(&rx);

        // A "> 15" red-fill rule over A1:A3 with NO subsequent value change: A2/A3 match, A1 doesn't.
        worker.process_batch(vec![add_cf(sheet, "A1:A3", "15")]);

        assert_eq!(
            cache_fill(&worker, sheet, 1, 0),
            Some(CF_RED),
            "A2 (20 > 15) is filled immediately on add — no value edit needed (BUG-1)"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 2, 0),
            Some(CF_RED),
            "A3 (30 > 15) is filled immediately on add"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "A1 (10) is below the threshold → not filled"
        );
        assert!(
            cond_fmt_updated_for(&drain_events(&rx), sheet),
            "the add republishes the CF map"
        );
    }

    #[test]
    fn cond_fmt_removed_on_delete_without_value_change() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Value + rule in one batch → the value edit forces the eval, so the fill is present even
        // pre-fix. This isolates the DELETE (value-free) batch as the thing under test.
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "150"),
            add_cf(sheet, "A1:A10", "100"),
        ]);
        drain_events(&rx);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            Some(CF_RED),
            "A1 (150 > 100) is filled after the add"
        );

        // Delete the rule with NO value edit — the fill must clear from the published cache at once.
        worker.process_batch(vec![Command::DeleteCondFmt { sheet, index: 0 }]);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "deleting the rule clears the fill without any value change (BUG-1)"
        );
        assert!(
            cond_fmt_updated_for(&drain_events(&rx), sheet),
            "the delete republishes the CF map"
        );
    }

    #[test]
    fn cond_fmt_updated_on_edit_without_value_change() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        // Value + "> 100" rule in one batch (fill present pre-fix), isolating the UPDATE batch.
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 0, 0, "50"),
            set_input(sheet, 1, 0, "150"),
            add_cf(sheet, "A1:A2", "100"),
        ]);
        drain_events(&rx);
        assert_eq!(
            cache_fill(&worker, sheet, 1, 0),
            Some(CF_RED),
            "A2 (150 > 100) starts filled"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            None,
            "A1 (50) does not match > 100"
        );

        // Lower the threshold to > 40 with NO value edit — A1 (50) now matches and must gain the fill.
        worker.process_batch(vec![Command::UpdateCondFmt {
            sheet,
            index: 0,
            range: "A1:A2".to_string(),
            spec: gt_rule("40"),
        }]);
        assert_eq!(
            cache_fill(&worker, sheet, 0, 0),
            Some(CF_RED),
            "A1 (50 > 40) gains the fill after the threshold edit — no value change (BUG-1)"
        );
        assert_eq!(
            cache_fill(&worker, sheet, 1, 0),
            Some(CF_RED),
            "A2 (150 > 40) stays filled"
        );
        assert!(
            cond_fmt_updated_for(&drain_events(&rx), sheet),
            "the update republishes the CF map"
        );
    }

    #[test]
    fn non_cf_value_edit_stays_on_fast_path() {
        let (mut worker, rx) = test_worker();
        let sheet = sheet0(&worker);
        let (rows, cols) = small_probes();
        worker.process_batch(vec![
            Command::SetViewport {
                sheet,
                rows: 0..6,
                cols: 0..6,
            },
            set_input(sheet, 1, 1, "42"),
        ]);
        drain_events(&rx);

        // A value edit on a workbook with no CF: the invalidation gate short-circuits (the published
        // CF map stays empty), and the cache still matches a fresh BASE engine re-read — no CF
        // behavior leaked onto the non-CF fast path.
        worker.process_batch(vec![set_input(sheet, 1, 1, "99")]);
        assert!(
            worker.shared.cond_fmt.read().is_empty(),
            "a non-CF workbook keeps the published CF map empty"
        );
        worker_cache_agrees(&worker, sheet, &rows, &cols);
    }
}
