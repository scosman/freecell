---
status: complete
---

# Phase 3: Value-dependent render cache

## Overview

P3 folds conditional formatting into the resident style cache and adds the one genuinely new
rendering coupling FreeCell has never had before: **cell values driving cell styles**. Two changes,
both gated so a workbook with no CF pays exactly nothing:

1. **Build/refresh through the extended style on CF sheets.** `build_sheet_cache` and `refresh_cell`
   gain a `cf: bool` flag. When `cf == false` they take the UNCHANGED fast path
   (`cell_own_style` → `render_style_from`). When `cf == true` a populated cell's fill/font
   differential comes from `document.extended_render_style(sheet, cell, theme)` (the effective
   base+CF style; the engine returns the base style when no rule matches, so every cell is correct).
   Side-table fields (num_fmt / font size / font family / border) are NOT part of the CF format this
   pass, so they keep resolving from the cell's base own style exactly as today. Band (row/col)
   styles keep the base path (CF is per-cell).

2. **Value-change invalidation.** After a recompute publishes new values, CF results can change with
   no CF command (a `CellIs` threshold crosses; a Top-N / average cell enters or leaves the set; a
   color scale re-interpolates). Because a rule can be **global**, the touched-cell mirror is
   insufficient — so for each resident CF sheet the worker rebuilds the whole style cache via the
   extended path (`build_and_store_cache`) and emits `StyleCacheUpdated`. This is the new
   coupling (value publish → style refresh) that did not exist before.

Grid paint is UNCHANGED: the overlay lives inside `RenderStyle` (fill / font_color / bold / italic),
which `grid/view.rs` already paints. `ExtendedStyle.icon/data_bar/rating` stay dropped (deferred
families). No UI in this phase.

**Performance invariant (unmistakable in code + here):** every added cost is behind a CF gate.
`build_sheet_cache`/`refresh_cell` branch on `cf`; the worker's value-invalidation short-circuits on
`self.shared.cond_fmt.is_empty()` (the published CF map is empty ⟺ no sheet has any rule, maintained
by P2) BEFORE touching any resident sheet, and only runs at all on a recompute (`needs_eval`). A
non-CF workbook therefore does zero extra reads and zero extra rebuilds — the exact existing fast
path.

## Steps

1. **`freecell-core/src/cache.rs` — enumerate resident sheets.** Add
   `SheetCaches::resident_ids(&self) -> Vec<SheetId>` (`self.sheets.keys().copied().collect()`), the
   snapshot the worker iterates to rebuild CF sheets after a recompute (taken as an owned Vec so the
   read lock is released before the per-sheet `&mut` rebuilds).

2. **`freecell-engine/src/cache.rs` — thread `cf: bool` into `build_sheet_cache`.**
   - Signature: `build_sheet_cache(doc, sheet_idx, cf: bool) -> Result<SheetCache, String>`.
   - In the per-cell loop over `ws.sheet_data`, choose the base render fields by the flag:
     ```rust
     let mut rs = if cf {
         // CF sheet: the cell's EFFECTIVE style (base + winning CF overlay) supplies the
         // fill/font differential. Side-table fields aren't part of the CF format this pass, so
         // they still resolve from the base own style below, exactly like the non-CF path.
         doc.extended_render_style(sheet_idx, cell, doc.workbook_theme())
     } else {
         render_style_from(&style, doc.workbook_theme())
     };
     ```
     then the existing four side-table resolutions (`num_fmt`/`font_size_q`/`font_family`/`border`
     from `style`, the base own style) and the existing `on_band` default-check are UNCHANGED.
   - Band and per-band loops are untouched (CF is per-cell).

3. **`freecell-engine/src/cache.rs` — thread `cf: bool` into `refresh_cell`.**
   - Signature: `refresh_cell(cache, doc, sheet_idx, cell, def_sz, def_name, cf: bool)`.
   - In the `Some(style)` arm, pick the base render fields the same way (`cf` → `extended_render_style`,
     else `render_style_from(&style, …)`); the side-table resolutions + on-band default handling stay
     as-is. The `None` arm (absent cell → clear) is unchanged. This keeps a **style** edit on a CF
     sheet (which does not recompute) correct: the edited cell keeps its CF overlay.

4. **`freecell-engine/src/document/cond_fmt.rs` — drop the `#[allow(dead_code)]`** on
   `extended_render_style` (now consumed by the cache).

5. **`freecell-engine/src/worker/run.rs` — pass the CF flag from the two callers.**
   - `build_and_store_cache`: after resolving `idx`, `let cf = self.doc.has_cond_fmt(idx);` and call
     `cache::build_sheet_cache(&self.doc, idx, cf)`.
   - `refresh_cache_cells`: after resolving `idx` (per refresh entry), `let cf =
     self.doc.has_cond_fmt(idx);` once (not per cell) and pass it to `cache::refresh_cell(...)` in the
     per-cell branch. The band-creating branch already delegates to `build_and_store_cache`, which
     computes its own flag.

6. **`freecell-engine/src/worker/run.rs` — value-change invalidation helper.** Add:
   ```rust
   fn refresh_cf_caches_after_recompute(&mut self, already_rebuilt: &[SheetId]) {
       // Fast gate: no CF anywhere → empty published map → nothing value-dependent to refresh.
       // Keeps a non-CF workbook on the exact fast path (no resident scan, no has_cond_fmt reads).
       if self.shared.cond_fmt.read().is_empty() {
           return;
       }
       let resident: Vec<SheetId> = self.shared.caches.read().resident_ids();
       for sheet in resident {
           if already_rebuilt.contains(&sheet) {
               continue; // already fully rebuilt by the caller (CF/structural op) this batch
           }
           let Some(idx) = self.resolve(sheet) else { continue };
           if !self.doc.has_cond_fmt(idx) {
               continue; // non-CF sheet → its cache is value-independent
           }
           if self.build_and_store_cache(sheet) {
               self.emit(WorkerEvent::StyleCacheUpdated { sheet });
           }
       }
   }
   ```
   A full rebuild per CF sheet per recompute is the intended correct choice (global rules depend on
   the whole range).

7. **`freecell-engine/src/worker/run.rs` — wire the helper into the value-changing publish paths.**
   Call it only where a recompute actually (re)published values, so a non-recompute publish
   (viewport, font, chart) never triggers it:
   - `apply_edit_batch` (covers normal edits AND undo/redo — both route here): right after
     `apply_cache_refresh(...)`, `if needs_eval { self.refresh_cf_caches_after_recompute(&cf_sheets); }`
     (`cf_sheets` = this batch's full-rebuild set, so a CF/structural op isn't rebuilt twice).
   - `commit_paste`, `apply_replace_all` (when `n > 0`), `commit_replacements`: after their
     touched-cell refresh loop, `self.refresh_cf_caches_after_recompute(&[]);` (these paths do no
     separate full CF rebuild). Each is guarded by the empty-map gate, so non-CF paste/replace is
     unaffected.

## Tests

**`freecell-engine/src/cache.rs` (build/refresh gate):**
- `cf_build_applies_matching_highlight_and_gate` — a `CellIs > 100` fill rule on `A1:A10`,
  `A1 = 150`, `A5 = 50`. `build_sheet_cache(cf = true)`: cell (0,0) render_style fill == RED, cell
  (4,0) unstored (base). `build_sheet_cache(cf = false)` on the SAME doc: cell (0,0) has NO fill
  (base own style carries none) — proves the flag gates the extended read.
- `cf_build_color_scale_interpolates_into_cache` — a 2-color (green→red) scale over `A1:A3` with
  `0/50/100`; `build(cf = true)` stores an interpolated fill on the mid cell that is neither endpoint.
- `cf_refresh_cell_keeps_overlay` — a `>100` fill rule, `A1 = 150`, `build(cf = true)` (A1 filled).
  Bold A1 in the engine, `refresh_cell(cf = true)` → A1 render_style is BOTH filled AND bold (the
  overlay survives a style edit); the same refresh with `cf = false` drops the fill (gate proof).

**`freecell-engine/src/worker/run.rs` (worker publish seam):**
- `cf_value_change_flips_cached_style_no_cf_command` — a **Top-1** rule on `A1:A3`, values
  `10/20/30` (A3 is top → A3 filled in the cache). `SetCellInput A1 = 100` (no CF command) → the
  cache now fills A1 and clears A3, proving a **global** rule re-evaluated the whole range from a
  value publish.
- `cf_threshold_value_change_flips_cached_style` — a `>100` rule on `A1:A10`; `A1 = 50` (unfilled) →
  `A1 = 150` (cache fill == RED) → `A1 = 50` (fill gone), each via `SetCellInput`, no CF command.
- `cf_color_scale_reflected_in_render_cache` — a green→red scale over `A1:A3` (`0/50/100`) with a
  covering viewport; the resident cache's mid cell holds an interpolated (non-endpoint) fill.
- `non_cf_value_edit_stays_on_fast_path` — a non-CF sheet, a covering viewport, a value edit: the
  published CF map stays empty (the invalidation gate short-circuits) and `worker_cache_agrees`
  (cache == fresh BASE engine re-read) holds — no CF behavior leaked onto a non-CF workbook.

Reuses the existing worker CF fixtures (`gt_rule`/`add_cf`) and cache helpers (`test_worker`,
`worker_cache_agrees`), adding a small `cache_fill(worker, sheet, r, c)` reader.

## Notes / non-goals

- Empty cells are not cached (the build iterates populated cells only), so a `Blanks` rule does not
  paint empty cells — consistent with the spec's "for each populated cell" and unchanged here.
- A value edit on a CF sheet may emit `StyleCacheUpdated` twice (the per-cell mirror, then the full
  CF rebuild); harmless (idempotent repaint) and confined to CF sheets.
- No grid-paint change → no pixel-suite work in this phase (deferred to P10, per the plan).
