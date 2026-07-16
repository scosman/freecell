# Research: fill handle, hide/unhide, autofit row height seams (2026-07-16)

Codebase findings feeding the functional spec + architecture. Paths are `app/crates/…`
(grid UI ≈ all in `freecell-app/src/grid/{view.rs, layout.rs, mod.rs, chart_layer.rs}`).

## (A) Drag fill handle + series autofill

- **Selection overlay** built at `grid/view.rs:2721-2758` (range fill via
  `layout::range_overlay_rects`, range border, active-cell border — all `rect_div`s in
  the content layer). The **chart resize handles** (`view.rs:2831-2850` +
  `chart_layer.rs:129-167`, `HANDLE_PX=8`, `HANDLE_HIT_HALF=7`) are the exact pattern
  for a corner handle square.
- **Drag state machines** on `GridView` (`view.rs:265-332`): `drag` (selection),
  `resize_drag`, `chart_drag`. Mouse trio `handle_mouse_down/move/up`
  (`view.rs:1183/1457/1508`). A fill drag = new `fill_drag` field mirroring
  `chart_drag`; preview = one more `rect_div(span_rect(target))` beside the selection
  overlays.
- **Auto-scroll during drag already built** (`maybe_start_autoscroll` `view.rs:2080`,
  16ms tick, `layout::edge_autoscroll_delta`) — currently gated on `DragMode::Cell`;
  fill drag must hook it too.
- **⌘D/⌘R path end-to-end:** `input.rs:36-90` → `GridEvent::FillDown/FillRight`
  (`grid/mod.rs:139-143`) → `window.rs:1553-1560` → `Command::FillDown/FillRight`
  (`protocol.rs:208-214`) → `run.rs:3156-3170` → `document.rs:553-629`.
- **Series detection is NOT reachable today:** `fill_down`/`fill_right` always seed
  `auto_fill_rows/columns` with a **1-tall/1-wide** source area (doc comment
  `document.rs:538-543`; tests `fill_down_copies_top_row_not_series` :1880). The
  engine's `detect_progression` needs a ≥2-value seed. **Drag fill passes the full
  selected block as the seed** (multi-row/col ⇒ series: 1,2,3… / Jan,Feb…) — needs a
  generalized document method, not new engine work.

## (B) Hide / unhide rows & columns

- **Header context menu** is the home: `HeaderMenu` (`view.rs:125-135`), opened by
  header right-click (`view.rs:1913-1947`, `run = resize_run_for(axis, index)`),
  rendered `header_menu_elements` (`view.rs:3279-3411`) — items are
  `(label, disabled, GridEvent)` tuples; Hide/Unhide slot in here. Event→worker mapping
  `window.rs:1623-1642`, commands `protocol.rs:292-318`.
- **Zero-size tracks render safely today:** `Axis` (`freecell-core/src/axis.rs`) accepts
  0.0 overrides; `index_at` (:110-141) never lands on a zero-size track (can't click
  into hidden); offsets/scrolling sum correctly; zero-width cell/header divs are
  harmless; `resize_hotspots` already skips coincident dividers.
- **But hidden ≠ resize-to-zero:** unhide must restore the prior height/width, and a
  0px manual resize must stay distinct — an explicit hidden concept is required.
  **No "hidden" concept exists anywhere FreeCell-side today** (grep confirms).
- **Geometry round-trip:** open-side `build_sheet_cache`
  (`freecell-engine/src/cache.rs:293-347`) reads `custom_width/custom_height` only —
  `r.hidden`/`col.hidden` are parsed by the fork's import but **never read** (also noted
  `specs/projects/feature-gaps-7-11/DECISIONS_TO_REVIEW.md:387`). Save-side rides
  IronCalc's model.
- **Fork half (per GAPS):** row-hidden `UserModel` setter (undoable) + column-hidden
  modelling/round-trip (`Col` has no hidden field) — two clean `fix/` branches.
- **No affordance today** between non-adjacent visible headers (no gap marker); a
  marker would be new header-strip render work. Unhide-via-spanning-selection maps
  cleanly onto the existing run-based menu.

## (C) Autofit row height (double-click row divider)

- **Column autofit to mirror** (gaps_closing_7_12 Phase 7, commit e61684a):
  `autofit_column` (`view.rs:1674-1695`) / `autofit_width_for_column` (:1704-1756,
  snapshot published cells → `measure_incell_text_width` fold) / constants :60-69;
  hotspot double-click branch `view.rs:3237-3242` (`click_count` match); emits
  `ResizeCommitted` reusing `SetColumnWidths`; no-op guard `commit_resize` :1642-1645.
- **Row divider hotspot EXISTS but has no double-click branch**
  (`view.rs:3248-3272` — unconditional `begin_resize`). Feature C = add the
  `click_count` match there, mirroring columns.
- **Height measurement machinery to reuse:** `measure_wrap_height` (`view.rs:3050-3084`,
  line_wrapper soft-wrap count × phi line box, clamp
  `[DEFAULT_ROW_HEIGHT_PX=24, MAX_AUTO_ROW_HEIGHT_PX=240]`), `WrapCell` struct :359.
  Autofit must measure **all** populated cells in the row (not just wrap-on ones) —
  snapshot like `autofit_width_for_column` does, fold `measure_wrap_height`-style.
  Engine-side analog: `autofit_row_ironcalc_px(font_px)`
  (`freecell-engine/src/cache.rs:81-83`).
- **Manual-vs-auto wrinkle:** `SetRowHeights` marks rows **manual** (`run.rs:758-764`)
  ⇒ exempt from future wrap auto-grow (`run.rs:1828`); `AutoGrowRowHeights` is
  cache-only (no undo). Excel's double-click autofit *returns the row to
  auto-tracking* — the spec must decide manual-mark vs clear-manual.

## Cross-cutting

- GridEvent enum + window→worker mapping: `grid/mod.rs:139-165`, `window.rs:1460-1670`;
  worker commands `protocol.rs:200-318`, routing `run.rs:3156-3231`.
- Geometry: `core/axis.rs`, `core/cache.rs:53-443`, engine seed `engine/cache.rs:293-347`.
- All three features move grid pixels ⇒ pixel-suite in scope (subset while iterating,
  one late full run + CI render gate).
