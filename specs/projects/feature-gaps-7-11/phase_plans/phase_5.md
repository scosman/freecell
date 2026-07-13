---
status: complete
---

# Phase 5: Verify right-click header Insert/Delete (§7)

## Overview

Verification-only pass over the already-shipped (mvp-gaps Phase 7) right-click row/column
header Insert/Delete context menu. Per `functional_spec.md §7` and `architecture.md §7` the
task is to confirm — via a code + test audit plus one Xvfb smoke launch — that:

1. Right-clicking a **row** header shows Insert above/below + Delete; a **column** header
   shows Insert left/right + Delete; labels reflect the selection **count**; items dispatch
   to the engine correctly.
2. The merged-cell guard still blocks displacing inserts/deletes.
3. Automated coverage exists for these paths.
4. The app still builds + launches without panic.
5. The project checks are green.

**Outcome: verified. No source code changed** (as expected for §7).

## What was confirmed (code audit)

Anchors are current as of this pass (files drifted slightly from the planning-time line
numbers).

### Menu structure, labels, and selection-count

`grid/view.rs::header_menu_elements` (now ~line 2755) builds the card:

- `count = menu.run.1 - menu.run.0 + 1`; `plural = if count == 1 { "" } else { "s" }`.
- Label helper `n("Insert", side) = format!("{verb} {count} {unit}{plural} {side}")`.
  - **Row** axis → `unit = "row"`, sides `"above"`/`"below"` → e.g. `"Insert 3 rows above"`,
    `"Insert 3 rows below"`, `"Delete 2 rows"`.
  - **Column** axis → `unit = "column"`, sides `"left"`/`"right"` → e.g.
    `"Insert 3 columns left"`, `"Insert 3 columns right"`, `"Delete 2 columns"`.
- Insert-before / Delete target the run start (`run.0`); Insert-after targets `run.1 + 1`
  (`after_at = end.saturating_add(1)`). Each enabled item emits the matching `GridEvent`
  (`InsertRows`/`InsertColumns`/`DeleteRows`/`DeleteColumns { at, count }`) and closes the menu.

The **count comes from the selection**: `handle_right_mouse_down` (~line 1625) computes the
run via `resize_run_for(axis, index)` (~line 1509). When the clicked header is inside a
full-row/full-column selection, the run is the whole selected span → the label shows the
multi-selection count ("Insert 3 rows above"). A right-click **outside** the current header
selection first selects that single header (Excel behavior), so the run collapses to count 1.

### Engine dispatch (end-to-end)

Item click → `GridEvent::{Insert,Delete}{Rows,Columns} { at, count }`
→ `shell/window.rs::route_grid_event` (~line 1436) →
`Command::{Insert,Delete}{Rows,Columns} { sheet: active, row|col: at, count }`
→ `worker/run.rs` dispatch (~line 2474) →
`document.rs::{insert_rows, insert_columns, delete_rows, delete_columns}` (~line 568) →
IronCalc `UserModel::{insert_rows, insert_columns, delete_rows, delete_columns}`. The
`count` is threaded through every layer (`GridEvent` → `Command` → `document.rs`), and each
op is undoable (per `protocol.rs` docs and the worker undo tests below).

### Merged-cell guard (two layers, both intact)

1. **UI disable** — `merge_block_flags(axis, run, merges)` (view.rs ~line 3092) calls
   `blocks_row_op`/`blocks_col_op` (`freecell-core/src/merge_guard.rs`:
   `merges.iter().any(|m| m.end.row >= row)` / `... end.col >= col`). Insert-before + Delete
   test `run.0`; Insert-after tests `run.1 + 1`. A blocked item renders disabled (opacity
   0.4, no click listener) and the card appends the footnote
   *"Sheet has merged cells — not yet supported here."*
2. **Worker authoritative** — `precheck` (run.rs ~line 1538) runs `merge_guard` on
   `Insert/Delete Rows/Columns` before applying, returning
   `EditRejectedReason::MergedCells` (→ dialog) so a stale UI still cannot displace a merge.
   Both layers share the same `blocks_*_op` predicates, so UI and engine agree.

## Existing automated test coverage

**`freecell-app` (`grid/view.rs`):**

- `right_click_column_header_opens_menu` (gpui::test) — right-click on the column-header
  strip opens `header_menu` with `axis == Col`; on the merge-free demo sheet nothing is
  blocked; Escape closes it.
- `merge_block_flags_match_predicate` (unit) — the `(insert_before, insert_after, delete)`
  block tuple for a column run across / past a merge, and the empty-merge (nothing blocked)
  case.
- `resize_run_uses_selection_and_clamps` (gpui::test) — `resize_run_for` returns the full
  selected run inside the selection and the single index outside — i.e. the value that drives
  the menu's `count`.
- `header_clicks_and_select_all` (gpui::test) — full column/row selection that feeds the run.

**`freecell-core` (`src/merge_guard.rs`):**

- `merge_guard_predicate` (unit) — boundary math of `blocks_row_op`/`blocks_col_op` on the
  K7:L10 fixture (blocks at/before the merge's last row/col, allows strictly past it).

**`freecell-engine` (`src/worker/run.rs`):**

- `insert_rows_shifts_and_undo` — an `InsertRows` shifts content down; `Undo` restores.
- `delete_columns_shifts_and_undo` — a `DeleteColumns` shifts content left; `Undo` restores.
- `merge_guard_blocks_and_allows_on_fixture` — over a **real** merged-cell xlsx (K7:L10):
  the merge parses into the resident cache (UI-guard layer); an insert **above** the merge is
  refused with `EditRejected { MergedCells }` and commits nothing (`ops_seen` unchanged); an
  insert **below** all merges applies (`ops_seen` grows, no rejection).

All of the above pass (ran by name — see checks). Together they cover: menu open + axis, the
count source (`resize_run_for`), the UI block-flag math, the worker's authoritative guard on
a real merge, and insert/delete apply-and-undo through the engine.

## Coverage observation (not a defect — no fix applied)

There is no test that asserts the **exact label string** ("Insert 3 rows above") or that a
menu-item click emits the right `GridEvent`/`Command` with the right `count`. Every **input**
to those is tested (count via `resize_run_for`; axis via `right_click_column_header_opens_menu`;
the block flags via `merge_block_flags_match_predicate`; apply/undo via the worker tests), and
the label is a deterministic `format!`, so this is a minor coverage gap, **not** a functional
gap. Left as an observation for a reviewer rather than adding a test, since §7 is a verify-only
phase ("file a bug only if a real gap surfaces") and the behavior is sound. A future
label/dispatch assertion in `header_menu_elements` would be a cheap nice-to-have.

## Smoke launch

`xvfb-run -a cargo run -p freecell-app` (under a 45 s watchdog): the app launched cleanly to
the welcome window — GPU adapter selected (llvmpipe/Vulkan), X11 window + colormap created,
event loop running until the timeout killed it (`EXIT=124`, expected). No panic; the log has
**no** `gpui::svg_renderer` "Failed to load bundled font" lines (Phase 1's suppression intact).

## Project checks

- `cargo fmt --all --check` → clean (`FMT_OK`).
- `cargo build --workspace` → clean.
- `cargo clippy --workspace --all-targets -- -D warnings` → clean.
- `cargo test --workspace` → green **except** the 2 `charts_roundtrip_libreoffice` tests,
  which fail in-container on headless LibreOffice/Java (known-accepted, unrelated). Totals:
  freecell-app 373, freecell-core 158, freecell-chart-model 93, freecell-engine 243,
  worker_seam 54, render_tests 16, plus the roundtrip/corpus/fixture suites — all passing.

(Toolchain note: cargo must be run from `app/` so the pinned `rust-toolchain.toml` 1.95.0
activates; `--manifest-path` from the repo root picks up the 1.94.1 default and errors on the
1.95 requirement. Not a project issue — just the invocation.)

## Conclusion

The right-click header Insert/Delete feature works per `functional_spec.md §7` and
`architecture.md §7`: correct row/column labels with selection-count, correct engine dispatch,
and a two-layer merged-cell guard (UI disable + authoritative worker rejection). Existing
tests cover the menu open, the count source, the block-flag math, the worker guard on a real
merge, and insert/delete apply+undo. Smoke launch clean, checks green (modulo the known
LibreOffice failures). **No code changed; no real gap found.**
