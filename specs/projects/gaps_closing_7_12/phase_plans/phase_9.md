---
status: complete
---

# Phase 9: Sum-section refinements + horizontal scroller control

## Overview

Owner feedback on the shipped Phase-1 status bar, in two parts:

- **9B** — a new **reusable horizontal-scroller control** (`chrome/h_scroller.rs`). Wraps a
  horizontally-scrollable content region and, **only when it overflows**, appends a static
  trailing section (divider + lucide `chevron-left`/`chevron-right` buttons, action-bar style,
  no visible scrollbar). Built first, then wired at two call sites (action bar, sheet-tab
  strip).
- **9A** — refinements to the selection-stats readout: adaptive decimal precision
  (`freecell-core/stats.rs::format_stat_value`, D9.1), vertical centering in `TAB_BAR_H`, a
  leading divider before the stats group, and always-visible (delivered by 9B pinning the stats
  group static to the right of the tab scroller).
- **9C** — a `CLAUDE.md` note that the project uses **lucide** for icons.

**Chrome-only → no pixel suite** (CLAUDE.md render-scope table). Validate with unit tests +
gpui view tests + `VisualTestContext` paint tests + an Xvfb smoke launch. Scope builds/tests to
`freecell-core` (stats) and `freecell-app` (chrome); always `cargo fmt --all --check`.

**Animation (D9.2):** the reusable helper is a stateless render function whose chevron
`on_click` closure only receives `&mut Window, &mut App` (no entity/view context to drive a
multi-frame tween or `cx.spawn` without coupling the control to a concrete view). Per D9.2 /
architecture §9 ("a non-animated clamp is an acceptable fallback if animation plumbing is
heavy — note it if so"), the chevrons perform a **non-animated clamped jump** of 0.8× viewport
width. Noted here and in the module doc.

## Steps

### Step 1 — New reusable `chrome/h_scroller.rs` control (9B core)

New module `app/crates/freecell-app/src/chrome/h_scroller.rs`; add `mod h_scroller;` to
`chrome/mod.rs`.

**Pure geometry (unit-tested exhaustively):**

```rust
/// Fraction of the viewport width one chevron click scrolls (D9.2).
pub(crate) const SCROLL_STEP_FRACTION: f32 = 0.8;
/// Float tolerance at the scroll extremes (sub-pixel rounding from taffy).
const EDGE_EPSILON: f32 = 0.5;

#[derive(Clone, Copy)]
pub(crate) enum ScrollDir { Left, Right }

/// Content overflows the viewport (chevron affordance needed). `max_scroll_x` = ScrollHandle::max_offset().x (>= 0).
pub(crate) fn overflows(max_scroll_x: f32) -> bool { max_scroll_x > EDGE_EPSILON }
/// Left chevron at its limit (fully scrolled to the start; offset near 0).
pub(crate) fn at_start(offset_x: f32) -> bool { offset_x >= -EDGE_EPSILON }
/// Right chevron at its limit (fully scrolled to the end; offset near -max_scroll_x).
pub(crate) fn at_end(offset_x: f32, max_scroll_x: f32) -> bool { offset_x <= -max_scroll_x + EDGE_EPSILON }
/// New clamped offset.x after a chevron click. offset_x <= 0, max_scroll_x >= 0, viewport > 0.
pub(crate) fn scroll_step(offset_x: f32, max_scroll_x: f32, viewport: f32, dir: ScrollDir) -> f32 {
    let step = viewport * SCROLL_STEP_FRACTION;
    let delta = match dir { ScrollDir::Left => step, ScrollDir::Right => -step };
    (offset_x + delta).clamp(-max_scroll_x, 0.0)
}
```

Sign convention: gpui `ScrollHandle` offset.x is 0 at the start and goes **negative** toward
the end (clamped to `[-max_offset.x, 0]`). Right chevron reveals content on the right → offset
more negative; left chevron → toward 0.

**State + render helper:**

```rust
#[derive(Clone, Default)]
pub struct HScroller { scroll: gpui::ScrollHandle }
impl HScroller { pub fn new() -> Self { Self::default() } }

/// Wrap `content` in a horizontally-scrollable region that shows chevron scroll buttons only
/// when the content overflows. `id` prefixes the region + chevron element ids (unique per call
/// site). Overflow/offset are read from the handle's *last-painted* measurements → one-frame lag
/// on first paint (standard gpui scroll-handle behaviour; the real chrome repaints on
/// resize/selection so it self-corrects immediately).
pub fn h_scroller(id: &'static str, scroller: &HScroller, content: impl IntoElement) -> impl IntoElement
```

- Read `max_scroll_x = f32::from(handle.max_offset().x)`, `offset_x = handle.offset().x`,
  `viewport = handle.bounds().size.width`.
- Scroll region: `div().id("{id}-scroll").flex().flex_1().min_w(px(0.)).overflow_x_scroll().track_scroll(handle).child(content)`.
- Outer row: `div().flex().items_center().flex_1().min_w(px(0.)).child(scroll_region)`.
- When `overflows(max_scroll_x)`: append a chevron section wrapped in a
  `div().debug_selector(|| "{id}-chevrons")` = `action_divider()` + left chevron + right chevron.
- Chevron button (gpui-component `Button`, `ghost().small()`, lucide icon
  `icons/chevron-left.svg` / `icons/chevron-right.svg`, `.disabled(at_limit)`):
  `on_click(move |_, window, _app| { let n = scroll_step(offset_x, max_scroll_x, viewport, dir); handle.set_offset(point(px(n), handle.offset().y)); window.refresh(); })`
  (handle is a cloned `ScrollHandle` — interior-mutable `Rc`).
- Reuse `action_divider()` from `view.rs` → make it `pub(super)`.

### Step 2 — Call site 1: action bar (`render_action_row`)

- Collect the existing button groups into a content div
  `div().flex().items_center().gap_1().min_w(px(ACTION_ROW_MIN_W)).child(...)...` (keeps the
  groups at natural width so they overflow rather than compress; `ACTION_ROW_MIN_W` stays used).
- Frame: `div().flex().items_center().w_full().h(px(ACTION_ROW_H)).px_2().bg().border_b()`
  `.child(h_scroller("action-row", &self.action_scroller, content))`
  `.when(self.eval.spinner(), |r| r.child(Spinner...))`.
- **Remove** `.min_w(ACTION_ROW_MIN_W)` and the internal `flex_1` spacer from the frame (the
  h_scroller is flex_1 and fills up to the spinner). Verified: at the test window width (1200 >
  natural ~1152) the groups fit → no chevrons → items lay out at unchanged positions (popover-hit
  tests unaffected).

### Step 3 — Call site 2: sheet-tab strip (`render_tab_bar`) + 9A.3/9A.4

- Build tabs + "+" add-sheet button into a scroller content div
  `div().flex().items_center().gap_1()`.
- Row: keep the frame (`relative`, mouse-move/up handlers, `h(TAB_BAR_H)`, bg, border) and:
  `.child(h_scroller("tab-bar", &self.tab_scroller, tabs))` (flex_1, fills space) → **drop** the
  old `div().flex_1()` spacer → **leading divider** `action_divider()` rendered **only when
  stats are present** (`self.stats_readout_parts().is_some()`) → `render_selection_stats` (static
  right content; empty when hidden). Drop indicator stays as the trailing absolute child.

### Step 4 — 9A.1 adaptive decimals (`freecell-core/src/stats.rs`)

- Rewrite `format_stat_value`: pick decimals by `|value|` tier via a private
  `decimals_for_magnitude(abs) -> usize` (`>=100 → 2`, `>=10 → 3`, `>=1 → 4`, else `5`). Keep the
  zero/sign/non-finite guards, `trim_trailing_zeros`, and `group_thousands`. Remove the now-unused
  `MAX_SIG_DIGITS`/`MAX_DECIMALS` significant-digit budgeting (the fixed per-tier decimal count
  inherently caps float noise). `format_stat_count` unchanged.
- Update the two Phase-1 tests whose expected output changes under the tier rule
  (`1_234_567.891 → "1,234,567.89"`, `-1234.567 → "-1,234.57"`).

### Step 5 — 9A.2 vertical centering (`render_selection_stats`)

- Add `.h_full().line_height(px(TAB_BAR_H))` to the stats group so its text centers in the bar.

### Step 6 — 9C CLAUDE.md lucide note

- Add a Conventions line: the project uses **lucide** icons (vendored under
  `app/crates/freecell-app/assets/icons/`, composed over the gpui-component Lucide bundle in
  `shell/assets.rs`; `chevron-left`/`chevron-right` resolve from the bundle — no new vendored
  file).

### Step 7 — Wiring

- Add `action_scroller: HScroller` + `tab_scroller: HScroller` fields to `ChromeView`; init both
  in `new()`. Re-export `HScroller`/`h_scroller` from `chrome/mod.rs` as needed (module-internal
  use is enough; export only if a test needs it).

## Tests

**`freecell-core/stats.rs` (pure, exhaustive):**
- `format_value_adaptive_decimals_by_tier`: tier boundaries `0.9999→"0.9999"`, `1→"1"`,
  `9.999→"9.999"`, `10→"10"`, `99.99→"99.99"`, `100→"100"`, `1000000.6666→"1,000,000.67"`,
  negatives (`-1234.567→"-1,234.57"`, `-5→"-5"`), zero (`0→"0"`), sub-ULP (`-1e-15→"0"`).
- Keep `format_value_caps_float_noise` (0.1+0.2 → "0.3"; 1/3 bounded) — still passes under tiers.
- Update `format_value_groups_and_trims` / `format_value_handles_sign_and_zero` expectations.

**`chrome/h_scroller.rs` (pure):**
- `overflows` true/false around `EDGE_EPSILON`.
- `at_start`/`at_end` at both extremes + mid-range.
- `scroll_step`: right moves offset by −0.8×viewport, left by +0.8×viewport, both clamped to
  `[-max, 0]` at the ends.

**`chrome/view.rs` (gpui / `VisualTestContext`):**
- `action_row_fits_has_no_chevrons`: wide window → `debug_bounds("action-row-chevrons")` is None.
- `action_row_overflow_shows_chevrons`: narrow window, paint twice → chevron section present.
- `tab_bar_overflow_shows_chevrons_and_keeps_stats_static`: many tabs, numeric selection →
  chevron section present AND `selection-stats` still paints to the right.
- `tab_bar_leading_divider_only_with_stats`: divider element gated on stats presence.
- `chevron_click_scrolls_offset`: `VisualTestContext::simulate_click` the right chevron →
  `tab_scroller` handle offset.x becomes negative; left chevron at start is a no-op.
- Existing `tab_bar_paints_stats_readout_when_present` still green (stats now static right).

**Smoke:** `xvfb-run -a cargo run -p freecell-app` opens the welcome window without panic.
