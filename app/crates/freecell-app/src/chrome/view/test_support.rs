//! Shared `#[cfg(test)]` support for the `chrome::view` module tree: the window harness the
//! view tests build against, the body stubs they mount, and the `ChromeView` test seams that
//! stand in for widget events.
//!
//! Moved verbatim out of the single-file `chrome/view.rs`
//! (`specs/projects/chrome-view-split`). The whole module is `#[cfg(test)]`, so the items
//! below carry no inner `#[cfg(test)]` of their own, and those a sibling's `mod tests` reaches
//! are `pub(super)` — scoped to the `view` subtree, never wider.
//!
//! Two different starting points, worth keeping straight: the 15 `ChromeView` seams were
//! private methods in a module-level `impl ChromeView` block of `view.rs`, and method privacy
//! is module-scoped — so their reach was `chrome::view` then, and `pub(super)` restores
//! *exactly* that now. The harness and its fixtures were private to `view::tests`, which is
//! narrower — they genuinely widen from that one test module to the `view` subtree, the cost of
//! sharing them across per-domain test modules (`architecture.md §3.4`). Anything used only
//! inside this file stays private.

use super::*;

use std::cell::RefCell;

use gpui::{size, TestAppContext};
use gpui_component::Root;

use crate::chrome::RecordingClient;

impl ChromeView {
    /// Test seam: simulate the user typing `text` into the content field (sets the widget
    /// text, then delivers the `Change` event the subscription would).
    pub(super) fn test_type(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.content_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        let handle = self.content_input.clone();
        self.on_content_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the content field.
    pub(super) fn test_press_enter(
        &mut self,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.content_input.clone();
        self.on_content_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: set the rename input's text.
    pub(super) fn test_rename_type(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rename_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Test seam: simulate typing `text` into the find field (sets the widget text, then delivers
    /// the `Change` event the subscription would).
    pub(super) fn test_find_type(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.find_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        let handle = self.find_input.clone();
        self.on_find_input_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: set the replace field's text (no event needed — replace reads it on demand).
    pub(super) fn test_replace_type(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.replace_input
            .update(cx, |input, cx| input.set_value(text, window, cx));
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the find field.
    pub(super) fn test_find_press_enter(
        &mut self,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.find_input.clone();
        self.on_find_input_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: the find field's current text.
    pub(super) fn find_field_text(&self, cx: &App) -> String {
        self.find_input.read(cx).value().to_string()
    }

    /// Test seam: the find field's current selection range (for the select-on-open check).
    pub(super) fn find_selection(&self, cx: &App) -> std::ops::Range<usize> {
        self.find_input.read(cx).selected_range()
    }

    /// Test seam: simulate typing `text` into the in-cell editor (sets the widget text, then
    /// delivers the `Change` event the subscription would).
    pub(super) fn test_incell_type(
        &mut self,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.edit.in_cell().clone();
        handle.update(cx, |input, cx| input.set_value(text, window, cx));
        self.on_incell_event(&handle, &InputEvent::Change, window, cx);
    }

    /// Test seam: simulate pressing Enter (optionally with Shift) in the in-cell editor.
    pub(super) fn test_incell_press_enter(
        &mut self,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let handle = self.edit.in_cell().clone();
        self.on_incell_event(
            &handle,
            &InputEvent::PressEnter {
                secondary: false,
                shift,
            },
            window,
            cx,
        );
    }

    /// Test seam: replicate the data-row Tab handler (commit + move right/left) without the
    /// widget-level `capture_key_down`.
    pub(super) fn test_data_row_tab(
        &mut self,
        shift: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.data_mode() == FieldMode::Editing {
            let dir = if shift {
                Direction::Left
            } else {
                Direction::Right
            };
            self.commit_and_move(dir, window, cx);
        }
    }

    /// Test seam: the in-cell editor's current text.
    pub(super) fn incell_text(&self, cx: &App) -> String {
        self.edit.in_cell().read(cx).value().to_string()
    }

    /// Test seam: the open in-cell overlay cell, if any.
    pub(super) fn incell_open(&self) -> Option<CellRef> {
        self.edit.open_cell()
    }

    /// Test seam: which editor currently drives the edit.
    pub(super) fn edit_origin(&self) -> EditOrigin {
        self.edit.origin()
    }

    /// Test seam: the captured chrome-local left-x of a dropdown trigger (BUG 2c anchoring).
    pub(super) fn anchor_x_of(&self, which: Anchor) -> f32 {
        self.anchor_x[which.idx()]
    }
}

/// A window hosting a `ChromeView` over a `RecordingClient`, plus a recording grid sink.
pub(super) struct Harness {
    pub(super) chrome: Entity<ChromeView>,
    pub(super) client: Rc<RecordingClient>,
    pub(super) grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>>,
    pub(super) window: gpui::WindowHandle<Root>,
}

pub(super) fn cell(row: u32, col: u32) -> CellRef {
    CellRef::new(row, col)
}

pub(super) fn build(cx: &mut TestAppContext, sheets: Vec<SheetTab>, active: SheetId) -> Harness {
    build_win(cx, sheets, active, 200.0)
}

/// [`build`] with a caller-chosen window height — the popover-click tests want a tall enough
/// window that every dropdown item lays out on-screen and can be hit by a simulated click. The
/// window matches the real document width (1200 px), wider than the action row's ~1152 px
/// natural width, so the row fits (no scroller chevrons) and its triggers lay out on-screen.
pub(super) fn build_win(
    cx: &mut TestAppContext,
    sheets: Vec<SheetTab>,
    active: SheetId,
    height: f32,
) -> Harness {
    build_sized(cx, sheets, active, 1200.0, height)
}

/// [`build_win`] with a caller-chosen window **width** too — the horizontal-scroller tests open
/// a narrow window so the action row / tab strip overflow and show chevrons.
pub(super) fn build_sized(
    cx: &mut TestAppContext,
    sheets: Vec<SheetTab>,
    active: SheetId,
    width: f32,
    height: f32,
) -> Harness {
    let client = Rc::new(RecordingClient::new());
    let grid_requests: Rc<RefCell<Vec<ChromeGridRequest>>> = Rc::new(RefCell::new(Vec::new()));

    cx.update(gpui_component::init);

    let client_for_window = client.clone();
    let reqs_for_window = grid_requests.clone();
    let mut chrome_out: Option<Entity<ChromeView>> = None;
    let chrome_slot = &mut chrome_out;

    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        let client_dyn: Rc<dyn ChromeClient> = client_for_window;
        let reqs = reqs_for_window;
        let sink = ChromeGridSink::new(move |req, _w, _cx| reqs.borrow_mut().push(req.clone()));
        let chrome = cx.new(|cx| ChromeView::new(client_dyn, sink, active, sheets, window, cx));
        *chrome_slot = Some(chrome.clone());
        Root::new(chrome, window, cx)
    });

    Harness {
        chrome: chrome_out.expect("chrome built"),
        client,
        grid_requests,
        window,
    }
}

pub(super) fn one_sheet(cx: &mut TestAppContext) -> Harness {
    build(cx, vec![SheetTab::new(SheetId(0), "Sheet1")], SheetId(0))
}

/// A stand-in for the hosted grid: an empty full-size body. Its only job is to make the chrome
/// **fill the window** (`render` flexes only when a body is present), so a popover's full-window
/// backdrop really spans the window height — the condition under which BUG A/B bites. With a
/// bodyless chrome the backdrop is only ~3 rows tall and the dropdown items lay out *below* it,
/// never overlapping it, so the regression would hide.
struct BodyStub;
impl gpui::Render for BodyStub {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // A concrete-height body so the chrome content — and thus the popover backdrop, which is
        // `size_full` of the chrome — spans well past the dropdown items. (`flex_1` alone won't
        // stretch it: the test Root sizes the chrome to its content, not the window.)
        div().h(px(500.0)).w_full()
    }
}

/// A short stand-in grid body, so the chrome (and thus the absolutely-positioned chart panel,
/// sized between the data row and the tab bar) is **height-constrained** — the condition under
/// which the panel's control stack overflows and must scroll + clip (item 7).
pub(super) struct ShortBodyStub;
impl gpui::Render for ShortBodyStub {
    fn render(&mut self, _w: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().h(px(40.0)).w_full()
    }
}

/// One sheet in a tall window with a (stub) grid body, for the popover-click tests: every item
/// lays out on-screen over a full-height backdrop.
pub(super) fn tall_sheet(cx: &mut TestAppContext) -> Harness {
    let h = build_win(
        cx,
        vec![SheetTab::new(SheetId(0), "Sheet1")],
        SheetId(0),
        600.0,
    );
    upd(&h, cx, |c, _w, cx| {
        let body: gpui::AnyView = cx.new(|_| BodyStub).into();
        c.set_grid_body(body, cx);
    });
    h
}

/// Runs `f` against the chrome with a live `Window`.
pub(super) fn upd<R>(
    h: &Harness,
    cx: &mut TestAppContext,
    f: impl FnOnce(&mut ChromeView, &mut Window, &mut Context<ChromeView>) -> R,
) -> R {
    h.window
        .update(cx, |_root, window, cx| {
            h.chrome.update(cx, |c, cx| f(c, window, cx))
        })
        .unwrap()
}

pub(super) fn tick(cx: &mut TestAppContext, ms: u64) {
    cx.executor().advance_clock(Duration::from_millis(ms));
    cx.run_until_parked();
}

/// A ready-made numeric aggregate for the reply-plumbing tests.
pub(super) fn numeric_stats() -> SelectionStats {
    SelectionStats {
        count: 5,
        numeric_count: 2,
        sum: 30.0,
        min: Some(10.0),
        max: Some(20.0),
    }
}

/// A1:A3 (a 3-cell column selection).
pub(super) fn multi_a1_a3() -> SelectionModel {
    SelectionModel {
        anchor: cell(0, 0),
        active: cell(2, 0),
    }
}

pub(super) fn two_sheets(cx: &mut TestAppContext) -> Harness {
    build(
        cx,
        vec![
            SheetTab::new(SheetId(0), "Sheet1"),
            SheetTab::new(SheetId(1), "Sheet2"),
        ],
        SheetId(0),
    )
}

/// A published rule row with a chosen storage `index`, `summary`, editability, and preview.
/// `priority` is irrelevant to rendering (the client publishes the list already
/// priority-sorted; the row order is the vec order), so it is fixed at 0.
pub(super) fn cf_view(
    index: u32,
    range: &str,
    summary: &str,
    editable: bool,
    preview: CfPreview,
) -> CfRuleView {
    CfRuleView {
        index,
        range: range.to_string(),
        priority: 0,
        editable,
        summary: summary.to_string(),
        preview,
        spec: None,
    }
}
