# Project: Window UI Scaling / Zoom

**Status:** Explored + declined (2026-07-08, task `gpui-scale-factor-explore`).

**Verdict:** We explored a per-window UI zoom and **decided not to build it.** Shipping true
window UI scaling at our pinned gpui rev would require either **forking and maintaining a zed
fork** (to add a settable, viewport-consistent per-window scale override) or a **large
rem-based rewrite of the app's layout including the perf-sensitive grid**. Both are too much
long-term scope/maintenance for the value, and gpui here is deliberately **pinned, not
forked**. So the approach is out — even if a throwaway spike could be made to render it, we
are not pursuing it. Recorded here so we (and future readers) don't re-explore this; the task
itself lived under git-ignored `.specs_skill_state/`.

## What was proposed

A per-window "zoom" for **spreadsheet** windows only (not welcome/about), scaling only the
window content area (not the OS title bar): a Window menu offering 100% (= gpui's
OS-DPI-derived default) plus 70/80/90/110/120/130/140/150% as multipliers of that default.
Non-goals for a spike would have been persistence, keyboard shortcuts, polish — just "does the
approach work."

The hard requirement was a **pure scale multiplier applied to everything** — no touching the
app's layout/sizing logic. The only gpui lever that scales *everything* uniformly (including
explicit `px` sizes) is `Window::scale_factor`, and gpui deliberately treats it as OS-owned
and read-only. That is what makes this a fork-or-rewrite decision rather than a small feature.

## GPUI API evidence (why there's no clean pure-scale lever)

From the gpui source at the pinned zed rev `1d217ee39d381ac101b7cf49d3d22451ac1093fe`
(`crates/gpui`):

- **`scale_factor` is a read-only, OS-derived getter.** `Window::scale_factor(&self) -> f32`
  (`src/window.rs:2384`) reads a **private** field (`window.rs:1026`). It is initialized from
  `platform_window.scale_factor()` (`window.rs:1338`) and **re-read from the platform on every
  resize** — `bounds_changed()` does `self.scale_factor = self.platform_window.scale_factor()`
  (`window.rs:2217`). So even an unsafe one-shot poke would be clobbered on the first resize.
- **No app-facing setter.** The platform trait `PlatformWindow` declares
  `fn scale_factor(&self) -> f32` (`src/platform.rs:626`) — a getter with no setter. Grep for
  `set_scale_factor` in **`crates/gpui` + `crates/gpui_platform`** returns **zero** hits. (The
  only `set_scale_factor` hits in the checkout are 2 in `crates/gpui_windows`, and they are OS
  `WM_DPICHANGED` DPI plumbing — reacting to the OS telling the window its DPI changed, not an
  app-facing override.)
- **`WindowOptions` has no scale field** (`src/platform.rs:1479`) — can't request a scaled window
  at open time either.
- **No element-subtree scale transform.** The only stacking-context transform helper is
  `Window::with_element_offset` (`window.rs:3224`) = translation only. `TransformationMatrix` is
  used solely by `paint_svg` for individual SVG glyph sprites (`window.rs:3989`), not for scaling a
  `div` subtree's layout + hit-testing. There is no public `with_transformation` on `Window`.
- **`set_rem_size` is runtime-settable but only scales rem-based units.** `Window::set_rem_size` /
  `with_rem_size` exist (`window.rs:2399`, backed by `rem_size_override_stack`) and would scale
  gpui's `_N` spacing helpers. But the app sets type and geometry in **explicit `px()`**
  (`text_size(px(13.0))`, window bounds `px(1200.0)`, dialog `w(px(360.0))`) and — critically — the
  **entire grid renders in px** (`grid/view.rs`: 69 `px(` sites, 0 rem; cell widths / row heights /
  fonts / gridlines / selection rects all flow from IronCalc geometry in px). `set_rem_size` would
  leave all of that unscaled — an inconsistent, broken result, the opposite of "a pure scale to
  everything."

## Key caveat: even the spike is non-trivial (a getter-only override clips, not zooms)

A tempting shortcut — patch `scale_factor()` to return `base × zoom` and see the window scale —
**does not work**, and this matters because it means even a throwaway spike is more than a
one-line poke. `scale_factor` is not just a rendering multiplier: it also drives
`viewport_size` / the framebuffer / GPU surface sizing. Multiplying only the getter enlarges the
scene (layout thinks it has more logical space) while the underlying surface stays fixed, so the
extra content is **clipped off the edges instead of zoomed**. A correct override has to *also*
scale the viewport / layout root so the surface and the scene stay consistent — which is exactly
the viewport-consistent per-window override that only a gpui fork can add cleanly. This is
further evidence for declining: there isn't even a cheap, correct spike here.

## Why we declined

Both routes that could actually ship this were rejected:

1. **Fork + maintain a zed fork — rejected.** A real build needs a per-window `scale_factor`
   override that (a) survives `bounds_changed`, (b) is app-settable (thread through `WindowOptions`
   and/or a `Window::set_scale_factor`), and (c) keeps the viewport/surface consistent with the
   scaled scene (see the caveat above). This is upstream-surgery in gpui, and gpui here is
   **pinned, not forked** — unlike the IronCalc fork policy, we deliberately do **not** maintain a
   zed fork. Taking this on means an ongoing rebase/maintenance burden against zed for one nicety.
2. **Rem-based end-to-end layout rewrite — rejected.** Convert the whole app — chrome *and* the
   grid's geometry/text — to express sizes in rem, then drive zoom with `set_rem_size`. This avoids
   the fork but is a **large change to the most performance-sensitive code** (the grid render path),
   and forfeits the "pure scale to everything" simplicity that motivated the feature.

**Net:** the long-term scope and maintenance cost of either route is out of proportion to the
value of window UI zoom right now, so it is not worth building. If we ever revisit, the
fork route is the only one that scales *everything* (px included) with zero app-layout changes,
and it must be weighed against our pinned-not-forked gpui posture; a real feature would also scope
to spreadsheet windows only, exclude the OS title bar, and add persistence + keyboard shortcuts.
