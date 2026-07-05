---
status: complete
---

# Phase 6: Borders

## Overview

Adds cell borders end-to-end: a `border: u16` field on `RenderStyle` interned into a new
`SheetCache.border_specs` side table (mirroring the Phase-4 `num_fmt` / Phase-5 `font_family`
side tables), a grid edge-paint pass, and a borders-preset popover on the action bar that
applies presets via IronCalc 0.7.1 `set_area_with_border` (`functional_spec.md §3.4/§3.6`,
`components/style_render.md`, `components/action_bar.md`, `architecture.md §3.4`).

## API verification (done before coding — pinned ironcalc_base 0.7.1)

- `UserModel::set_area_with_border(&mut self, range: &Area, border_area: &BorderArea) -> Result<(), String>`
  exists (`src/user_model/border.rs:346`), undoable (one `push_diff_list`), band-aware
  (full columns → `set_columns_with_border`, full rows → `set_rows_with_border`), and applies
  a heavier-wins fix-up to **all four** adjacent strips (above/below/left/right — verified
  `border.rs:380-503`).
- `BorderArea { item: BorderItem, r#type: BorderType }` has `pub(crate)` fields + no constructor
  but derives `Serialize/Deserialize` → build via `serde_json::from_value(json!({"item":{"style":
  "thin","color":"#000000"},"type":"All"}))`. `BorderType` variants serialize PascalCase
  (`All|Inner|Outer|Top|Right|Bottom|Left|None`, plus unused `CenterH/CenterV`);
  `BorderStyle` serde is lowercase. `BorderArea` is re-exported at `ironcalc_base::BorderArea`.
- `Style.border: Border` is public; `Border { left/right/top/bottom: Option<BorderItem>, .. }`,
  `BorderItem { style: BorderStyle, color: Option<String> }` — all public → the cache build reads
  them directly.
- **Spec correction (weight map):** `architecture.md §1.1` lists `Hair`/`Dashed` variants that do
  **not** exist at 0.7.1. The real 9 `BorderStyle` variants map:
  Thin/Dotted→1; Medium/MediumDashed/MediumDashDot/MediumDashDotDot/SlantDashDot→2; Thick/Double→3.

## Steps

### freecell-core

1. **`src/border.rs` (new)** — engine-free border render types:
   ```rust
   pub struct Edge { pub weight: u8 /*1|2|3*/, pub color: Rgb }   // Copy+Eq+Hash
   pub struct BorderSpec { pub top: Option<Edge>, pub right: Option<Edge>,
                           pub bottom: Option<Edge>, pub left: Option<Edge> } // Copy+Eq+Hash+Default
   impl BorderSpec { pub const NONE; pub fn is_none(&self) -> bool }
   /// Heavier of two opposing edges; ties → `own` (the drawing cell's own edge).
   pub fn effective_edge(own: Option<Edge>, neighbor: Option<Edge>) -> Option<Edge>
   ```
   `BorderSpec::default() == NONE` so a default cell interns to index 0.

2. **`src/style.rs`** — add `pub border: u16` to `RenderStyle` (stays `Copy+Eq+Hash+Default`;
   `0` = index into `border_specs[0] = NONE`). Update `render_style_default_is_plain`.

3. **`src/cache.rs`** — add the `border_specs: Vec<BorderSpec>` side table to `SheetCache` +
   `SheetCacheBuilder` (seed `[0]=NONE`), `intern_border_spec`/`border_spec`/`border_specs()`
   mirroring `intern_num_fmt`/`num_fmt_code` exactly (linear-scan intern, `u16::MAX` overflow guard
   → resolves to NONE). Thread through `build()`.

4. **`src/lib.rs`** — `pub mod border;` + `pub use border::{BorderSpec, Edge, effective_edge};`.

### freecell-engine

5. **`src/cache.rs`** — add `border_weight(&BorderStyle) -> u8` (the verified 9-variant map),
   `edge_from(&BorderItem) -> Edge` (weight + `parse_color`, default `#000`), and
   `border_spec_from(&Border) -> BorderSpec`. In `build_sheet_cache` (per-cell + row-band +
   col-band loops) and `refresh_cell`, resolve `rs.border = intern_border_spec(border_spec_from(
   &style.border))`. Update `render_style_from` doc (borders no longer dropped) and
   `assert_cache_agrees` to also compare the resolved `BorderSpec`.

6. **`src/document.rs`** — `set_borders(&mut self, sheet_idx, range: CellRange, preset: BorderPreset)
   -> Result<(), String>`: map preset → `BorderType` tag, build `BorderArea` via `serde_json`,
   call `self.model.set_area_with_border(&area_of(sheet_idx, range), &ba)`. Thin/black only.

### freecell-engine worker

7. **`src/worker/protocol.rs`** — `pub enum BorderPreset { All, Inner, Outer, Top, Bottom, Left,
   Right, None }` (engine-free, `Copy+Eq`) + `Command::SetBorders { sheet, range, preset }`.

8. **`src/worker/run.rs`** — route `SetBorders` into the coalesced **edit** bucket (it is one
   undoable style-only diff-list, band-aware — like `SetStylePath`); `apply_one` →
   `doc.set_borders(idx, range, preset)?` → `AppliedKind::StyleOnly`; `op_of` →
   `AppliedOp::Cells { sheet, range: expand_by_one(range) }` so the mirror refresh covers the four
   adjacent strips the engine fixed up (expansion clamps to sheet bounds; a full row/col stays
   band-creating → full rebuild).

9. **`src/worker/mod.rs` + `src/lib.rs`** — re-export `BorderPreset`.

### freecell-app

10. **`src/grid/view.rs`** — snapshot `visible_border_specs: Vec<BorderSpec>` in `resolve_frame`
    (alongside `visible_font_families`); in `build_grid_layers`, after the cell-fill loop, append
    border edge quads. **Draw-once rule** (each shared edge drawn exactly once): for each visible
    cell with `border != 0`, always draw its **right** + **bottom** effective edges; draw its
    **left** only when `col == cols.start` or the left neighbor is unbordered; **top** only when
    `row == rows.start` or the top neighbor is unbordered. Effective edge = `effective_edge(own,
    neighbor-opposing)`. Quads are absolute `rect_div`s (`weight` px, centered on the boundary),
    appended after fills so they paint over gridlines/fills.

11. **`src/chrome/view.rs`** — Borders button (`⊞ ▾`) at the `[Borders land in Phase 6 here.]`
    marker + a `borders_open` popover (4×2 preset icon grid: All/Inner/Outer/None over
    Top/Bottom/Left/Right); `apply_borders(preset)` commits any pending edit, degraded-guards, and
    sends `Command::SetBorders`. Close on pick/backdrop; close in `set_degraded`.

### render harness

12. **`render-tests/src/scene.rs`** — `Inject::Border(u32,u32,BorderSpec)` + `.border(row,col,spec)`
    (interns into the real cache). Helper ctors for thin-black specs.

13. **`render-tests/src/cases.rs`** — new border cases (see Tests).

## Tests

- **core `border.rs`**: `effective_edge_heavier_wins_and_tie_prefers_own`;
  `border_spec_none_is_default`.
- **core `cache.rs`**: `border_spec_interning_dedups` (NONE→0, equal specs dedup, round-trip,
  out-of-range→NONE, builder + built cache agree; `border` participates in `StyleId` identity).
- **core `style.rs`**: default includes `border == 0`.
- **engine `cache.rs`**: `border_weight_mapping_all_nine_styles` (all 9 `BorderStyle` variants);
  `cache_carries_border_from_file` (set a border via `set_area_with_border`, rebuild → resolved
  `BorderSpec` + side table correct; plain cell unstored); `assert_cache_agrees` extended to
  borders (existing fixture sweeps still green).
- **engine `document.rs`**: `set_borders_roundtrips_through_engine` (All preset → cell `.border`
  has all four thin edges; None clears).
- **engine `worker/run.rs`**: `set_borders_applies_and_undo` (bounded range → one undo step; cache
  reflects + undo reverts); `set_borders_full_column_is_band` (full-column preset lands as a band,
  does not materialize 1M cells; adjacent-strip refresh via the expanded range).
- **chrome `view.rs`**: `borders_popover_toggles`; `apply_borders_sends_command`;
  `borders_disabled_in_degraded_mode`.
- **render suite (names final; regen on the pinned runner — see DECISIONS):**
  `border_all_thin`, `border_outer_medium`, `border_heavier_edge_wins`, `border_over_fill`,
  `border_shared_edge_adjacent`, `border_none_clear`. All **additive** (no existing baseline
  changes). Recorded in `DECISIONS_TO_REVIEW.md`.
