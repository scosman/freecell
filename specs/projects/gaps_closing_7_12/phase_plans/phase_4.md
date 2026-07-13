---
status: complete
---

# Phase 4: Paste Values (⌘⇧V)

## Overview

Add **Paste Values** — the minimum paste-special (values only; no formulas, no formatting;
the target keeps its own formatting). Bind the reserved-but-unbound `Shift+V`
(`secondary && shift`) → a new `GridEvent::PasteValues` → `Command::PasteValues` → a
values-only paste in the worker. Phase 5 (context menu) reuses the same `GridEvent::PasteValues`.

Source = the **internal clipboard** (a prior in-app ⌘C/⌘X). When only an external TSV is on the
system clipboard, ⌘⇧V falls back to a normal `PasteTsv` (external TSV is already values —
nothing to strip). One paste-values = **one undo step**. Sizing/overflow follow the existing
paste rules (single-cell/exact-divisor fills the selection; block from the anchor; oversized →
Overflow).

## Mechanism decision (D5.1 / D5.2) — and a justified deviation from architecture §5

D5.1 = **values only, no number format**. D5.2 = **FreeCell-side, no fork op**. Both honored.

Architecture §5 proposed *reusing IronCalc's clipboard `csv` re-pasted through `paste_csv_string`*.
Grounding the code revealed that `csv` is built from `get_formatted_cell_value` (the **formatted
display** string), and `set_user_input` (which `paste_csv_string` calls per token) runs
`parse_formatted_number`, which **infers and applies a number format** for grouped/currency/percent/
date-looking strings. Re-pasting the formatted csv would therefore (a) mis-type formatted numbers
and (b) **apply a format**, violating D5.1's "no formatting, target keeps its own". So this phase
keeps the architecture's *intent* (FreeCell-side, computed values, one undo step, existing paste
sizing) but captures the **typed underlying computed values** at copy time and writes them through
the batched one-undo `set_user_inputs` path with explicit per-cell literal handling. This is the
"literal-write path" the architecture's own edge-case note offered as the alternative to a `'`
prefix, and it makes tiling trivial. No fork change.

Per-cell computed value → paste **token** (re-parsed by `set_user_input` on paste):
- `Number(n)` → plain unformatted `f64` `to_string()` (never grouped/scientific) → re-parses to the
  same number with **no** format inferred (`parse_formatted_number` returns `None`) → target keeps
  its format. Full precision (better than the formatted csv's rounded display).
- `Boolean` → `TRUE`/`FALSE` → re-parses to the boolean.
- An **error** cell (`get_cell_type == ErrorValue`, whose value surfaces as its error string) →
  the error string as-is → re-parses back to the same error value (spec: "pastes as that error
  value").
- Any other **string** → apostrophe-quoted (`'…`) → forced literal text. This is the load-bearing
  edge case: a value that merely *looks* like a formula (`=x`, `+x`, `-x`, `@x`) is never
  re-interpreted as a formula, and text-vs-number typing is preserved (a text `"12"` stays text).
  The `'` sets only the quote_prefix marker; number format / font / fill / borders are untouched.
- Empty → `""` → clears the target cell (matches pasting a blank source cell).

## Steps

1. **`freecell-engine/src/document.rs`**
   - Add `values: Vec<Vec<String>>` to `CopiedRange` (the copied block's computed-value tokens,
     row-major over the effective clamped source rectangle).
   - `copy_range`: after extracting the clamped `range`, build `values` via a new
     `copied_value_tokens(&self, sheet_idx, range)` and return it on `CopiedRange`.
   - New `value_token(&self, sheet_idx, row, col) -> String` (1-based coords) implementing the
     token table above; `copied_value_tokens` iterates the 1-based inclusive rectangle.
   - New `paste_values(&mut self, dest_idx, anchor, values: &[Vec<String>], paste_w, paste_h)
     -> Result<(), String>`: **tile** the `values` grid (src_h × src_w) across `paste_h × paste_w`
     (`token = values[dr % src_h][dc % src_w]`), write the whole block through **one**
     `model.set_user_inputs(&batch)` call (single undo entry), then re-select the pasted rectangle
     via `set_view_selection` so the caller reads it back.

2. **`freecell-engine/src/worker/protocol.rs`** — add
   `Command::PasteValues { sheet: SheetId, target: CellRange }` (doc: values-only sibling of
   `PasteInternal`; same sizing/overflow; one undo step; replies `Pasted` / `PasteRejected`).

3. **`freecell-engine/src/worker/run.rs`**
   - Add `values: Vec<Vec<String>>` to `ClipboardSlot`; `apply_copy` stores `copied.values`.
   - Bucket `Command::PasteValues` with the other clipboard ops (the `clip @ (…)` arm); dispatch it
     in `apply_clipboard_op` → `apply_paste_values`.
   - New `apply_paste_values(&mut self, dest, target)` mirroring `apply_paste_internal`: degraded
     guard; `take` the slot; degenerate-range guard; `fill = fill_target_dims(slot.range, target)`
     with the `MAX_REFRESH_CELLS` cap on a fill; `paste_fits` overflow pre-check on the source dims;
     resolve `dest`; `run_guarded_paste(|doc| doc.paste_values(dest_idx, anchor, &slot.values,
     paste_w, paste_h))`; on success `commit_paste` (one touch range) and **keep** the slot
     (values paste is repeatable; never clears the source, even for a cut slot).
   - Update the two direct `ClipboardSlot { … }` test literals with `values: …`.

4. **`freecell-app/src/grid/input.rs`** — add `GridKeyCommand::PasteValues`; bind
   `secondary && shift && key == "v"` before the `secondary && !shift` block. Update the existing
   "shift reserved" assertion and add a focused binding test.

5. **`freecell-app/src/grid/mod.rs`** — add `GridEvent::PasteValues` (unit variant, mirrors
   `Paste`; the window supplies sheet + target from the last selection). Reused by Phase 5.

6. **`freecell-app/src/grid/view.rs`** — in the key-command match,
   `GridKeyCommand::PasteValues => emit(&GridEvent::PasteValues)`.

7. **`freecell-app/src/shell/clipboard.rs`** — add `paste_values(&mut self, sheet, target, client,
   cx)` mirroring `paste`: internal branch (system clipboard still ours) → `Command::PasteValues`;
   foreign/absent branch → `Command::PasteTsv` (external TSV is already values). Add test coverage.

8. **`freecell-app/src/shell/window.rs`** — add `GridEvent::PasteValues` handler mirroring
   `GridEvent::Paste` (commit any pending edit first, then `clipboard.borrow_mut().paste_values(…)`).
   Reuses the existing `Pasted`/`PasteRejected` reply handling — no new `WorkerEvent`.

## Tests

- **document.rs `value_token` / `paste_values`:** number → plain repr (no format applied, target
  format preserved); boolean → TRUE/FALSE; error round-trips as the error; a text `"=x"` →
  quote-prefixed literal (content stays `=x`, not a formula); a text `"12"` stays text; tiling of a
  single-cell source across a larger block.
- **run.rs (worker):**
  - copy a **formula** cell (e.g. `=1+2`) then `PasteValues` → target holds the literal `3` (a
    number), the source formula is unchanged, and the target keeps its own number format.
  - **`=x` edge:** a cell whose computed/display value is the string `=x` → `PasteValues` → target
    shows literal `=x` (not a `#NAME?`/`#ERROR!` formula result).
  - single-cell source fills a larger exact-multiple selection (tiling parity).
  - oversized/overflow → `PasteRejected { Overflow }`, slot kept.
  - `PasteValues` is **one undo step** (one `Undo` reverts the whole pasted block).
  - empty slot → `PasteRejected { NothingToPaste }`.
- **input.rs:** `command_for_key("v", true, true, _) == Some(PasteValues)`; bare/`!shift` `v` still
  `Paste`.
- **clipboard.rs:** `paste_values` routes internal → `PasteValues`, foreign/absent → `PasteTsv`/no-op
  (reuse the `decide_paste` routing).
