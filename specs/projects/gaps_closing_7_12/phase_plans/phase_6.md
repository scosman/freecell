---
status: complete
---

# Phase 6: Number-format preset breadth

## Overview

Widen the number-format dropdown from today's 7 flat presets to a grouped model of 23
presets across 9 groups (General, Number, Currency, Accounting, Date, Time, Percent,
Scientific, Text; a proposed Fraction group was **deferred** — see Step 2). This is
**UI-only** — IronCalc already renders arbitrary format
codes; each preset is a `(label, code)` pair routed through the existing `apply_num_fmt`
→ `apply_style_path(StylePath::NumFmt, code)` command path. Work is concentrated in
`freecell-core/src/format_ui.rs` (the pure preset model + reverse map + a new
`toggle_thousands` helper) and `chrome/view.rs` (restructured popover + a thousands-separator
action-bar button). Also fixes a pre-existing `collapsible_match` clippy warning in
`chrome/view.rs` (Phase 1 status-bar code) as an in-scope cleanup.

Not a pixel-suite surface (dropdown chrome; cell values are engine-rendered) → validate with
unit + gpui view tests + an Xvfb smoke launch.

## Steps

1. **`format_ui.rs` — extend `Category`.** Add `Accounting` and `Scientific` variants and
   their `label()` arms ("Accounting"/"Scientific"). Keep the existing
   General/Number/Currency/Percent/Date/Time/Text/Custom. (A `Fraction` variant was
   originally planned but **deferred** — see the Step 2 deferral note.)

2. **`format_ui.rs` — grouped preset model.** Replace the flat `DROPDOWN_FORMATS` const with:
   ```rust
   pub struct NumFmtPreset { pub label: &'static str, pub code: &'static str }
   pub struct NumFmtGroup  { pub category: Category, pub presets: &'static [NumFmtPreset] }
   pub const NUM_FMT_GROUPS: &[NumFmtGroup] = &[ /* inventory below */ ];
   ```
   Inventory (D6.1, functional_spec §6 proposal + Accounting from the phase prompt):
   - General: General → `general`
   - Number: `0.00`, `#,##0.00`, `#,##0`, `#,##0.00;[Red]-#,##0.00`
   - Currency: `$#,##0.00`, `€#,##0.00`, `£#,##0.00`, `¥#,##0.00`
   - Accounting: `$#,##0.00;($#,##0.00)`
   - Date: `m/d/yyyy`, `yyyy-mm-dd`, `d-mmm-yyyy`, `mmm d, yyyy`, `m/d/yy`
   - Time: `h:mm AM/PM`, `h:mm:ss AM/PM`, `h:mm`, `[h]:mm:ss`
   - Percent: `0.00%`, `0%`
   - Scientific: `0.00E+00`
   - Text: `@`

   **Fraction deferred (engine limitation, CR 2026-07-13).** The proposed Fraction preset
   (`# ?/?`) was dropped along with the `Category::Fraction` variant: IronCalc's `?/?` fraction
   formatting is effectively unimplemented (`1.5` → `"  /2"`, garbled for every input, not even a
   `#VALUE!`), and this batch is FreeCell-side / no-fork. It needs a fork implementation — tracked
   in root `PROJECTS.md` + `projects/fraction-number-format.md`. Scientific renders correctly and
   ships.

   **Currency codes use the engine's bracket form for `£`/`¥` (CR 2026-07-13).** IronCalc's format
   lexer only accepts the *bare* symbols `$`/`€`; bare `£#,##0.00`/`¥#,##0.00` fail to parse →
   `#VALUE!`. Those two presets use `[$£]#,##0.00`/`[$¥]#,##0.00` (the `[`/`]` don't trip the
   decimals/thousands adjustability gate). `$`/`€` stay bare (verified rendering).

3. **`format_ui.rs` — reverse map over the grouped model.** Rewrite `num_fmt_category` to
   keep the case-insensitive `general` short-circuit, then search `NUM_FMT_GROUPS` for an
   exact code match returning that group's category; unknown → `Custom`. Single source of
   truth (no separate table).

4. **`format_ui.rs` — `toggle_thousands`.** Add a pure `toggle_thousands(code) ->
   Option<String>` sibling of `adjust_decimals`. Gate on `is_decimals_adjustable(code) &&
   code.contains('0')` (single-section, no exponent/quoted/escaped, has an integer digit
   placeholder — so General/Text/Date/Time/multi-section/scientific return `None`). If the
   code contains the canonical `#,##0` grouping placeholder, strip it (`replacen("#,##0",
   "0", 1)`); else insert `#,##0` before the first `0` (add grouping). Round-trips for all
   dropdown-native numeric codes (`0.00`↔`#,##0.00`, `0`↔`#,##0`, `$0.00`↔`$#,##0.00`,
   `0.00%`↔`#,##0.00%`, `€0.00`↔`€#,##0.00`).

5. **`view.rs` — imports.** Swap `DROPDOWN_FORMATS` for `NUM_FMT_GROUPS` and add
   `toggle_thousands` to the `freecell_core::format_ui` import.

6. **`view.rs` — restructure `render_num_fmt_popover`.** Iterate `NUM_FMT_GROUPS`; for a
   group with >1 preset render a muted section header (`group.category.label()`, `MUTED_TEXT`,
   `text_xs`); render each preset as the existing ghost/small `Button` whose `.selected(...)`
   exact-matches the active cell's code (normalize `general` case-insensitively). Give the
   card `.id("numfmt-menu").max_h(px(320.0)).overflow_y_scroll()` (mirror the font-family
   popover) since the list is now tall.

7. **`view.rs` — thousands-separator action-bar button (D6.2).** Add a `thousands-sep`
   ghost/small button between the decimals-dec button and the following `action_divider()`,
   using a new vendored `icons/thousands-separator.svg`, tooltip "Thousands separator",
   `.disabled(!self.toggle_thousands_enabled())`, `.selected(self.thousands_active())`,
   `on_click` → `self.toggle_thousands_separator(window, cx)`. Add the three methods:
   `toggle_thousands_enabled` (not degraded && `toggle_thousands(code).is_some()`),
   `thousands_active` (enabled && code contains `#,##0`), `toggle_thousands_separator`
   (apply the toggled code via `apply_num_fmt`, no-op when `None`).

8. **`assets/icons/thousands-separator.svg` + `shell/assets.rs`.** Add a Lucide-style
   tintable SVG (`fill="none"`, `stroke="currentColor"`) — three `0` rings + a comma —
   and register it in the `FREECELL_ICONS` `include_bytes!` list (kept in sync per the file's
   doc + the `vendored_icons_load_and_are_tintable` test).

9. **`view.rs` — clippy cleanup.** Collapse the `collapsible_match` at the
   `WorkerEvent::SelectionStats` arm (`if req_id == self.stats_seq` → an arm guard).

## Tests

- `format_ui.rs`:
  - `category_reverse_map_covers_all_presets` — every preset code in `NUM_FMT_GROUPS` maps
    back through `num_fmt_category` to its own group's category (structural invariant).
  - Update `category_exact_matches` to cover the new categories (Scientific `0.00E+00`,
    Accounting, the bracketed `£`/`¥` currency codes) and `category_custom_fallback` (drop the
    now-preset `yyyy-mm-dd`; keep genuinely-custom codes like `0.000`).
  - `toggle_thousands_adds_and_removes` — the round-trips in step 4.
  - `toggle_thousands_gated_off` — `general`, `@`, dates, times, `0.00E+00`, and the
    multi-section red-negative preset all return `None`.
- `freecell-engine` (`document.rs`) — engine-render guard (CR 2026-07-13):
  - `every_num_fmt_preset_code_renders_without_parse_error` — runs **every** `NUM_FMT_GROUPS` code
    through IronCalc's `format_number` on representative values, asserting no parse error / `#VALUE!`
    (would have caught the bare-`£`/`¥` and Fraction breakage), plus symbol spot-checks for the
    bracketed `£`/`¥` presets.
- `view.rs` (gpui):
  - `num_fmt_category_label_reflects_new_categories` — a `0.00E+00` cell → "Scientific"; a
    `$#,##0.00;($#,##0.00)` cell → "Accounting".
  - `num_fmt_preset_pick_emits_grouped_code` — applying a Date preset (`yyyy-mm-dd`) routes
    that exact code to `SetStylePath{NumFmt}`.
  - `thousands_toggle_adds_and_removes_grouping` — `0.00` cell: enabled, not active, toggle →
    `#,##0.00`; `#,##0.00` cell: enabled, active, toggle → `0.00`.
  - `thousands_toggle_disabled_for_date_and_degraded` — a date cell → disabled + no-op;
    degraded → disabled.
</content>
