---
status: complete
---

# Component: Engine Worker (`freecell-engine`)

The per-window thread that owns the IronCalc `UserModel` and implements the validated
SP1 seam (`experiments/round-2/01-async-interop/findings.md`, carried to `UserModel`
by `experiments/round-3/A-cache-sync/findings.md`). Everything IronCalc lives behind
this component; no other code imports `ironcalc`.

## Purpose and scope

**Does:** load/save xlsx; apply edit batches; run `evaluate()`; publish viewport
snapshots; build/update the style & geometry caches (see `style_cache.md` — the cache
logic is its own component, but it executes on this thread); answer cell-content
reads; enforce robustness (stack size, catch_unwind, input-cap re-check); dirty-op
accounting.

**Does not:** render, own selection, decide UI policy (dialogs/prompts), parse user
intent (commands arrive pre-validated and typed).

## Public interface

```rust
pub struct DocumentClient { /* cheap handle owned by the window */ }

impl DocumentClient {
    /// Spawns the worker (64 MiB stack). `source`: NewWorkbook | OpenFile(PathBuf).
    pub fn spawn(source: DocumentSource) -> (DocumentClient, WorkerEventReceiver);
    pub fn send(&self, cmd: Command);            // non-blocking, never fails visibly;
                                                 // worker-gone => EditRejected via event closed-channel handling
    pub fn publication(&self) -> Arc<Publication>;      // latest swapped snapshot
    pub fn caches(&self) -> Arc<RwLock<SheetCaches>>;
    pub fn generation(&self) -> Arc<AtomicU64>;
}
```

`Command` / `WorkerEvent` enums as sketched in `architecture.md §2` — this doc is
authoritative for semantics:

| Command | Undoable | Triggers eval | Notes |
|---|---|---|---|
| `SetCellInput` | ✅ | ✅ | maps to `set_user_input`; input string re-checked against the cap (defense in depth), reject → `EditRejected` |
| `ClearCells` | ✅ | ✅ | range iterated → engine cell-clear API (contents only, keep style) |
| `SetStyleAttr` | ✅ | ❌ (no recompute needed; styles don't affect values) — still publishes cache deltas | engine range-style path (`font.b`, `font.i`, `font.u`, `fill.fg_color`); multi-cell toggle resolution (any-lacking → set-all) computed worker-side from cache state |
| `AddSheet` / `RenameSheet` / `DeleteSheet` | ✅ (where engine history covers; else documented) | ✅ (delete/rename can affect formulas) | validation already done UI-side; worker re-validates name, rejects politely |
| `Undo` / `Redo` | — | ✅ | engine history; after applying, re-read styles/geometry for the affected cells (history entry's recorded touch-set) and ship cache deltas |
| `SetViewport` | — | ❌ | stores the overscanned window; triggers immediate re-publish from current model state (cheap, SP4: <2 ms/1.8k cells; runs between evals) |
| `GetCellContent` | — | ❌ | `get_cell_content` → `CellContent` reply |
| `Save` | — | ❌ | serialize → temp file in target dir → fsync → atomic rename → `Saved{ops_seen}` |
| `Shutdown` | — | — | drop model, exit loop |

## Internal design

### Main loop (drain → apply → eval → publish)

```
loop {
    cmd = rx.recv()                      // block when idle
    batch = [cmd] + rx.try_iter()        // DRAIN: coalescing happens here
    split batch into: edits[], control[] // control = viewport/save/read/shutdown
    if !edits.is_empty() {
        emit EvalStarted
        result = catch_unwind(|| {
            pause_evaluation-style batching if available, else sequential apply
            for e in edits { apply(e) }  // each may auto-eval in UserModel;
                                         // use the batch/pause API confirmed in round-3 A
            evaluate-if-needed
        })
        match result {
            Ok(_)  => { ops_seen += edits.len() }
            Err(_) => { emit EditRejected; /* model may be poisoned: */ rebuild_from_last_save_or_mark_degraded() }
        }
        repull viewport → build Publication → swap Arc → generation.fetch_add(1)
        emit EvalFinished + Published
        apply mirrored style-cache deltas (style_cache.md) → emit StyleCacheUpdated
    }
    handle control[] (viewport re-publish, save, reads, shutdown)
}
```

Decisions locked here:

- **Coalescing** = full channel drain before applying (SP1's 30→1 measured behavior).
- **Publish-then-bump ordering**: `Publication` Arc swap happens strictly before the
  generation increment (SP1 race fix); UI reads generation first, then the Arc — safe
  either way, but the ordering guarantees a bump always has fresh data behind it.
- **Eval strategy**: rely on `UserModel`'s auto-evaluate per edit *unless* the
  batch API (`pause_evaluation` / `resume_evaluation`, confirmed present in round-3 A)
  is usable — use it for multi-edit batches so a batch costs one eval. This is the
  documented intent of that API.
- **catch_unwind poisoning policy**: a caught panic means the model state is suspect.
  MVP policy: emit `EditRejected` and set a `degraded` flag; if the model still
  responds to a probe read, continue (D showed panics don't occur today — this path
  is belt-and-braces); if unresponsive/poisoned, emit `WorkerDegraded` and the UI
  shows the error bar with Save As (serializes from whatever `get_model()` still
  returns). Do not silently continue after a second panic: stop accepting edits.
- **Stack**: `thread::Builder::new().name("eval-worker").stack_size(64 << 20)`.
- **Input cap**: `freecell_core::formula_cap::validate(&input) -> Result<(), CapError>`
  — length > 8192 chars or paren-nesting depth > 64. Checked in the UI before send
  *and* re-checked here (the worker is the security boundary for the abort).

### Publication build

For the stored overscanned window (~3× visible, clamped to sheet bounds) on the active
sheet: iterate populated cells in range (engine iteration API from the B matrix;
worst-case per-cell probe over the window is acceptable at ≤ ~20k cells given SP4's
read costs — measure in the perf harness). Per cell: `get_formatted_cell_value` (+
format color if exposed via the `Formatted` path) →
`PublishedCell { row, col, display_text, text_color }`. Empty cells are
omitted (the grid defaults them). `Publication { sheet, rows, cols, cells,
generation }`. Raw formula text is **not** published — the formula bar requests it
per selection via `GetCellContent` (architecture-round call).

`ArcSwapish` = `parking_lot::Mutex<Arc<Publication>>` or `arc_swap::ArcSwap` — use
`arc_swap` (tiny, well-known crate) unless dependency-count pressure says otherwise.

### File I/O

- **Open**: `load_from_xlsx(path, locale, tz)` (defaults: `en`, system tz) → wrap
  `UserModel::from_model` → build active-sheet caches → publish → `Loaded{sheets}`.
  Errors mapped to `LoadError::{NotXlsx, Corrupt, PasswordProtected, Io}` with the
  underlying message preserved for the dialog.
- **New**: `new_empty` equivalent (one sheet "Sheet1", locale/tz as above).
- **Save**: `get_model()` → xlsx writer → `NamedTempFile` in the destination
  directory → write → fsync → `persist` (atomic rename). On any error the original
  file is untouched → `SaveFailed{reason}`. Success → `Saved{ops_seen: current}`.
- Save serializes with evals on this thread — acceptable (writes are user-initiated;
  the indicator shows). Save does NOT trigger an eval.

### Threading & channels

- UI → worker: `std::sync::mpsc::Sender<Command>` (unbounded; commands are small).
- Worker → UI: `smol::channel::unbounded::<WorkerEvent>()`; the window owns a gpui
  foreground task: `while let Ok(ev) = rx.recv().await { entity.update(cx, |w, cx|
  w.on_worker_event(ev, cx)) }`.
- Shared: `Arc<AtomicU64>` generation, `arc_swap` publication,
  `Arc<RwLock<SheetCaches>>` (written from the worker thread only).

## Dependencies

Depends on: `freecell-core`, `ironcalc`/`ironcalc_base` (=0.7.1), `arc_swap`,
`parking_lot` (or std RwLock — pick parking_lot, gpui already pulls it), `tempfile`,
`smol` (channel), `thiserror`, `tracing`. Depended on by: `freecell-app`,
`render-tests` (fixture documents), perf harness.

## Test plan (Linux CI — all headless)

Port the SP1/round-3 test patterns as real integration tests:

- `coalesce_n_edits_one_eval` (+ negative control asserting the counter instrumentation
  can fail): flood 30 edits, assert 1 eval via an eval-counter probe.
- `publish_before_bump`: subscriber thread spins on generation; on every bump the
  publication generation must equal the counter (run under load).
- `staleness_bound`: edit during a long eval (large fixture) → value appears within
  the next publish; UI-side reads never block.
- `viewport_republish_on_scroll` / `sheet_switch_publishes_new_sheet`.
- `input_cap_rejects_abort_reproducers`: round-3 D's ~490-depth and ~2832-term
  reproducer strings are **rejected** by the cap (and never reach the engine); plus
  boundary cases 64/65 depth, 8192/8193 length.
- `catch_unwind_recovery`: inject a panicking apply via a test-only command; assert
  `EditRejected`, worker alive, subsequent edits work.
- `dirty_ops_accounting`: edit→save→edit→undo sequences; `Saved.ops_seen` semantics.
- `open_save_roundtrip_*`: values / formulas / styles / number formats / multi-sheet /
  sheet-rename; via fixture files + reopen-and-compare (leaning on SP5 fidelity).
- `save_atomic_on_failure`: unwritable destination → original file byte-identical.
- `open_failures`: corrupt zip, empty file, wrong extension content, password file →
  typed errors, no panic.
- `formula_errors_are_values`: `#DIV/0!`, `#CIRC!` (1000-ring cycle fixture from D)
  return as display text promptly.
