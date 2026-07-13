//! A reusable horizontal-scroller control (`functional_spec.md §9B`, `architecture.md §9`,
//! decisions D9.2/D9.3).
//!
//! [`h_scroller`] wraps a horizontally-scrollable content region and, **only when the content
//! overflows** its viewport, appends a **static** trailing section — a divider plus lucide
//! `chevron-left` / `chevron-right` buttons styled like the action bar's — while keeping the
//! scrollbar hidden (no visible track). When the content fits it renders exactly as the bare
//! content region, so a call site is byte-for-byte unchanged until it actually overflows.
//!
//! **Two call sites** ([`super::view`]): the action-row button groups (scroll on a small window)
//! and the sheet-tab strip (tabs scroll; the selection-stats group is pinned static to the right,
//! so it can never be pushed off — `functional_spec.md §9A.4`).
//!
//! **Scroll model.** gpui's [`ScrollHandle`] offset x is `0` at the start and grows **negative**
//! toward the end, clamped to `[-max_offset.x, 0]`; `max_offset().x` is the overflow amount (`> 0`
//! ⇒ overflowing) and `bounds().size.width` is the viewport width. These are the **last-painted**
//! measurements, so overflow detection lags the very first paint by one frame — standard gpui
//! scroll-handle behaviour; the real chrome repaints on resize / selection change and self-corrects
//! immediately.
//!
//! **Animated (D10.2).** A chevron click no longer jumps: it stores the clamped destination (0.8×
//! the viewport width away, via [`scroll_step`]) in the scroller's interior-mutable `target` field
//! and calls `window.refresh()` to kick off the first redraw. Each render, while `target` is `Some`,
//! [`h_scroller`] lerps the offset a fixed fraction ([`anim_step`]) toward it and re-requests the
//! next frame via `window.request_animation_frame()` (which notifies the current view, `ChromeView`
//! — valid only from paint, which is why the *click* uses `refresh()` instead), snapping + clearing
//! `target` on arrival ([`anim_arrived`]). The short (~8-frame) slide self-terminates: frames are
//! requested **only** while animating, so idle wheel/trackpad scrolling is never fought (a wheel
//! scroll *during* an in-flight slide is briefly overridden until the slide converges). (This
//! replaces the D9.2 non-animated clamp — the Phase-9 fallback taken before this plumbing was in
//! place.)

use std::cell::Cell;
use std::rc::Rc;

use gpui::{div, point, prelude::*, px, App, ClickEvent, ScrollHandle, SharedString, Window};
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::{Disableable as _, Icon, Sizable as _};

use super::view::action_divider;

/// Fraction of the viewport width one chevron click scrolls (D9.2).
pub(crate) const SCROLL_STEP_FRACTION: f32 = 0.8;

/// Float tolerance at the scroll extremes — taffy rounds the scroll max to two decimals, so an
/// "at the limit" test needs a small slop rather than an exact equality.
const EDGE_EPSILON: f32 = 0.5;

/// Which chevron was clicked — the direction the *content* moves relative to the viewport.
#[derive(Clone, Copy)]
pub(crate) enum ScrollDir {
    /// Left chevron: reveal content toward the start (offset toward `0`).
    Left,
    /// Right chevron: reveal content toward the end (offset toward `-max`).
    Right,
}

/// Whether the content overflows the viewport horizontally — the condition under which the chevron
/// affordance is shown. `max_scroll_x` is [`ScrollHandle::max_offset`]`().x` (always `>= 0`).
pub(crate) fn overflows(max_scroll_x: f32) -> bool {
    max_scroll_x > EDGE_EPSILON
}

/// Whether the content is scrolled fully to the start (the left chevron disables). `offset_x` is
/// [`ScrollHandle::offset`]`().x` (`<= 0`).
pub(crate) fn at_start(offset_x: f32) -> bool {
    offset_x >= -EDGE_EPSILON
}

/// Whether the content is scrolled fully to the end (the right chevron disables).
pub(crate) fn at_end(offset_x: f32, max_scroll_x: f32) -> bool {
    offset_x <= -max_scroll_x + EDGE_EPSILON
}

/// The new clamped offset x after a chevron click: 0.8× the viewport width in the clicked
/// direction, clamped to `[-max_scroll_x, 0]` (D9.2). `offset_x <= 0`, `max_scroll_x >= 0`.
pub(crate) fn scroll_step(offset_x: f32, max_scroll_x: f32, viewport: f32, dir: ScrollDir) -> f32 {
    let step = viewport * SCROLL_STEP_FRACTION;
    let delta = match dir {
        ScrollDir::Left => step,
        ScrollDir::Right => -step,
    };
    (offset_x + delta).clamp(-max_scroll_x, 0.0)
}

/// Fraction of the remaining distance to the target one animation frame closes (D10.2). `< 1`, so
/// each frame strictly shrinks the gap (monotonic, no overshoot); `0.6` yields a quick ~7-8-frame
/// slide for the biggest chevron steps (fewer for smaller ones) — obviously a scroll, not a jump.
const ANIM_STEP_FRACTION: f32 = 0.6;

/// Distance (px) within which the chevron slide lands exactly on its target and stops, instead of
/// dribbling ever-smaller sub-pixel frames. Sub-pixel, so the snap is invisible.
const ANIM_SNAP_EPSILON: f32 = 0.5;

/// One animation frame of the chevron slide (D10.2): move `offset_x` a fixed fraction
/// ([`ANIM_STEP_FRACTION`]) of the remaining distance toward `target_x`, clamped to the scroll
/// range `[-max_scroll_x, 0]`. `factor < 1` ⇒ each frame strictly shrinks `|target_x - offset_x|`
/// (monotonic convergence, no overshoot); the caller snaps + stops once [`anim_arrived`].
/// `offset_x <= 0`, `max_scroll_x >= 0`. Pure (like [`scroll_step`]).
pub(crate) fn anim_step(offset_x: f32, target_x: f32, max_scroll_x: f32) -> f32 {
    (offset_x + (target_x - offset_x) * ANIM_STEP_FRACTION).clamp(-max_scroll_x, 0.0)
}

/// Whether the chevron slide is within [`ANIM_SNAP_EPSILON`] of its `target_x` — the point at which
/// the caller lands exactly on the target and ends the animation (so it never dribbles sub-pixel
/// frames or lingers to fight a manual scroll). Pure.
pub(crate) fn anim_arrived(offset_x: f32, target_x: f32) -> bool {
    (target_x - offset_x).abs() <= ANIM_SNAP_EPSILON
}

/// The persistent scroll state a call site owns (one per scroller). Holds the gpui
/// [`ScrollHandle`] whose interior-mutable offset the chevron buttons drive and the render helper
/// reads. Cloneable / `Default`; a fresh scroller starts at offset 0 with no measured overflow.
#[derive(Clone, Default)]
pub struct HScroller {
    scroll: ScrollHandle,
    /// The clamped destination offset of an in-flight chevron slide (D10.2), or `None` when idle.
    /// Interior-mutable + `Rc` so the chevron `on_click` (which gets only `&mut Window`, no view
    /// context) can arm a slide and the render helper can step/clear it — while `HScroller` stays
    /// `Clone`. `Default` = `None`, so `new()` / existing call sites are unchanged.
    target: Rc<Cell<Option<f32>>>,
}

impl HScroller {
    /// A new scroller, scrolled to the start.
    pub fn new() -> Self {
        Self::default()
    }

    /// The current horizontal scroll offset (`<= 0`; `0` at the start, `-max` at the end) — the
    /// last-painted value. A test-only inspection seam for asserting a chevron click moved the
    /// offset (unused in normal builds, so gated to keep them warning-free).
    #[cfg(test)]
    pub(crate) fn offset_x(&self) -> f32 {
        f32::from(self.scroll.offset().x)
    }

    /// Whether a chevron slide is currently in flight (`target` armed). A test-only seam for
    /// asserting a click armed the animation and that it self-clears on arrival.
    #[cfg(test)]
    pub(crate) fn is_animating(&self) -> bool {
        self.target.get().is_some()
    }
}

/// Wrap `content` in a horizontally-scrollable region that appends chevron scroll buttons only
/// when the content overflows (`functional_spec.md §9B`). `id` prefixes the region and chevron
/// element ids, so it MUST be unique per call site.
///
/// Fits → renders as the bare scroll region (no chevrons, no visible scrollbar). Overflows →
/// appends a static `divider + ◄ + ►` section (action-bar style), each chevron disabled at its
/// limit. The returned row is `flex_1` so the call site's static neighbours (the eval spinner /
/// the stats group) sit to its right and the scroll region fills the space between.
pub fn h_scroller(
    id: &'static str,
    scroller: &HScroller,
    window: &mut Window,
    content: impl IntoElement,
) -> impl IntoElement {
    let handle = &scroller.scroll;
    let max_scroll_x = f32::from(handle.max_offset().x);
    let viewport = f32::from(handle.bounds().size.width);

    // Drive one frame of the chevron slide (D10.2). While `target` is armed, lerp the offset toward
    // it and request the next frame; on arrival, snap exactly and clear `target`. `offset_x` then
    // tracks the live animated position, so the chevrons' disabled state + click base match what the
    // user currently sees. Frames are requested ONLY here (in the `else`), so the tween runs solely
    // while animating and stops the instant it converges — idle wheel/trackpad scrolling is never
    // fought (only a scroll landing mid-slide is briefly overridden until the ~8 frames finish).
    let mut offset_x = f32::from(handle.offset().x);
    if let Some(target) = scroller.target.get() {
        // Re-clamp the destination to the CURRENT scroll range every frame: a resize mid-slide can
        // shrink `max_scroll_x` (even to 0 when the content now fits), and the slide must still
        // terminate at the reachable edge instead of chasing an unreachable target forever.
        let goal = target.clamp(-max_scroll_x, 0.0);
        let next = if anim_arrived(offset_x, goal) {
            scroller.target.set(None);
            goal
        } else {
            window.request_animation_frame();
            anim_step(offset_x, goal, max_scroll_x)
        };
        handle.set_offset(point(px(next), handle.offset().y));
        offset_x = next;
    }

    let scroll_region = div()
        .id(SharedString::from(format!("{id}-scroll")))
        .flex()
        .flex_1()
        // Let the region shrink below its content so `overflow_x_scroll` clips + scrolls instead of
        // the content forcing the row wider (which would never overflow).
        .min_w(px(0.0))
        .overflow_x_scroll()
        .track_scroll(handle)
        .child(content);

    let mut row = div()
        .flex()
        .items_center()
        .flex_1()
        .min_w(px(0.0))
        .child(scroll_region);

    if overflows(max_scroll_x) {
        row = row.child(
            div()
                .id(SharedString::from(format!("{id}-chevrons")))
                .debug_selector(move || format!("{id}-chevrons"))
                .flex()
                .items_center()
                .child(action_divider())
                .child(chevron_button(
                    format!("{id}-chevron-left"),
                    "icons/chevron-left.svg",
                    at_start(offset_x),
                    scroller.target.clone(),
                    offset_x,
                    max_scroll_x,
                    viewport,
                    ScrollDir::Left,
                ))
                .child(chevron_button(
                    format!("{id}-chevron-right"),
                    "icons/chevron-right.svg",
                    at_end(offset_x, max_scroll_x),
                    scroller.target.clone(),
                    offset_x,
                    max_scroll_x,
                    viewport,
                    ScrollDir::Right,
                )),
        );
    }
    row
}

/// One chevron button: a ghost/small action-bar-styled `Button` with a lucide chevron icon that,
/// on click, **arms** an animated slide (D10.2) — it stores the clamped [`scroll_step`] destination
/// in `target` and requests an animation frame; [`h_scroller`] steps the offset toward it over the
/// next few frames. Disabled at its limit. The offset math is snapshotted from the last paint (the
/// same values the affordance was decided on), so a click always moves relative to what the user
/// currently sees. Wrapped in a `debug_selector`'d div (`id`) so a paint test can target this exact
/// chevron.
#[allow(clippy::too_many_arguments)]
fn chevron_button(
    id: String,
    icon_path: &'static str,
    disabled: bool,
    target: Rc<Cell<Option<f32>>>,
    offset_x: f32,
    max_scroll_x: f32,
    viewport: f32,
    dir: ScrollDir,
) -> impl IntoElement {
    let selector = id.clone();
    div().debug_selector(move || selector.clone()).child(
        Button::new(SharedString::from(id))
            .icon(Icon::empty().path(icon_path))
            .ghost()
            .small()
            .disabled(disabled)
            .on_click(move |_: &ClickEvent, window: &mut Window, _app: &mut App| {
                // Arm the slide toward the clamped destination and trigger the first redraw. A click
                // handler runs during event dispatch, not paint, so `request_animation_frame()` (it
                // reads `current_view`, paint-only) would panic here — `window.refresh()` is the
                // redraw lever available. The redraw runs `h_scroller`, which then drives the tween
                // frame-to-frame via `request_animation_frame()` from inside render (paint context).
                let dest = scroll_step(offset_x, max_scroll_x, viewport, dir);
                target.set(Some(dest));
                window.refresh();
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflows_only_past_the_epsilon() {
        assert!(!overflows(0.0), "no overflow when content fits");
        assert!(!overflows(0.3), "sub-epsilon slop is not an overflow");
        assert!(overflows(1.0));
        assert!(overflows(500.0));
    }

    #[test]
    fn at_start_and_at_end_detect_the_limits() {
        // Start: offset at 0 (or a hair positive from rounding).
        assert!(at_start(0.0));
        assert!(at_start(0.2));
        assert!(!at_start(-40.0));
        // End: offset at -max (within epsilon).
        let max = 300.0;
        assert!(at_end(-300.0, max));
        assert!(at_end(-299.8, max));
        assert!(!at_end(-100.0, max));
        // Mid-range is neither limit.
        assert!(!at_start(-100.0));
        assert!(!at_end(-100.0, max));
    }

    #[test]
    fn scroll_step_moves_by_eighty_percent_of_viewport() {
        let viewport = 200.0;
        let max = 500.0;
        // From the start, the right chevron scrolls the content left (offset more negative).
        assert_eq!(
            scroll_step(0.0, max, viewport, ScrollDir::Right),
            -160.0,
            "0.8 * 200 = 160 toward the end"
        );
        // The left chevron from mid-range moves back toward 0 by the same step.
        assert_eq!(scroll_step(-300.0, max, viewport, ScrollDir::Left), -140.0);
    }

    #[test]
    fn scroll_step_clamps_at_both_ends() {
        let viewport = 200.0; // step = 160
        let max = 100.0; // max scroll is less than one step
                         // Right past the end clamps to -max, not -160.
        assert_eq!(scroll_step(0.0, max, viewport, ScrollDir::Right), -100.0);
        // Left past the start clamps to 0, not +.
        assert_eq!(scroll_step(-100.0, max, viewport, ScrollDir::Left), 0.0);
        // A right step already at the end stays pinned at -max.
        assert_eq!(scroll_step(-100.0, max, viewport, ScrollDir::Right), -100.0);
    }

    #[test]
    fn anim_step_converges_monotonically_without_overshoot() {
        // Slide from the start toward a target near the end; every frame must shrink the gap and
        // never cross the target (factor < 1 ⇒ monotonic, no overshoot).
        let target: f32 = -320.0;
        let max: f32 = 400.0;
        let mut offset: f32 = 0.0;
        let mut prev_gap = (target - offset).abs();
        let mut frames = 0;
        while !anim_arrived(offset, target) {
            offset = anim_step(offset, target, max);
            let gap = (target - offset).abs();
            assert!(
                gap < prev_gap,
                "each frame shrinks the gap: {gap} !< {prev_gap}"
            );
            assert!(
                offset >= target,
                "never overshoots past the target: {offset} < {target}"
            );
            prev_gap = gap;
            frames += 1;
            assert!(frames < 60, "must converge, not spin forever");
        }
        // A representative full step lands in a small, "quick slide" number of frames.
        assert!(
            (1..=10).contains(&frames),
            "a full chevron step is a quick ~4-8 frame slide, got {frames}"
        );
    }

    #[test]
    fn anim_arrived_snaps_only_within_epsilon() {
        // Outside the snap epsilon it keeps sliding; within it, it's done.
        assert!(!anim_arrived(-10.0, -320.0));
        assert!(!anim_arrived(-319.0, -320.0), "1px out is not yet arrived");
        assert!(anim_arrived(-320.0, -320.0));
        assert!(
            anim_arrived(-319.6, -320.0),
            "within 0.5px of the target counts as arrived"
        );
    }

    #[test]
    fn anim_step_stays_within_scroll_range() {
        let max = 200.0;
        // A mid-flight step from the start toward the end stays in [-max, 0].
        let mid = anim_step(0.0, -160.0, max);
        assert!(
            (-max..=0.0).contains(&mid),
            "intermediate offset in range, got {mid}"
        );
        // A stale target past the end is clamped to -max, never further.
        assert_eq!(anim_step(-190.0, -500.0, max), -max);
        // A target at the start never yields a positive offset.
        assert!(anim_step(-1.0, 0.0, max) <= 0.0);
    }
}
