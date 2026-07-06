//! The macOS custom-titlebar row (`architecture.md §7.1`, `functional_spec.md §4.1`,
//! `ui_design.md §1`).
//!
//! A 36 px `CHROME_BG` row with the centered document title that doubles as the platform
//! window drag region. It pairs with the transparent-titlebar `WindowOptions` set in
//! [`super::app`] (repositioned traffic lights, hidden system title). The row is **built
//! unconditionally** — it is just a `div`, so the render-test harness renders it on Linux to
//! pixel-check its look (`architecture.md §9` `titlebar_row`). Whether it is *included* in a
//! real window is gated on [`MACOS_TITLEBAR`] at the call sites (the document window, the
//! Welcome window, and the window options), so Linux is completely unaffected (server
//! decorations as today).
//!
//! **Outstanding gate.** The *native* macOS integration — the transparent titlebar, the
//! repositioned traffic lights, and drag / double-click-zoom / fullscreen behavior at the
//! pinned gpui rev — is the §7.1 **30-minute on-device smoke** and can only be verified on a
//! Mac. The gpui APIs this uses were verified present at the pinned rev; the pre-agreed
//! fallback if the on-device smoke finds glitches is to flip [`MACOS_TITLEBAR`] to `false`
//! (one line, no gpui bump) — see `DECISIONS_TO_REVIEW.md`, Phase 8.

use gpui::{div, prelude::*, px, rgb, FontWeight, MouseButton, SharedString, WindowControlArea};

/// Master switch for the macOS custom titlebar (`architecture.md §7.1`). `cfg!(target_os =
/// "macos")` so Linux never draws it and never sets the transparent-titlebar window option.
///
/// The §7.1 pre-agreed flag-off fallback (if the on-device smoke finds traffic-light /
/// fullscreen glitches at the pinned rev) is to set this to `false`: it removes both the
/// transparent-titlebar `WindowOptions` and the drawn row everywhere in one edit — no gpui bump.
pub const MACOS_TITLEBAR: bool = cfg!(target_os = "macos");

/// The titlebar row height in px (`ui_design.md §1`).
pub const TITLEBAR_HEIGHT: f32 = 36.0;

/// Action-bar grey — matches the chrome (`ui_design.md §1`).
const CHROME_BG: u32 = 0xF3F3F3;
/// Bottom hairline under the row.
const HAIRLINE: u32 = 0xD9D9D9;
/// Centered title text colour (`ui_design.md §1`: 13 px, medium, `#3C3C3C`).
const TITLE_TEXT: u32 = 0x3C3C3C;

/// Builds the custom titlebar row: a 36 px `CHROME_BG` bar (bottom hairline) that is a window
/// drag region, with the document `title` centered. `title` already carries any `— Edited`
/// suffix (the caller decides). No custom buttons — the OS-drawn traffic lights sit over the
/// left edge (repositioned by the transparent-titlebar window option).
pub fn titlebar_row(title: impl Into<SharedString>) -> impl IntoElement {
    div()
        .h(px(TITLEBAR_HEIGHT))
        .w_full()
        .flex_shrink_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(CHROME_BG))
        .border_b_1()
        .border_color(rgb(HAIRLINE))
        // The whole row is the platform window drag region.
        //
        // `window_control_area(Drag)` records the row as a drag region for the client-side-decoration
        // backends (Linux/Windows), whose caption hit-testing reads gpui's `on_hit_test_window_control`
        // callback. It is forward-looking scaffolding today: every real-window call site gates the row
        // on `MACOS_TITLEBAR`, so in real windows it is only mounted on macOS (the Linux render fixture
        // mounts it unconditionally, but only to pixel-check the div's look) — and on macOS that
        // callback is a no-op
        // at the pinned gpui rev, while a transparent (`appears_transparent`) titlebar makes AppKit
        // treat the whole content view as non-draggable. So `window_control_area` never moves the
        // window on its own; that inertness is the regression from the previous native/opaque bar.
        //
        // We therefore start the platform move directly on mouse-down. `start_window_move` maps to
        // AppKit `performWindowDragWithEvent:`, which Apple documents as the call to make from
        // `mouseDown:`: it runs its own event-tracking loop that disambiguates click vs drag (a
        // stationary click never moves the window) and honours the system "double-click title bar"
        // (zoom) preference — so double-click-zoom is handled implicitly, no separate handler.
        //
        // This is deliberately NOT gpui-component `TitleBar`'s pattern. That one does NOT call
        // `start_window_move` on down; it sets a `should_move` flag on down (cleared on up / down-out)
        // and calls `start_window_move` on the following `on_mouse_move` — a stateful design that keeps
        // a plain click out of AppKit's tracking loop, which matters for a titlebar packed with
        // interactive controls. The direct call is the better fit here: this row hosts no interactive
        // gpui children and no click-sensitive state (the traffic lights are separate OS `NSButton`s),
        // so AppKit swallowing a click's mouse-up is harmless — while requiring the *press* to
        // originate on the row avoids a grid range-drag that overshoots up into the titlebar spuriously
        // moving the window, which a *naive* move-triggered variant (one without gpui-component's
        // `should_move` origin flag) would allow. Because `start_window_move` is
        // a macOS-native call the headless test platform stubs with `unimplemented!()`, the click /
        // mouse-up and double-click-zoom behaviour here can only be exercised by the §7.1 on-device
        // smoke (add it to that checklist).
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |_, window, _| window.start_window_move())
        .child(
            div()
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(rgb(TITLE_TEXT))
                .child(title.into()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titlebar_height_is_36() {
        assert_eq!(TITLEBAR_HEIGHT, 36.0);
    }

    #[test]
    fn macos_titlebar_matches_target() {
        // The master switch tracks the compile target: Linux never draws the row (so the window
        // render + server decorations stay unaffected), macOS always does — subject to the §7.1
        // on-device smoke. Flipping this const to `false` is the pre-agreed flag-off fallback.
        assert_eq!(MACOS_TITLEBAR, cfg!(target_os = "macos"));
    }

    #[test]
    fn titlebar_row_builds_an_element() {
        // A smoke build of the row (no window needed) — proves the fluent chain type-checks
        // against the pinned gpui element/interactive traits, including the `on_mouse_down`
        // window-drag handler (`start_window_move`). The drag itself is a macOS-native AppKit
        // behavior (`performWindowDragWithEvent:`) that the headless test platform stubs with
        // `unimplemented!()`, so it can only be exercised by the §7.1 on-device smoke — not here.
        let _ = titlebar_row("Budget.xlsx — Edited");
    }
}
