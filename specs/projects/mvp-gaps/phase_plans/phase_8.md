---
status: complete
---

# Phase 8: Titlebar (macOS) + closeout

## Overview

The final mvp-gaps phase: the macOS custom titlebar (§7.1) plus project closeout —
reconciling the render-baseline suite, wiring a borders perf assertion, extending the
smoke checklist, and sweeping GAPS.md. §7.2 (cap popover) and §7.3 (`.back` backup)
were delivered in Phase 1 (verified present — not re-implemented here).

**Environment honesty.** This is a headless Linux container — NOT macOS, and NOT the
pinned Xvfb+lavapipe render/perf runner. So:

- The §7.1 code is implemented per spec and **compiles + tests green on Linux** (guarded
  so Linux is unaffected), but the **30-minute macOS on-device smoke is an OUTSTANDING
  GATE** requiring a human on a Mac (recorded in DECISIONS). The gpui APIs it relies on
  were **verified present at the pinned rev** (`TitlebarOptions`, `traffic_light_position`,
  `WindowControlArea::Drag`, the `.window_control_area(..)` fluent method,
  `WindowOptions.titlebar`) — so the §7.1 flag-off fallback is NOT triggered on API
  grounds; only the on-device behavior remains unverified.
- Render baselines and the perf gate must be run on the pinned runner (documented; no PNGs
  committed).

## Steps

1. **`shell/titlebar.rs` (new):** a shared, non-cfg titlebar builder used by both the app
   window and the render harness.
   - `pub const MACOS_TITLEBAR: bool = cfg!(target_os = "macos")` — the single master
     switch (the §7.1 pre-agreed flag-off fallback = flip this to `false`, one line).
   - `pub const TITLEBAR_HEIGHT: f32 = 36.0`.
   - `pub fn titlebar_row(title: impl Into<SharedString>) -> impl IntoElement` — a 36px
     `CHROME_BG` row, bottom `HAIRLINE`, `.window_control_area(WindowControlArea::Drag)`,
     centered 13px medium `#3C3C3C` title text (ui_design §1). Not cfg-guarded (it is just
     a div; the render harness renders it on Linux).
2. **`shell/mod.rs`:** `pub mod titlebar;`.
3. **`shell/app.rs`:** add `fn titlebar_options() -> Option<TitlebarOptions>` returning
   `Some(TitlebarOptions { appears_transparent: true, traffic_light_position:
   Some(point(px(12.), px(12.))), title: None })` when `titlebar::MACOS_TITLEBAR` else
   `None`; set `titlebar: titlebar_options(),` in both `document_window_options()` and
   `welcome_window_options()`. Import `point`, `TitlebarOptions`.
4. **`shell/window.rs`:** in `WorkbookWindow::render`, prepend
   `.children(titlebar::MACOS_TITLEBAR.then(|| titlebar::titlebar_row(self.titlebar_title())))`
   as the first visual child. Add `fn titlebar_title(&self) -> String` =
   `window_title(document_name(path), dirty, true)` (the custom titlebar text always
   carries the `— Edited` suffix, ui_design §1). Keep the existing `set_window_title` calls
   (they feed the hidden native title/Exposé).
5. **`shell/welcome.rs`:** restructure `render` into an outer `flex_col size_full` with the
   macOS titlebar (title "FreeCell") on top and the existing centered content moved into a
   `flex_1` inner container — so Linux (no titlebar) is pixel-identical to today.
6. **Render harness — `titlebar_row` + `text_color_red` cases** (`render-tests`):
   - `cases.rs`: add `titlebar: Option<&'static str>` field + builder to `RenderCase`; add
     `text_color_red` (a cell with an explicit red font color — constructible via the
     existing `font_color` inject) and `titlebar_row` (the titlebar div over a short grid).
   - `render.rs`: when `case.titlebar` is set, wrap the grid in a `TitlebarScene` wrapper
     view (`flex_col[titlebar_row(title), grid]`); else mount the grid directly (unchanged).
   - `render_suite.rs`: add both names to the `render_cases!` list (keeps
     `case_names_match_table` green).
7. **Perf assertion — 500-bordered-cell viewport** (§9):
   - `perf.rs`: `build_bordered_fixture()` builds a small, narrow-geometry sheet with an
     all-thin `BorderSpec` interned into **every** cell of the region (cache-resident borders;
     borders + geometry only, no cell values); a headless unit test asserts the fixture is
     dense-bordered.
   - `perf_harness.rs`: after the main sweep, measure a frame over a viewport showing ≥500
     bordered cells; assert the frame stays inside the bordered region, then FORCE+ASSERT the
     **actual distinct bordered-cell count** (`rows×cols` from the frame's ranges) `>= 500` and
     gate `frame_render_ns` under `CI_FRAME_MAX_NS`. The content-element count is reported
     separately (never conflated with the cell count). Contributes to the `--gate` verdict + JSON.
8. **Smoke checklist:** append mvp-gaps successor items to
   `specs/projects/mvp/smoke_checklist.md` (formatting, fonts, borders, clipboard, editing
   feel, resize/headers/insert-delete, titlebar), marking macOS/on-device vs headless.
9. **GAPS.md sweep:** mark gaps closed by Phases 1–7 resolved (with pointers); leave the
   functional_spec §7 out-of-scope items intact.
10. **DECISIONS_TO_REVIEW.md:** the §7.1 on-device gate + fallback; the consolidated
    all-phases render-baseline regen list; the render-suite §9 reconciliation
    (`format_red_negative` deferred — no Scene num_fmt setter; the conceptual align-default
    names mapped to existing cases); the perf gate deferral to the pinned runner.

## Tests

- **`titlebar_row` unit** (freecell-app, headless): `titlebar_row("X")` builds an element
  and `TITLEBAR_HEIGHT == 36.0`; `MACOS_TITLEBAR == false` on Linux (so the window/welcome
  render omits it — Linux unaffected). A `titlebar_title` test: dirty → `— Edited` suffix
  present regardless of `title_uses_suffix()`.
- **Window render smoke** (existing gpui-context tests stay green): the document + welcome
  windows still build/render on Linux with no titlebar child.
- **`perf::bordered_fixture_is_dense_bordered`** (headless): every cell in the region has a
  non-zero `RenderStyle.border`; the region is ≥500 cells.
- **Render suite guards:** `case_names_match_table` stays green with the two new cases; both
  skip cleanly without `FREECELL_RENDER=1`.
- **No baseline PNGs committed;** `cargo test` without `FREECELL_RENDER` stays green.
</content>
</invoke>
