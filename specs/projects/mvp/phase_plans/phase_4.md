---
status: complete
---

# Phase 4: Eval worker seam

## Overview

Build the per-window **eval worker** in `freecell-engine`: the thread that owns the
IronCalc `UserModel` (via the Phase-3 `WorkbookDocument`) and implements the validated SP1
seam carried to `UserModel` by round-3 A. This phase delivers the command/event loop,
drain-coalescing (N edits → 1 eval), publish-then-bump generation ordering, the viewport
`Publication` build, the 64 MiB worker stack, the worker-side input-cap re-check (the
security boundary for the round-3 D abort class), `catch_unwind` + the degraded policy,
dirty-op accounting, and the full seam test suite **including a negative control** that
proves the coalescing instrumentation can register failure.

Everything IronCalc stays behind `freecell-engine`. The public seam (`DocumentClient`,
`Command`, `WorkerEvent`) exposes only `freecell-core` / `std` / Phase-3 typed-error types —
never an `ironcalc` type. Cache **build/deltas** are explicitly Phase 5; this phase creates
the shared `Arc<RwLock<SheetCaches>>` surface (empty) so the seam is complete.

## Steps

1. **Deps** (`app/Cargo.toml` + `crates/freecell-engine/Cargo.toml`): add `arc-swap = "1"`
   (publication `ArcSwap`), `async-channel = "2"` (worker→UI event channel — the type
   `smol::channel` re-exports; keeps the headless engine free of an async runtime),
   `parking_lot = "0.12"` (the shared `RwLock<SheetCaches>`; no lock poisoning on a worker
   panic), and `tracing` (apply/eval/publish debug timings). All already resolved in
   `Cargo.lock` (pulled transitively), so no new fetch.

2. **`document.rs` — worker-facing edit/read methods** (keep all IronCalc mechanism in the
   single adapter): `pause_evaluation`/`resume_evaluation`/`evaluate` wrappers;
   `set_cell_input(sheet_idx, cell, &str)`; `clear_contents(sheet_idx, CellRange)`;
   `font_flag(sheet_idx, cell, FontFlag)->bool` + `set_font_flag(sheet_idx, CellRange,
   FontFlag, bool)` (toggle mechanism; `FontFlag {Bold,Italic,Underline}` → `font.b/.i/.u`);
   `set_fill(sheet_idx, CellRange, Option<Rgb>)` (`fill.fg_color` = hex or `""` to clear);
   `add_sheet`/`rename_sheet(idx,&str)`/`delete_sheet(idx)`; `undo`/`redo`;
   `sheet_properties()->Vec<(u32 sheet_id, String name)>` (stable-id ↔ index map source).
   Each mutator returns `Result<(), String>` (raw engine message; the worker maps it).

3. **`worker/protocol.rs` — the boundary contract** (engine-free public types):
   - `Command` per the component-doc table: `SetCellInput{sheet,cell,input}`,
     `ClearCells{sheet,range}`, `SetStyleAttr{sheet,range,attr}`, `AddSheet`,
     `RenameSheet{sheet,name}`, `DeleteSheet{sheet}`, `Undo`, `Redo`,
     `SetViewport{sheet,rows,cols}`, `GetCellContent{sheet,cell,req_id}`,
     `Save{path,req_id}`, `Shutdown`. `sheet: SheetId` throughout (stable id; see Decisions).
     `#[cfg(test)] TestPanic` variant for the catch_unwind injection.
   - `StyleAttr { Bold, Italic, Underline, Fill(Option<Rgb>) }` — bold/italic/underline are
     **toggles** resolved worker-side (component doc: "any-lacking → set-all"); `Fill` is a
     direct set/clear.
   - `WorkerEvent`: `Loaded{sheets}`, `LoadFailed{error:LoadError}`, `Published`,
     `EvalStarted`, `EvalFinished`, `CellContent{req_id,raw}`, `Saved{req_id,ops_seen}`,
     `SaveFailed{req_id,error:SaveError}`, `EditRejected{reason}`, `StyleCacheUpdated{sheet}`
     (defined now; emitted in P5), `SheetsChanged{sheets}`, `WorkerDegraded{reason}`.
   - `SheetMeta { id: SheetId, name: String }`; `EditRejectedReason { InputCap(InputRejection),
     InvalidSheetName(SheetNameError), Engine(String), EnginePanic, Degraded }`.

4. **`worker/client.rs` — `DocumentClient` + shared surfaces**: `Shared { publication:
   ArcSwap<Publication>, generation: AtomicU64, committed_ops: AtomicU64, caches:
   Arc<RwLock<SheetCaches>> }`. `spawn(DocumentSource) -> (DocumentClient,
   WorkerEventReceiver)` builds the std `mpsc<Command>` + `async_channel<WorkerEvent>`,
   the `Shared`, and the 64 MiB thread (`WORKER_STACK_SIZE = 64<<20`, name `eval-worker`)
   which loads the document (on the worker thread), emits `Loaded`/`LoadFailed`, then runs
   the loop. `send`, `publication`, `caches`, `generation`, `committed_ops` getters.
   `WorkerEventReceiver` newtype (recv/recv_blocking/try_recv/recv_timeout; hides
   async_channel).

5. **`worker/run.rs` — the loop** (`Worker` owns `WorkbookDocument` + `Arc<Shared>` +
   `event_tx` + `active_sheet` + stored viewport + `ops_seen` + `eval_count` + `panic_count`
   + `degraded`). `run(cmd_rx)`: `recv()` (park when idle) → `[first] + try_iter()` (DRAIN =
   coalescing) → `process_batch`. `process_batch`:
   - apply `SetViewport`s → update `active_sheet` + clamped stored viewport;
   - pre-validate edits **outside** `catch_unwind` (input-cap re-check for `SetCellInput`;
     sheet-name re-check for `RenameSheet`) → `EditRejected` + drop invalid;
   - if `degraded`: `EditRejected{Degraded}` for surviving edits;
   - else if surviving edits: `EvalStarted` →
     `catch_unwind(AssertUnwindSafe(pause; apply each; resume; if applied>0 { evaluate();
     eval_count+=1 }))` → `Ok`: `ops_seen += applied`, mirror `committed_ops`; `Err`: reset
     pause, degraded policy (probe read; 1st-panic-responsive → `EditRejected{EnginePanic}`,
     else / 2nd panic → `WorkerDegraded`) → `EvalFinished` → **publish-then-bump** →
     `Published` → `SheetsChanged` if a sheet op applied;
   - else if only a viewport changed: republish (no eval) → `Published`;
   - handle `GetCellContent` → `CellContent`; `Save` → atomic save → `Saved{ops_seen}` /
     `SaveFailed`; `Shutdown` → break.
   - **publish-then-bump**: `gen = generation+1`; build `Publication{active_sheet, clamped
     rows/cols, generation:gen, cells}` (iterate the clamped overscan window, skip empties;
     `text_color:None` in P4 — see Decisions); `publication.store(Arc::new(pub))`
     **(Release)** then `generation.store(gen)` **(Release)**.

6. **`lib.rs`**: `pub mod worker;` + re-export `DocumentClient`, `WorkerEventReceiver`,
   `Command`, `WorkerEvent`, `StyleAttr`, `SheetMeta`, `EditRejectedReason`.

## Tests

In-crate unit tests (`worker/run.rs`, deterministic — drive the loop/`process_batch`
directly, no thread-timing):
- `drain_coalesces_burst_into_one_eval`: 30 `SetCellInput` + `Shutdown` pushed to the
  channel, `run()` drains them into one batch → `eval_count == 1`; final values correct.
- `negative_control_eval_counter_detects_no_coalesce` (**the negative control**): the same
  `process_batch`/`eval_count` fed 30 single-edit batches → `eval_count == 30`, proving the
  `== 1` assertion above is discriminating (the counter is not hard-wired to 1).
- `catch_unwind_recovery`: `TestPanic` → `EditRejected{EnginePanic}`, worker not degraded,
  a following real edit applies (worker alive).
- `second_panic_degrades_and_refuses_edits`: two `TestPanic`s → `WorkerDegraded`; a later
  edit → `EditRejected{Degraded}`; a `Save` still succeeds (escape hatch).
- `worker_side_cap_rejects_abort_reproducers`: the D ~490-depth / ~2832-term reproducers +
  64/65 depth + 8192/8193 length via `SetCellInput` → `EditRejected{InputCap}`, and the cell
  is never written (engine untouched).
- `ops_seen_accounting`: edit→edit→undo → `committed_ops`/`Saved.ops_seen` = 3 (undo counts).
- `publication_build_skips_empties_and_formats`: values fixture edits → publication cells
  carry engine display text; empty cells omitted; out-of-window cells absent.
- `style_toggle_any_lacking_sets_all`: mixed bold/plain range → toggle sets all bold; a
  second toggle clears all.

Integration tests (`tests/worker_seam.rs`, via public `DocumentClient`):
- `spawn_new_workbook_emits_loaded` / `spawn_open_bad_file_emits_load_failed` (corrupt /
  empty / ole / text → typed `LoadFailed`, no panic/hang).
- `set_viewport_publishes` / `sheet_switch_publishes_new_sheet` / `edit_updates_publication`.
- `publish_before_bump` (concurrent reader spins on generation; `publication.generation >=
  observed generation` always — catches a reversed bump/store).
- `staleness_bound` (edit becomes visible within the next publish; `publication()` reads are
  wait-free `arc_swap` loads).
- `formula_errors_are_values` (`=1/0` → publication `#DIV/0!`; circular fixture → `#CIRC!`).
- `save_through_worker_roundtrips` + `save_atomic_on_failure` (rename-onto-directory failure
  → `SaveFailed`, original untouched — root-proof).
- `get_cell_content_replies` (raw `=formula` via request/response).
- `sheet_add_rename_delete_emit_sheets_changed`; `undo_redo_through_worker`.
</content>
</invoke>
