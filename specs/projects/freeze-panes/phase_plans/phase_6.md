---
status: complete
---

# Phase 6: Render validation (dedicated late phase)

## Overview

The dedicated late render-validation phase (`architecture.md §7`, `implementation_plan.md`
Phase 6). No new app behavior — it adds the render-test scaffolding for freeze panes and its
baselines, then validates the whole pixel suite. The seven `freeze_*` cases are the pixel proof of
the Phase 3 four-quadrant render + the Phase 4 band-publishing fix.

## Steps (as built)

1. **Scene builder (`render-tests/src/scene.rs`).** Added `.frozen_rows(m)` / `.frozen_cols(k)`
   (mirroring `.hide_row`), which drive a **real** `Command::SetFrozen` through the worker so the
   published `SheetCache` carries `frozen_rows`/`frozen_columns` the grid reads to pin the bands.
   Sent per axis independently, before `SetViewport`, matching the header-menu one-axis-per-action
   path.
2. **Seven cases (`render-tests/src/cases.rs`).** A `labeled(...)` helper fills a cell block with
   `r{R}c{C}` labels so a mis-pinned or blank (unpublished) band is obvious. Cases:
   `freeze_top_row` (M=1), `freeze_rows_band` (M=3), `freeze_first_col` (K=1), `freeze_cols_band`
   (K=3), `freeze_four_quadrant` (M=2, K=2), `freeze_scrolled_body` (M=2, K=2 + deep `.reveal`),
   `freeze_divider` (M=1, K=1, unscrolled).
3. **Macro registration (`render-tests/tests/render_suite.rs`).** Added the seven names to the
   `render_cases!` macro so each case is generated as a `#[test]` asserting against its baseline
   and the `case_names_match_table` drift guard stays in sync with `cases::all()`.
4. **Baselines.** Generated + eyeballed all seven, committed under `render-tests/baselines/`.

## Validation results

- **Full pixel suite** (`render_tests.sh test`, ~759s under lavapipe): all pixel cases green. The
  seven `freeze_*` cases assert **7/7 pass** against their baselines; the `case_names_match_table`
  drift guard passes; net **160/160** `render_suite` tests green.
- **Baselines eyeballed + USER-APPROVED (UI signoff passed).** Each `freeze_*` baseline was read
  and confirmed correct:
  - `freeze_top_row` / `freeze_rows_band`: the frozen row/row-band pinned at top, divider beneath,
    body discontinuously scrolled deep below it.
  - `freeze_first_col` / `freeze_cols_band`: the frozen column/column-band pinned at left, vertical
    divider (gray `0x9E9E9E`, confirmed at x=147), body scrolled right (clean partial leading body
    column at the divider).
  - `freeze_four_quadrant`: the full split — pinned corner + top band (h-scrolled) + left band
    (v-scrolled) + body (both), both dividers meeting at the corner.
  - `freeze_scrolled_body`: **the Phase-4 band-publishing proof** — with the body scrolled deep
    (rows → 36-45, cols → K-N), the frozen top + left bands still show their VALUES, not blank.
  - `freeze_divider`: divider isolation — horizontal divider at y=47 + vertical at x=147, gray
    `0x9E9E9E`, distinct from ordinary gridlines, drawn even with no scroll.
- **Checks:** `cargo build -p render-tests` + `cargo test -p render-tests --lib` (16) green;
  `cargo fmt --all --check` clean.
- **Doc-sync:** `architecture.md §7` `freeze_cols_band` corrected `K=2` → `K=3` to match the
  implemented case (the CR mild note).

The CI `render` gate is dispatched by the coordinator separately.
