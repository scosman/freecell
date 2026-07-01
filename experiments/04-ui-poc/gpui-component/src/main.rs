//! gpui-component PoC — a spreadsheet grid on gpui-component's virtualized `DataTable`.
//!
//! The head-to-head counterpart to `raw-gpui/` (functional_spec §6.E, architecture §7),
//! sharing the same static provider and `poc_core` harness/reporting so the numbers are
//! directly comparable. Two purposes in one macOS/Metal app:
//! 1. **Interactive** — scroll/pan to judge feel.
//! 2. **"Run Test"** — a scripted scroll/jump sequence driven via
//!    `TableState::scroll_to_row` / `scroll_to_col`, measuring per-frame render time +
//!    newly-visible-cell load latency, printing PASS/FAIL vs §5.4 and writing
//!    `../results/gpui-component-runtest.json`. Menu item or `--run-test` flag.
//!
//! macOS/Metal only. Do NOT build on Linux (see `Cargo.toml`). Run via `../scripts/`.

mod table;

use std::time::Instant;

use gpui::{
    App, Context, Entity, IntoElement, ParentElement, Styled, Window, WindowOptions, actions, div,
    prelude::*,
};
use gpui_component::{
    Root,
    table::{DataTable, TableState},
};
use gpui_platform::application;

use datagen::CellSource;
use poc_core::{FrameSample, Harness, PocConfig, Viewport};
use table::SheetDelegate;

actions!(gpui_component_poc, [RunTest, Quit]);

fn results_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(|p| p.join("results"))
        .unwrap_or_else(|| std::path::PathBuf::from("results"))
}

fn commit() -> String {
    std::env::var("POC_COMMIT").unwrap_or_else(|_| "unknown".to_string())
}

fn report_date() -> String {
    std::env::var("POC_DATE").unwrap_or_else(|_| "unknown".to_string())
}

/// Drive state for the scripted "Run Test" run.
struct RunState {
    harness: Harness,
    prev_rows: std::ops::Range<u32>,
    prev_cols: std::ops::Range<u32>,
    finished: bool,
}

/// The root view: hosts the `DataTable` and, when running, drives + measures the harness.
struct SheetView {
    cfg: PocConfig,
    provider: datagen::SyntheticSheet,
    table: Entity<TableState<SheetDelegate>>,
    run: Option<RunState>,
    out_dir: std::path::PathBuf,
    date: String,
    commit: String,
}

impl SheetView {
    fn new(cfg: PocConfig, out_dir: std::path::PathBuf, date: String, commit: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let provider = datagen::SyntheticSheet::new(cfg.seed, cfg.rows, cfg.cols);
        let delegate = SheetDelegate::new(cfg);
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .col_resizable(true)
                .col_movable(true)
        });
        Self {
            cfg,
            provider,
            table,
            run: None,
            out_dir,
            date,
            commit,
        }
    }

    fn start_run_test(&mut self, cx: &mut Context<Self>) {
        self.run = Some(RunState {
            harness: Harness::scripted(&self.cfg),
            prev_rows: 0..0,
            prev_cols: 0..0,
            finished: false,
        });
        cx.notify();
    }

    /// Maps a scroll offset (px) to a row / col index using the provider's average sizes
    /// — the component scrolls by index (`scroll_to_row`/`scroll_to_col`), not by pixel,
    /// so the scripted pixel viewport is converted to the nearest index.
    fn viewport_to_indices(&self, vp: Viewport) -> (usize, usize) {
        let row = (vp.scroll_y / table::UNIFORM_ROW_HEIGHT as f64) as usize;
        // Columns are variable-width; approximate via the average column width so a
        // horizontal jump lands in the right neighbourhood (exact index isn't required
        // to exercise the load/render path).
        let avg_col_w = 110.0_f64;
        let col = (vp.scroll_x / avg_col_w) as usize;
        (
            row.min(self.cfg.rows.saturating_sub(1) as usize),
            col.min(self.cfg.cols.saturating_sub(1) as usize),
        )
    }

    fn finish_run_test(&mut self, cx: &mut Context<Self>) {
        let samples: Vec<FrameSample> = self
            .run
            .as_ref()
            .map(|r| r.harness.samples().to_vec())
            .unwrap_or_default();
        match poc_core::finalize(
            "gpui-component",
            &self.date,
            &self.commit,
            &samples,
            &self.out_dir,
        ) {
            Ok((report, path)) => {
                println!("{}", report.summary());
                println!("results written to {}", path.display());
            }
            Err(e) => eprintln!("failed to write results: {e}"),
        }
        self.run = None;
        cx.quit();
    }
}

impl Render for SheetView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Advance + measure the scripted run, if active.
        if let Some(run) = self.run.as_mut() {
            if run.finished {
                self.finish_run_test(cx);
            } else {
                match run.harness.next_viewport() {
                    Some(vp) => {
                        let (row, col) = self.viewport_to_indices(vp);
                        // The visible span is viewport-sized; approximate for the
                        // newly-visible count and for timing the provider pulls.
                        let vis_rows = (self.cfg.content_height()
                            / table::UNIFORM_ROW_HEIGHT)
                            .ceil() as u32
                            + self.cfg.overscan_rows;
                        let vis_cols = (self.cfg.content_width() / 110.0).ceil() as u32
                            + self.cfg.overscan_cols;
                        let cur_rows = row as u32..(row as u32 + vis_rows).min(self.cfg.rows);
                        let cur_cols = col as u32..(col as u32 + vis_cols).min(self.cfg.cols);

                        // Time the newly-visible provider pulls (the §5.4 cell-load
                        // budget), mirroring the raw-gpui measurement.
                        let load_start = Instant::now();
                        let mut newly_visible = 0u32;
                        for r in cur_rows.clone() {
                            let row_new = !run.prev_rows.contains(&r);
                            for c in cur_cols.clone() {
                                if row_new || !run.prev_cols.contains(&c) {
                                    newly_visible += 1;
                                    let _ = self.provider.cell(r, c);
                                }
                            }
                        }
                        let cell_load_ns = load_start.elapsed().as_nanos() as u64;

                        // Programmatically scroll the component, then measure the render
                        // by timing the state update it triggers.
                        let render_start = Instant::now();
                        self.table.update(cx, |state, cx| {
                            state.scroll_to_row(row, cx);
                            state.scroll_to_col(col, cx);
                        });
                        let frame_render_ns = render_start.elapsed().as_nanos() as u64;

                        run.harness.record(FrameSample {
                            frame_render_ns,
                            cell_load_ns,
                            newly_visible,
                        });
                        run.prev_rows = cur_rows;
                        run.prev_cols = cur_cols;
                        window.request_animation_frame();
                    }
                    None => {
                        // Mark finished; next frame writes results and quits.
                        run.finished = true;
                        window.request_animation_frame();
                    }
                }
            }
        }

        div()
            .size_full()
            .bg(rgb_white())
            .child(DataTable::new(&self.table).bordered(true))
    }
}

fn rgb_white() -> gpui::Rgba {
    gpui::rgb(0xFFFFFF)
}

fn main() {
    let auto_run = std::env::args().any(|a| a == "--run-test");

    let app = application().with_assets(gpui_component_assets::Assets);
    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        cx.activate(true);

        cx.set_menus([gpui::Menu::new("gpui-component-poc").items([
            gpui::MenuItem::action("Run Test", RunTest),
            gpui::MenuItem::separator(),
            gpui::MenuItem::action("Quit", Quit),
        ])]);
        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());

        let cfg = PocConfig::default();
        let out_dir = results_dir();
        let date = report_date();
        let commit = commit();

        // Build the view inside the window (it needs a Window to create the TableState),
        // then wrap it in gpui-component's required `Root` element.
        let view_cell = std::rc::Rc::new(std::cell::RefCell::new(None::<Entity<SheetView>>));
        let view_cell_for_window = view_cell.clone();
        cx.open_window(WindowOptions::default(), move |window, cx| {
            let view = cx.new(|cx| {
                SheetView::new(cfg, out_dir.clone(), date.clone(), commit.clone(), window, cx)
            });
            *view_cell_for_window.borrow_mut() = Some(view.clone());
            // gpui-component requires the top-level window element to be a `Root`
            // (see gpui-component/examples/*). `Root::new` takes `impl Into<AnyView>`,
            // so the view entity is passed directly.
            cx.new(|cx| Root::new(view, window, cx))
        })
        .expect("failed to open window");

        // Route the "Run Test" action + `--run-test` flag into the SheetView.
        if let Some(view) = view_cell.borrow().clone() {
            let action_view = view.clone();
            cx.on_action(move |_: &RunTest, cx: &mut App| {
                action_view.update(cx, |v, cx| v.start_run_test(cx));
            });
            if auto_run {
                cx.defer(move |cx| {
                    view.update(cx, |v, cx| v.start_run_test(cx));
                });
            }
        }
    });
}
