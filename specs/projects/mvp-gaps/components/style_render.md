---
status: complete
---

# Component: Style Cache & Render Extensions (fonts, borders, num-fmt, types)

## Purpose and scope

Everything that flows engine → resident cache → pixels for the new formatting:
`RenderStyle`/`SheetCache` extensions (font family/size, borders, num-fmt strings,
merge list), the border paint algorithm, per-cell font rendering, row auto-grow, and
the publication additions (cell kind + resolved text color). NOT responsible for:
control UI (action_bar.md), engine write APIs (architecture §3).

Touches: `freecell-core/src/{style,cache,publication}.rs`,
`freecell-engine/src/{cache.rs, worker/run.rs}`, `freecell-app/src/grid/view.rs`.
Architecture refs: §1, §3.3–3.4. All engine facts cited there are verified.

## Data model (final shape)

`RenderStyle` additions (stays `Copy + Eq + Hash`, interned):

```rust
pub font_size_q: u16,   // quarter-points; 0 = engine default (11pt)
pub font_family: u16,   // idx into SheetCache.font_families; 0 = default
pub border: u16,        // idx into SheetCache.border_specs; 0 = none
pub num_fmt: u16,       // idx into SheetCache.num_fmts; 0 = "general"  (NEW vs architecture v1 —
                        // needed by the action bar's category display + decimals ±)
```

`SheetCache` side tables (built worker-side, swapped atomically with the cache):

```rust
pub font_families: Vec<SharedString>,   // [0] = ""
pub num_fmts: Vec<SharedString>,        // [0] = "general"
pub border_specs: Vec<BorderSpec>,      // [0] = NONE
pub merges: Vec<CellRange>,             // parsed from worksheet().merge_cells (guard UI)
pub struct BorderSpec { pub top/right/bottom/left: Option<Edge> }
pub struct Edge { pub weight: u8 /*1|2|3*/, pub color: Rgb }
```

`PublishedCell` addition: `pub kind: CellKind` (`Number|Date|Text|Bool|Error`), and
`text_color` actually populated. Full derivation (get_cell_type mapping, date-format
heuristic, `[Red]` via the public `formatter::format_number`, precedence explicit
font color > format color) is specified in architecture §1.2 — implement exactly that;
the color-index table (0–6 named + 56-entry classic indexed palette) lives as consts
in `freecell-core::format_color`.

## Internal design

### Cache build (freecell-engine/src/cache.rs, build + refresh paths)

The build already reads each populated/styled cell's engine `Style` and interns
`RenderStyle`s (cache.rs:143-151), and already resolves row/col **band** styles
(document.rs:340-353; precedence cell > row band > col band > default — verified
engine-side, worksheet.rs:76-105). Extensions ride the same loop:

1. Interning maps built per rebuild: `HashMap<String,u16>` for families and num-fmt
   strings, `HashMap<BorderSpec,u16>` for borders (entry 0 pre-seeded defaults).
2. Per style resolved: `font.name`→family idx, `font.sz`→`(sz*4) as u16` (0 when it
   equals the engine default 11), `border`→BorderSpec idx (weight map: Thin/Dotted/
   Hair/Dashed→1, Medium*/SlantDashDot→2, Thick/Double→3; all drawn solid; color
   from the border item, default `#000`), `num_fmt`→idx.
3. Band styles contribute the same fields (a whole-column font/border from a band
   resolves into the cell's effective RenderStyle exactly like fill does today).
4. `merges`: parse each `"K7:L10"` A1 string once (shared parser in freecell-core,
   also used by the worker guard). Parse failures: skip entry, log (defensive vs
   hostile files; never panic).

Invalidation: unchanged — style edits already trigger cache rebuild + publish
(architecture §2: style-only ops rebuild without evaluation).

### Border painting (grid/view.rs cell loop)

Rule (simple, correct under overdraw): **for each visible cell whose
`RenderStyle.border != 0`, draw all four of its effective edges.**

- Effective edge = the heavier of the cell's own edge and the adjacent neighbor's
  opposing edge (`max_by weight`, ties → the cell's own). Neighbor styles come from
  the same frame-held cache snapshot (sparse map lookup; cells without styles have
  no border — the common case short-circuits).
- Two bordered neighbors each draw the shared edge; both compute the same effective
  edge, so the overdraw is pixel-identical (harmless by construction).
- Edge quad geometry: vertical edge between cols c,c+1 at `x = right(c) - w/2`,
  width `w` (1–3 px), spanning the cell's row height; horizontal analog. Drawn as
  absolute divs appended **after** the cell content layer (paints over gridlines);
  gridline underneath is simply covered.
- Viewport boundary: a bordered cell at the left/top viewport edge still draws its
  left/top edge (its own spec suffices; the off-screen neighbor could only make it
  heavier — accepted 1-frame-class approximation, disappears when both are visible).
- Perf: quads only for bordered cells (≤4 each). CI harness assertion: 500-bordered-
  cell viewport stays within the existing frame budget (architecture §9).

### Per-cell font rendering (grid/view.rs `cell_element`, :1231-1282)

- `font_family != 0` → `.font_family(cache.font_families[idx].clone())`;
  `font_size_q != 0` → `.text_size(px(q as f32 / 4.0 * 96.0/72.0))` (pt→px);
  combine with existing bold/italic/underline. Missing families fall back via gpui's
  fallback stack (verified) — style preserved, display-only fallback.
- The live-mirror and optimistic-pending cells intentionally ignore font fields
  (default font — functional spec §1.2).
- Vertical centering: unchanged (div layout centers); tall fonts in short rows clip
  exactly like long text clips horizontally today — auto-grow (below) prevents the
  common case.

### Row auto-grow (worker, inside the SetFont command handler)

After the `on_paste_styles` write (architecture §3.3): for each row in the clamped
target area the row grows **proportionally**, keeping the default row-height :
font-size ratio (`DEFAULT_ROW_HEIGHT_PX : DEFAULT_CELL_FONT_PX` = 24 : 13) at every
size — `needed_device = font_px * 24/13`, converted to IronCalc storage px by
`cache::autofit_row_ironcalc_px` (`= font_px * 25/13`, ceil'd). Read
`get_row_height(row)`; collect rows where `needed > current`; coalesce contiguous runs
with equal target and apply `set_rows_height(run)` per run, then the existing cache
geometry batch update. Never shrinks; never runs on open; lands adjacent to the style
diff in history (undo = height step + style step — two steps, recorded in
DECISIONS_TO_REVIEW as accepted).

The grow is proportional rather than "line box + fixed padding" because gpui renders
each cell at a line box ≈1.618× the glyph (its default), while the engine stores row
heights in IronCalc px. The old `ceil(font_px*1.25)+4` formula grew a large-font row
*less* than that line box, so the overflowing box inverted top/bottom vertical
alignment (align-top drifted toward the bottom and vice-versa). A proportional row
(ratio 1.85 > 1.618) always contains the line box with the same proportional slack the
default cell has, keeping alignment meaningful at any font size.

## Dependencies

Depends on: engine Style/border/num-fmt reads (all verified), existing cache
build/refresh + generation plumbing, A1-range parser (shared with grid_structure
guard). Depended on by: action_bar (num_fmts + families tables, merge list via
cache), grid_structure (merges), grid rendering, render suite.

## Test plan

Unit (freecell-core):
- `border_weight_mapping_all_nine_styles`; `border_spec_interning_dedups`.
- `effective_edge_heavier_wins_and_tie_prefers_own`.
- `date_format_heuristic` — `m/d/yyyy`✓, `[Red]0.00`✗, `"months"@`✗ (quoted literal
  stripped), `h:mm`✓, `#,##0.00`✗, `yyyy\-mm`✓.
- `format_color_index_table` — named 0–6 + `[Color 12]` classic palette.
- `font_size_q_roundtrip_and_default_zero`; `a1_range_parse_valid_and_hostile`.
- `autofit_row_keeps_default_ratio` — 13px default font → no grow (default row); larger
  fonts grow the row proportionally (device height = font_px * 24/13), always clearing
  the font's ≈1.618× line box.
Engine integration (freecell-engine):
- `cache_carries_font_border_numfmt_from_file` — fixture xlsx with fonts/borders/
  formats: resolved RenderStyle fields + side tables correct.
- `band_font_resolves_into_cells` — column-band font from file reaches a cell's
  RenderStyle.
- `set_font_grows_rows_and_undo_two_steps`.
- `published_kind_and_red_color` — negative currency cell publishes Date/Number kinds
  correctly + red text_color; explicit font color wins.
- `merge_list_parsed_into_cache` (and survives rebuild).
Render suite (names final): `border_all_thin`, `border_outer_medium`,
`border_heavier_edge_wins`, `border_over_fill`, `font_family_serif`,
`font_size_24_row_grown`, `text_color_red`, `format_red_negative`,
`align_number_default_right`, `align_error_center`, `align_explicit_beats_default`,
`font_missing_family_fallback`.
Perf: bordered-viewport frame-budget assertion (harness).
