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

use freecell_core::input_cap::validate_input;
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::{limits, CellRange, CellRef, Publication, PublishedCell, SheetId};

use crate::document::{DocumentSource, FontFlag, WorkbookDocument};

use super::client::Shared;
use super::protocol::{Command, EditRejectedReason, SheetMeta, StyleAttr, WorkerEvent};

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
    /// A style-only edit — publishes (cache deltas in P5; a repaint now) but needs **no**
    /// recompute: styles don't affect values (component-doc command table).
    StyleOnly,
    /// An add/rename/delete sheet — needs a recompute (can affect formulas) and also emits
    /// `SheetsChanged`.
    SheetOp,
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
        worker.emit(WorkerEvent::Loaded { sheets });

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
        let mut saves: Vec<(PathBuf, u64)> = Vec::new();
        let mut viewport_changed = false;
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
                }
                Command::GetCellContent {
                    sheet,
                    cell,
                    req_id,
                } => reads.push((sheet, cell, req_id)),
                Command::Save { path, req_id } => saves.push((path, req_id)),
                Command::Shutdown => shutdown = true,
                edit @ (Command::SetCellInput { .. }
                | Command::ClearCells { .. }
                | Command::SetStyleAttr { .. }
                | Command::AddSheet
                | Command::RenameSheet { .. }
                | Command::DeleteSheet { .. }
                | Command::Undo
                | Command::Redo) => edits.push(edit),
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

        for (sheet, cell, req_id) in reads {
            let raw = match self.resolve(sheet) {
                Some(idx) => self.doc.cell_content(idx, cell).unwrap_or_default(),
                None => String::new(),
            };
            self.emit(WorkerEvent::CellContent { req_id, raw });
        }

        for (path, req_id) in saves {
            match self.doc.save(&path) {
                Ok(()) => self.emit(WorkerEvent::Saved {
                    req_id,
                    ops_seen: self.ops_seen,
                }),
                Err(error) => self.emit(WorkerEvent::SaveFailed { req_id, error }),
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
        // Clean rejects (no panic risk): input cap + sheet-name re-check.
        let mut valid: Vec<Command> = Vec::new();
        for edit in edits {
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

        self.emit(WorkerEvent::EvalStarted);

        // The IronCalc apply+eval is the only panic-prone region (round-3 D belt-and-braces).
        let started = Instant::now();
        let outcome = {
            let doc = &mut self.doc;
            catch_unwind(AssertUnwindSafe(move || {
                doc.pause_evaluation();
                let mut applied = 0u64;
                let mut needs_eval = false;
                let mut sheets_changed = false;
                let mut engine_errors: Vec<String> = Vec::new();
                for edit in &valid {
                    match apply_one(doc, edit) {
                        Ok(AppliedKind::Cell) => {
                            applied += 1;
                            needs_eval = true;
                        }
                        // Style-only edits don't affect values → skip the recompute.
                        Ok(AppliedKind::StyleOnly) => applied += 1,
                        Ok(AppliedKind::SheetOp) => {
                            applied += 1;
                            needs_eval = true;
                            sheets_changed = true;
                        }
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
                (applied, needs_eval, sheets_changed, engine_errors)
            }))
        };

        self.emit(WorkerEvent::EvalFinished);

        match outcome {
            Ok((applied, needs_eval, sheets_changed, engine_errors)) => {
                for msg in engine_errors {
                    self.emit(WorkerEvent::EditRejected {
                        reason: EditRejectedReason::Engine(msg),
                    });
                }
                if applied == 0 {
                    return false;
                }
                if needs_eval {
                    self.eval_count += 1;
                }
                self.ops_seen += applied;
                self.shared
                    .committed_ops
                    .store(self.ops_seen, Ordering::Release);
                self.publish();
                self.emit(WorkerEvent::Published);
                if sheets_changed {
                    let sheets = self.sheet_metas();
                    self.emit(WorkerEvent::SheetsChanged { sheets });
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
            _ => Ok(()),
        }
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
                        if let Ok(text) = self.doc.formatted_value(idx, CellRef::new(row, col)) {
                            if !text.is_empty() {
                                cells.push(PublishedCell {
                                    row,
                                    col,
                                    display_text: text,
                                    // Number-format colour ([Red]-style) is a palette index in
                                    // the pinned engine, not an RGB; mapping it belongs with the
                                    // Phase-5 style cache (which owns the colour table). P4
                                    // publishes text only (DECISIONS_TO_REVIEW).
                                    text_color: None,
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

    /// The sheet list as `SheetMeta` (stable id + current name), in workbook order.
    fn sheet_metas(&self) -> Vec<SheetMeta> {
        self.doc
            .sheet_properties()
            .into_iter()
            .map(|(id, name)| SheetMeta {
                id: SheetId(id),
                name,
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
        Command::SetStyleAttr { sheet, range, attr } => {
            let idx = resolve_idx(doc, *sheet)?;
            apply_style(doc, idx, *range, *attr)?;
            // Styles don't affect values → no recompute needed (component-doc command table).
            Ok(AppliedKind::StyleOnly)
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

/// Apply a style attribute across a range. Bold/italic/underline are toggles resolved from the
/// current state ("any cell lacks it → set the whole range, else clear it"); `Fill` is a
/// direct set/clear.
fn apply_style(
    doc: &mut WorkbookDocument,
    idx: u32,
    range: CellRange,
    attr: StyleAttr,
) -> Result<(), String> {
    let flag = match attr {
        StyleAttr::Fill(fill) => return doc.set_fill(idx, range, fill),
        StyleAttr::Bold => FontFlag::Bold,
        StyleAttr::Italic => FontFlag::Italic,
        StyleAttr::Underline => FontFlag::Underline,
    };
    // Toggle resolution. P4 reads current state per cell from the engine; P5's resident cache
    // makes this an O(1)-ish map lookup. Ranges are user selections (bounded), not full sheets.
    let mut any_lacking = false;
    'scan: for row in range.rows() {
        for col in range.cols() {
            if !doc.font_flag(idx, CellRef::new(row, col), flag)? {
                any_lacking = true;
                break 'scan;
            }
        }
    }
    doc.set_font_flag(idx, range, flag, any_lacking)
}

/// Resolve a stable [`SheetId`] to a worksheet index, or an engine-style error message.
fn resolve_idx(doc: &WorkbookDocument, sheet: SheetId) -> Result<u32, String> {
    doc.sheet_properties()
        .iter()
        .position(|(id, _)| *id == sheet.0)
        .map(|i| i as u32)
        .ok_or_else(|| format!("no sheet with id {}", sheet.0))
}

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
    use freecell_core::input_cap::{InputRejection, MAX_INPUT_LEN, MAX_NESTING_DEPTH};

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
        };
        if let Some(first) = worker.sheet_metas().first() {
            worker.active_sheet = first.id;
        }
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
}
