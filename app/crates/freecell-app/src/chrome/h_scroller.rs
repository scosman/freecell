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
//! **Non-animated (D9.2 fallback).** A chevron click jumps the offset by 0.8× the viewport width,
//! clamped, with **no** tween. D9.2 asks for an animated scroll but sanctions a non-animated clamp
//! "if animation plumbing is heavy". It is: this is a stateless render helper whose chevron
//! `on_click` only receives `&mut Window, &mut App` — no entity/view context to drive a multi-frame
//! tween or `cx.spawn` without coupling the control to a concrete view. So the clamp is used and
//! called out here.

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

/// The persistent scroll state a call site owns (one per scroller). Holds the gpui
/// [`ScrollHandle`] whose interior-mutable offset the chevron buttons drive and the render helper
/// reads. Cloneable / `Default`; a fresh scroller starts at offset 0 with no measured overflow.
#[derive(Clone, Default)]
pub struct HScroller {
    scroll: ScrollHandle,
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
    content: impl IntoElement,
) -> impl IntoElement {
    let handle = &scroller.scroll;
    let max_scroll_x = f32::from(handle.max_offset().x);
    let offset_x = f32::from(handle.offset().x);
    let viewport = f32::from(handle.bounds().size.width);

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
                    handle.clone(),
                    offset_x,
                    max_scroll_x,
                    viewport,
                    ScrollDir::Left,
                ))
                .child(chevron_button(
                    format!("{id}-chevron-right"),
                    "icons/chevron-right.svg",
                    at_end(offset_x, max_scroll_x),
                    handle.clone(),
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
/// on click, jumps the scroll offset by [`scroll_step`] and forces a repaint. Disabled at its
/// limit. The offset math is snapshotted from the last paint (the same values the affordance was
/// decided on), so a click always moves relative to what the user currently sees. Wrapped in a
/// `debug_selector`'d div (`id`) so a paint test can target this exact chevron.
#[allow(clippy::too_many_arguments)]
fn chevron_button(
    id: String,
    icon_path: &'static str,
    disabled: bool,
    handle: ScrollHandle,
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
                let new_x = scroll_step(offset_x, max_scroll_x, viewport, dir);
                handle.set_offset(point(px(new_x), handle.offset().y));
                // The offset lives in the handle's interior-mutable `Rc`; a repaint re-reads it (and
                // the chevrons' disabled state). `window.refresh()` is the only redraw lever a button
                // click's `&mut Window` offers (no view `cx.notify()` here).
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
}
