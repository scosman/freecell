//! Standalone chrome demo (Phase 9 spot-check capture).
//!
//! Mounts a [`ChromeView`] over a [`RecordingClient`] double — no engine — so the action
//! row, data row, and sheet tab bar can be eyeballed / captured under Xvfb+lavapipe the same
//! way the grid render spike is. Not shipped; a developer aid. Quits after a timer (the
//! render-spike safety valve — headless has no compositor, so a paint deadline never fires).

use std::rc::Rc;
use std::time::Duration;

use gpui::{prelude::*, px, size, App, AsyncApp, Bounds, WindowBounds, WindowOptions};
use gpui_component::Root;
use gpui_platform::application;

use freecell_app::chrome::{ChromeClient, ChromeGridSink, ChromeView, RecordingClient, SheetTab};
use freecell_core::{CellRef, RenderStyle, SelectionModel, SheetId};
use freecell_engine::WorkerEvent;

fn main() {
    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.activate(true);

        let bounds = Bounds {
            origin: Default::default(),
            size: size(px(1000.0), px(120.0)),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                ..Default::default()
            },
            |window, cx| {
                let chrome = cx.new(|cx| {
                    // A double populated so the surfaces show real content: B7 is bold
                    // (the B toggle lights), holds a formula, on three sheets.
                    let recording = RecordingClient::new();
                    recording.set_style(
                        SheetId(0),
                        CellRef::new(6, 1),
                        RenderStyle {
                            bold: true,
                            ..Default::default()
                        },
                    );
                    let client: Rc<dyn ChromeClient> = Rc::new(recording);
                    let sheets = vec![
                        SheetTab::new(SheetId(0), "Sheet1"),
                        SheetTab::new(SheetId(1), "Sales").with_content(true),
                        SheetTab::new(SheetId(2), "Q1 Budget").with_content(true),
                    ];
                    let mut view = ChromeView::new(
                        client,
                        ChromeGridSink::noop(),
                        SheetId(0),
                        sheets,
                        window,
                        cx,
                    );
                    view.on_selection_changed(
                        SelectionModel::single(CellRef::new(6, 1)),
                        window,
                        cx,
                    );
                    view.on_worker_event(
                        WorkerEvent::CellContent {
                            req_id: 1,
                            raw: "=SUM(A1:A5)".into(),
                        },
                        window,
                        cx,
                    );
                    view
                });
                cx.new(|cx| Root::new(chrome, window, cx))
            },
        )
        .expect("failed to open chrome demo window");

        cx.spawn(async move |cx: &mut AsyncApp| {
            cx.background_executor()
                .timer(Duration::from_millis(6000))
                .await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}
