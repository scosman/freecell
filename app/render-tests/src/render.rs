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
use gpui::{
    div, prelude::*, px, rgb, size, App, AsyncApp, Bounds, Context, Entity, Point, SharedString,
    Window, WindowBounds, WindowOptions,
};
use gpui_component::input::InputState;
use gpui_component::Root;
use gpui_platform::application;

use freecell_app::grid::{GridEventSink, GridView};
use freecell_app::shell::titlebar::titlebar_row;
use freecell_chart_model::Chart;
use freecell_core::CellRef;

use crate::cases;
use crate::chart_scene;
use crate::scene::build_sources;

/// The `titlebar_row` case's root: the macOS custom titlebar row (`architecture.md §7.1`) drawn
/// over the real grid (a flex column), so the harness pixel-checks the row's own look on Linux.
/// The row is built unconditionally (it is just a div — the master `MACOS_TITLEBAR` switch gates
/// only the *real* windows, not this render fixture).
struct TitlebarScene {
    title: SharedString,
    grid: Entity<GridView>,
}

impl Render for TitlebarScene {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .size_full()
            .child(titlebar_row(self.title.clone()))
            .child(self.grid.clone())
    }
}

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
    let titlebar = case.titlebar;
    let charts = case.charts;
    let selected_chart = case.selected_chart;
    let auto_grow = case.auto_grow;

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        // Register the bundled Inter faces + set Inter as the UI font, so render-test captures
        // use the same font the app does (matches main.rs).
        freecell_app::shell::register_fonts(cx);
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
                    let sheet = view.active_sheet();
                    // In-grid charts (P8): install the case's ChartLayer on the active sheet, so the
                    // grid paints them over the cells at each chart's anchor rect.
                    if !charts.is_empty() {
                        view.set_sheet_charts(sheet, std::sync::Arc::from(charts), cx);
                    }
                    // A selected chart (P18) draws the selection outline + resize handles.
                    if let Some(id) = selected_chart {
                        view.set_selected_chart(Some(id), cx);
                    }
                    // Editing-feel overlays (Phase 2): a live mirror and/or an open in-cell editor.
                    if let Some((row, col, text)) = mirror {
                        view.set_edit_state(
                            Some((sheet, CellRef::new(row, col), text.into())),
                            None,
                            None,
                            false,
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
                        view.set_edit_state(None, Some(CellRef::new(row, col)), None, false, cx);
                    }
                    // Wrap-driven auto-grow (`functional_spec.md §3`): run the real render-thread
                    // measurement once, up front, so the captured frame shows the grown row heights.
                    // The live measure→worker→republish loop can't complete in-capture (single static
                    // frame, shut-down worker), so this test hook applies the measured heights to the
                    // shared cache directly (skipping rows with an existing override = manual).
                    if auto_grow {
                        view.autogrow_measure_now(window, cx);
                    }
                    view
                });
                // gpui-component requires the top-level window element to be a `Root`. A
                // `titlebar_row` case wraps the grid under the macOS custom titlebar row (§7.1);
                // all other cases mount the bare grid (unchanged — no baseline perturbation).
                let root_view: gpui::AnyView = match titlebar {
                    Some(title) => cx
                        .new(|_| TitlebarScene {
                            title: title.into(),
                            grid: grid.clone(),
                        })
                        .into(),
                    None => grid.into(),
                };
                cx.new(|cx| Root::new(root_view, window, cx))
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

/// A gpui view that renders one chart full-window, through the **real**
/// [`freecell_app::chart::chart_element`] widgets (the P1-lifted render layer). gpui-component
/// requires the top-level window element to be a `Root`, which wraps this view. `chart_element`
/// returns `None` only for a kind no widget handles; the fallback is then a uniform white frame,
/// which the capture harness's blank-guard ([`crate::capture`], `colors <= 1`) rejects as a loud
/// "blank capture" error — the right outcome for a misconfigured scene (fail loudly, never a
/// silent green). Every seeded scene renders, so the fallback never fires in practice.
struct ChartSceneView {
    chart: Chart,
}

impl Render for ChartSceneView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        freecell_app::chart::chart_element(&self.chart)
            .unwrap_or_else(|| div().size_full().bg(rgb(0xFFFFFF)).into_any_element())
    }
}

/// Runs the gpui app for a single named chart scene: opens a viewport-sized window hosting the
/// chart widget (standalone — no grid, `functional_spec.md §4.2` in-grid `ChartLayer` is P8) and
/// quits after `exit_after_ms`. The capture harness ([`crate::capture::render_charts`]) forces
/// presentation (`xrefresh`) and grabs the window — the same Linux path the grid uses.
pub fn run_chart_scene(scene_name: &str, exit_after_ms: u64) -> Result<()> {
    let scene =
        chart_scene::get(scene_name).ok_or_else(|| anyhow!("unknown chart scene: {scene_name}"))?;
    let (w, h) = scene.viewport;
    let chart = scene.chart;

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        // Register the bundled Inter faces (as the grid path + the app do) so chart text — title,
        // axis labels, legend — is font-stable across platforms/CI, matching the committed baseline.
        freecell_app::shell::register_fonts(cx);
        cx.activate(true);

        // A window at the screen origin sized exactly to the scene viewport, so the capture (which
        // finds the window by its size, no window manager under Xvfb) grabs exactly the chart.
        let bounds = Bounds {
            origin: Point::default(),
            size: size(px(w as f32), px(h as f32)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            move |window, cx| {
                let view = cx.new(|_| ChartSceneView { chart });
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open chart render-test window");

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
