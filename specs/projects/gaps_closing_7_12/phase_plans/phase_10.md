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

---

# Phase 10.2 + 10.3: h-scroller overflow fix + animated chevron scroll

## Overview

Two small post-use feedback tweaks to the Phase-9 horizontal scroller (`functional_spec.md
§10.2/§10.3`, `architecture.md §10`, decision **D10.2**), both chrome-only (action bar +
tab bar) → **not** pixel-suite scope; validated with the crate's gpui view tests +
`VisualTestContext` paint tests + an Xvfb smoke launch (no render suite).

- **10.2 (bug fix)** — the action-bar h-scroller shows its chevrons while every button is still
  visible. Cause: `render_action_row`'s button-group `div` carries
  `.min_w(px(ACTION_ROW_MIN_W))` with `ACTION_ROW_MIN_W = 1152.0`, a hand-estimated
  (self-documented-as-drift-prone) over-estimate. When 1152 > the true natural button width, the
  surplus is trailing empty space *inside* the scroll content, so `h_scroller`'s
  `max_offset().x > 0` overflow check fires early (chevrons + a gap to the right of the find
  button). Fix: drop the `min_w` + the `ACTION_ROW_MIN_W` const and put `.flex_shrink_0()` on the
  button-group content — flexbox default shrink = 1 was the *only* reason `min_w` was there
  (to stop the buttons compressing). The content then sits at its exact natural width (no
  compression — Phase 9's "scroll, don't squish" intent preserved), so chevrons appear **only**
  when the buttons genuinely don't fit.

- **10.3 (D10.2)** — replace `h_scroller`'s non-animated clamped jump (Phase 9's sanctioned D9.2
  fallback) with a fast, clearly-visible animated slide. gpui's
  `window.request_animation_frame()` notifies the current view (`ChromeView`) each frame and needs
  only `&mut Window` — which the chevron `on_click` already has (no view `Context`/entity
  plumbing). A chevron click stores the clamped destination in an interior-mutable `target` on the
  scroller and requests a frame; `h_scroller()` (now given `&mut Window`) lerps the offset toward
  `target` each render, re-requesting frames only while animating, and snaps + clears `target` on
  arrival. Frames are driven **only** while `target` is `Some`, so the tween never fights manual
  wheel/trackpad scrolling.

## Steps

### 1. `chrome/h_scroller.rs` — animation state + pure step math

- Add `use std::cell::Cell;` + `use std::rc::Rc;`.
- Add two tuning consts + a snap epsilon:
  - `ANIM_STEP_FRACTION: f32 = 0.6` — per-frame fraction of the remaining gap closed (a quick
    ~7-8-frame slide for the biggest steps; fewer for smaller ones — "obviously a scroll, not a
    teleport").
  - `ANIM_SNAP_EPSILON: f32 = 0.5` — land exactly + end the slide within this distance (px).
- Two **pure, unit-tested** fns (mirroring `scroll_step`/`overflows`):
  - `pub(crate) fn anim_step(offset_x: f32, target_x: f32, max_scroll_x: f32) -> f32` —
    `(offset_x + (target_x - offset_x) * ANIM_STEP_FRACTION).clamp(-max_scroll_x, 0.0)`. `factor <
    1` ⇒ each frame strictly shrinks the gap (monotonic, no overshoot); always clamped to the
    scroll range.
  - `pub(crate) fn anim_arrived(offset_x: f32, target_x: f32) -> bool` —
    `(target_x - offset_x).abs() <= ANIM_SNAP_EPSILON` (the snap/stop trigger).
- Add `target: Rc<Cell<Option<f32>>>` to `HScroller` (`Default` = `None` ⇒ existing call
  sites/`new()` unchanged; struct stays `Clone, Default`). `None` = idle; `Some(dest)` = a slide
  toward the clamped destination is in flight.
- Add a `#[cfg(test)] pub(crate) fn is_animating(&self) -> bool` inspection seam (asserts a click
  armed / a finished slide cleared `target`).

### 2. `chrome/h_scroller.rs` — drive the slide in `h_scroller()`

- Add a `window: &mut Window` param: `h_scroller(id, scroller, window, content)`.
- Before building the row, after snapshotting `max_scroll_x` / `offset_x` / `viewport`, drive one
  frame of the slide when `target` is `Some`:

  ```rust
  let mut offset_x = f32::from(handle.offset().x);
  if let Some(target) = scroller.target.get() {
      let next = if anim_arrived(offset_x, target) {
          scroller.target.set(None);        // within snap distance → land exactly, end the slide
          target
      } else {
          window.request_animation_frame();  // still sliding → schedule the next frame
          anim_step(offset_x, target, max_scroll_x)
      };
      handle.set_offset(point(px(next), handle.offset().y));
      offset_x = next;                        // chevron disabled-state + click base track the live pos
  }
  ```

  Frames are requested ONLY inside the `else` (while animating) → the tween self-terminates and
  never fights wheel/trackpad scrolling.

### 3. `chrome/h_scroller.rs` — chevron click arms the slide

- `chevron_button` gains a `target: Rc<Cell<Option<f32>>>` param (it no longer needs the
  `ScrollHandle` — it arms the target instead of setting the offset); `h_scroller` passes
  `scroller.target.clone()` into each chevron.
- `on_click`: compute the clamped destination via the existing `scroll_step`, store it, and kick
  the first redraw:

  ```rust
  let dest = scroll_step(offset_x, max_scroll_x, viewport, dir);
  target.set(Some(dest));
  window.refresh();
  ```

  **Implementation note (constraint found at build time):** a click handler runs during event
  dispatch, **not** paint, and `window.request_animation_frame()` reads `current_view()`, which is
  paint/prepaint/layout-only — so calling it from `on_click` **panics**. The click therefore uses
  `window.refresh()` (valid anywhere, like the Phase-9 code) to trigger the first redraw; the
  frame-to-frame `request_animation_frame()` lives in `h_scroller`'s render (paint context), where
  it is valid. Same D10.2 mechanism/outcome — only the *first-frame trigger* differs from the
  architecture's sketch.
- Re-clamp the armed target to the CURRENT `max_scroll_x` each frame (`target.clamp(-max, 0)`) so a
  resize mid-slide (which can shrink `max_scroll_x`, even to 0 when the content now fits) still
  terminates the slide at the reachable edge instead of requesting frames forever.
- Update the module doc block (the "Non-animated (D9.2 fallback)" paragraph) to describe the
  D10.2 animated slide.

### 4. `chrome/view.rs` — 10.2 fix + thread `&mut Window`

- Delete the `ACTION_ROW_MIN_W` const (~L158) and its running-tally comment (~L149-157).
- In `render_action_row`, on the `groups` content `div`: replace `.min_w(px(ACTION_ROW_MIN_W))`
  with `.flex_shrink_0()`; update the adjacent comment (drop the `min_w ≈ ACTION_ROW_MIN_W`
  reference → "natural width, `flex_shrink_0` so it never compresses"). Fixed-width children
  (font-family `w(140)`, font-size `w(56)`) lay out unchanged.
- Thread `&mut Window` through the render path (its sole caller is the top-level `render`):
  - `fn render(&mut self, window: &mut Window, cx)` — un-underscore `_window`.
  - `fn render_action_row(&self, window: &mut Window, cx)` and
    `fn render_tab_bar(&self, window: &mut Window, cx)` — add the param; forward it to the two
    `h_scroller(...)` calls (`h_scroller("action-row", &self.action_scroller, window, groups)` /
    `h_scroller("tab-bar", &self.tab_scroller, window, tabs)`). Update the `render` call sites.

## Tests

Pure (`h_scroller.rs`, alongside `scroll_step`/`overflows`):
- `anim_step_converges_monotonically` — from a far offset toward a target, repeated `anim_step`
  strictly shrinks `|target - offset|` every frame and never overshoots.
- `anim_step_snaps_within_epsilon` (via `anim_arrived`) — `anim_arrived` is true within
  `ANIM_SNAP_EPSILON`, false outside; a slide reaches the arrived state in a small (~4-8) number
  of frames for a representative step.
- `anim_step_clamps_within_range` — an intermediate step (and a target at the edge) stays within
  `[-max, 0]`; never positive, never past `-max`.

gpui (`chrome/view.rs`, alongside the existing scroller tests):
- **10.2** `action_row_natural_width_is_under_the_old_estimate` — at a wide window, the painted
  natural width of the action-row button group (`action-row-groups` debug_selector) is `< 1152`
  (proves the const over-estimated); at a viewport just above that natural width (still `< 1152`)
  the action row reports **no** chevrons (the bug: it used to show them there). A genuinely narrow
  viewport still overflows (existing `action_row_overflow_shows_chevrons` retained).
- **10.3** `chevron_click_animates_to_target` (rework of `chevron_click_scrolls_and_clamps`) —
  clicking the right chevron arms the slide (`tab_scroller.is_animating()` true, offset still ~0
  before any frame); pumping frames (`refresh` + `run_until_parked`) steps the offset
  progressively negative and monotonically, then it settles (`is_animating()` false) at the
  clamped `scroll_step` destination. The left chevron at the start stays a no-op (disabled → no
  target armed). Add a small `pump_frame`/`pump_frames(n)` test helper (refresh + run_until_parked
  per frame).
- **10.3** `chevron_animation_clamps_at_end` — repeated right-chevron clicks + frame pumps drive
  the tab scroller to `-max` and no further (`at_end`), and the right chevron then disables.
