---
status: complete
---

# Phase 7: Auto-grow rows — wrap-driven vertical growth + manual-height flag

## Overview

Adds the **new** wrap-driven row auto-grow (§3.2) and the **manual-height flag** (§3.3) on top
of the already-shipping font-size (`SetFont`) + explicit-newline (IronCalc auto-fit) auto-grow,
which are **retained untouched**. The one architectural wrinkle: soft-wrapped height depends on
glyph metrics + column width, which the worker cannot compute (no gpui text system), so the
measurement happens on the **UI/render thread** (where a `Window` exists) using gpui's
`LineWrapper`. A bounded, debounced feedback loop applies the measured height:

- The grid, post-layout, measures each **visible wrap-on** cell's wrapped height at its column
  width, takes the row max, clamps to `[default, MAX_AUTO_ROW_HEIGHT_PX]`, and — only when a row's
  **wrap inputs changed** (a per-row content/style/font/column-width **signature** diff, NOT its
  height) — emits `GridEvent::AutoGrowRows`. The window routes it to a **new, distinct**
  `Command::AutoGrowRowHeights` (auto rows only).
- The worker applies it as a **cache-only** geometry update (`SheetCache::set_row_heights`) —
  final height = `max(base_ironcalc_height, wrap)` per row, so it never shrinks below a
  font/newline need. It does **not** touch IronCalc, bump `ops_seen`, or push an undo `Touch`, so
  it adds **no user-visible undo step** (§3.4, the value-edit-auto-fit precedent). It republishes
  `StyleCacheUpdated` **only when a height actually changed**.
- Convergence: because dirtiness is keyed on the wrap **inputs** signature (not the row height), a
  height-only republish leaves the signature unchanged → the row is not re-dirtied → **no re-emit
  → converges in one frame** with no oscillation.

Independently revertible: all new code is additive (a new command, two worker maps, grid
measurement state, a routed event). Reverting this one commit removes them together and cannot
break Phases 1–6.

## Steps

1. **`freecell-core/src/cache.rs`** — add the shared cap constant:
   `pub const MAX_AUTO_ROW_HEIGHT_PX: f32 = DEFAULT_ROW_HEIGHT_PX * 10.0;` (240 px ≈ 10 default
   lines; content beyond it clips within the wrapped cell).

2. **`worker/protocol.rs`** — add
   `Command::AutoGrowRowHeights { sheet: SheetId, heights: Vec<(u32, f32)> }` (device-px wrap
   need per 0-based row; a value `<= default` drops the row's wrap contribution). Doc: cache-only,
   non-undoable, distinct from `SetRowHeights` so the worker never marks these rows manual.

3. **`worker/run.rs`** — worker-owned state + application:
   - Add fields `manual_rows: HashMap<SheetId, HashSet<u32>>` and
     `wrap_heights: HashMap<SheetId, BTreeMap<u32, f32>>` (init in `load_and_run` + `test_worker`).
   - `process_batch`: bucket `AutoGrowRowHeights` into `autogrow_ops`; apply each after the edit +
     font batches via `apply_auto_grow(sheet, heights)`.
   - `apply_auto_grow`: skip when unresolved/degraded; skip **manual** rows (dropping any stale
     `wrap_heights` entry for them); store/clear each auto row's clamped wrap height; compute the
     per-row cache override = `max(row_override_px(base), wrap)`; apply to the resident cache via
     `set_row_heights` **only for rows whose height actually changes**; emit `StyleCacheUpdated` iff
     any changed. No `ops_seen`/`Touch`/IronCalc mutation.
   - `apply_edit_batch`: after a successful batch, mark every `SetRowHeights` (a **user** resize)
     row **manual** and drop its `wrap_heights` entry.
   - `build_and_store_cache` → `&mut self` (cascade `refresh_cache_cells`,
     `ensure_active_cache_built` to `&mut self`): **seed** `manual_rows[sheet]` on the FIRST build
     from the freshly built cache's `row_overrides` keys (a loaded `custom_height` row = manual);
     then **project** persisted `wrap_heights` (auto rows only) into the built cache as
     `max(base, wrap)` so grown heights survive every full rebuild (resize / insert / delete /
     band edit).

4. **`grid/mod.rs`** — add `GridEvent::AutoGrowRows { heights: Vec<(u32, f32)> }`.

5. **`grid/view.rs`**:
   - New reusable state: `wrap_cells: Vec<WrapCell>` (row, text, font_px, bold, italic, family,
     col_w) and `wrap_sig: HashMap<u32, u64>` (per active-sheet row wrap-input signature). Clear
     `wrap_sig` in `set_active_sheet`.
   - `build_grid_layers`: clear `wrap_cells` at the top; in the cell loop push a `WrapCell` for
     each **published, non-empty, wrap-on** cell (mirror cells excluded — their `attr_style` is
     `None`), only on the render path (`timing.is_none()` → the perf harness is untouched).
   - `measure_wrap_row_px(cells, window)`: for each cell split on `\n`, count soft-wrap boundaries
     via `window.text_system().line_wrapper(font, size).wrap_line(&[LineFragment::text(line)], w)`;
     `cell_h = lines * font_px*1.25 + (default − CELL_FONT_PX*1.25)`; row height = max over cells;
     clamp `[default, MAX]`.
   - `run_autogrow(frame, window, cx)` (called in `render` after `build_grid_layers`): group
     `wrap_cells` by row, compute each row's signature; **measure only dirty rows** (signature ≠
     `wrap_sig`) and any **shrink** rows (in `wrap_sig`, visible, no wrap cell now → default);
     `emit(AutoGrowRows{…})` if non-empty; update `wrap_sig`.
   - `autogrow_measure_now(window, cx)` (public, render-harness hook): resolve a frame, populate
     `wrap_cells`, measure each visible wrap row, and **apply** the height directly to the shared
     cache for rows **without** an existing non-default override (emulating the worker's manual-skip
     using file/injected overrides as the manual signal). Test-scaffolding: the harness renders a
     single static frame with a shut-down worker, so the live loop can't round-trip in-capture.
   - Update the stale "no auto-grow — GAPS F1" comment in `cell_element`.

6. **`shell/window.rs`** — route `GridEvent::AutoGrowRows { heights }` →
   `Command::AutoGrowRowHeights { sheet: active_sheet, heights }`.

7. **`render-tests`**: add `auto_grow: bool` to `RenderCase` (+ `.auto_grow()` builder);
   `run_render_scene` calls `grid.autogrow_measure_now(window, cx)` for opt-in cases before the
   Root mounts (pre-first-paint, so the grown height shows). Add `autogrow_` cases (§3.5).

## Tests

- **Worker unit** (`run.rs`):
  - `set_row_heights_marks_row_manual`: a `SetRowHeights` (user resize) puts the rows in
    `manual_rows`; a later `AutoGrowRowHeights` for that row is **skipped** (height unchanged).
  - `auto_grow_row_heights_grows_without_marking_manual_or_undo`: `AutoGrowRowHeights` grows the
    cache row, does **not** add `manual_rows`, does **not** change `ops_seen`, and a following
    `Undo` reverts the *prior value edit* (not the height) — i.e. no double-undo.
  - `auto_grow_row_shrinks_and_survives_rebuild`: growing then sending a `<= default` height
    shrinks the row back; and a grown auto height **survives a full rebuild** (a column resize on
    another column) via the `wrap_heights` projection.
  - `loaded_custom_height_row_is_manual`: a row built with a `custom_height` (via `SetRowHeights`
    on a fresh build path) is treated as manual and unaffected by `AutoGrowRowHeights`.
- **Grid gpui view** (`grid/view.rs`, `TestAppContext`):
  - `autogrow_measures_wrapped_height_and_emits_once`: a wrap-on long-text cell emits one
    `AutoGrowRows` with a height > default; a second identical measure pass emits **nothing** (the
    signature is unchanged) — the convergence / no-oscillation assertion (dirty set empties).
  - `autogrow_measure_now_grows_default_row_but_not_overridden`: the harness hook grows a
    default-height wrap row and leaves an override (manual) row unchanged.
- **Render** (`autogrow_` prefix, opt-in real measurement):
  `autogrow_wrap_grows`, `autogrow_narrow_col_more_lines`, `autogrow_wide_col_fewer_lines`,
  `autogrow_manual_row_unchanged` (injected height → skipped), `autogrow_cap_clip` (very long text
  clipped at the cap), `autogrow_large_font_grows` (24 pt regression — font auto-grow still grows).
  Iterate with `render_tests.sh test autogrow_`; generate + eyeball; isolation-check the baseline
  dir.
