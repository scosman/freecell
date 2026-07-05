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

use std::collections::HashSet;

use freecell_core::input_cap::validate_input;
use freecell_core::sheet_name::validate_sheet_name;
use freecell_core::tsv::{paste_fits, tsv_dims};
use freecell_core::{limits, CellKind, CellRange, CellRef, Publication, PublishedCell, SheetId};

use crate::cache;
use crate::document::{DocumentSource, FontFlag, WorkbookDocument};

use super::client::Shared;
use super::protocol::{Command, EditRejectedReason, PasteError, SheetMeta, StyleAttr, WorkerEvent};

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
    Cells { sheet: SheetId, range: CellRange },
    Sheets,
    Undo,
    Redo,
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
    /// Per-op cache touch-sets, aligned 1:1 with IronCalc's undo stack. A new undoable edit
    /// pushes here (clearing `redo_touches`); `Undo` pops here → `redo_touches`; `Redo` the
    /// reverse. Re-reading the popped touch-set keeps the cache in agreement across undo/redo.
    undo_touches: Vec<Touch>,
    /// The redo side of the touch-set history (mirrors IronCalc's redo stack).
    redo_touches: Vec<Touch>,
    /// The range clipboard slot (`architecture.md §6`): `Some` after a copy/cut, replaced by the
    /// next copy/cut, and cleared after a cut is pasted (single-use).
    clipboard: Option<ClipboardSlot>,
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
            undo_touches: Vec::new(),
            redo_touches: Vec::new(),
            clipboard: None,
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
        worker.emit(WorkerEvent::Loaded { sheets });
        worker.emit(WorkerEvent::StyleCacheUpdated {
            sheet: worker.active_sheet,
        });

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
        // Clipboard ops (copy/cut/paste) run one-by-one after the edit batch — a paste is one
        // undo entry, and running it after the batch keeps the undo/touch-set stacks aligned.
        let mut clipboard_ops: Vec<Command> = Vec::new();
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
                clip @ (Command::CopySelection { .. }
                | Command::PasteInternal { .. }
                | Command::PasteTsv { .. }) => clipboard_ops.push(clip),
                edit @ (Command::SetCellInput { .. }
                | Command::ClearCells { .. }
                | Command::SetStyleAttr { .. }
                | Command::SetStylePath { .. }
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

        // Activating a sheet (a viewport switch to it) builds its style/geometry cache on first
        // visit, then stays resident (`components/style_cache.md §Lifecycle`).
        if viewport_changed && self.ensure_active_cache_built() {
            self.emit(WorkerEvent::StyleCacheUpdated {
                sheet: self.active_sheet,
            });
        }

        // Clipboard ops after the edit batch (each is standalone; a paste carries its own eval +
        // publish + one undo entry).
        for clip in clipboard_ops {
            self.apply_clipboard_op(clip);
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

        // Snapshot the sheet list so a change (add/rename/delete — including via undo/redo) is
        // detected by comparison after the batch, driving both `SheetsChanged` and the cache-map
        // reconcile without threading a flag out of every undo path.
        let sheets_before = self.sheet_metas();

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
                if needs_eval {
                    self.eval_count += 1;
                }
                self.ops_seen += applied;
                self.shared
                    .committed_ops
                    .store(self.ops_seen, Ordering::Release);
                self.publish();
                self.emit(WorkerEvent::Published);

                // Mirror the applied ops into the style/geometry cache (re-read touched cells;
                // maintain the undo/redo touch-set stacks), then ship `StyleCacheUpdated` deltas.
                self.mirror_applied_ops(applied_ops, &sheets_before);

                // A changed sheet list (add/rename/delete, or an undo/redo of one) re-syncs the
                // tab bar. Compared by value so undo/redo of a sheet op is caught too.
                let sheets_after = self.sheet_metas();
                if sheets_after != sheets_before {
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
            Command::PasteInternal { sheet, anchor } => self.apply_paste_internal(sheet, anchor),
            Command::PasteTsv {
                sheet,
                anchor,
                text,
            } => self.apply_paste_tsv(sheet, anchor, &text),
            // Only the three clipboard commands are bucketed here.
            _ => {}
        }
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
    fn apply_paste_internal(&mut self, dest: SheetId, anchor: CellRef) {
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
        // Overflow pre-check against the slot's effective (dimension-clamped) source dims.
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
                doc.paste_clipboard(dest_idx, anchor, source_idx, source_range, data, cut)
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
        self.publish();
        self.emit(WorkerEvent::Published);

        // One paste = one engine undo entry → one touch-entry (possibly multi-range), and a
        // fresh edit invalidates the redo stack.
        self.undo_touches.push(Touch::Ranges(touched.clone()));
        self.redo_touches.clear();
        for sheet in self.refresh_cache_cells(&touched) {
            self.emit(WorkerEvent::StyleCacheUpdated { sheet });
        }

        self.emit(WorkerEvent::Pasted {
            sheet: dest,
            range: pasted,
        });
    }

    /// Mirror a batch's applied ops into the resident cache (`components/style_cache.md
    /// §Lifecycle`): maintain the undo/redo touch-set stacks, reconcile the caches map when the
    /// sheet set changed, re-read the touched cells, and emit `StyleCacheUpdated` per changed
    /// sheet. Runs after the eval + publish (styles don't depend on the recompute).
    fn mirror_applied_ops(&mut self, applied_ops: Vec<AppliedOp>, sheets_before: &[SheetMeta]) {
        let mut refresh: Vec<(SheetId, CellRange)> = Vec::new();
        for op in applied_ops {
            match op {
                AppliedOp::Cells { sheet, range } => {
                    self.undo_touches.push(Touch::Cells { sheet, range });
                    self.redo_touches.clear(); // a fresh edit invalidates the redo stack
                    refresh.push((sheet, range));
                }
                AppliedOp::Sheets => {
                    self.undo_touches.push(Touch::Sheets);
                    self.redo_touches.clear();
                }
                AppliedOp::Undo => {
                    if let Some(touch) = self.undo_touches.pop() {
                        refresh.extend(touch_refresh_ranges(&touch));
                        self.redo_touches.push(touch);
                    }
                }
                AppliedOp::Redo => {
                    if let Some(touch) = self.redo_touches.pop() {
                        refresh.extend(touch_refresh_ranges(&touch));
                        self.undo_touches.push(touch);
                    }
                }
            }
        }

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
    fn refresh_cache_cells(&self, refresh: &[(SheetId, CellRange)]) -> Vec<SheetId> {
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
                let mut guard = caches.write();
                if let Some(cache) = guard.get_mut(*sheet) {
                    for row in range.rows() {
                        for col in range.cols() {
                            let _ =
                                cache::refresh_cell(cache, &self.doc, idx, CellRef::new(row, col));
                        }
                    }
                    // Mirror IronCalc's row-height auto-fit over the touched rows (one axis
                    // rebuild). Cheap: a non-band range spans a bounded number of rows.
                    let heights: Vec<(u32, Option<f32>)> = range
                        .rows()
                        .map(|row| (row, cache::row_override_px(&self.doc, idx, row)))
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
    fn build_and_store_cache(&self, sheet: SheetId) -> bool {
        let idx = match self.resolve(sheet) {
            Some(i) => i,
            None => {
                self.shared.caches.write().remove(sheet);
                return false;
            }
        };
        match cache::build_sheet_cache(&self.doc, idx) {
            Ok(built) => {
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

    /// Ensure the active sheet has a resident cache, building it if absent. Returns whether this
    /// call built one (so the caller emits `StyleCacheUpdated`) — `false` if it was already
    /// resident or the build failed (in which case the entry stays absent, not stale).
    fn ensure_active_cache_built(&self) -> bool {
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
        Command::SetStyleAttr { sheet, range, .. } | Command::SetStylePath { sheet, range, .. } => {
            AppliedOp::Cells {
                sheet: *sheet,
                range: *range,
            }
        }
        Command::AddSheet | Command::RenameSheet { .. } | Command::DeleteSheet { .. } => {
            AppliedOp::Sheets
        }
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
        Touch::Sheets => Vec::new(),
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
    use freecell_core::Rgb;

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
            undo_touches: Vec::new(),
            redo_touches: Vec::new(),
            clipboard: None,
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
        let (worker, _rx) = test_worker();
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
            anchor: CellRef::new(0, 2),
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
            anchor: CellRef::new(0, 2),
        }]);
        assert_eq!(value_at(&worker, 0, 2), "7");
        assert_eq!(value_at(&worker, 0, 0), "", "the cut source is cleared");
        drain_events(&rx);

        // The slot is consumed → a second paste has nothing to paste.
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            anchor: CellRef::new(0, 4),
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
        });
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            anchor: CellRef::new(0, 0),
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
            anchor: CellRef::new(limits::MAX_ROWS - 1, 0),
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
            anchor: CellRef::new(0, 2),
        }]);
        assert_eq!(value_at(&worker, 0, 2), "42");
        assert!(
            worker.clipboard.is_some(),
            "a copy stays on the slot after the first paste"
        );
        drain_events(&rx);

        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            anchor: CellRef::new(0, 4),
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
        });
        worker.process_batch(vec![Command::PasteInternal {
            sheet,
            anchor: CellRef::new(0, 0),
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
            anchor: CellRef::new(0, 2),
        }]);
        assert_eq!(value_at(&worker, 0, 2), "5");
        drain_events(&rx);

        // One undo removes the whole paste.
        worker.process_batch(vec![Command::Undo]);
        assert_eq!(value_at(&worker, 0, 2), "", "one undo reverts the paste");
        assert_eq!(value_at(&worker, 0, 0), "5", "the copy source is untouched");
    }
}
