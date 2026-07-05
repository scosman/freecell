---
status: complete
---

# Phase 5: Fonts (family + size)

## Overview

Adds per-cell **font family** and **font size** to the render pipeline and the action bar,
plus **row auto-grow** on a size change. Extends the same `RenderStyle`/`SheetCache` model
Phase 4 used for `num_fmt`, and reuses the action-bar group/dropdown/degraded patterns.

Because IronCalc 0.7.1 exposes **no** `font.name` / absolute-`font.sz` `update_range_style`
path (only `font.size_delta` — verified `user_model/common.rs:116-142`), font writes go
through `UserModel::on_paste_styles(&[Vec<Style>])` (verified `common.rs:1172`, undoable,
tiles over the engine's view selection). A new `Command::SetFont` drives it.

Primary specs: `components/action_bar.md`, `components/style_render.md`; also
`functional_spec.md §3.2`, `architecture.md §1.1/§3.3/§5.2`, `ui_design.md §2`.

### Verified engine facts (pinned ironcalc_base 0.7.1)

- `Font { sz: i32, name: String, family: i32, .. }`; `Font::default()` is **`sz: 13`,
  `name: "Calibri"`** (`types.rs:410`) — **not** the 11pt the specs state. `Styles::default()`
  seeds `fonts: vec![Font::default()]`, and `new_empty` uses `styles: Default::default()`
  (`new_empty.rs`), so a fresh workbook's default cell is 13pt Calibri. The workbook default
  is resolvable via public fields: `workbook.styles.cell_xfs[0].font_id` → `styles.fonts[id]`.
- `on_paste_styles(&[Vec<Style>])` pastes into the engine's **view selection** (reads
  `view.range`), tiling the grid `% height/% width`; expands the selection if the grid is
  larger; pushes **one** `push_diff_list` (one undo entry); re-selects the pasted rect.
- `get_style_for_cell(sheet,row,col) -> Style` (`model.rs:1990`), `get_cell_style_or_none`
  (`model.rs:1997`), `set_rows_height(sheet,r0,r1,h)` (`common.rs:1081`, one diff-list),
  `get_row_height(sheet,row)` (`common.rs:1108`, IronCalc px, default 28). `set_selected_*`
  (`ui.rs:81/92/118`) reachable via the existing `set_view_selection`.
- gpui: `cx.text_system().all_font_names() -> Vec<String>` (`text_system.rs:88`);
  `Styled::font_family(impl Into<SharedString>)` (`styled.rs:708`),
  `Styled::text_size(impl Into<AbsoluteLength>)` (`styled.rs:538`).

### Key design decision — the "default font" sentinel (recorded in DECISIONS)

`RenderStyle::font_size_q == 0` / `font_family == 0` mean **"the workbook's default font"**,
NOT a hardcoded 11pt. The build/refresh loops resolve these relative to the workbook default
(`document.default_font()`), exactly as `font.color` is resolved relative to black today. This
keeps every default cell — new-workbook (13pt Calibri) **and** opened-file (file default) —
interning to `RenderStyle::default()`, so it renders at the grid default (Inter,
`CELL_FONT_PX = 13`) with **zero baseline change** and **no behaviour change for opened files**
(they render exactly as today). Only a cell whose font *differs* from the workbook default gets
an explicit family/size. Following the spec's literal "11pt" would make every new-workbook cell
non-default and render at 13pt→17px, changing every baseline — so it is explicitly rejected.

Render pt→px uses the spec's `q/4 * 96/72` factor; auto-grow uses the same factor against
`get_row_height` (IronCalc's 28px-default space), keeping text and row height on one factor.

## Steps

### Data model (freecell-core)

1. **`style.rs` — `RenderStyle`**: add two `Copy+Eq+Hash` fields (stays interned):
   ```rust
   pub font_size_q: u16,   // quarter-points; 0 = workbook default
   pub font_family: u16,   // idx into SheetCache.font_families; 0 = workbook default
   ```
   `Default` gives both `0`. Update the module test + doc comment (drop the "font family/size
   intentionally absent" clause).

2. **`cache.rs` — `font_families` side table** (mirror the `num_fmts` machinery exactly):
   - `SheetCache` + `SheetCacheBuilder` gain `font_families: Vec<Arc<str>>`, seeded `[0] = ""`.
   - `fn intern_font_family_into(table, name) -> u16` — empty/`""` → `0`; else linear
     scan/append; `u16::MAX` overflow guard like `intern_num_fmt_into`.
   - `SheetCache::intern_font_family(&mut self, name)` + `SheetCacheBuilder::intern_font_family`.
   - `SheetCache::font_family_name(&self, id) -> &str` (`0`/out-of-range → `""`).
   - `SheetCache::font_families(&self) -> &[Arc<str>]` (grid snapshot source).
   - Thread `font_families` through `build()`. Update the send+sync guard is unaffected (Arc<str>).

### Engine (freecell-engine)

3. **`document.rs`**:
   - `pub(crate) fn default_font(&self) -> (i32, String)` — read `workbook.styles.cell_xfs[0]
     .font_id` → `styles.fonts[id]`, returning `(sz, name.clone())`; out-of-range → `(13,
     "Calibri")` (the `Font::default()` fallback; never panics on a hostile file).
   - `pub(crate) fn set_font(&mut self, sheet_idx, range, family: Option<&str>, size_pt:
     Option<f64>, default_name: &str) -> Result<(), String>` — the `on_paste_styles` flow:
     1. `set_view_selection(sheet_idx, range)` (reuse — anchor on edge).
     2. build row-major `Vec<Vec<Style>>` over `range`: each cell `= get_style_for_cell`,
        then `size_pt` → `font.sz = pt.round() as i32`; `family` → `Some("")`/`Some(name)`
        sets `font.name = default_name`/`name` (System-Default clears to the workbook default;
        `None` leaves it).
     3. `self.model.on_paste_styles(&styles)`.
   - `pub(crate) fn set_row_heights_run(&mut self, sheet_idx, row_start, row_end, px: f64)`
     → `user_model.set_rows_height(sheet, r0+1, r1+1, px)` (1-based; one diff-list per run).
   - `pub(crate) fn clamp_to_used(&self, sheet_idx, range) -> Result<CellRange,String>` —
     only band/select-all shapes (`start.row==0 && end.row==MAX_ROWS-1` or the col analog)
     clamp to `worksheet.dimension()` (intersect, 1-based→0-based); a bounded selection returns
     unchanged (`architecture.md §5.2`). Empty intersection → an empty (start>end) range the
     caller treats as no-op.

4. **`cache.rs` (engine)** — resolve the font fields in the build + refresh loops, mirroring
   how `num_fmt` is resolved (the caller sets the index after `render_style_from`):
   - `render_style_from` stays returning `font_size_q: 0, font_family: 0` (like `num_fmt`).
   - `fn font_size_q_of(sz: i32, default_sz: i32) -> u16` — `sz == default_sz || sz <= 0` → `0`;
     else `(sz as u16).saturating_mul(4)`.
   - `build_sheet_cache`: resolve `let (def_sz, def_name) = doc.default_font();` once, then per
     cell/row-band/col-band style set `rs.font_size_q = font_size_q_of(style.font.sz, def_sz)`
     and `rs.font_family = if style.font.name == def_name { 0 } else {
     builder.intern_font_family(&style.font.name) }` — **before** the `!= default` store gate
     (so a font-only cell is stored, matching `num_fmt`).
   - `refresh_cell`: same, reading `doc.default_font()` once per call (bounded; the >100k path
     rebuilds instead).
   - `assert_cache_agrees` (test): zero `font_family` in the structural compare (a cache-local
     index, like `num_fmt`) but resolve `font_size_q` on the engine side via `font_size_q_of`
     so it compares directly; add a family-**name** agreement check (`cache.font_family_name`
     vs the default-relative expected name).

5. **`worker/protocol.rs`**: add
   ```rust
   Command::SetFont { sheet: SheetId, range: CellRange, family: Option<String>, size_pt: Option<f64> }
   ```
   Doc: style-only (no eval); `family: Some("")` = System Default (clear); the too-large reply
   dialogs. (`WorkerEvent`/`EditRejectedReason` unchanged — reuse `Engine(msg)` for the cap,
   which the window already routes to an error dialog, `window.rs:456`.)

6. **`worker/run.rs`** — bucket `SetFont` with the clipboard ops (its own op, NOT the generic
   `apply_one`, because it emits a **variable** number of engine diff-lists — one style +
   K height runs — and the touch-set must stay 1:1 with the undo stack). Add
   `fn apply_set_font(&mut self, sheet, range, family, size_pt)`:
   - degraded → `EditRejected{Degraded}`; unresolved sheet → return.
   - `clamped = doc.clamp_to_used(idx, range)`; empty → return (no-op).
   - cap: `range_area(&clamped) > MAX_REFRESH_CELLS (100k)` → `EditRejected{Engine("Selection
     too large for font changes")}`; return.
   - guarded (reuse the `run_guarded_paste` shape or an inline `catch_unwind`): pause; `doc
     .set_font(idx, clamped, family, size_pt, &def_name)`; then **auto-grow** (only when
     `size_pt` is `Some`): `needed = (pt * 96.0/72.0 * 1.25).ceil() + 4.0`; scan
     `clamped.rows()` for `needed > get_row_height`, coalesce contiguous runs, one
     `set_row_heights_run` per run; resume (no `evaluate` — style-only).
   - bookkeeping: `ops_seen += diff_lists`; publish + `Published`; push
     `Touch::Cells{sheet, clamped}` **once per diff-list** (style + each height run), clear
     `redo_touches`; `refresh_cache_cells(&[(sheet, clamped)])` (re-reads styles + heights);
     emit `StyleCacheUpdated`.
   - `op_of`/`apply_one` `SetFont` arms are **not** added (it never enters the generic path);
     the `process_batch` router buckets it into a new `font_ops` (or the clipboard bucket).

### Action bar (freecell-app)

7. **`chrome/client.rs`**: add `fn font_family_name(&self, sheet, cell) -> Option<String>` to
   `ChromeClient` (resolve `render_style.font_family` → `cache.font_family_name`; no style → the
   default `""`). `DocumentClient` + `RecordingClient` (+ `set_font_family`) implement it.

8. **`chrome/view.rs`**:
   - Fields: `font_names: Rc<Vec<SharedString>>` (fetched once in `new` via
     `window.text_system().all_font_names()`, "System Default" prepended, sorted-unique),
     `active_font_family: Option<String>`, `active_font_size_q: Option<u16>` (both refreshed
     with `active_style` in `on_selection_changed` + `refresh_active_style`), `font_family_open:
     bool`, `font_size_open: bool`.
   - Commands (all `commit_pending_edit` first, degraded-guarded, log-only):
     `apply_font_family(name: Option<String>)` → `SetFont{family: Some(name|""), size_pt:None}`
     ("System Default" → `Some("")`); `apply_font_size(pt: f64)` → `SetFont{family:None,
     size_pt: Some(pt)}`. Close the respective popover.
   - Getters: `font_family_label()` (active name or "System Default"), `font_size_label()`
     (`font_size_display(active_font_size_q.unwrap_or(0))`).
   - Render: prepend the two dropdown buttons at the head of `render_action_row` (before B I U,
     per `ui_design.md §2`): family (140px) + size (56px), `.disabled(self.degraded)`,
     `.selected(open)`. Two popovers mirroring `render_num_fmt_popover`: family = scrolling menu
     of `font_names` (`overflow_y_scroll`, `max_h`), size = fixed list `8,9,10,11,12,14,16,18,
     20,24,28,36`. Raise `ACTION_ROW_MIN_W` (620 → ~816) for the two new groups (record value).
   - `set_degraded` also closes the two new popovers.

### Grid render (freecell-app)

9. **`grid/view.rs`**:
   - Snapshot the family table alongside `visible_styles` (line ~357): a
     `visible_font_families: Vec<SharedString>` field from `cache.font_families()` (convert
     `Arc<str>` → `SharedString` via `.to_string()`), taken under the same lock.
   - In the cell loop resolve `font_family = style.font_family` → the snapshot name (skip `0`)
     and `font_size_q`, pass both to `cell_element`.
   - `cell_element`: after the bold/italic/underline block, `if s.font_size_q != 0 {
     el = el.text_size(px(s.font_size_q as f32 / 4.0 * 96.0/72.0)); }` and `if let Some(name) =
     family_name { el = el.font_family(name); }`. The mirror/pending path passes `None` (default
     font — `style_render.md`).

### Render-tests

10. **`scene.rs`**: add `Inject::Font(row, col, family: Option<String>, size_q: u16)` +
    `Scene::font(row, col, family: Option<&str>, pt: Option<f32>)`. `apply_injections`
    interns the family into the cache (`cache.intern_font_family`), computes `size_q =
    pt*4`, merges onto the cell's base `RenderStyle` (like `Align`/`FontColor`), and for a
    grown row also `cache.set_row_height`.

11. **`cases.rs`**: add render cases (names from `style_render.md`/`architecture.md §9`):
    `font_family_serif` (a family present on the pinned runner — see DECISIONS font-availability
    risk), `font_size_24_row_grown` (size_q 96 + grown row height), `font_missing_family_fallback`
    (a bogus family name → gpui fallback). Keep table sorted.

12. Add to `DECISIONS_TO_REVIEW.md`: (a) the 13pt-vs-11pt default correction + workbook-default
    sentinel; (b) SetFont = style + height ⇒ up to K+1 undo steps; (c) the CELL_FONT_PX vs
    96/72 render seam; (d) the exact baseline cases to regenerate + the font-availability risk.

## Tests

Unit (`freecell-core::cache`):
- `font_family_interning_dedups_and_default_is_zero` — `""`→0, distinct names distinct ids,
  round-trip via `font_family_name`, out-of-range→`""`, builder+built agree.

Unit (`freecell-core::format_ui`): `font_size_display_*` already covers the display (Phase 4).

Engine (`freecell-engine::cache`):
- `build_carries_font_from_file` — a cell with a non-default `font.name`/`sz` resolves to a
  non-zero `font_family`/`font_size_q` (name round-trips); a default-font cell stays 0/0.
- `band_font_resolves_into_cells` — a column-band font reaches a cell's `RenderStyle`.
- `default_font_detects_workbook_default` — new-workbook default (13/Calibri) → cells 0/0.
- existing `assert_cache_agrees` fixtures stay green (font compare added).

Engine (`freecell-engine::document`):
- `set_font_applies_family_and_size` — `set_font` sets `font.name`/`sz` over a range; System
  Default (`Some("")`) resets to the workbook default; one `on_paste_styles` undo entry.

Worker (`freecell-engine::worker::run`):
- `set_font_grows_rows_and_reflects_cache` — `SetFont{size_pt:24}` over a range → cache
  `font_size_q==96` + the rows grew; StyleOnly (no eval).
- `set_font_undo_reverts_size_and_height` — undo(s) restore the original size + row height
  (cache re-read agrees).
- `set_font_full_column_clamps_to_used` — a full-column `SetFont` clamps (does not materialise
  1M cells).
- `set_font_too_large_selection_rejects` — a >100k clamped area → `EditRejected{Engine(...)}`.
- `set_font_degraded_rejected`.

Chrome (`chrome/view.rs`, `RecordingClient`):
- `font_family_pick_emits_setfont` / `font_family_system_default_emits_empty`.
- `font_size_pick_emits_setfont`.
- `font_dropdowns_reflect_active_cell` (label = active family/size).
- `font_controls_disabled_in_degraded_mode`.

Render suite (NOT run here — baselines regenerate on the pinned runner):
`font_family_serif`, `font_size_24_row_grown`, `font_missing_family_fallback` (new, additive).

## Automated checks (from `app/`, iterate until clean)

`cargo fmt --all --check` · `cargo clippy --workspace --all-targets -- -D warnings` ·
`cargo build --workspace` · `cargo test --workspace` (without `FREECELL_RENDER`).
</content>
</invoke>
