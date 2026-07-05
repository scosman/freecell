---
status: complete
---

# Phase 3: Range clipboard

## Overview

Phase 3 delivers the range clipboard from `functional_spec.md §2` /
`components/clipboard.md` / `architecture.md §6`: Cmd/Ctrl+C/X/V on the focused grid, an
engine-native internal payload (values + formulas with Excel relative-reference adjustment +
styles, one undo step) and plain-text TSV interop with other apps.

The engine already exposes everything needed — **verified against ironcalc_base 0.7.1**
(`~/.cargo/registry/.../ironcalc_base-0.7.1`):

- `UserModel::copy_to_clipboard(&self) -> Result<Clipboard, String>` (`common.rs:1765`) —
  clamps the copied range to `worksheet.dimension()` (full-column/select-all copy is cheap),
  returns `Clipboard { csv, data, sheet, range }` (fields `pub(crate)`, struct **not**
  re-exported, but `#[derive(Serialize, Deserialize)]`).
- `UserModel::paste_from_clipboard(source_sheet, source_range: (i32,i32,i32,i32), clipboard:
  &ClipboardData, is_cut) -> Result<(),String>` (`common.rs:1811`) — pastes at the engine's
  **selected view** anchor, Excel-adjusts refs (`extend_copied_value` / `move_cell_value_to_area`),
  on cut clears the source, pushes **one** diff list (one undo step), then re-selects the pasted
  area. `ClipboardData = HashMap<i32,HashMap<i32,ClipboardCell>>` **is** re-exported
  (`ironcalc_base::ClipboardData`) and is `Deserialize`.
- `UserModel::paste_csv_string(area: &Area, csv: &str)` (`common.rs:1926`) — **tab-delimited**
  (`ReaderBuilder::delimiter(b'\t')`), `has_headers(false)`, values-as-user-input, one diff list,
  handles `\r\n`; uses only `area.{sheet,row,column}` (width/height ignored) and re-selects the
  pasted area.
- `UserModel::set_selected_sheet` / `set_selected_cell` / `set_selected_range` (`ui.rs:80,92,118`)
  + `get_selected_view` (`ui.rs:51`) — all on `views[0]` (`view_id == 0`, `model.rs:957`), so the
  reads/writes agree. `set_selected_range` requires the selected cell to sit on a range **edge**;
  we always set the anchor to the range's top-left (always on the edge, incl. full row/col/select-all).

So no roadblock — the architecture's source audit holds.

**Coordinate note.** Protocol commands use the codebase's `SheetId` + `CellRef`/`CellRange`
(0-based), matching every existing command; the `document.rs` adapter is the only place that
converts to IronCalc 1-based coords (via the existing `to_engine_coords`). (`clipboard.md`
sketches `sheet: u32` / `anchor: (i32,i32)` — same values, idiomatic types.)

## Steps

1. **`freecell-core/src/tsv.rs` (new)** — pure TSV dims + overflow predicate (unit-tested
   headless), used worker-side. Register `pub mod tsv;` in `lib.rs`.
   ```rust
   /// (width, height) of a TSV block, parsed by the SAME `csv` crate + reader config the
   /// engine's `paste_csv_string` uses (delimiter `\t`, default CRLF terminator + `"` quoting,
   /// `flexible(true)`). A provable UPPER BOUND on the engine's written rectangle in BOTH dims
   /// (it drops blank/ragged records → fewer rows; writes only the first record's width →
   /// fewer columns). Empty / terminator-only text → (0, 0).
   pub fn tsv_dims(text: &str) -> (u32, u32);
   /// Whether a `width × height` block pasted with its top-left at `anchor` (0-based) fits
   /// inside the Excel-max sheet (no partial paste past the edge).
   pub fn paste_fits(anchor: CellRef, width: u32, height: u32) -> bool;
   ```
   > CR follow-ups: an `\n`-only split undercounted bare-`\r` line endings (height), and a
   > physical-line scan undercounted quoted-newline field widths (width) — each bypassed the
   > overflow guard → partial un-undoable paste. Computing dims through the engine's own `csv`
   > parser eliminates the divergence class in both dims (regression tests
   > `dims_bare_cr_counts_records`, `dims_quote_aware_width`, and the `overflow_predicate`
   > quoted-payload assertion). `freecell-core` gains a `csv` dep (pure-Rust, headless-safe).

2. **`freecell-engine/Cargo.toml`** — add `serde_json.workspace = true` (needed to
   `to_value` the un-nameable `Clipboard` and `from_value` a `ClipboardData`; `serde_json` is
   already a `[workspace.dependencies]` pin).

3. **`freecell-engine/src/document.rs`** — the only IronCalc-touching clipboard code:
   ```rust
   pub(crate) struct CopiedRange { pub tsv: String, pub data: serde_json::Value,
                                   pub range: (i32,i32,i32,i32) }
   /// Set view selection to `range` then copy; returns the trimmed TSV, the serialized
   /// `ClipboardData` (as JSON — the concrete cell type is private), and the engine's
   /// effective (dimension-clamped) source range.
   pub(crate) fn copy_range(&mut self, sheet_idx: u32, range: CellRange)
       -> Result<CopiedRange, String>;
   /// Set the dest view selection to `anchor`, deserialize `data_json` → `ClipboardData`, and
   /// paste (undoable, one diff list). Caller pauses eval around it.
   pub(crate) fn paste_clipboard(&mut self, dest_idx: u32, anchor: CellRef, source_idx: u32,
       source_range: (i32,i32,i32,i32), data_json: &serde_json::Value, cut: bool)
       -> Result<(), String>;
   /// Set the dest view selection to `anchor` and paste a tab-delimited TSV at it (undoable).
   pub(crate) fn paste_tsv(&mut self, dest_idx: u32, anchor: CellRef, text: &str)
       -> Result<(), String>;
   /// The engine's current view selection as a 0-based `CellRange` (read back after a paste to
   /// mirror the pasted area into FreeCell's `SelectionModel`).
   pub(crate) fn selected_range_0based(&self) -> CellRange;
   ```
   `copy_range` extracts `data`/`range`/`csv` from `serde_json::to_value(&clip)` (the struct is
   `Serialize` but not nameable). A private `set_view_selection(sheet_idx, range)` does
   `set_selected_sheet` → `set_selected_cell(top-left)` → `set_selected_range(range)`.

4. **`freecell-engine/src/worker/protocol.rs`** — three commands + three events + the paste
   error enum:
   ```rust
   Command::CopySelection { sheet: SheetId, range: CellRange, cut: bool }   // reply: CopyReady
   Command::PasteInternal { sheet: SheetId, anchor: CellRef }               // reply: Pasted / PasteRejected
   Command::PasteTsv      { sheet: SheetId, anchor: CellRef, text: String } // reply: Pasted / PasteRejected

   WorkerEvent::CopyReady { tsv: String }
   WorkerEvent::Pasted { sheet: SheetId, range: CellRange }
   WorkerEvent::PasteRejected { reason: PasteError }
   pub enum PasteError { Overflow, NothingToPaste }
   ```
   Re-export `PasteError` from `worker/mod.rs` + `lib.rs`.

5. **`freecell-engine/src/worker/run.rs`** — worker-side handling:
   - `struct ClipboardSlot { sheet: SheetId, range: (i32,i32,i32,i32), data: serde_json::Value,
     cut: bool }` and a `clipboard: Option<ClipboardSlot>` field (init `None` in both constructors).
   - `process_batch`: bucket the three commands into `clipboard_ops` and run them **after** the
     edit batch (keeps the undo/touch-set stacks aligned with the engine undo stack), before
     reads/saves. Add them to the exhaustive routing match (no catch-all).
   - `Touch::Ranges(Vec<(SheetId, CellRange)>)` — a paste is one undo entry that may touch two
     ranges (pasted dest + cut source, possibly cross-sheet). `mirror_applied_ops`'s Undo/Redo
     arms push each range into `refresh`.
   - `apply_copy(sheet, range, cut)`: resolve idx; guarded `copy_range`; stash the slot
     (`sheet`, engine range, `data`, `cut`); emit `CopyReady { tsv }`. Panic → `handle_caught_panic`;
     engine `Err` → log (copy is not dialog-worthy).
   - `run_guarded_paste(f)`: shared scaffold — `EvalStarted`; `catch_unwind { pause; f(doc);
     resume; evaluate; read selected_range_0based }`; `EvalFinished` → `PasteOutcome::{Applied(range),
     EngineError, Panicked}` (mirrors `apply_edit_batch`'s pause/catch/eval + recovery).
   - `apply_paste_internal(dest, anchor)`: `None` slot → `PasteRejected{NothingToPaste}`;
     degraded → `EditRejected{Degraded}`; source dims from `slot.range`, `!paste_fits` →
     `PasteRejected{Overflow}`; else `run_guarded_paste(|d| d.paste_clipboard(...))`. On `Applied`:
     `eval_count += 1`, `ops_seen += 1`, store committed_ops, `publish` + `Published`, push
     `Touch::Ranges([dest pasted, source(on cut)])` + clear redo, refresh those ranges +
     `StyleCacheUpdated`, emit `Pasted`; on cut clear the slot (single-use).
   - `apply_paste_tsv(dest, anchor, text)`: degraded check; `tsv_dims`; empty → no-op;
     `!paste_fits` → `PasteRejected{Overflow}`; else `run_guarded_paste(|d| d.paste_tsv(...))`,
     same post-processing with `Touch::Ranges([dest pasted])`.

6. **`freecell-app/src/grid/input.rs`** — extend `GridKeyCommand` with `Copy`, `Cut`, `Paste`;
   `command_for_key` maps `secondary && !shift` + `c`/`x`/`v` to them (before the plain-key
   match, so it never collides with type-to-replace, which needs no modifier).

7. **`freecell-app/src/grid/mod.rs`** — `GridEvent::Copy { cut: bool }` and `GridEvent::Paste`.

8. **`freecell-app/src/grid/view.rs`** — `handle_key_down` dispatches the three new commands to
   `emit(GridEvent::Copy{cut})` / `emit(GridEvent::Paste)`. The existing early-return when the
   in-cell editor is open, and the fact that a data-row edit steals focus (grid not focused),
   already scope the shortcuts to "grid focused, not editing" — the data-row / in-cell inputs
   keep native text clipboard behavior for free.

9. **`freecell-app/src/shell/clipboard.rs` (new)** — the UI coordinator:
   ```rust
   pub struct ClipboardCoordinator { last_copy_text: Option<String> }
   impl ClipboardCoordinator {
       pub fn copy(&mut self, sheet, sel: SelectionModel, cut: bool, client: &DocumentClient); // → CopySelection
       pub fn on_copy_ready(&mut self, tsv: String, cx: &mut App);   // write system clipboard + record
       pub fn paste(&mut self, sheet, anchor: CellRef, client: &DocumentClient, cx: &mut App);
   }
   ```
   `paste` reads `cx.read_from_clipboard()?.text()`: empty/None → no-op; `== last_copy_text` →
   `PasteInternal`; else `last_copy_text = None` + `PasteTsv { text }`. Register `mod clipboard;`.

10. **`freecell-app/src/shell/window.rs`** — wire it in:
    - `SinkShared` gains `clipboard: RefCell<ClipboardCoordinator>`.
    - `make_grid_sink`: `GridEvent::Copy{cut}` → `shared.clipboard.borrow_mut().copy(active_sheet,
      shared.last_selection.get(), cut, &client)`; `GridEvent::Paste` → commit any pending edit via
      `chrome.on_edit_commit_requested` (abort if it stays editing — cap-rejected), then
      `shared.clipboard.borrow_mut().paste(active_sheet, last_selection.range().start, &client, cx)`.
    - `on_worker_event` (exhaustive): `CopyReady{tsv}` → `clipboard.on_copy_ready(tsv, cx)`;
      `Pasted{sheet,range}` (if `sheet == active`) → set grid selection to the pasted rect +
      sync `last_selection` + `chrome.on_selection_changed`; `PasteRejected{Overflow}` → error
      dialog "Paste doesn't fit…"; `PasteRejected{NothingToPaste}` → log only.

## Tests

Unit — `freecell-core::tsv`:
- `dims_simple`, `dims_trailing_newline`, `dims_ragged`, `dims_single_token` — `tsv_dims`.
- `overflow_predicate` — `paste_fits` at/over the sheet edge (incl. Excel-max corner).

Worker unit — `freecell-engine/src/worker/run.rs` (`test_worker()` + `process_batch`):
- `cut_slot_single_use` — second `PasteInternal` after a cut → `PasteRejected{NothingToPaste}`.
- `paste_internal_overflow_rejected` — anchor near the sheet edge → `PasteRejected{Overflow}`,
  no cells written.
- `copy_then_paste_internal_mirrors_and_selects` — copy A1:A2, paste at C1, worker publishes the
  pasted cells + emits `Pasted` with the C1:C2 range + a `StyleCacheUpdated`.
- `paste_tsv_overflow_rejected`, `paste_tsv_writes_cells` — TSV path dims + write.
- `copy_reply_carries_tsv` — `CopySelection` emits `CopyReady` whose TSV is tab/newline joined.

Engine integration — `freecell-engine/tests/worker_seam.rs` (real `DocumentClient`):
- `copy_paste_values_and_styles_roundtrip` — bold cell copied → pasted keeps value + bold.
- `copy_paste_adjusts_relative_refs` — `=A1` copied one row down pastes as `=A2`; `$A$1` holds.
- `cut_paste_moves_and_source_cleared` — cut B2:B3 → C-column; source cleared, one publish.
- `paste_undo_single_step` — one `Undo` restores pre-paste state (incl. the cut source).
- `full_column_copy_clamps_to_used_range` — a full-column `CopySelection` copies only
  `dimension()` cells (cheap; no 1M-row materialization / no hang under the test timeout).
- `cross_sheet_internal_paste` — copy on Sheet1, paste on Sheet2.
- `paste_tsv_dims_and_types` — `"1\t2\n=A1\ttrue\n"` → number / formula(=A1→2) / bool cells.
- `paste_tsv_crlf_and_ragged_rows` — `\r\n` + a short row: no panic, dims sane.

UI-level — `freecell-app` window test (detached client + injected events / test clipboard):
- `copy_writes_system_clipboard` — `CopyReady` fold writes the TSV to `cx`'s clipboard.
- `paste_prefers_internal_when_text_matches` — clipboard text == last copy → `PasteInternal`.
- `paste_falls_back_to_tsv` — foreign clipboard text → `PasteTsv` with that text.
  (Assert via the coordinator's decision, keeping it worker-independent under the detached client.)

## Render / baselines

Paste changes cell values + styles that render, but it reuses the existing publication + style
cache render path (no new `RenderCase` fields). No new render cases are needed and no baselines
change; `cargo test --workspace` (no `FREECELL_RENDER`) stays green. Recorded in
`DECISIONS_TO_REVIEW.md`.

## Decisions to record (DECISIONS_TO_REVIEW.md)

- External **TSV paste** feeds foreign text to the engine as user input; per `architecture.md §8`
  it relies on `catch_unwind` + the 64 MiB worker stack rather than the per-cell input cap
  (which only re-checks `SetCellInput`). Flagged as a residual round-3 D surface for owner review.
- `ClipboardSlot.sheet` stores the stable `SheetId` (resolved to an index at paste time), not the
  volatile worksheet index, so a copy survives sheet add/reorder before paste.
- TSV paste of empty tokens leaves existing cells untouched (engine skips them) — Excel clears;
  accepted deviation (already noted in `clipboard.md`).
