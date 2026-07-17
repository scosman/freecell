---
status: draft
---

# Phase 7: Color-scale editor

## Overview

P6 shipped the highlight-rule editor but deliberately (a) omitted the color-scale kinds from the
rule-type dropdown and (b) gated OFF the ✎ edit control for `ColorScale` rows. The engine + list
already support color scales end-to-end (P1 makes a concrete-RGB `ColorScale` an editable rule with
`spec: Some`). P7 completes color-scale **authoring**: a 2-vs-3-color editor body with preset chips,
per-stop threshold-kind / value / color controls, and a gradient preview; `cf_build_spec` /
`cf_state_from_spec` handle the `ColorScale` kinds; the ✎ edit control re-opens for editable scale
rows; validation requires a value on each Number/Percent/Percentile stop.

Design decision (state ownership): the stop **value** lives in `CfEditorState.scale` (authoritative),
mirrored into the `cf_stop_value_inputs` widgets. On any editor input event the stop inputs are
**synced back** into `scale`, so `cf_build_spec` / `cf_validate` read `scale` directly (no extra
string params), and the build↔seed round-trip is pure over `scale`. Seeding the inputs uses
`set_value`, which suppresses the change event (no spurious sync).

## Steps

1. **`chrome/cond_fmt.rs`** — extend the editor model:
   - Add `ColorScale2`, `ColorScale3` to `CfEditorKind` (update the doc comment: they are now
     constructed, so no dead-code concern).
   - Add `pub scale: Vec<CfColorStop>` to `CfEditorState` (2–3 stops for a color scale; empty for
     highlight kinds). Import `CfColorStop`. Initialize `scale: Vec::new()` in `new()`.

2. **`chrome/view.rs` imports + `ChromeView` state:**
   - Add `CfColorStop, CfThresholdKind` to the `freecell_core` import list.
   - Add field `cf_stop_value_inputs: Vec<Entity<InputState>>` (near `cf_formula_input`), 3 inputs
     created in the constructor and each subscribed to `on_cf_input_event`.
   - Add `CfMenu::StopKind(usize)` for the per-stop threshold dropdowns.

3. **Editor methods (`ChromeView`):**
   - `select_cf_kind`: when the chosen kind is `ColorScale2/3`, seed `editor.scale` from the default
     preset (2 or 3 stops) and seed the stop inputs; for a highlight kind clear `scale` + stop inputs.
   - `set_cf_scale_arity(three, window, cx)`: the 2-vs-3 segmented control — resize `scale`
     preserving endpoints (insert a Percentile-50 midpoint / drop the middle), set the kind, reseed.
   - `apply_cf_scale_preset(colors: &[u32], window, cx)`: a preset chip — rebuild `scale` from the
     preset colors (first `Min`, last `Max`, middle `Percentile` 50), reseed the stop inputs.
   - `set_cf_stop_kind(i, kind, window, cx)`: the per-stop threshold dropdown — set `scale[i].kind`;
     `Min/Max/Number` → no seeded value (blank), `Percent/Percentile` → 50; reseed input `i`.
   - `set_cf_stop_color(i, color, cx)`: a stop swatch click — set `scale[i].color`.
   - `sync_cf_scale_values(cx)`: read the 3 stop inputs; for a color-scale editor, write parsed
     `f64`s (or `None`) into the matching Number/Percent/Percentile `scale` stops.
   - `seed_cf_stop_inputs(scale, window, cx)`: `set_value` each of the 3 inputs from the stop's value
     string (empty for Min/Max or a blank Number).
   - Extend `on_cf_input_event` to call `sync_cf_scale_values` before `notify`.
   - `open_cf_editor`: after `seed_cf_inputs`, also `seed_cf_stop_inputs(&state.scale, …)`.
   - `save_cf_editor`: call `sync_cf_scale_values` first (defensive) so the built spec is current.

4. **Rendering:**
   - `render_cf_editor`: branch on the kind — for `ColorScale2/3` render `section("Color scale",
     render_cf_scale_editor)` and **hide** the operands, the format editor, and the Stop-if-true
     checkbox (ColorScale carries no `CfFormat`); highlight kinds keep the existing body.
   - `render_cf_scale_editor(editor, cx)`: a 2-vs-3 `cf_segmented`; a preset-chips row (per arity,
     each a clickable banded gradient + label); one stop row per stop (position label, threshold-kind
     `cf_dropdown` — `Min` only on the first endpoint, `Max` only on the last, `Number/Percent/
     Percentile` anywhere; a value `Input` shown only for Number/Percent/Percentile; a
     `render_cf_stop_color` swatch grid); a full-width banded **gradient preview** (`cf-scale-preview`).
   - `render_cf_stop_color(i, current, cx)`: the `FILL_PALETTE` swatch grid (reused from the P6
     fill/text row, minus the "none" button) setting `scale[i].color`.
   - `render_cf_operands`: add an (unreached) `ColorScale2/3` arm returning an empty element so the
     match stays exhaustive.

5. **Pure helpers / consts / conversions:**
   - Append `(ColorScale2, "colorscale2", "2-color scale")` + `(ColorScale3, …, "3-color scale")` to
     `CF_KIND_MENU` (the rule-type dropdown's "Color scale" group at the end).
   - `CF_SCALE_PRESETS_2` / `CF_SCALE_PRESETS_3` (white-blue / green-white; green-yellow-red /
     red-yellow-green), `CF_STOP_KIND_MIN/MAX/MID`, `CF_STOP_TAGS`, `CF_SCALE_MID_DEFAULT`.
   - `cf_threshold_kind_label`, `cf_stops_from_colors(&[u32])`, `cf_fmt_stop_value(f64)`,
     `StopPos` + `cf_stop_pos` + `cf_stop_label`.
   - `cf_row_controls`: `edit = row.editable` (drop the P6 ColorScale gate — a theme-colored scale is
     already `editable == false`).
   - `cf_validate`: `ColorScale2/3` → error if any Number/Percent/Percentile stop has `value: None`.
   - `cf_build_spec`: `ColorScale2/3` → `CfRuleSpec::ColorScale { stops: editor.scale.clone() }`.
   - `cf_state_from_spec`: `ColorScale { stops }` → seed `scale` + kind (`len >= 3` ⇒ ColorScale3),
     returning `Some` (re-enables edit-seeding).

## Tests

- `cf_add_color_scale_saves_add_command` — open editor, select ColorScale3, Save → one `AddCondFmt`
  whose spec is a `ColorScale` of 3 stops (Min / Percentile-50 / Max).
- `cf_scale_preset_sets_stops` — after selecting ColorScale3, applying a preset sets `editor.scale`
  to that preset's 3 stops.
- `cf_scale_arity_toggle_resizes` — ColorScale2 (2 stops) → toggle to 3 ⇒ `scale.len() == 3`;
  back to 2 ⇒ `scale.len() == 2`, endpoints preserved.
- `cf_edit_color_scale_seeds_and_saves_update` — a published editable `ColorScale` row seeds 3 stops
  + kind on edit; Save sends `UpdateCondFmt` with the ColorScale spec.
- `cf_scale_number_stop_without_value_blocks_save` — `cf_validate` returns a message for a Number
  stop with `value: None` (pure).
- `cf_color_scale_row_edit_enabled` — repurpose the P6 `..._edit_disabled` test: an editable
  ColorScale row's edit control is now enabled (delete still enabled; a Badge row still not editable).
- `cf_build_spec_and_state_round_trip_color_scale` — sibling to the highlight round-trip: for a
  2-stop and a 3-stop scale, `cf_build_spec` produces the expected `ColorScale` spec and
  `cf_state_from_spec(build)` reproduces the state (kind + scale) exactly.
- (Covered) `cf_list_renders_one_row_per_rule` already asserts a ColorScale row renders its gradient
  preview swatch.
