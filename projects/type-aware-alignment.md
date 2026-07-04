# Type-Aware Default Cell Alignment (+ number-format text color)

**Status: Future (deferred from MVP Phase 13, 2026-07-04).**

## Goal

Render the two per-cell display attributes that require the engine to publish more than
a display string, matching `functional_spec.md §3.6`:

1. **Type-based default horizontal alignment.** When a cell has *no explicit* alignment
   in its style, Excel's defaults apply: **text left, numbers/dates right,
   booleans/errors center.** FreeCell currently defaults **all** cells to left.
2. **`[Red]`-style number-format text color.** A number format like `#,##0.00;[Red]…`
   colors the displayed text (e.g. negatives red). FreeCell currently renders all display
   text in the default color.

## Current MVP behavior (the limitation)

- `PublishedCell { row, col, display_text, text_color }` carries the engine's formatted
  string and an *optional* color that the worker **always publishes as `None`**
  (`worker/run.rs build_publication`). It carries **no value type**.
- The grid resolves alignment as `style.h_align.unwrap_or(Align::Left)`
  (`grid/view.rs cell_element`) — so a General-format number, date, boolean, or error
  renders **left-aligned** unless the file's style sets explicit alignment (explicit
  alignment *does* render correctly — see `cell_align_*` baselines).
- Result, visible in the committed render baselines: `cell_number_plain` (`42`),
  `cell_number_currency` (`$1,234.50`), `cell_number_percent` (`50%`), `cell_date_default`
  (`2021-01-01`), `cell_boolean` (`TRUE`), and `cell_error_div0` all render left-aligned;
  `cell_number_negative_red` (`-1,234.50`) renders in the default (not red) color.

Phase 6 recorded this deferral ("type-based default right-alignment needs the engine to
publish the value type … deferred to Phase 11 engine wiring"); Phase 11 did not land it,
and Phase 13 (completion sweep, **no new features**) chose to **track it as a known
limitation** rather than change the publication schema + regenerate ~10 baselines at the
finish line.

## Why it's a limitation, not a blocker

The MVP is a "workable functional proof of concept," explicitly "not design-polished"
(`functional_spec.md §1`). Values, formats (currency/percent/date/thousands), and error
text are all **correct**; only the default *alignment* and the format *color* differ from
Excel. Explicit alignment and all other rendering are correct.

## Work when picked up

1. Extend `PublishedCell` with the engine's value type (or a pre-resolved default
   `Align`) — e.g. read IronCalc `get_cell_type` in `build_publication` and map
   Number/Date→Right, Boolean/Error→Center, Text→Left.
2. Populate `text_color` from the number format's palette-index color (the Phase-4
   deferral: map IronCalc's `Formatted.color` palette index → RGB via the indexed-color
   table that belongs with the style cache).
3. In `grid/view.rs`, use the published default when `style.h_align` is `None`; apply
   `text_color` when present.
4. Update the render-test fixtures (`render-tests/src/scene.rs`) that construct
   `PublishedCell`s; regenerate + **eyeball** the affected baselines (`cell_number_*`,
   `cell_boolean`, `cell_date_default`, `cell_error_*`, `cell_number_negative_red`,
   `grid_mixed_content`); keep the foreground render suite green.
5. Add render cases for the type-based defaults (e.g. `cell_number_default_right`,
   `cell_boolean_default_center`) so the behavior is pinned.
