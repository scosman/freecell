//! The gpui side of the harness: open ONE window sized to a case's viewport with the **real**
//! [`GridView`] over engine-built sources, then self-quit. The capture harness
//! ([`crate::capture`]) forces presentation (`xrefresh`) and grabs the X root — the Phase-1
//! spike's proven Linux capture path (`DECISIONS_TO_REVIEW.md`, Phase 1).
//!
//! This mirrors `freecell-app/src/main.rs`: a gpui-component `Root` hosts the grid, and a real
//! executor timer quits after `exit_after_ms` (a paint-path deadline does NOT fire headless
//! under Xvfb — the window renders once).

use std::time::Duration;

use anyhow::{anyhow, Result};
use gpui::{px, size, App, AppContext as _, AsyncApp, Bounds, Point, WindowBounds, WindowOptions};
use gpui_component::input::InputState;
use gpui_component::Root;
use gpui_platform::application;

use freecell_app::grid::{GridEventSink, GridView};
use freecell_core::CellRef;

use crate::cases;
use crate::scene::build_sources;

/// Runs the gpui app for a single named case: builds its engine-backed sources, opens a
/// viewport-sized window with the real grid, and quits after `exit_after_ms`.
pub fn run_render_scene(case_name: &str, exit_after_ms: u64) -> Result<()> {
    let case = cases::all()
        .into_iter()
        .find(|c| c.name == case_name)
        .ok_or_else(|| anyhow!("unknown render case: {case_name}"))?;

    // Drive the real engine to produce the Publication + SheetCaches (blocking setup, before the
    // window opens).
    let sources = build_sources(&case.scene)?;
    let (w, h) = case.viewport;
    let selection = case.selection;
    let loading = case.loading.map(str::to_string);
    let force_scrollbars = case.force_scrollbars;
    let reveal = case.reveal;
    let mirror = case.mirror;
    let in_cell = case.in_cell;

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.activate(true);

        // A window at the screen origin sized exactly to the case viewport, so `import -window
        // root` cropped to `w×h+0+0` captures exactly the grid (no window manager under Xvfb, so
        // the window maps at its requested origin with no decorations).
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(w as f32), px(h as f32)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let grid = cx.new(|cx| {
                    let mut view = GridView::new(sources, GridEventSink::noop(), cx);
                    // Every render-test capture freezes animated overlays (the loading spinner)
                    // so the grabbed frame is deterministic regardless of wall-clock time.
                    view.set_freeze_spinner(true, cx);
                    if let Some(sel) = selection {
                        view.set_selection(sel, cx);
                    }
                    if let Some(name) = loading.clone() {
                        view.set_loading(Some(name), cx);
                    }
                    if force_scrollbars {
                        view.set_force_scrollbars(true, cx);
                    }
                    if let Some((row, col)) = reveal {
                        view.scroll_cell_into_view(row, col, cx);
                    }
                    // Editing-feel overlays (Phase 2): a live mirror and/or an open in-cell editor.
                    let sheet = view.active_sheet();
                    if let Some((row, col, text)) = mirror {
                        view.set_edit_state(
                            Some((sheet, CellRef::new(row, col), text.into())),
                            None,
                            None,
                            cx,
                        );
                    }
                    if let Some((row, col, text)) = in_cell {
                        let input = cx.new(|cx| {
                            let mut state = InputState::new(window, cx);
                            state.set_value(text, window, cx);
                            state
                        });
                        view.set_incell_input(input, cx);
                        view.set_edit_state(None, Some(CellRef::new(row, col)), None, cx);
                    }
                    view
                });
                // gpui-component requires the top-level window element to be a `Root`.
                cx.new(|cx| Root::new(grid, window, cx))
            },
        )
        .expect("failed to open render-test window");

        // Self-quit off a real executor timer (independent of rendering).
        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(exit_after_ms))
                .await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });

    Ok(())
}
