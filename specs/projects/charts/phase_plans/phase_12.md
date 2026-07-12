---
status: complete
---

# Phase 12: Data labels & number formats (line)

## Overview

Harden the LINE chart with OOXML **data labels** (`c:dLbls`) and complete **number-format**
coverage on labels. Per the implementation plan (P12) and `functional_spec §4` (P2 data labels
+ number formats), this phase:

1. Models data labels in `freecell-chart-model` (gpui/ironcalc-free), attached to `Series`
   (chart-level `c:dLbls` is resolved into each series at parse time, so the `Chart` struct — 52
   literal call sites — is untouched).
2. Parses `c:dLbls` (series-level + chart-group-level default) in `freecell-engine`.
3. Renders data labels on the line renderer — value / percent / category-name / series-name /
   legend-key per the `show*` toggles, with the label value formatted through the **P6 numFmt
   applier** (`apply_number_format`).
4. Reconciles the fidelity accessor: shown labels on a **line** now render → **Faithful**
   (scoped like markers were in P6); a **supported** `numFmt` no longer degrades (the applier
   renders it). Kept degrading (honest): shown labels on a **non-line** group (not rendered
   yet — their phases are P22+; renumbered from P16+), **per-point** `c:dLbl` overrides (custom per-point
   text/position/deletion we don't render), and an **unsupported** `numFmt` code the applier
   falls back to general on (dates / scientific / fractions / multi-section / conditionals).

`ui_design §2.2/§6.B` + coverage-matrix §F (`c:dLbls`) are the fidelity targets. The real Excel
line-chart workbook (`chart1.xml`) was inspected for realism: Excel emits `c:dLbls` at the
**series** level (after `c:marker`, before `c:cat`), with all `show*` toggles (`showLegendKey`,
`showVal`, `showCatName`, `showSerName`, `showPercent`) and a `separator`; a chart-group-level
`c:dLbls` may also appear as a default. Number formats appear as `<c:numFmt formatCode="…"
sourceLinked="…"/>` on axes and inside `c:dLbls`.

## Steps

1. **`freecell-chart-model/src/label.rs` (new module).** A gpui/ironcalc-free `DataLabels`:
   ```rust
   pub struct DataLabels {
       pub show_legend_key: bool,
       pub show_value: bool,
       pub show_category_name: bool,
       pub show_series_name: bool,
       pub show_percent: bool,
       pub number_format: Option<String>, // c:dLbls/c:numFmt formatCode
       pub separator: Option<String>,     // c:separator (default ", ")
       pub position: Option<DataLabelPosition>, // c:dLblPos (t/b/ctr/l/r)
   }
   pub enum DataLabelPosition { Center, Left, Right, Above, Below }
   ```
   - `DataLabelPosition::from_ooxml("t"|"b"|"ctr"|"l"|"r")`.
   - `DataLabels::is_shown()` → any of the five `show_*`.
   - `DataLabels::label_text(series_name, category, value, percent)` → the composed text:
     parts `[series_name, category, value, percent]` in order, each gated by its toggle, joined
     by `separator` (default `", "`); the **value** part formatted via
     `apply_number_format(number_format?, value)` (else general); the **percent** part (a
     fraction, `None` when total is 0) formatted `"NN%"`. Legend key is a **swatch** (drawn by
     the renderer), not text — excluded here.
   - Builder helpers + `Default`. Full doc comments (doc-clean crate).
2. **`freecell-chart-model/src/lib.rs`.** Add `pub mod label;` re-exports (`DataLabels`,
   `DataLabelPosition`); add `data_labels: Option<DataLabels>` to `Series` (default `None` in the
   two constructors) + `Series::with_data_labels(self, DataLabels)` builder. Update `Series` doc.
3. **`freecell-chart-model/src/numfmt.rs`.** Add `pub(crate) fn renders_faithfully(code: &str)
   -> bool`: `true` for empty/`General`, `false` for multi-section (`;`) or conditional
   (`[<`/`[>`/`[=`) codes, else `FormatSpec::parse(code).is_some()`. Refresh the module doc: the
   applier is now used for **data labels** too (P12), not "the full engine is P12".
4. **`freecell-chart-model/src/fidelity.rs`.** Reconcile the curated set:
   - Replace `custom_number_format` → `unsupported_number_format`: degrade only when
     `!renders_faithfully(code)` (supported codes now render → Faithful).
   - Replace `data_labels_shown` usage with `unsupported_data_labels`: per-point `c:dLbl`
     (`contains_element(xml, "dLbl")`, boundary-safe vs `dLbls`) degrades on any group; else
     shown labels degrade only on a **non-line** group (`!is_line_chart`). `data_labels_shown`
     keeps the toggle scan. Document the scoping + combo caveat (like markers).
   - Update the module doc's "auto-dropped / scoped as support arrives" note for P12.
5. **`freecell-engine/src/chart/load.rs`.** Parse `c:dLbls`:
   - `parse_data_labels(node) -> Option<DataLabels>` reading the five `show*` toggles,
     `c:numFmt@formatCode`, `c:separator` text, `c:dLblPos@val`.
   - In `parse_series`: read the series' own `c:dLbls`; in `parse_chart_xml`: read the
     chart-group-level `c:dLbls` (direct child of the group) as the default and apply it to any
     series lacking its own (OOXML: series `dLbls` replaces chart-level for that series).
   - Set `series.data_labels`. Add parse unit tests.
6. **`freecell-app/src/chart/line.rs`.** Render labels:
   - `LineSeries` carries `data_labels: Option<DataLabels>` + `name: Option<String>`.
   - In `Plot::paint`, after markers: for each series whose `data_labels.is_shown()`, for each
     finite point, compose `label_text` (percent = value / finite-series-total), position it per
     `DataLabelPosition` (default **Above**; t/b/ctr/l/r → directional offset from the point),
     and paint via `gpui_component::plot::label::{Text, PlotLabel}`. When `show_legend_key`, paint
     a small series-color swatch just left of the text (width via `measure_text_width`).
   - Keep painting deterministic + explicit-colored (headless capture).
7. **`app/render-tests/src/chart_scene.rs`.** Add standalone scenes (each one baseline PNG):
   - `chart_line_value_labels` — single-series line, `show_value` + currency `numFmt`
     (`"$#,##0"`) → labels like `$1,222` (value + numFmt).
   - `chart_line_percent_labels` — single-series line, `show_percent` → each point's share of
     the series total as `NN%` (percent path).
   - `chart_line_named_labels` — single-series line, `show_series_name` + `show_category_name`
     + `show_value` + `show_legend_key` → composed multi-part label with the swatch.
   Register them in `all()`; extend the scene unit tests.
8. **`app/render-tests/src/cases.rs`.** The `grid_chart_degraded_badge` scene's source is
   `<c:lineChart><c:dLbls><c:showVal val="1"/></c:dLbls></c:lineChart>` — now **Faithful** (P12
   renders it), which would drop its badge. Switch its source to a still-Degraded line source
   (`<c:line3DChart/>`, a 3-D→2-D degrade) so the badge case holds. The rendered Chart model is
   unchanged and fidelity stays Degraded, so `grid_chart_degraded_badge.png` is **byte-identical**
   (no baseline change). Update the scene comment.
9. **Render baselines.** `render_tests.sh generate --only chart_`; eyeball each new PNG (labels at
   the right positions, correct content + number formatting) + confirm no *unexpected* existing
   baseline moved. Commit baselines with the code.

## Tests

- **model/label.rs:** `label_text` composes parts in order + honors separator; value uses numFmt;
  percent renders `NN%`; `is_shown`; empty when all-off; `DataLabelPosition::from_ooxml`.
- **model/numfmt.rs:** `renders_faithfully` — true for General/percent/currency/thousands/decimals;
  false for date/scientific/fraction/multi-section/conditional.
- **model/fidelity.rs:** shown labels on a `lineChart` → Faithful; shown labels on a non-line
  group → Degraded; per-point `c:dLbl` → Degraded (line included); `dLbls` vs `dLbl` boundary;
  supported `numFmt` → Faithful; unsupported `numFmt` (date) → Degraded; conditional `numFmt`
  (`[>1000]`) → Degraded (keeps the quote-aware-scan case); benign all-off `dLbls` stays Faithful.
- **engine/load.rs:** `parse_data_labels` reads toggles + numFmt + separator + position;
  series-level `dLbls`; chart-group-level default applied to series without their own; a chart
  with shown value labels parses to `Some(DataLabels{show_value:true,…})` and stays Faithful.
- **app/line.rs:** `LinePlot::multi_series` carries `data_labels` + `name` into `LineSeries`; a
  shown-labels series builds; percent total handles all-finite.
- **render-tests/chart_scene.rs:** the three new scenes exist, are `chart_`-prefixed, line kind,
  and carry their intended label toggles/numFmt.
- **Render subset:** `render_tests.sh test chart_` green after generating baselines.
