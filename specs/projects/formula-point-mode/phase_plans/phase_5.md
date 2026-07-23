---
status: complete
---

# Phase 5: Render validation

## Overview

The dedicated, late render-validation phase (`architecture.md §9`, `implementation_plan.md` Phase 5).
No new product behaviour — this phase adds the two in-scope pixel baselines for the feature's grid
render surfaces (the reference-highlight overlay from Phase 2 and the point-drag preview marquee from
Phase 3), regenerates + eyeballs them, runs the **full** pixel suite once, and drives the CI `render`
gate to green.

In-scope pixels (`CLAUDE.md` Render tests / `architecture.md §8`):
- **Grid reference highlights** — rich translucent fill + colored border per same-sheet reference, in
  its assigned palette slot (`view.rs` overlay pass, `REF_HIGHLIGHT_FILL_ALPHA` / `ref_slot_border`).
- **Point-drag preview** — a 2px **dashed** indigo marquee (`POINT_PREVIEW_BORDER = 0x4F46E5`),
  distinct from the solid-blue selection rectangle (`ACCENT = 0x2563EB`) and the colored highlights.

There is no in-editor token coloring in v0.5 (deferred), so no editor-coloring baseline. The two new
cases are the whole pixel delta.

## Steps

### A. Render harness — new case inputs (`render-tests/src/cases.rs`, `render.rs`)

1. Add two optional fields to `RenderCase` (each defaulting to none, so no existing baseline moves):
   - `ref_highlights: Vec<(CellRange, u8)>` — the same-sheet `(target, slot)` list the chrome would
     push through `EditState`; the harness threads it into `GridView::set_edit_state` (matching the
     real `refresh_edit_grid_state` payload).
   - `point_drag: Option<(CellRef, CellRange)>` — an armed point-drag `(origin, last_range)` for a
     static capture of the preview marquee (analogue of the existing `fill_drag` field).
   Add matching builder methods `ref_highlights(...)` / `point_drag(...)`; init both to empty/None in
   `RenderCase::new`.

2. `GridView::set_point_drag_preview(origin, last_range, cx)` (new pub render-test/debug hook, beside
   `set_fill_drag_preview`): arms `self.point_drag = Some(PointDrag { origin, last_range })` +
   `cx.notify()`, so a static capture shows the preview without a synthesized live drag. The normal
   app never calls it (the drag machine owns the state).

3. `render.rs`: thread the two new inputs — pass the case's `ref_highlights` into the `set_edit_state`
   call in the mirror branch (replacing the `Vec::new()` placeholder), and after the editing overlays
   are applied, call `set_point_drag_preview` when `point_drag` is `Some`.

### B. The two cases (`render-tests/src/cases.rs`)

4. `formula_ref_highlight_same_sheet` (GRID_VP): a data-row formula edit open over **B2** with
   `mirror = "=A1+SUM(C3:E7)+B2"` and `selection = B2`, plus three same-sheet highlights:
   `(A1, slot 0)` single cell, `(C3:E7, slot 1)` range, `(B2, slot 2)` **self-reference coinciding
   with the active cell** (the CR-Mild z-order check). Backing inputs label A1 / C3 / E7 / B2 so the
   highlights land over visible content. Proves: single + range highlight, distinct palette slots,
   fill + border, and the self-ref highlight painted ABOVE the active-cell selection outline.

5. `formula_ref_point_preview` (GRID_VP): a formula edit open over **B2** mid-point-drag, showing all
   three coexisting overlays distinctly — `selection = B2` (solid-blue active-cell outline),
   `ref_highlights = [(A1, slot 4 orange)]` (a highlight in a hue far from both blues, so the eyeball
   is unambiguous), and `point_drag = (C3, C3:E7)` (the dashed indigo marquee). `mirror = "=A1+C3:E7"`
   reflects the in-progress text. Proves the preview marquee is distinct from selection + highlight.

## Tests (render baselines are the tests)

- `formula_ref_highlight_same_sheet` baseline — eyeballed: fill+border highlights, distinct slots,
  self-ref z-order over the active cell.
- `formula_ref_point_preview` baseline — eyeballed: dashed indigo marquee distinct from the solid
  selection rectangle and the colored highlight.
- The unit/gpui tests for these surfaces already exist (Phase 2/3): `ref_highlights_for_test`,
  `set_edit_state_threads_ref_highlights`, `point_drag_*`. This phase adds no logic, only fixtures.

## Checks (run cargo from `app/`)

- `cargo build -p render-tests`
- Regenerate the two baselines: `render_tests.sh generate --only formula_ref` (build both bins first
  via the wrapper), then **eyeball** each PNG.
- Full suite once under a `timeout` (~10 min) + watchdog: `render_tests.sh test` — assert every case
  == baseline; justify/accept any diff (only the two new + any genuinely overlay-shifted case).
- `cargo fmt --all --check`.
- CI: dispatch the `render` workflow on `claude/formula-point-mode-specs-pb78bu`; poll to green.

## CR-Mild verification (during eyeball)

- (i) `POINT_PREVIEW_BORDER` (indigo `0x4F46E5`, dashed) vs selection `ACCENT` (blue `0x2563EB`,
  solid): confirm distinctness. Both constants are fixed (theme-independent in the grid, which always
  renders on white `CELL_BG`), so the light/dark relationship is identical; dash style + hue separate
  them.
- (ii) Highlight z-order at a self-reference: the `(B2, slot 2)` highlight coincides with the active
  cell; confirm the colored ref highlight paints above the active-cell selection outline (overlay
  order: selection → highlights → in-cell/handle).
