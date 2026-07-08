---
status: complete
---

# Phase 2: Part 2 — Border engine + rendering foundation

## Overview

Plumbs **border line style + color** end-to-end through the existing formatting seam
(`freecell-core` render model → `freecell-engine` protocol/document/worker/cache →
`freecell-app` grid render), and implements the two new grid paint patterns (dashed +
double). No IronCalc fork changes (`architecture.md §0`). **Explicitly out of scope
(Phase 3):** the borders popover redesign, pen/target view state, and target icons. The
existing preset buttons keep working — the command just carries a default line + color so
nothing breaks.

- `Edge` gains a `LinePattern` discriminant (Solid/Dashed/Double); heavier-wins is unchanged
  (the winning edge carries its own pattern).
- `SetBorders` gains `line: BorderLine` + `color: Option<Rgb>`; the hardcoded thin-black
  border item in `document.rs::set_borders` is parameterized.
- Cache maps `BorderStyle` → (weight, pattern) with a **solid fallback** for the deferred
  styles (Dotted / dash-dot family / SlantDashDot render solid, unchanged — no regression on
  files already containing them).
- Grid renders dashed (run of short rects) and double (two 1px strips) edges; solid unchanged.

## Steps

1. **`freecell-core/src/border.rs`** — add `LinePattern { Solid, Dashed, Double }` (derive
   `Default` = `Solid`, plus `Debug/Clone/Copy/PartialEq/Eq/Hash`); add `pattern: LinePattern`
   to `Edge`. Keep `Edge::new(weight, color)` producing a **solid** edge (backward-compatible —
   every existing caller means solid); add `Edge::with_pattern(weight, color, pattern)`.
   `effective_edge` unchanged (whole `Edge` carried, weight still decides the winner). Update
   the module/struct docs (Dotted/dashed no longer "all drawn solid").
2. **`freecell-core/src/lib.rs`** — export `LinePattern` from `border`.
3. **`freecell-engine/src/worker/protocol.rs`** — add `BorderLine { ThinSolid, MediumSolid,
   ThickSolid, Dashed, Double }` (derive `Default` = `ThinSolid`) with
   `style_tag() -> &'static str` (`ThinSolid→"thin"`, `MediumSolid→"medium"`,
   `ThickSolid→"thick"`, `Dashed→"mediumdashed"`, `Double→"double"`). Extend
   `Command::SetBorders` with `line: BorderLine` + `color: Option<Rgb>`. Update the `SetBorders`
   + `BorderPreset` doc comments (no longer "thin black only").
4. **`freecell-engine/src/document.rs`** — parameterize `set_borders`:
   `set_borders(sheet_idx, range, border_type: &str, style_tag: &str, color_hex: &str)`; build
   the `BorderArea` JSON with `"style": style_tag, "color": color_hex` (drop the hardcoded
   `thin`/`#000000`). Update the doc comment.
5. **`freecell-engine/src/worker/run.rs`** — in the `SetBorders` arm, resolve
   `color_hex = color.map(|c| format!("#{:06X}", c.to_hex())).unwrap_or("#000000")` and call
   `doc.set_borders(idx, *range, preset.border_type_tag(), line.style_tag(), &color_hex)`. Still
   `AppliedKind::StyleOnly`; the `AppliedOp::Cells { .. }` `expand_by_one_cell` arm already uses
   `..` (unchanged).
6. **`freecell-engine/src/cache.rs`** — add `border_pattern(&BorderStyle) -> LinePattern`
   (`MediumDashed→Dashed`, `Double→Double`, everything else → `Solid` fallback); `edge_from`
   builds `Edge::with_pattern(border_weight, color, border_pattern)`. Import `LinePattern`.
7. **`freecell-app/src/grid/view.rs`** — `vertical_edge_quad`/`horizontal_edge_quad` return
   `Vec<AnyElement>` and branch on `edge.pattern`: `Solid` → today's single strip; `Dashed` → a
   run of short filled rects (dash/gap constants, clamped to the span); `Double` → two 1px
   parallel strips spanning the weight. The four call sites use `.extend(...)` instead of
   `.push(...)`.
8. **`freecell-app/src/chrome/view.rs`** — `apply_borders` sends the default pen
   (`line: BorderLine::ThinSolid, color: None`) so the existing preset buttons are unchanged.
   Import `BorderLine`.
9. **render-tests** — `Scene::border` already takes a `BorderSpec`; add a pattern-aware `edge`
   helper in `cases.rs` and new cases `border_dashed_all`, `border_double_all`,
   `border_pattern_mixed`; register them in `render_suite.rs`. Regenerate + eyeball baselines;
   confirm the six existing border baselines are byte-unchanged (solid path untouched).

## Tests

- **core `border.rs`**: extend `border_spec_none_is_default` / add a case asserting
  `Edge::new` defaults to `Solid` and `Edge::with_pattern` carries the pattern; `effective_edge`
  still picks the heavier edge (and its pattern rides along).
- **engine `document.rs`**: extend `set_borders_applies_all_and_none_clears` to pass a pen
  (`mediumdashed` + `#FF0000`) and assert the written `BorderItem.style`/`.color`; add a case
  that paints `Outer` over a cell with an existing interior/opposite edge and asserts the
  non-targeted edge survives (non-destructive per-type application).
- **worker `run.rs`**: update the two existing `SetBorders` constructions for the new fields;
  add a test sending `line: Dashed, color: Some(red)` and asserting the resolved cache `Edge`
  has `pattern == Dashed` and the red color.
- **cache `cache.rs`**: add `border_pattern_mapping_all_nine_styles` (Dashed/Double/solid
  fallback for the seven others); extend `border_spec_from_reads_all_four_edges_and_colour` to
  assert `pattern` on a MediumDashed / Double edge.
- **chrome `view.rs`**: update the `SetBorders` construction/match in the borders tests; the
  existing `apply_borders_sends_command_over_selection` still asserts one `SetBorders` over the
  selection (now with the default line + `color: None`).
- **render cases (manual gate)**: `border_dashed_all`, `border_double_all`,
  `border_pattern_mixed` — baselines regenerated + eyeballed; full `render_tests.sh test` green
  with the six existing border baselines byte-identical (solid/weight/color unchanged).
