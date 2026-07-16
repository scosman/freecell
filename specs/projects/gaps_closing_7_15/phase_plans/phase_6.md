---
status: complete
---

# Phase 6: Render validation (final phase)

The mandatory late render-validation pass for the three grid-pixel-moving features: the fill
handle + drag preview (Phase 4 / §3), hidden-track zero-size geometry (Phase 5 / §4), and autofit
row height (Phase 3 / §5). Regenerate + eyeball the affected baselines, add dedicated new cases for
the new affordances, and run the **full** pixel suite green.

## Harness fix required first (Phase 1 fallout)

The render harness did not compile: Phase 1 (autocomplete) widened `GridView::set_edit_state` from
5 to 7 args (added `Option<AutocompleteDisplay>` + `Option<SharedString>`), but
`render-tests/src/render.rs` still called it with 5. Fixed both call sites to pass `None, None` for
the two new autocomplete/signature-hint overlays (those are chrome, out of pixel scope). Without
this the suite could not build, so this was step 0.

## What moved, pixel-wise

- **Fill handle (Phase 4).** An always-visible 8px handle square (`chart_layer::HANDLE_PX`) now
  draws at the bottom-right corner of the selection in every non-editing grid scene. Because the
  default selection is A1, it appears in essentially every non-chart grid baseline.
- **Hidden tracks (Phase 5).** No existing baseline hides any track, so — as predicted — the
  hidden-track geometry moved **no** existing baseline; it only appears in the new dedicated case.
- **Autofit row (Phase 3).** A user gesture only; injects no state into any existing scene, so it
  moved no baseline.

## Key finding — the handle is BELOW the perceptual-diff threshold

Run 1 of the full suite (before regenerating) came back **147 passed / 3 failed**, the 3 failures
being only the new cases (no committed baseline yet). Every one of the 104 fill-handle-affected
existing baselines **passed unchanged**. The reason: the perceptual diff tolerates up to **0.5%**
differing pixels (`diff.rs` `fail_fraction = 0.005`), and the 8×8 handle square is ~38 changed
pixels — well under 0.5% of even the smallest (480×160) viewport. So the handle is a real,
sub-threshold pixel change: it does **not** fail existing baselines, but the baselines were
regenerated anyway so they truthfully show the handle (and a future handle regression is catchable).

This was cross-confirmed by `generate_baselines`, which reported "3 new, **0 changed**, 141
unchanged" (its own comparison is the same perceptual metric) while `git` saw **104 byte-modified**
+ 3 new — i.e. 104 baselines gained the handle bytes, all perceptually sub-threshold.

## Fill-handle-only verification (no unexpected regressions)

For a representative sample (`cell_plain`, `grid_selection_range`, `grid_loading_overlay`) the
old→new baseline diff was **exactly 38 changed pixels each, localized precisely at the selection's
bottom-right corner** (the handle) — nothing else moved (text/borders/fills/geometry all identical).
The 37 pre-existing baselines that stayed **byte-identical** are exactly the plan's NOT-AFFECTED set:
the 34 standalone `chart_*` scenes (no grid/selection) + the 3 `incell_editor_*` cases (in-cell
editor suppresses the handle).

**One deviation from the phase_4.md prediction:** `grid_loading_overlay` was predicted NOT-AFFECTED
(overlay "occludes" the handle), but it did pick up the 38px handle at the A1 corner — the loading
spinner/text is centered lower and does not cover A1's corner, so the handle shows. Benign: still
fill-handle-only, and the refreshed baseline reflects reality.

## New baseline cases added (3)

Authored in `render-tests/src/cases.rs` + registered in the `render_cases!` list
(`tests/render_suite.rs`), eyeballed correct:

1. **`fill_handle_multicell`** — a B2:C3 range selection (values 1/2/3/4); the handle square draws
   at C3's bottom-right corner. The dedicated proof of the handle on a multi-cell range.
2. **`fill_drag_preview`** — a live fill drag: a 2-cell vertical seed B2:B3 dragged down to B2:B7,
   so the grid paints the 2px accent target-region preview rectangle over the fill span (the handle
   is replaced by the preview during the drag). Pixel proof of the drag-preview overlay, otherwise
   uncovered.
3. **`hidden_row_and_col`** — a 4×4 labelled block with row index 2 and column B hidden; the hidden
   tracks collapse to zero size (headers jump A→C and 2→4, neighbours abut with no cell/header/
   gridline), driven through the real `SetRowsHidden`/`SetColumnsHidden` worker path.

### Harness additions supporting the new cases

- **`GridView::set_fill_drag_preview(seed, target, axis, cx)`** — a `pub` render-test/debug hook
  (same pattern as `set_force_scrollbars` / `set_freeze_spinner`) that arms the fill-drag state so a
  static capture can show the preview rectangle without synthesizing a live mouse gesture.
- **`Scene::hide_row(start,end)` / `Scene::hide_col(start,end)`** — drive real
  `Command::SetRowsHidden` / `SetColumnsHidden` through the worker in `build_sources`, so the
  published cache carries the zero-size hidden geometry (the same path the header-menu Hide uses).
- A `RenderCase.fill_drag` field wired through `render.rs`.

## Result

- Full pixel suite (run 2, after regeneration): **green — all 150 cases == baseline.**
- Baselines: **104 refreshed** (fill handle) + **3 new** = 107 touched; 37 unchanged (chart_* +
  in-cell).
- CI `render` gate: **left for the manager to dispatch** on the branch (per the task).
</content>
</invoke>
