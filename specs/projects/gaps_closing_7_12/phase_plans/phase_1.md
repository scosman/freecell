---
status: complete
---

# Phase 1: Status bar with selection stats

## Overview

Add a live selection-statistics readout (`Sum · Average · Count`, click-toggle for `Min · Max`)
on the **right side of the sheet-tab bar** (owner decision D1.2 — no second bar). Stats are
**worker-computed** over `selection ∩ populated cells`, so they are correct even when the
selection extends past the published viewport (e.g. a full-column selection). The value path
rides the existing worker command pipeline: a new `Command::SelectionStats` query →
`document.rs::selection_stats` → `WorkerEvent::SelectionStats` → the chrome renders it.

Decisions resolved on the recommended defaults: **D1.1** — error cells count in `Count` but are
excluded from the math (text and errors are treated identically). **D1.2** — right of the tab bar.

## Steps

1. **`freecell-core/src/stats.rs` (new)** — the engine-free aggregate + readout formatter (so it
   crosses the worker seam and is unit-testable):
   - `pub struct SelectionStats { count: u64, numeric_count: u64, sum: f64, min: Option<f64>, max: Option<f64> }`
     with `EMPTY`, `push_number(n)`, `push_non_numeric()`, `average() -> Option<f64>`,
     `has_numeric() -> bool`.
   - `pub fn format_stat_value(f64) -> String` — compact General: thousands separators on the
     integer part, trailing zeros trimmed, precision capped at 11 significant digits (so float
     noise like `0.1+0.2` shows `0.3`). `pub fn format_stat_count(u64) -> String` — grouped
     integer. Private `group_thousands` + `trim_trailing_zeros` helpers.
   - Re-export `SelectionStats`, `format_stat_value`, `format_stat_count` from `lib.rs`; add
     `pub mod stats;`.

2. **`freecell-engine/src/worker/protocol.rs`** — add the query + reply:
   - `Command::SelectionStats { sheet: SheetId, range: CellRange, req_id: u64 }` (a read).
   - `WorkerEvent::SelectionStats { req_id: u64, stats: SelectionStats }` (import
     `freecell_core::SelectionStats`).

3. **`freecell-engine/src/document.rs`** — `pub(crate) fn selection_stats(&self, sheet_idx: u32,
   range: CellRange) -> SelectionStats`. Walk `worksheet().sheet_data` (populated cells only, like
   `find_matches`), filter to cells inside `range`, classify each via the existing `cell_value`
   (`CellData::Number → push_number`; `Bool`/`Text` → `push_non_numeric`; `Empty` → skip). Errors
   arrive as `CellData::Text` (D1.1: counted, not summed). No `dimension()` call needed — the
   populated-cell walk already restricts to the used range. Add `SelectionStats` to the
   `freecell_core` import.

4. **`freecell-engine/src/worker/run.rs`** — route `Command::SelectionStats` in `process_batch`
   into a new `stats_ops: Vec<(SheetId, CellRange, u64)>` bucket (a pure read, alongside `reads`).
   After the edit batch (so it observes the batch's mutations), for each: `let stats = resolve(sheet)
   .map(|idx| doc.selection_stats(idx, range)).unwrap_or(SelectionStats::EMPTY); emit(WorkerEvent::
   SelectionStats { req_id, stats })`.

5. **`freecell-app/src/chrome/view.rs`** — chrome state + request + render:
   - Fields: `selection_stats: Option<SelectionStats>`, `stats_show_minmax: bool`, `stats_seq: u64`.
   - `const STATS_DEBOUNCE: Duration = from_millis(120)`.
   - `fn request_selection_stats(&mut self, cx)`: bump `stats_seq`; single-cell/empty selection →
     clear `selection_stats` + notify + return (the bump invalidates any in-flight reply); else
     capture `(sheet, range, seq)` and arm a `STATS_DEBOUNCE` timer that sends
     `Command::SelectionStats { req_id: seq }` only if `stats_seq` is still `seq` (debounce collapses
     drag-select). Call it at the end of `on_selection_changed`.
   - `on_worker_event`: add `WorkerEvent::SelectionStats { req_id, stats }` arm — accept only when
     `req_id == stats_seq` (drop a stale/superseded reply), store + notify.
   - `stats_readout_parts(&self) -> Option<Vec<String>>` (pure; `None` when hidden = no stats or no
     numeric cell): `["Sum: …", "Average: …", "Count: …"]`, plus `["Min: …", "Max: …"]` when
     `stats_show_minmax`. `pub fn toggle_stats_minmax(&mut self, cx)` flips the session-only toggle.
   - `render_tab_bar`: after the tabs + "+" button add a `div().flex_1()` spacer then the stats
     group (right-aligned in the same `TAB_BAR_H` row) — a clickable (`toggle_stats_minmax`) row of
     the readout parts; empty when hidden (row stays present → stable layout).
   - Test accessor: `pub fn selection_stats_text(&self) -> Option<String>` = parts joined.

6. **`freecell-app/src/shell/window.rs`** — route `WorkerEvent::SelectionStats { .. }` to the chrome
   (add to the CellContent/FindResults forwarding arm). In the `WorkerEvent::Published` arm, call
   `self.chrome.update(cx, |c, cx| c.refresh_selection_stats(cx))` so an edit that changes a value
   inside a still-active multi-cell selection re-computes (functional_spec §1 live-update). Expose
   `pub fn refresh_selection_stats(&mut self, cx)` = `request_selection_stats`.

## Tests

- **`freecell-core` (stats):**
  - `format_stat_value` — integers grouped (`1234 → "1,234"`), trailing zeros trimmed
    (`1234.50 → "1,234.5"`), float noise capped (`0.1+0.2 → "0.3"`), negatives, sub-1 (`1/3`),
    zero, millions.
  - `format_stat_count` — `5 → "5"`, `1_000_000 → "1,000,000"`.
  - `SelectionStats` folding — `push_number`/`push_non_numeric` drive count/numeric_count/sum/
    min/max/average; `has_numeric` false with no numeric cell.
- **`freecell-engine` (document):** `selection_stats` over a numeric/text/bool/error/blank mix →
  correct `count` (non-empty), `numeric_count`, `sum`, `min`, `max`; error counted not summed
  (D1.1); a **full-column** range on a sparse sheet aggregates only populated cells; an all-text
  selection → `has_numeric() == false`; an empty range → `EMPTY`.
- **`freecell-engine` (worker):** a `Command::SelectionStats` batch replies
  `WorkerEvent::SelectionStats` carrying the computed aggregate for the populated cells.
- **`freecell-app` (chrome gpui):**
  - Multi-cell selection → after the debounce tick a `Command::SelectionStats` is sent; a single
    cell selection sends none and shows no readout.
  - A `SelectionStats` reply renders `Sum/Average/Count`; `toggle_stats_minmax` adds `Min/Max`.
  - A stale reply (superseded `req_id`) is dropped.
  - An all-text reply (`numeric_count == 0`) renders no readout.

## Validation

Non-pixel (tab-bar chrome is out of pixel-suite scope per the render-scope table): crate-scoped
`cargo build`/`test` for `freecell-core`, `freecell-engine`, `freecell-app`; `cargo fmt --all
--check`; an Xvfb smoke launch of `freecell-app`.
