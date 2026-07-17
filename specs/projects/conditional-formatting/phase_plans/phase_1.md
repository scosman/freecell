---
status: draft
---

# Phase 1: Engine-free CF types + wrapper + conversions (headless)

## Overview

Prove the engine seam for conditional formatting entirely headless — no worker protocol, no UI.
Three things land:

1. The engine-free CF vocabulary in `freecell-core` (`cond_fmt` module) — plain `serde` data that the
   worker protocol (P2) and the UI (P4+) will share. Stays gpui-free AND ironcalc-free so the
   `dependency_rule.rs` guard stays green.
2. The IronCalc↔core conversions in `freecell-engine::cond_fmt_convert` — the only place a CF-related
   IronCalc type is touched, isolated so nothing leaks past `WorkbookDocument`.
3. The `WorkbookDocument` CF methods (add/update/delete/raise/lower/list/`has_cond_fmt`/
   `extended_render_style`) that delegate to the pinned fork's `UserModel` CF API through those
   conversions and return only engine-free `freecell-core` types.

This is the real technical risk (the value-dependent extended-style read + the engine mapping), so it
is de-risked first. Exit = `engine_cf.md §7` engine tests green.

## Steps

1. **`freecell-core/src/color.rs`** — add `Serialize, Deserialize` to `Rgb`'s derive list so the CF
   types (which embed `Rgb`) can be `serde` data. Additive; nothing else changes.

2. **`freecell-core/src/cond_fmt.rs`** (new) — the engine-free CF vocabulary per `engine_cf.md §1`:
   - `CfFormat { fill: Option<Rgb>, text_color: Option<Rgb>, bold: bool, italic: bool }` (Copy, Default).
   - Enums `CfValueOp` (Gt/Lt/Ge/Le/Eq/Ne/Between/NotBetween), `CfTextOp`
     (Contains/NotContains/BeginsWith/EndsWith/Equals), `CfPeriod` (13 parameterless periods),
     `CfThresholdKind` (Min/Max/Number/Percent/Percentile).
   - `CfColorStop { kind: CfThresholdKind, value: Option<f64>, color: Rgb }`.
   - `CfRuleSpec` (CellIs/Text/TimePeriod/Top/Average/DuplicateValues/Blanks/Errors/Formula/ColorScale)
     with a `format(&self) -> Option<&CfFormat>` + `format_mut(&mut self) -> Option<&mut CfFormat>`
     helper (ColorScale → None).
   - `CfPreview { Highlight{fill,text_color} | ColorScale{colors} | Badge(String) }`.
   - `CfRuleView { index, range, priority, editable, summary, preview, spec: Option<CfRuleSpec> }`.
   - All derive `Debug, Clone, PartialEq, Serialize, Deserialize` (Copy where possible). Register
     `pub mod cond_fmt;` + re-export the types in `freecell-core/src/lib.rs`.

3. **`freecell-engine/src/cond_fmt_convert.rs`** (new) — conversions per `engine_cf.md §3`, importing
   `ironcalc_base::cf_types::*` and `ironcalc_base::types::{Color, Dxf, DxfFont, Fill}`. All pure:
   - `rgb_to_color(Rgb) -> Color::Rgb("#RRGGBB")`; `color_to_rgb(&Color) -> Option<Rgb>` (via
     `cache::parse_color`; Theme/None → None).
   - `cf_format_to_dxf(&CfFormat) -> Dxf`; `merge_cf_format_into_dxf(&CfFormat, Dxf) -> Dxf`
     (preserve existing strike/u/sz + border/num_fmt/alignment; set fill + b/i/color from the format;
     drop the font if nothing remains); `dxf_to_cf_format(&Dxf) -> CfFormat`.
   - `cf_rule_spec_to_input(&CfRuleSpec, Dxf) -> CfRuleInput` — 1:1 by variant (Top{bottom}→Top10/
     Bottom10, Average{below}→Above/BelowAverage, DuplicateValues{unique}→Unique/DuplicateValues,
     Blanks{no_blanks}→NotBlanks/Blanks, Errors{no_errors}→NoErrors/Errors, TimePeriod date1/date2=None,
     ColorScale stops→ColorScaleThreshold).
   - `cf_rule_to_view(index, range, priority, &CfRule, Option<Dxf>) -> CfRuleView` — authorable variants
     → editable+summary+preview+reconstructed spec; deferred families (DataBar/IconSet/IconRating) and
     deferred variants (TimePeriod Between/NotBetween/Next7Days, ColorScale w/ a `Cfvo::Formula` stop) →
     `editable:false, spec:None, preview:Badge(label)`.
   - Small enum-mapping helpers (value/text/period op ↔ ironcalc operator) + a `summary` builder.
   - Register `mod cond_fmt_convert;` in `freecell-engine/src/lib.rs`.

4. **`freecell-engine/src/document.rs`** — add a `user_model(&self) -> &UserModel<'static>` pub(crate)
   read accessor (mirrors `user_model_mut`, bumps the instrument counter) and `mod cond_fmt;`.

5. **`freecell-engine/src/document/cond_fmt.rs`** (new child module) — the `WorkbookDocument` CF methods
   per `engine_cf.md §4` / architecture §4.1:
   - `add_cond_fmt(sheet, range, &CfRuleSpec) -> Result<(), String>` — dxf from `cf_format_to_dxf`
     (Dxf::default for ColorScale) → `cf_rule_spec_to_input` → `add_conditional_formatting`.
   - `update_cond_fmt(sheet, index, range, &CfRuleSpec)` — fetch existing dxf, `merge_cf_format_into_dxf`,
     `update_conditional_formatting` (ColorScale skips the merge).
   - `delete_cond_fmt` / `raise_cond_fmt` / `lower_cond_fmt` — delegate to the matching UserModel method.
   - `cond_fmt_rules(sheet) -> Result<Vec<CfRuleView>, String>` — map `get_conditional_formatting_list`,
     fetching each rule's dxf, through `cf_rule_to_view` (already priority-sorted desc).
   - `has_cond_fmt(sheet) -> bool` — `!ws.conditional_formatting.is_empty()`, degrade to false.
   - `extended_render_style(sheet, cell: CellRef, &Theme) -> RenderStyle` —
     `render_style_from(&get_extended_cell_style(...).style, theme)`; degrade to `RenderStyle::default()`
     + `tracing::warn!` on an engine `Err`.

## Tests

**`freecell-core::cond_fmt` (inline):**
- `cf_rule_spec_serde_round_trips` — a CellIs and a ColorScale spec survive JSON round-trip.
- `format_accessor_returns_none_for_color_scale` / `some_for_dxf_variants`.

**`freecell-engine::cond_fmt_convert` (inline, pure):**
- `rgb_color_round_trip` — `color_to_rgb(rgb_to_color(x)) == Some(x)`; Theme/None → None; #AARRGGBB tolerated.
- `cf_format_dxf_round_trip` — fill+bold+italic+text_color → Dxf → back; empty format → font `None`.
- `merge_preserves_unmodeled_dxf_fields` — an existing Dxf with border + strike/u/sz keeps them while
  fill/b/i/color are replaced; clearing all font attrs drops the font.
- `spec_to_input_every_arm` — each `CfRuleSpec` variant maps to the expected `CfRuleInput` variant
  (Top/Bottom, Above/Below, Unique/Duplicate, NotBlanks/Blanks, NoErrors/Errors, TimePeriod dates None,
  ColorScale thresholds/cfvo).
- `rule_to_view_authorable_sets_spec_and_editable` (CellIs, Text, ColorScale) and
  `rule_to_view_deferred_family_is_badge` (DataBar/IconSet/IconRating → Badge, non-editable, spec None)
  and `rule_to_view_deferred_variant_is_badge` (TimePeriod NotBetween, ColorScale w/ Formula cfvo).

**`freecell-engine::document::cond_fmt` (inline, real UserModel):**
- `add_then_list_reflects_rule` — add CellIs `> 100` over `B2:B20`; list has index 0, that range,
  priority 1, summary "Cell value > 100", editable, `Highlight` preview, `Some(spec)` equal to input.
- `round_trip_spec_through_engine` — add each authorable variant, read back, `view.spec == input spec`.
- `update_changes_format_and_range` — update a rule's format+range; readback reflects both.
- `delete_removes_rule` — list empty after delete.
- `raise_lower_reorders_priority` — two rules; raise the lower one → it sorts first; lower → back.
- `has_cond_fmt_gate` — false on empty sheet, true after add.
- `extended_style_reflects_rule` — `> 100` fill rule: A1=150 → fill; A5=50 → base (no fill).
- `extended_style_flips_on_value_change` — set A1=150 (fill) then A1=50 (no fill) then 150 again — no CF
  command in between.
- `color_scale_interpolates` — 2-color Min→green/Max→red over values 0/50/100: middle cell fill is
  `Some` and distinct from both endpoints.

Verify with `cargo build -p freecell-core -p freecell-engine`, `cargo test -p freecell-core --lib`,
`cargo test -p freecell-engine --lib`, `cargo fmt --all --check` (run from `app/`).
