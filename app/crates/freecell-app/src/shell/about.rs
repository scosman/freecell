//! [`AboutView`] — the standalone About window (`functional_spec.md §4`, `ui_design.md §6`,
//! `architecture.md §9`).
//!
//! A small, fixed-size, single-instance window that replaces the old About modal overlay. It is a
//! `Render` + `Focusable` entity like [`WelcomeView`](super::WelcomeView) but **stateless** — pure
//! static content: the FreeCell wordmark, the "The open spreadsheet" tagline, the build version
//! (`env!("CARGO_PKG_VERSION")`), a hairline, then two label→value link rows (Homepage / Built
//! with). Links open in the user's default browser via gpui's [`App::open_url`], with a pointer
//! cursor (no hover underline). It registers **no document actions**, so Save / Undo / etc. are
//! disabled while it is frontmost; the app opens/activates/tracks/closes it (`super::app`).

use gpui::{
    div, prelude::*, px, rgb, App, ClickEvent, Context, FocusHandle, Focusable, FontWeight, Window,
};

use super::fonts::{LINK_FAMILY, TAGLINE_FAMILY, WORDMARK_FAMILY};
use super::{titlebar, CloseWindow};

// Shared chrome/titlebar palette tokens (`ui_design.md §0`) — mirrored here as the established
// pattern (titlebar.rs / chrome/view.rs / welcome.rs carry the same values); no new hexes beyond
// the one LINK accent below.
const CHROME_BG: u32 = 0xF3F3F3; // window background (matches the mockup's light card)
const HAIRLINE: u32 = 0xD9D9D9;
const TEXT: u32 = 0x1F1F1F;
const MUTED_TEXT: u32 = 0x555555;
/// Tertiary text (the faint version line) — a step lighter than [`MUTED_TEXT`] (`ui_design.md §6`).
const FAINT_TEXT: u32 = 0x9A9A9A;
/// The one link/accent token (`ui_design.md §6`): our design system has no link color and
/// gpui-component's theme exposes none at the pinned rev, so a single blue constant serves every
/// link (not the mockup's exact hex, and not per-link colors).
const LINK: u32 = 0x2563EB;

/// The homepage repository — opened in the browser; shown as [`HOMEPAGE_LABEL`].
const HOMEPAGE_URL: &str = "https://github.com/scosman/freecell";
/// The homepage's shortened display label (`ui_design.md §6`).
const HOMEPAGE_LABEL: &str = "github.com/scosman/freecell";
/// The IronCalc engine's site (a "Built with" link).
const IRONCALC_URL: &str = "https://www.ironcalc.com";
/// The GPUI framework's site (a "Built with" link).
const GPUI_URL: &str = "https://gpui.rs";

/// The version line: the real build version, never the mockup's illustrative "1.0"
/// (`functional_spec.md §4.1`).
const VERSION_LINE: &str = concat!("Version ", env!("CARGO_PKG_VERSION"));

/// The About window's view — stateless static content (`architecture.md §9.1`).
pub struct AboutView {
    focus_handle: FocusHandle,
}

impl AboutView {
    /// Builds the About view, focused so its global-action key bindings (incl. `Cmd/Ctrl+W`)
    /// resolve on the window.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        window.set_window_title("About FreeCell");
        Self { focus_handle }
    }

    /// The homepage URL this window opens (tests).
    #[cfg(test)]
    pub(crate) fn homepage_url(&self) -> &'static str {
        HOMEPAGE_URL
    }

    /// The IronCalc "Built with" URL (tests).
    #[cfg(test)]
    pub(crate) fn ironcalc_url(&self) -> &'static str {
        IRONCALC_URL
    }

    /// The GPUI "Built with" URL (tests).
    #[cfg(test)]
    pub(crate) fn gpui_url(&self) -> &'static str {
        GPUI_URL
    }

    /// The build version shown on the About window — `env!("CARGO_PKG_VERSION")` (tests).
    #[cfg(test)]
    pub(crate) fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

impl Focusable for AboutView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AboutView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("about")
            .track_focus(&self.focus_handle)
            .key_context("About")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(CHROME_BG))
            // Cmd/Ctrl+W closes the About window; closing the last window quits the app via the
            // registry (`app.rs on_window_closed`).
            .on_action(cx.listener(|_this, _: &CloseWindow, window, _cx| window.remove_window()))
            // macOS custom titlebar (§7.1); omitted on Linux (server decorations). The About
            // window uses the BLANK row — no centered title, no bottom hairline — so the top bar
            // is a clean, seamless CHROME_BG band with only the traffic lights (`ui_design.md §6`).
            // The OS-level window name is still set via `set_window_title` (accessibility / Window
            // menu), just not drawn.
            .children(titlebar::MACOS_TITLEBAR.then(titlebar::titlebar_row_plain))
            .child(render_body())
    }
}

/// The window body: a top-packed identity block (wordmark / tagline / version), a hairline, then
/// the two label→value link rows (`ui_design.md §6`). Top-packed with a deliberate rhythm (not
/// vertically centered) — the window height (`app.rs about_window_options`) is tuned so the bottom
/// whitespace balances the top padding.
fn render_body() -> impl IntoElement {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .pt(px(28.0))
        .px(px(30.0))
        .pb(px(24.0))
        .gap(px(20.0))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.0))
                .child(
                    // The wordmark rides the Inter **Display ExtraBold** single-face family — a
                    // genuinely heavier & tighter cut than the RIBBI Bold, resolved by one family
                    // name on every platform. gpui at the pinned rev exposes no letter-spacing API,
                    // so the Display cut's built-in tight tracking stands in for the mockup's
                    // -0.03em (the exact tracking isn't settable in code). The weight is stated for
                    // intent; the lone face resolves regardless.
                    div()
                        .font_family(WORDMARK_FAMILY)
                        .font_weight(FontWeight::EXTRA_BOLD)
                        .text_size(px(30.0))
                        .text_color(rgb(TEXT))
                        .child("FreeCell"),
                )
                .child(
                    div()
                        .font_family(TAGLINE_FAMILY)
                        .text_size(px(14.0))
                        .text_color(rgb(MUTED_TEXT))
                        .child("The open spreadsheet"),
                )
                .child(
                    div()
                        .pt(px(4.0))
                        .text_size(px(12.0))
                        .text_color(rgb(FAINT_TEXT))
                        .child(VERSION_LINE),
                ),
        )
        .child(div().h(px(1.0)).w_full().bg(rgb(HAIRLINE)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(11.0))
                .child(info_row(
                    "Homepage",
                    link("about-homepage", HOMEPAGE_LABEL, HOMEPAGE_URL).into_any_element(),
                ))
                .child(info_row("Built with", built_with().into_any_element())),
        )
}

/// One label→value row: a muted label on the left, its value right-aligned (`ui_design.md §6`).
fn info_row(label: &'static str, value: gpui::AnyElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .justify_between()
        .gap(px(16.0))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(MUTED_TEXT))
                .child(label),
        )
        .child(value)
}

/// The "Built with" value — `IronCalc` · `GPUI`, each a link, with a `MUTED_TEXT` separator.
fn built_with() -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(px(6.0))
        .child(link("about-ironcalc", "IronCalc", IRONCALC_URL))
        .child(
            div()
                .text_size(px(13.0))
                .text_color(rgb(MUTED_TEXT))
                .child("·"),
        )
        .child(link("about-gpui", "GPUI", GPUI_URL))
}

/// A clickable text link: the Inter **SemiBold** single-face family, `LINK`-colored, pointer cursor
/// (no hover underline), opening `url` in the default browser on click via gpui's [`App::open_url`]
/// (`architecture.md §9.1`).
fn link(id: &'static str, label: &'static str, url: &'static str) -> impl IntoElement {
    div()
        .id(id)
        .font_family(LINK_FAMILY)
        .text_size(px(13.0))
        .text_color(rgb(LINK))
        .cursor_pointer()
        .on_click(move |_: &ClickEvent, _window, cx| cx.open_url(url))
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{size, TestAppContext};
    use gpui_component::Root;

    /// Builds an `AboutView` in a test window for the content assertions.
    fn about(cx: &mut TestAppContext) -> gpui::Entity<AboutView> {
        cx.update(gpui_component::init);
        let mut out = None;
        let slot = &mut out;
        cx.open_window(size(px(460.0), px(340.0)), |window, cx| {
            let view = cx.new(|cx| AboutView::new(window, cx));
            *slot = Some(view.clone());
            Root::new(view, window, cx)
        });
        out.expect("about view built")
    }

    #[gpui::test]
    fn about_view_exposes_link_urls_and_version(cx: &mut TestAppContext) {
        let view = about(cx);
        cx.update(|cx| {
            let about = view.read(cx);
            assert_eq!(about.homepage_url(), "https://github.com/scosman/freecell");
            assert_eq!(about.ironcalc_url(), "https://www.ironcalc.com");
            assert_eq!(about.gpui_url(), "https://gpui.rs");
            assert_eq!(about.version(), env!("CARGO_PKG_VERSION"));
        });
    }
}
