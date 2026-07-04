//! [`WelcomeView`] — the small fixed launch window (`functional_spec.md §2.2`).
//!
//! App name + **New Spreadsheet** + **Open…**. It registers **no document actions**, so
//! Save / Undo / etc. are disabled while it is frontmost (`components/app_shell.md §Menus &
//! actions`). It can also host an app-level error / About dialog when no document window is
//! around to own it.

use gpui::{div, prelude::*, px, rgb, App, ClickEvent, Context, FocusHandle, Focusable, Window};
use gpui_component::button::{Button, ButtonVariants as _};

use super::{CloseWindow, FreeCellApp};

const BG: u32 = 0xF7F7F7;
const TEXT: u32 = 0x1F1F1F;
const MUTED_TEXT: u32 = 0x555555;
const CARD_BG: u32 = 0xFFFFFF;
const HAIRLINE: u32 = 0xD9D9D9;

/// A dialog the welcome window can host when there's no document window to own it.
#[derive(Debug, Clone)]
enum WelcomeModal {
    Error { title: String, detail: String },
    About,
}

/// The launch window.
pub struct WelcomeView {
    focus_handle: FocusHandle,
    modal: Option<WelcomeModal>,
}

impl WelcomeView {
    /// Builds the welcome view, focused so its global-action key bindings resolve.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        window.set_window_title("FreeCell");
        Self {
            focus_handle,
            modal: None,
        }
    }

    /// Shows an app-level error dialog on the welcome window.
    pub fn show_error(
        &mut self,
        title: impl Into<String>,
        detail: impl Into<String>,
        cx: &mut Context<Self>,
    ) {
        self.modal = Some(WelcomeModal::Error {
            title: title.into(),
            detail: detail.into(),
        });
        cx.notify();
    }

    /// Shows the About dialog on the welcome window.
    pub fn show_about(&mut self, cx: &mut Context<Self>) {
        self.modal = Some(WelcomeModal::About);
        cx.notify();
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.modal = None;
        cx.notify();
    }

    /// Whether a dialog is currently showing (tests).
    pub fn has_modal(&self) -> bool {
        self.modal.is_some()
    }
}

impl Focusable for WelcomeView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WelcomeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("welcome")
            .track_focus(&self.focus_handle)
            .key_context("Welcome")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(rgb(BG))
            // Cmd/Ctrl+W closes the welcome window (matches the platform convention). Closing
            // the last window quits the app via the registry (`app.rs on_window_closed`).
            .on_action(cx.listener(|_this, _: &CloseWindow, window, _cx| window.remove_window()))
            .child(
                div()
                    .text_size(px(28.0))
                    .font_weight(gpui::FontWeight::BOLD)
                    .text_color(rgb(TEXT))
                    .child("FreeCell"),
            )
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(MUTED_TEXT))
                    .child("A fast, Excel-compatible spreadsheet."),
            )
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(
                        Button::new("new-spreadsheet")
                            .label("New Spreadsheet")
                            .primary()
                            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                                FreeCellApp::new_workbook(cx);
                            })),
                    )
                    .child(
                        Button::new("open-file")
                            .label("Open…")
                            .on_click(cx.listener(|_this, _: &ClickEvent, _window, cx| {
                                FreeCellApp::open_via_panel(cx);
                            })),
                    ),
            )
            .children(self.render_modal(cx))
    }
}

impl WelcomeView {
    fn render_modal(&self, cx: &mut Context<Self>) -> Option<gpui::AnyElement> {
        let modal = self.modal.as_ref()?;
        let (title, body) = match modal {
            WelcomeModal::Error { title, detail } => (title.clone(), detail.clone()),
            WelcomeModal::About => (
                "FreeCell".to_string(),
                "A GPU-rendered, Excel-compatible spreadsheet.\nMVP proof of concept.".to_string(),
            ),
        };
        Some(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(rgb(0x000000).opacity(0.3))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_3()
                        .p_4()
                        .w(px(320.0))
                        .bg(rgb(CARD_BG))
                        .border_1()
                        .border_color(rgb(HAIRLINE))
                        .rounded_lg()
                        .shadow_lg()
                        .child(
                            div()
                                .font_weight(gpui::FontWeight::BOLD)
                                .text_color(rgb(TEXT))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(13.0))
                                .text_color(rgb(MUTED_TEXT))
                                .child(body),
                        )
                        .child(div().flex().justify_end().child(
                            Button::new("welcome-ok").label("OK").primary().on_click(
                                cx.listener(|this, _: &ClickEvent, _window, cx| this.dismiss(cx)),
                            ),
                        )),
                )
                .into_any_element(),
        )
    }
}
