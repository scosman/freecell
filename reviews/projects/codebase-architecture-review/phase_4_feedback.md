# Phase 4: Chart subsystem

Scope: `freecell-chart-model` (4.5k), `freecell-engine/src/chart/` (~11k), `freecell-app/src/chart/`
+ `grid/chart_layer.rs` (~4k), plus the chart lifecycle code in `freecell-engine/src/worker/run.rs`
and the chart tests. Read-only architectural review; no builds run.

---

## What's Good

- **[app/crates/freecell-chart-model/Cargo.toml:1] The three-layer seam is real, and it is the
  strongest boundary in the repo.** `freecell-chart-model` has *zero* `[dependencies]` — not
  gpui, not ironcalc, not even an XML crate. That is not a claim in a doc comment; it is
  structurally enforced by the manifest. The app layer touches `freecell_engine::chart` in exactly
  one place (`shell/demo.rs:130`, a demo path) and the perf binary; everything else goes through
  the model. Compared with the rest of the codebase this is unusually disciplined layering, and it
  is the reason the model crate's ~90 unit tests run headless.

- **[app/crates/freecell-engine/src/chart/save.rs:935] Save is source-first, not
  round-trip-through-the-model — the right call.** `patch_chart_source` re-parses the retained
  `chartN.xml`, diffs it against the target model, and splices *byte ranges* for only the fields
  that changed, in the file's own namespace prefixes. Unmodeled DrawingML (gradients, effects,
  `txPr`, layout, `extLst`) survives byte-for-byte. The alternative — parse → model → regenerate —
  would have been far easier to write and catastrophically lossy. The `_FOLLOWING` schema-order
  tables (`save.rs:1006-1108`) that anchor a fresh insert before the first present later sibling
  are a genuinely clever, correct solution to "insert a child into arbitrary third-party XML and
  stay schema-valid".

- **[app/crates/freecell-app/src/grid/chart_layer.rs:266] The grid seam is clean and correctly
  costed.** `ChartPlacement` is a `Copy` 40-byte struct (anchor + fidelity); the heavy `Chart`
  stays behind a shared `Arc<[ChartSpec]>`. The per-frame path scans placements and culls with
  `is_offscreen`, materializing render elements only for on-screen charts. Geometry enters through
  a `GridGeometry` trait so the anchor→pixel mapping is unit-tested against a mock. `in_grid.rs`
  keeps the fidelity→render-mode decision out of the grid entirely and testable without a `Frame`.
  This is what "bolted on" does *not* look like.

- **[app/crates/freecell-chart-model/src/spec.rs:171] Make-illegal-states-unrepresentable is used
  where it earns its keep.** `Origin::{Loaded{source}, Authored}` makes "authored charts have no
  retained source" unrepresentable rather than a pair of fields to keep in sync; `ChartBody::
  Unsupported` does the same for "retained but unparseable". `display_fidelity` is a derived
  accessor rather than stored state, so it cannot go stale.

- **[app/crates/freecell-chart-model/src/downsample.rs:35] The decimation algorithms are correct
  and honest.** Min/max bucketing (not stride sampling) for ordered lines, uniform linspace for
  unordered point clouds, identity below the budget so committed baselines cannot move, and
  applied at the paint call site only — the retained model keeps every point. The doc comments
  explain *why* the two algorithms differ. This is the best-reasoned module in the subsystem.

- **[app/crates/freecell-engine/tests/charts_corpus.rs:1] Failure posture is "retain and badge",
  not "drop".** An unparseable chart part becomes `ChartBody::Unsupported`, keeps its source,
  anchor, and ranges, draws a placeholder, and byte-preserves on save. The broken-drawing fixtures
  assert the discovery walk is per-drawing resilient. That is the correct default for a file-format
  app.

---

## Critical (must fix)

- **[app/crates/freecell-engine/src/chart/load.rs:605 + app/crates/freecell-chart-model/src/fidelity.rs:175]
  Combo charts render silently wrong and are badged `Faithful`.**
  `parse_chart_xml` takes the **first** chart-group element in `c:plotArea`
  (`.find(|n| is_chart_group(...))`) and ignores every subsequent one. `source_fidelity` has no
  multi-group detector at all — it only asks "does this part contain a `lineChart`?", never "how
  many groups are there?". So an ordinary Excel combo chart (bar + line on a secondary axis — one
  of the most common non-trivial chart shapes in real workbooks) loads as bars only, with the line
  series **absent from the picture**, classified `Faithful`, and drawn with no badge. The same
  applies to a secondary value axis: `parse_axes` (load.rs:1045) takes the first `catAx` + first
  `valAx` and drops the rest.
  This is the exact failure mode the entire `fidelity.rs` module exists to prevent, and it is the
  most likely one to hit a real user. The classifier's own docs acknowledge the "combo caveat" six
  times — for *scoping* — while missing that the combo case is not merely mis-scoped, it is
  silently truncated.
  **Direction:** count chart-group children at parse time and force `Fidelity::Degraded` (or
  `Unsupported`) for >1, before shipping anything else in this subsystem. Longer term, classify
  from the parsed structure rather than a text scan (see the Moderate finding on `fidelity.rs`).

- **[app/crates/freecell-engine/src/chart/save.rs:1126, write.rs:308,
  app/crates/freecell-app/src/shell/window.rs:1650] The workbook theme is never read, and saving a
  themed chart writes the wrong color into the file.**
  Nothing in the repo parses `xl/theme/theme1.xml` (`grep clrScheme` → zero non-comment hits).
  `app/src/chart/style.rs:64` hardcodes `ThemePalette::office_default()` with a comment saying "P8
  threads the actual workbook `clrScheme`" — P8 shipped; it does not.
  Worse, three separate call sites hand-roll theme resolution as
  `ThemePalette::office_default().color(*slot)` instead of calling the model's own
  `ChartColor::resolve(&palette)` — so they *also* silently discard the `lumMod`/`lumOff` tint the
  model went to the trouble of parsing and storing. `save.rs::chart_color_srgb` is on the **write**
  path: editing a series color on a chart that referenced `accent1` replaces a `schemeClr` with a
  literal `srgbClr` computed from the wrong palette and with the tint stripped. That is permanent,
  silent color corruption of a user's file, and `fidelity.rs` explicitly *excludes* `schemeClr`
  from the degrading set ("a theme reference we now resolve to a color") so it is not even badged.
  **Direction:** parse `theme1.xml` in the engine and thread a real `ThemePalette` through
  `ChartSpec` (or the snapshot) to both the renderer and the save patcher; delete the three
  hand-rolled resolvers and route every caller through `ChartColor::resolve`.

- **[app/crates/freecell-engine/src/chart/save.rs:1318 + chrome.rs:86] Editing data labels destroys
  the rest of the `c:dLbls` subtree.**
  The chrome patcher upserts data labels by **whole-node replacement**:
  `upsert_child(src, ser, &["dLbls"], Some(chrome::dlbls_element(...)), ...)`. `dlbls_element`
  emits only `numFmt`, `dLblPos`, five `show*` flags, and `separator`. Everything else the file had
  inside that element is discarded: per-point `<c:dLbl>` overrides (custom text, per-point
  position, per-point deletion), `c:txPr` label typography, `c:spPr`, `c:showLeaderLines`,
  `c:leaderLines`, `c:delete`, `c:extLst`.
  This is the one concrete hole in an otherwise genuinely preserve-unknown save path — and it is
  precisely where `fidelity.rs::unsupported_data_labels` already knows we mis-render (`c:dLbl`
  presence degrades). The user sees a badge saying "may not display as intended", toggles a label
  checkbox to fix it, and the per-point labels are gone from the file forever.
  **Direction:** patch `c:dLbls` the way `patch_series_color` already patches `c:spPr` — upsert the
  individual child elements (`showVal`, `dLblPos`, `numFmt`, …) inside the existing node instead of
  replacing it. The pattern is already in the file; it just was not applied here.

- **[app/crates/freecell-engine/src/worker/run.rs:2436 + chart/write.rs:641] Inserting a chart into
  a workbook that already has one makes the file unsaveable.**
  `save_workbook` runs `reinject_live_charts` (which re-injects a `<drawing>` into every
  chart-bearing worksheet) and *then* `write_authored_charts`, which hard-errors on any target
  worksheet whose XML already contains `<drawing`:
  `"worksheet {sheet_part} already carries a <drawing>; ... not yet supported"`. Open a real Excel
  workbook with a chart, insert a second chart on the same sheet, hit Save → `SaveError`, with no
  recovery path offered. The failure is loud rather than lossy, which is better than the
  alternative, but "your workbook cannot be saved" for an ordinary two-step user action is a
  product-breaking gap in how the three write modes compose.
  A second-order consequence sits at run.rs:2495: with any authored chart present,
  `chart_source_path` is deliberately **not** advanced to the just-saved file, so every subsequent
  save stays pinned to the original file on disk. Move or delete that original and saving fails
  permanently.
  **Direction:** the drawing-merge case has to be built (append `twoCellAnchor` frames + rels to an
  existing drawing part) before the insert feature can be considered shipped. Until then the UI
  must at minimum refuse the insert up front rather than at save time.

---

## Moderate (should fix)

- **[app/crates/freecell-chart-model/src/fidelity.rs:1] `fidelity.rs` is a fourth, textual
  re-implementation of the OOXML chart schema — and it is the wrong place for it.**
  The subsystem now encodes OOXML chart knowledge in four independent places that must be kept in
  sync by hand: `load.rs` (roxmltree parse), `write.rs` (full synthesize), `chrome.rs` (patch
  fragments), and `fidelity.rs` (a hand-rolled tag scanner — `any_opening_tag`, `tag_close_offset`,
  `opens_tag_name`, `attr_value`, ~130 lines of bespoke XML lexing in a crate that has no XML
  dependency *by design*). The scanner exists solely so the model crate can stay
  dependency-free, and it pays for that with a classifier that cannot bind an element to its
  enclosing chart group — the limitation the module's own docs apologize for at lines 259, 343,
  400, 420, and 439, and the root cause of the combo Critical above.
  The *concept* of a fidelity badge is legitimate and well-motivated: "render what we can, badge
  what we degrade, placeholder what we can't" is the honest answer for a partial OOXML reader, and
  deriving it on demand so it auto-clears as support lands is genuinely smart. The *implementation*
  is not: it is a second parser racing the real one.
  **Direction:** classify from the structure the loader already produced (it has a DOM, it knows
  the groups, it knows which features it dropped) and hand the resulting `Fidelity` to the model,
  or move `source_fidelity` into the engine and keep only the `Fidelity` enum in the model crate.
  Two sub-symptoms worth fixing regardless:
  - `is_extended_chart` (fidelity.rs:250) is a bare `xml.contains("chartex")` among a file full of
    carefully boundary-aware helpers. A cached cell string or series name containing that
    substring forces the whole chart to the "Unsupported" placeholder.
  - `display_fidelity()` reads like a getter but performs ~25–30 full passes over the chart XML.
    It happens to be called once per chart at install (`ChartPlacement::from_spec`), but nothing
    in its signature or docs stops the next caller from putting it in a loop.

- **[app/crates/freecell-engine/src/chart/authoring.rs:1] 1,622 lines of test-fixture generation
  ship in the production engine crate.** The module is `pub mod authoring;` in `chart/mod.rs`, not
  `#[cfg(test)]`, and every single caller is a test (`load.rs` / `save.rs` / `binding.rs` test mods
  plus four integration test files). It builds hand-crafted `.xlsx` packages: content types, rels,
  sharedStrings, styles, drawings, chart parts. All of it compiles into every release binary.
  It also **collides in name** with `chart-model/src/authoring.rs`, which is real product code
  (the insert-menu templates). Two files named `authoring.rs` in the same subsystem, one product,
  one fixtures.
  **Direction:** move to a `dev-dependencies` fixtures crate or gate behind
  `#[cfg(any(test, feature = "fixtures"))]`, and rename it to `fixtures.rs` (the engine already has
  a top-level `fixtures.rs`, which is where a reader would look).

- **[app/crates/freecell-engine/src/worker/run.rs:2242-3200] ~1,200 lines of chart lifecycle,
  mutation, and undo/redo live inside a 9,288-line `run.rs`, while `worker/charts.rs` is 39
  lines.** `insert_authored_chart`, `set_chart_anchor/range/type/chrome`, `delete_chart`,
  `push_chart_undo`, `undo_chart_op`, `redo_chart_op`, `undo_chart_entry`, `redo_chart_entry`,
  `apply_chrome_edit`, `resolve_authored_chart`, `next_chart_part`, `existing_chart_parts`,
  `charts_by_sheet_with_authored`, `store_chart_snapshot`. The subsystem has three dedicated
  crates/modules and then dumps its entire controller into the largest file in the engine, where it
  is interleaved with cell editing, find/replace, conditional formatting, and everything else the
  worker does.
  **Direction:** `worker/charts.rs` already exists and is nearly empty — move the chart command
  handlers and the chart undo stack into it (or a `worker/charts/` directory) with `Worker` passed
  in, so the chart edit surface is reviewable as a unit.

- **[app/crates/freecell-engine/src/worker/run.rs:2606,2686,2760,2817] The chart edit surface has
  four inconsistent behaviours and no stated model.** Insert / delete / anchor / range are
  undoable; chrome and type changes are **not** undoable but *do* clear the redo stack.
  `set_chart_type` and `set_chart_range` `tracing::warn!` and silently no-op for a **loaded** chart
  (they only handle `authored_charts`), while `set_chart_chrome` works on both provenances. So
  "change this chart's type" works on a chart you inserted and silently does nothing on a chart you
  opened, and "change its title" undoes differently from "move it". A user cannot form a mental
  model of this, and neither can the next maintainer.
  **Direction:** pick one undo policy for all chart edits and make loaded-vs-authored capability
  differences explicit at the protocol level (reject with a reason the UI can show) rather than a
  `warn!` into the void.

- **[app/crates/freecell-app/src/chart/area.rs:142-199 vs scatter.rs:158-215 vs line.rs, bar.rs,
  bubble.rs] The shared renderer abstraction is real but stops one level too early.**
  `cartesian.rs` (gridline/axis clipping), `ticks.rs` (nice scale), `stacking.rs` (cumulative math),
  `chrome.rs` (frame + legend) all do genuine shared work — this is not per-type copy-paste at the
  algorithm level. But the ~25-line prologue of every cartesian `Plot::paint` *is* copy-paste:
  compute `plot_left/right/top/bottom`, build the `ScaleLinear`/`ScalePoint`, map ticks to
  gridline coordinates, construct the `PlotAxis` label call with the identical
  `.x_axis(false).y_label_side(AxisLabelSide::Start).stroke(hsla(AXIS_STROKE))` chain. It appears
  five times. So do the constants: `VALUE_AXIS_GUTTER = 46.0`, `PLOT_TOP_GAP`, `PLOT_RIGHT_GAP`,
  and `TARGET_TICKS = 5` are re-declared in line.rs, bar.rs, area.rs, scatter.rs, and bubble.rs.
  Change the gutter for one chart type and the others drift.
  Relatedly, `paint_marker` and `line_width_px` — shared by line *and* scatter — live in
  `line.rs:260/110`, so `scatter.rs` imports drawing primitives from a sibling *chart type*.
  **Direction:** extract a `CartesianFrame` that owns the rect, the two scales, the gridlines, and
  the label axis; move `paint_marker`/`line_width_px` into a `marks.rs` beside `cartesian.rs`.

- **[app/crates/freecell-chart-model/src/lib.rs:475] Five of six renderers ignore most of the
  `Axis` model, and `fidelity.rs` exists largely to paper over that.**
  Only the line renderer honors `Axis::min`/`max`/`reversed` and the gridline flags — the
  classifier says so itself (`unsupported_axis_scaling` returns `false` for a line chart and
  degrades everything else). bar/area/pie/scatter/bubble parse those fields, store them, round-trip
  them, and drop them at paint time. Same story for markers (line + scatter only) and data labels
  (line + pie-percent only).
  This is the proportionality answer. Six chart types at roughly 60% depth is a worse product than
  three at 100%, and it has a permanent carrying cost: every partial feature adds a scoped detector
  to `fidelity.rs` (which is why that file is 1,272 lines and growing), plus a phase-scoped
  paragraph of prose explaining which renderer honors what. Roughly half of `fidelity.rs` is
  bookkeeping for an unevenly-implemented renderer set. The subsystem grew by adding types
  (P22 bar layout, P24 pie, P25 scatter, P26 bubble) faster than it deepened them, and the badge
  system absorbed the debt.
  **Direction:** before adding a seventh type or another chrome feature, level the existing six
  against the `Axis` model. Each feature that becomes universal deletes a detector.

- **[app/crates/freecell-chart-model/src/theme.rs:198 vs app/crates/freecell-app/src/chart/palette.rs:49]
  `rgb_to_hsl`/`hsl_to_rgb` are copy-pasted across the seam, and have already drifted.**
  theme.rs uses `.rem_euclid(6.0)` / `.rem_euclid(2.0)`; palette.rs uses `% 6.0` / `% 2.0`. Those
  differ for negative operands. Two copies of a color-space conversion in a subsystem where the
  model crate is *already* the shared dependency of both callers is pure accident.
  Separately, the Office theme constants exist twice: `ThemePalette::office_default()`
  (chart-model) and `freecell_core::palette::FILL_PALETTE` (core) carry the same accent hexes
  (`0x4472C4`, `0xED7D31`, …) in two crates with no relationship.
  **Direction:** `hsl` helpers belong in the model crate only (palette.rs should import them);
  the Office palette should have one definition.

- **[app/crates/freecell-chart-model/src/numfmt.rs:1] The chart layer reimplements a number-format
  engine the workbook already ships.** 383 lines implementing a bounded subset of the ECMA-376
  format grammar, existing only because `chart-model` must be ironcalc-free — and then
  `renders_faithfully` is exported *back* to `fidelity.rs` so the badge system can admit which
  codes the reimplementation gets wrong (dates, scientific, fractions, multi-section,
  conditionals). Meanwhile cell rendering formats numbers through IronCalc, which handles all of
  those correctly. The seam constraint that makes the model crate valuable is here paying for
  itself in duplicated, deliberately-inferior functionality.
  **Direction:** have the engine format tick/label strings when it builds the model (it has
  IronCalc), or factor number formatting into a shared ironcalc-free crate used by both cells and
  charts. Either removes 383 lines *and* two `fidelity.rs` detectors.

---

## Mild (consider fixing)

- **[app/crates/freecell-engine/src/chart/load.rs:1136, save.rs:1663, binding.rs:216] `fn child(node,
  name)` is defined identically three times in one module.** Likewise `attr_escape` in `write.rs:494`
  and `chrome.rs:22`, and a near-identical `escape_xml` in `save.rs:1656` (which escapes `>`, while
  the two `attr_escape`s escape `"`). Three tiny XML helpers with three subtly different escape
  sets, in the same module tree, is exactly how an escaping bug eventually lands. Consolidate into
  `xlsx.rs`, which is already the shared-helpers home.

- **[app/crates/freecell-chart-model/src/downsample.rs:21,96] `MAX_PAINT_VERTICES` /
  `MAX_PAINT_MARKERS` are renderer paint budgets living in the model crate.** The *algorithms*
  belong there — they are pure, testable, and correctly gpui-free. The *budgets* are a property of
  the thing doing the painting (chosen, per the doc comment, against expected pixel widths). Export
  the functions; let the renderer own the numbers.

- **[app/crates/freecell-engine/tests/charts_corpus.rs:24,
  charts_roundtrip_libreoffice.rs:32] Round-trip validation rests on one real Excel workbook
  containing only line charts, plus fixtures written by the same code that reads them.**
  `write_corpus_fixture` and friends generate the "all types + edge cases" corpus from
  `chart/authoring.rs`, so parse-side and fixture-side share assumptions; the LibreOffice test
  asserts only that "a chart part exists and parses back as a line chart". Nothing asserts that a
  *patched* chart retained its unmodeled DrawingML — which is the central claim of the save
  architecture. Add golden-XML assertions on `patch_chart_source` output (patch a fixture with a
  gradient fill / `txPr` / `extLst` and diff), and get real Excel-authored bar/pie/scatter/combo
  files into the corpus.

- **[app/crates/freecell-engine/src/chart/save.rs:164] `build_live_patches` re-opens the source zip
  once per chart**, and `reinject` opens it again. The code comments this ("if either grows, thread
  ONE open `ZipArchive`"). At 4 charts it is invisible; the `chart_perf` harness already runs a
  K=1000 scenario elsewhere, so the shape is foreseeable. Cheap to fix now, annoying later.

- **[app/crates/freecell-app/src/grid/view.rs:185-210, 1883, 2056, 4037-4090] Chart drag state,
  selection, hit-testing, and paint are threaded through the 10,627-line `grid/view.rs` (344 chart
  references).** The *geometry* is properly extracted into `chart_layer.rs` and the *presentation*
  into `in_grid.rs` — both good — but the interaction state machine (`ChartDrag`, `ChartDragMode`,
  `selected_chart`, the mouse handlers) lives in the grid monolith. This is mostly a Phase-3
  concern, noted here because it is the one part of the chart vertical slice with no module of its
  own.

---

## Phase Summary

**The layering works.** This is the best-factored subsystem I have looked at in this codebase, and
the `chart-model` seam is not a fiction: a zero-dependency model crate, an engine that parses into
it, an app that renders from it, and essentially no cross-layer reach-through. The save
architecture (retain source, splice byte ranges for changed fields only) is the correct and harder
answer to round-trip fidelity, and the grid integration is properly costed rather than bolted on.
Round-trip data preservation is, on the whole, **structural** — features FreeCell does not model
are preserved because they are never touched.

**But the subsystem grew faster than it was designed, and the fidelity model is where the debt
accumulated.** ~20k LOC — a fifth of the codebase — is charts, in an app whose thesis is a fast
grid, and the investment went into *breadth* (six chart types, three write modes, theming, markers,
number formats, downsampling) rather than depth. Only the line renderer implements the `Axis` model
it parses; the other five drop it and `fidelity.rs` badges the gap. That file is now 1,272 lines of
hand-rolled XML text-scanning in a crate that deliberately has no XML parser, encoding a fourth
independent copy of the OOXML schema alongside `load.rs`, `write.rs`, and `chrome.rs`. It is not a
"symptom of unresolved modelling" in the sense of the model being wrong — the model is good — it is
a symptom of a *renderer* that implements the model unevenly, plus a seam constraint that forced
the classifier into the wrong layer.

**Four things must be fixed before this can be called shipped.** Combo charts truncate to their
first group and are badged Faithful — the exact silent-wrong-render the fidelity system exists to
prevent, on one of the most common real-world chart shapes. Workbook themes are never parsed, so
every `schemeClr` resolves against a hardcoded Office palette, and three call sites bypass the
model's own `ChartColor::resolve` to write that wrong RGB (tint stripped) back into the user's
file. Editing data labels replaces the whole `c:dLbls` subtree, destroying per-point overrides and
label typography — the one real hole in an otherwise excellent preserve-unknown save path.
And inserting a chart onto a sheet that already has one produces a hard save failure with no
recovery, because the byte-preserve and write-from-model paths cannot compose on a shared drawing.

**Most important single finding:** the combo-chart truncation (Critical #1). Everything else in this
subsystem is built to be honest about what it cannot draw; this case draws two-thirds of a chart,
tells the user it is faithful, and the classifier's architecture — a whole-part text scan that
cannot bind an element to its chart group — is structurally incapable of catching it.
