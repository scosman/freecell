---
status: complete
---

# Phase 5: Style & Geometry Cache

## Overview

Phase 5 builds the always-resident style & geometry cache so the render path makes **zero
engine calls** and the grid renders fully styled during a multi-second eval
(`components/style_cache.md`, `architecture.md §6`). Phases 2–4 laid the read model
(`freecell-core::cache::{SheetCaches, SheetCache, SheetCacheBuilder, StyleId}` + `RenderStyle`
+ `Axis`) and a worker that already classifies `SetStyleAttr` as `StyleOnly` (publishes, no
recompute) and holds an empty `Arc<RwLock<SheetCaches>>`. This phase fills that cache in.

Deliverables:

1. **Engine cache builder/mutator** (`freecell-engine::cache`) — the only place that reads
   IronCalc geometry + `Style` and converts them to engine-free `RenderStyle`/px:
   - `RenderStyle` derivation from `ironcalc_base::types::Style` (bold/italic/underline, fill,
     font colour, h-align, num-format-is-default) with a robust `#RRGGBB`/`#AARRGGBB` colour
     parser.
   - **Unit conversion, one place**: IronCalc's geometry getters return **pixels already**
     (default col 125 px / row 28 px — `ironcalc_base/src/constants.rs`); FreeCell's chosen
     defaults are 100 px / 24 px (`ui_design.md §3.3`). Convert overrides by the ratio
     `freecell_default / ironcalc_default` so an IronCalc-default track maps exactly to the
     FreeCell default and deviations scale proportionally.
   - **Build-on-activation**: scan the worksheet's populated cells + row/col band collections
     (`get_model().workbook.worksheet(idx)` → `sheet_data`/`rows`/`cols`), intern styles, fill
     geometry overrides, build via `SheetCacheBuilder`.
2. **In-place mutation** on `SheetCache` (freecell-core) so mirror-on-edit touches only the
   changed cells (cheap), plus geometry setters that rebuild the affected `Axis`.
3. **Worker integration**: build the active sheet's cache on load + on sheet activation;
   **mirror the issued op** (re-read the touched cells' styles) after every cell edit;
   **undo/redo touch-set re-read** via a worker-side history of per-op touch-sets aligned 1:1
   with IronCalc's undo stack; reconcile caches on sheet add/delete; emit `StyleCacheUpdated`.
4. **Agreement-contract tests + negative control**: after every mutation the mirrored cache
   must equal a fresh engine re-read (`get_style_for_cell` + size getters) over touched +
   probe + random cells. A negative control skips one update and asserts the helper FAILS.

## Steps

### A. `freecell-core::cache` — make `SheetCache` mutable in place

1. Add a private `style_ids: HashMap<RenderStyle, StyleId>` to `SheetCache` (dedup by
   `RenderStyle`, which is `Eq + Hash`) alongside the existing `resolved: Vec<RenderStyle>`.
   Add `intern(&mut self, RenderStyle) -> StyleId`. `SheetCacheBuilder` also keys its intern on
   this map (replace the linear scan) and moves it into the built cache so the two paths share
   one canonical resolved table.
2. Refactor `SheetCacheBuilder`'s consuming setters (`cell_style`/`row_style`/`col_style`/
   `row_height`/`col_width`) to delegate to non-consuming `&mut` variants
   (`push_cell_style`, …) so the engine's build loop can drive the builder without move-per-cell.
3. Add public in-place mutators to `SheetCache` (called by the worker under the write lock):
   - `set_cell_style(row, col, RenderStyle)` / `clear_cell_style(row, col)`
   - `set_row_band_style(row, RenderStyle)` / `clear_row_band_style(row)`
   - `set_col_band_style(col, RenderStyle)` / `clear_col_band_style(col)`
   - `set_row_height(row, px)` / `reset_row_height(row)` (updates `row_overrides`, rebuilds
     `row_axis`); symmetric `set_col_width`/`reset_col_width`.
   - `is_on_band(row, col) -> bool` (`row_styles`∋row || `col_styles`∋col) — the shadowing check.
   Interning never GCs unused ids (matches builder; bounded distinct styles) — documented.

### B. `freecell-engine::cache` — the IronCalc-facing builder/mutator (new module)

4. `render_style_from(&Style) -> RenderStyle` + `parse_color(&str) -> Option<Rgb>`
   (`#RRGGBB`, tolerate `#AARRGGBB`, else `None`). Map: `font.b/i/u`; `fill.fg_color` present
   → `Some(rgb)` else `None`; `font.color` → `Some(rgb)` unless pure black/absent → `None`
   (RenderStyle's "None = grid default"); `alignment.horizontal` Left/Center/Right → `Some`,
   General/other → `None`; `num_format_is_default = num_fmt.eq_ignore_ascii_case("general")`.
   Assert `render_style_from(&Style::default()) == RenderStyle::default()`.
5. Geometry conversion consts (documented, cited to `constants.rs`):
   `IRONCALC_DEFAULT_COL_WIDTH_PX=125.0`, `IRONCALC_DEFAULT_ROW_HEIGHT_PX=28.0`;
   `col_px(ic) = ic * (100/125)`, `row_px(ic) = ic * (24/28)`.
6. `build_sheet_cache(doc, sheet_idx, rows, cols) -> SheetCache`:
   - defaults 100/24; iterate `ws.cols` (custom_width → col overrides; `style` → col bands over
     `min..=max`), `ws.rows` (custom_height → row override; `custom_format && s!=0` → row band —
     matches IronCalc's `get_cell_style_index` which only applies a row band when
     `custom_format`), and `ws.sheet_data` (per-cell own style via `get_cell_style_or_none`;
     store when non-default, or when default **and** on a band cell — to reproduce IronCalc's
     "a cell present in `sheet_data` shadows the band with its own style").
   - `rows`/`cols` = `freecell_core::limits::MAX_ROWS/MAX_COLS` (the axes span the full sheet).
7. `refresh_cell(cache, doc, sheet_idx, row, col)` — the mirror primitive: read the cell's own
   style; `Some(non-default)` → `set_cell_style`; `Some(default)` → shadow if `is_on_band` else
   `clear_cell_style`; `None` (absent from `sheet_data`) → `clear_cell_style`. This exactly
   reproduces `get_cell_style_index`, so the resolved `render_style` matches `get_style_for_cell`.
8. `WorkbookDocument` pub(crate) accessors the cache module needs: `inner_model() -> &Model`,
   `cell_own_style`, `resolved_cell_style`, `row_band_style`, `col_band_style`, `row_height_px`,
   `col_width_px`, `worksheet(idx) -> &Worksheet`. Keeps IronCalc reads inside the engine crate.

### C. Worker integration (`freecell-engine::worker::run`)

9. On load (`load_and_run`): build the active sheet's cache, insert under the caches write lock
   **before** emitting `Loaded` (so first paint has geometry+styles), then emit
   `StyleCacheUpdated{active}`.
10. Worker gains `undo_stack: Vec<Touch>` / `redo_stack: Vec<Touch>` where
    `Touch = Cells{sheet, range} | Sheets`, kept 1:1 with IronCalc's undo history.
11. `apply_one` returns, per applied edit, an `AppliedOp` (`Cells{sheet,range}` / `Sheets` /
    `Undo` / `Redo`) so post-eval bookkeeping can: push touch + clear redo (new edit); pop→refresh
    →push-other (undo/redo). Accumulate cells-to-refresh + a `sheets_dirty` flag.
12. After the eval closure (Ok branch): take the caches write lock once →
    `refresh_cell` every accumulated (sheet, cell) whose sheet cache exists; if `sheets_dirty`,
    reconcile the map (drop caches for absent `SheetId`s) and rebuild the active sheet's cache.
    Emit `StyleCacheUpdated{active}` when the active cache changed. A pathologically large refresh
    range (> a cap) falls back to a full active-sheet rebuild.
13. On `SetViewport` switching to an unbuilt sheet: build-on-activation, insert, emit
    `StyleCacheUpdated{sheet}`.

## Tests

### freecell-core (`cache.rs`)
- `mutators_intern_and_resolve`: `set_cell_style`/`clear`, band set/clear, dedup via `style_ids`.
- `geometry_mutation_rebuilds_axis`: `set_row_height`/`reset` change `total_height`/offsets.
- `is_on_band_detects_band_membership`.
- (existing builder/resolution/geometry tests keep passing after the refactor.)

### freecell-engine (`cache.rs` unit)
- `render_style_from_default_is_plain` + each attribute (bold/italic/underline/fill/font-colour/
  align/num-fmt) maps correctly; `parse_color` goldens (`#RRGGBB`, `#AARRGGBB`, junk→None).
- `unit_conversion_goldens`: 125→100, 28→24, 250→200, 56→48 (± eps), with source-cited consts.

### freecell-engine (integration `tests/style_cache.rs`) — the agreement contract
- `assert_cache_agrees(doc, caches, sheet, probes)` helper: for touched + fixed-probe-grid +
  seeded-random cells, assert `cache.render_style(r,c).unwrap_or_default() ==
  render_style_from(get_style_for_cell(r,c))`, and geometry (`row_height`/`col_width`/totals) vs
  the engine size getters (converted).
- `build_matches_engine_styled_fixture`: a fixture with per-cell styles + a row band + a col band
  + custom row height/col width + cross cells → full agreement sweep.
- `build_matches_engine_empty` and `build_matches_engine_band_only`.
- `excel_max_geometry_totals_match_engine`: 1M-row default totals equal the engine (converted).
- `mirror_set_style_each_attr`: bold/italic/underline/fill/no-fill, single + multi-range +
  overlapping a band → agreement after each.
- `mirror_set_cell_input_on_band_shadows`: typing into a banded row updates the cell to its own
  (default) style, still agrees.
- `undo_redo_agreement_walk`: scripted interleaved edit/undo/redo (incl. across a sheet add) →
  agreement after **every** step.
- `negative_control_skips_one_update_detects_divergence`: deliberately skip a mirror refresh →
  `assert_cache_agrees` returns `Err` (proves discriminating power).
- `interner_dedups`: N cells sharing a style → one `StyleId`; distinct → distinct.
- `sheet_switch_builds_on_activation`.
- Perf smoke: `render_style` + axis offset over a 2k-cell viewport ≪ 1 ms (guards O(n) lookups).

### worker seam (`tests/worker_seam.rs`)
- `style_edit_updates_cache_and_emits_stylecacheupdated`: after `SetStyleAttr`, the public
  `caches()` shows the mirrored `RenderStyle` and a `StyleCacheUpdated` event arrives.
- `load_builds_active_sheet_cache`: after `Loaded`, `caches()` is populated for the active sheet.
