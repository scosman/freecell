---
status: complete
---

# Phase 10: Feedback tweaks — 10.1 number-format dropdown basics-first + "More ▸" submenu

## Overview

Phase 6 replaced the number-format dropdown's original short 7-preset list with a grouped
23-preset scrollable menu, which regressed the basics (common formats now require scrolling).
Phase 10.1 restores a **basics-first** menu: the dropdown opens to the original 7 flat presets
(no scroll) plus a trailing **"More ▸"** row that reveals the full Phase-6 grouped inventory.
Nothing from Phase 6 is deleted — the breadth is relocated behind "More ▸".

**D10.1 mechanism — DRILL-IN (chosen).** The architecture prefers a flyout but explicitly
allows drill-in when a flyout is awkward with the existing custom-`div` popover machinery. A
flyout here is awkward: the current popover is a single fixed-anchor occluded card
(`.top(ACTION_ROW_H).left(anchor_x)`) over one full-screen backdrop; a flyout would need a
second card anchored to the *dynamically-positioned* "More ▸" row, whose vertical offset and
the main card's right edge are not known at render time without measurement. Drill-in reuses
the exact same card/backdrop/occlude/dismiss machinery — clicking "More ▸" swaps the card's
content to the grouped list with a "◂ Back" row that restores the basics. This is the clean
fit, so we take it.

Chrome-only (dropdown popover) → **no pixel suite** per CLAUDE.md; validate with gpui view
tests + `VisualTestContext` paint tests + an Xvfb smoke launch.

## Steps

### 1. `freecell-core/src/format_ui.rs` — basic set + predicates

- Add `pub const BASIC_FORMATS: &[NumFmtPreset]` = the exact pre-Phase-6 `DROPDOWN_FORMATS`
  set recovered from `382f075^` (7 presets, labeled by their category name as the original
  menu showed them):

  ```rust
  pub const BASIC_FORMATS: &[NumFmtPreset] = &[
      NumFmtPreset { label: "General",  code: "general" },
      NumFmtPreset { label: "Number",   code: "#,##0.00" },
      NumFmtPreset { label: "Currency", code: "$#,##0.00" },
      NumFmtPreset { label: "Percent",  code: "0.00%" },
      NumFmtPreset { label: "Date",     code: "m/d/yyyy" },
      NumFmtPreset { label: "Time",     code: "h:mm AM/PM" },
      NumFmtPreset { label: "Text",     code: "@" },
  ];
  ```

  Every basic code is already a member of `NUM_FMT_GROUPS`, so `num_fmt_category` (the reverse
  map, single source of truth off `NUM_FMT_GROUPS`) keeps working unchanged for both levels,
  and `NUM_FMT_GROUPS` stays intact as the full/"More" inventory.
- Add two pure predicates (testable, no engine/gpui):
  - `pub fn is_basic_num_fmt(code: &str) -> bool` — exact-match against `BASIC_FORMATS`,
    normalizing `general` case (engine may echo `"General"`).
  - `pub fn is_more_only_num_fmt(code: &str) -> bool` — `num_fmt_category(code) !=
    Category::Custom && !is_basic_num_fmt(code)` — i.e. a *recognized* preset code that is not
    in the basic set (used to mark "More ▸" active + to open onto the match).

### 2. `chrome/view.rs` — state field

- Add `num_fmt_more_open: bool` to `ChromeView` (drill-in view state), default `false` in the
  constructor.
- Reset it to `false` at every popover-close site so the popover always reopens basics-first:
  `apply_num_fmt` (~L1552), `set_degraded` (~L2243), and the backdrop dismiss closure inside
  `render_num_fmt_popover` (~L4368).

### 3. `chrome/view.rs` — open behavior (open onto the match)

- Extend imports (L34-37): add `BASIC_FORMATS`, `is_more_only_num_fmt`.
- Add a private helper `num_fmt_active_code(&self) -> String` (normalizes `general` case) —
  reused by the toggle and the render.
- In `toggle_num_fmt_popover` (~L1610): when opening, set `num_fmt_more_open =
  is_more_only_num_fmt(&active)` so a cell whose format lives only in "More" opens directly
  onto the grouped list with the match highlighted (architecture: "have it open onto the
  matched group"); when closing, set it `false`.

### 4. `chrome/view.rs` — `render_num_fmt_popover` restructure

Split the inner menu into two builders; keep the shared backdrop + card wrapper.

- `num_fmt_basic_menu(&self, active_code: &str, cx) -> gpui::Div`: the 7 `BASIC_FORMATS` as
  flat ghost buttons (id/selector `numfmt-<code>`, same as today, so existing selectors still
  resolve), each `.selected(preset.code == active_code)` and `.on_click` → `apply_num_fmt`.
  Then a trailing **"More ▸"** row: `Button::new("numfmt-more")`, debug_selector
  `"numfmt-more"`, `.selected(is_more_only_num_fmt(active_code))`, `.on_click` →
  `this.num_fmt_more_open = true; cx.notify();`.
- `num_fmt_more_menu(&self, active_code: &str, cx) -> gpui::Div`: a leading **"◂ Back"** row
  (`Button::new("numfmt-back")`, selector `"numfmt-back"`, `.on_click` →
  `this.num_fmt_more_open = false; cx.notify();`), then the **verbatim** Phase-6 grouped
  render (section header for multi-preset groups + `numfmt-<code>` buttons highlighted by
  exact code) — the full `NUM_FMT_GROUPS` inventory.
- `render_num_fmt_popover` picks the body by `self.num_fmt_more_open`, wraps it in the same
  `.absolute().backdrop(...)` + occluded `#numfmt-card` (keep `.max_h(320).overflow_y_scroll()`
  — harmless for the short basic list, which won't scroll; needed for the long More list). The
  backdrop dismiss closure also resets `num_fmt_more_open = false`.

## Tests

Pure (`format_ui.rs`):
- `basic_formats_are_the_original_seven` — `BASIC_FORMATS` has the 7 exact codes from `382f075^`.
- `every_basic_code_reverse_maps` — each basic code reverse-maps through `num_fmt_category`
  (basic set is a subset of `NUM_FMT_GROUPS`, so the map stays consistent).
- `is_basic_vs_more_only` — `is_basic_num_fmt`/`is_more_only_num_fmt` for a basic active code
  (`$#,##0.00` → basic, not more-only), a More-only preset (`0.00E+00`, `yyyy-mm-dd` → more-only,
  not basic), General (basic), and a Custom code (`0.000` → neither).

gpui (`chrome/view.rs`):
- `num_fmt_basic_menu_shows_seven_without_more_inventory` — open popover; all 7 `numfmt-<code>`
  basics + `numfmt-more` painted; a More-only item (`numfmt-0.00E+00`) NOT painted; no
  `numfmt-back`.
- `num_fmt_more_reveals_full_grouped_list` — open, click `numfmt-more`; `num_fmt_more_open`
  true; `numfmt-back` + a More-only item (`numfmt-0.00E+00`) now painted.
- `num_fmt_more_back_restores_basics` — drill in, click `numfmt-back`; back to basics
  (`numfmt-more` painted again, `numfmt-0.00E+00` gone).
- `num_fmt_basic_pick_applies_and_closes` — click a basic preset (`numfmt-#,##0.00`) → emits
  that code, popover closed, `num_fmt_more_open` reset.
- `num_fmt_more_pick_applies_and_closes` — drill in, click a More-only preset
  (`numfmt-yyyy-mm-dd`) → emits that code, popover + more reset closed.
- `num_fmt_opens_onto_more_for_more_only_active` — active cell format `0.00E+00`; toggling open
  lands in More view (`num_fmt_more_open == true`); active `$#,##0.00` opens basics-first
  (`false`).
- `num_fmt_paint_both_levels` (`VisualTestContext`) — basic level paints `numfmt-card` + basics
  + `numfmt-more`; after drilling in, the card paints `numfmt-back` + grouped More items.
