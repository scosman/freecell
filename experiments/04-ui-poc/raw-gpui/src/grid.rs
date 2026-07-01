//! The custom virtualized spreadsheet grid on raw gpui primitives.
//!
//! Virtualization is entirely ours: [`poc_core::Axis`] maps the scroll offset to the
//! visible row/col range (variable sizes, segment-summed prefix sums + binary search),
//! and each visible cell is an **absolutely-positioned** `div` at
//! `Axis::offset_of(index)`. Only viewport + overscan cells exist as elements, so the
//! Excel-max grid (1M × 16K) costs a viewport's worth of elements per frame.
//!
//! Two drive modes share the same `render`:
//! - **Interactive:** `on_scroll_wheel` mutates `scroll` and `cx.notify()`s.
//! - **Run Test:** a [`poc_core::Harness`] advances the viewport one frame at a time; we
//!   time our own render + newly-visible provider pulls, record a [`FrameSample`], and
//!   request the next frame until the script ends, then finalize and quit.
//!
//! macOS/Metal only — see the crate note in `Cargo.toml`.

use std::time::Instant;

use gpui::{
    Context, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels, ScrollWheelEvent,
    Styled, Window, div, px, rgb,
};

use datagen::{CellSource, SyntheticSheet, column_label};
use poc_core::{
    Align, Axis, FrameSample, Harness, PocConfig, RenderCell, Viewport,
    style::{GRIDLINE_GREY, HEADER_BG, HEADER_TEXT},
};

/// How the grid is currently being driven.
enum Mode {
    /// The human scrolls interactively; no measurement.
    Interactive,
    /// The scripted "Run Test" harness is running; each frame is measured.
    RunTest {
        harness: Harness,
        /// Visible ranges from the previous frame, to compute newly-visible cells.
        prev_rows: std::ops::Range<u32>,
        prev_cols: std::ops::Range<u32>,
    },
}

/// The grid view/entity.
pub struct Grid {
    cfg: PocConfig,
    provider: SyntheticSheet,
    row_axis: Axis,
    col_axis: Axis,
    /// Current scroll offset of the content area, in px.
    scroll_x: f64,
    scroll_y: f64,
    mode: Mode,
    /// Where results JSON is written and the report date/commit stamped in.
    out_dir: std::path::PathBuf,
    date: String,
    commit: String,
    /// Set once the Run Test finishes, so `render` can print + quit exactly once.
    finished: bool,
}

impl Grid {
    /// Builds the grid over a fresh synthetic provider sized by `cfg`.
    pub fn new(cfg: PocConfig, out_dir: std::path::PathBuf, date: String, commit: String) -> Self {
        let provider = SyntheticSheet::new(cfg.seed, cfg.rows, cfg.cols);
        let row_axis = Axis::new(cfg.rows, move |r| provider.row_height(r));
        let col_axis = Axis::new(cfg.cols, move |c| provider.col_width(c));
        Self {
            cfg,
            provider,
            row_axis,
            col_axis,
            scroll_x: 0.0,
            scroll_y: 0.0,
            mode: Mode::Interactive,
            out_dir,
            date,
            commit,
            finished: false,
        }
    }

    /// Switches the grid into scripted "Run Test" mode (from the menu action or the
    /// `--run-test` CLI flag). The next frames advance the harness and measure.
    pub fn start_run_test(&mut self, cx: &mut Context<Self>) {
        let harness = Harness::scripted(&self.cfg);
        self.mode = Mode::RunTest {
            harness,
            prev_rows: 0..0,
            prev_cols: 0..0,
        };
        self.finished = false;
        cx.notify();
    }

    /// Clamps a scroll offset to the valid range for its axis.
    fn clamp_scroll(&self, x: f64, y: f64) -> (f64, f64) {
        let max_x = (self.col_axis.total() - self.cfg.content_width() as f64).max(0.0);
        let max_y = (self.row_axis.total() - self.cfg.content_height() as f64).max(0.0);
        (x.clamp(0.0, max_x), y.clamp(0.0, max_y))
    }

    /// Renders one header cell (column letter or row number).
    fn header_cell(text: String, w: Pixels, h: Pixels, left: Pixels, top: Pixels) -> impl IntoElement {
        div()
            .absolute()
            .left(left)
            .top(top)
            .w(w)
            .h(h)
            .flex()
            .items_center()
            .justify_center()
            .bg(rgb(HEADER_BG))
            .border_1()
            .border_color(rgb(GRIDLINE_GREY))
            .text_color(rgb(HEADER_TEXT))
            .text_xs()
            .child(text)
    }

    /// Renders one data cell from a [`RenderCell`] at an absolute position.
    fn data_cell(rc: RenderCell, w: Pixels, h: Pixels, left: Pixels, top: Pixels) -> impl IntoElement {
        let mut cell = div()
            .absolute()
            .left(left)
            .top(top)
            .w(w)
            .h(h)
            .px_1()
            .flex()
            .items_center()
            .bg(rgb(rc.fill))
            .border_1()
            .border_color(rgb(GRIDLINE_GREY))
            .text_color(rgb(rc.text_color))
            .text_sm()
            .overflow_hidden()
            .whitespace_nowrap();

        cell = match rc.align {
            Align::Left => cell.justify_start(),
            Align::Center => cell.justify_center(),
            Align::Right => cell.justify_end(),
        };
        if rc.bold {
            cell = cell.font_weight(FontWeight::BOLD);
        }
        if rc.italic {
            cell = cell.italic();
        }
        cell.child(rc.text)
    }
}

impl gpui::Render for Grid {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // If the harness is active, advance to this frame's scripted viewport BEFORE we
        // compute what's visible, so the measured render reflects the new position.
        let running = matches!(self.mode, Mode::RunTest { .. });
        if running {
            if let Mode::RunTest { harness, .. } = &mut self.mode {
                match harness.next_viewport() {
                    Some(Viewport { scroll_x, scroll_y }) => {
                        let (x, y) = self.clamp_scroll(scroll_x, scroll_y);
                        self.scroll_x = x;
                        self.scroll_y = y;
                    }
                    None => {
                        // Script exhausted: finalize once.
                        self.finish_run_test(cx);
                        // Fall through to render the final frame.
                    }
                }
            }
        }

        // Use the actual window viewport if available; else the configured size.
        let vp = window.viewport_size();
        let content_w = (f32::from(vp.width) - self.cfg.row_header_width).max(0.0) as f64;
        let content_h = (f32::from(vp.height) - self.cfg.col_header_height).max(0.0) as f64;

        let render_start = Instant::now();

        let rows = self
            .row_axis
            .visible_range(self.scroll_y, content_h, self.cfg.overscan_rows);
        let cols = self
            .col_axis
            .visible_range(self.scroll_x, content_w, self.cfg.overscan_cols);

        // --- measure the newly-visible-cell load (the §5.4 cell-load budget) ---------
        // Time exactly the provider pulls for cells that entered the viewport this frame,
        // building the RenderCells we then draw. On the first/interactive frame prev is
        // empty, so everything is "new".
        let (prev_rows, prev_cols) = match &self.mode {
            Mode::RunTest {
                prev_rows,
                prev_cols,
                ..
            } => (prev_rows.clone(), prev_cols.clone()),
            Mode::Interactive => (0..0, 0..0),
        };

        let load_start = Instant::now();
        let mut cells: Vec<(u32, u32, RenderCell)> =
            Vec::with_capacity(((rows.end - rows.start) * (cols.end - cols.start)) as usize);
        let mut newly_visible: u32 = 0;
        for r in rows.clone() {
            let row_new = !prev_rows.contains(&r);
            for c in cols.clone() {
                let is_new = row_new || !prev_cols.contains(&c);
                if is_new {
                    newly_visible += 1;
                }
                let data = self.provider.cell(r, c);
                cells.push((r, c, RenderCell::from_cell(&data)));
            }
        }
        let cell_load_ns = load_start.elapsed().as_nanos() as u64;

        // --- build the elements ------------------------------------------------------
        let rhw = px(self.cfg.row_header_width);
        let chh = px(self.cfg.col_header_height);
        let mut children: Vec<gpui::AnyElement> = Vec::with_capacity(cells.len() + 64);

        // Data cells, positioned relative to the content origin (offset by headers minus
        // the current scroll).
        for (r, c, rc) in cells {
            let x = self.col_axis.offset_of(c) - self.scroll_x;
            let y = self.row_axis.offset_of(r) - self.scroll_y;
            let w = px(self.col_axis.size_of(c));
            let h = px(self.row_axis.size_of(r));
            children.push(
                Self::data_cell(
                    rc,
                    w,
                    h,
                    px(x as f32) + rhw,
                    px(y as f32) + chh,
                )
                .into_any_element(),
            );
        }

        // Column headers (letters) across the top strip.
        for c in cols.clone() {
            let x = self.col_axis.offset_of(c) - self.scroll_x;
            let w = px(self.col_axis.size_of(c));
            children.push(
                Self::header_cell(column_label(c), w, chh, px(x as f32) + rhw, px(0.0))
                    .into_any_element(),
            );
        }
        // Row headers (1-based numbers) down the left gutter.
        for r in rows.clone() {
            let y = self.row_axis.offset_of(r) - self.scroll_y;
            let h = px(self.row_axis.size_of(r));
            children.push(
                Self::header_cell((r + 1).to_string(), rhw, h, px(0.0), px(y as f32) + chh)
                    .into_any_element(),
            );
        }
        // Top-left corner cap.
        children.push(Self::header_cell(String::new(), rhw, chh, px(0.0), px(0.0)).into_any_element());

        let render_ns = render_start.elapsed().as_nanos() as u64;

        // --- record + advance the harness -------------------------------------------
        if let Mode::RunTest {
            harness,
            prev_rows: pr,
            prev_cols: pc,
        } = &mut self.mode
        {
            harness.record(FrameSample {
                frame_render_ns: render_ns,
                cell_load_ns,
                newly_visible,
            });
            *pr = rows.clone();
            *pc = cols.clone();
            // Keep the frames coming until the script is exhausted.
            window.request_animation_frame();
        }

        // Root: a relative, full-size, white container holding the absolute cells. Give
        // it an id so `on_scroll_wheel` works (interactive mode).
        div()
            .id("grid-root")
            .relative()
            .size_full()
            .bg(rgb(0xFFFFFF))
            .overflow_hidden()
            .on_scroll_wheel(cx.listener(|this, ev: &ScrollWheelEvent, window, cx| {
                if matches!(this.mode, Mode::RunTest { .. }) {
                    return; // ignore user input during a scripted run
                }
                let line_h = window.line_height();
                let delta = ev.delta.pixel_delta(line_h);
                let (x, y) = this.clamp_scroll(
                    this.scroll_x - f32::from(delta.x) as f64,
                    this.scroll_y - f32::from(delta.y) as f64,
                );
                this.scroll_x = x;
                this.scroll_y = y;
                cx.notify();
            }))
            .children(children)
    }
}

impl Grid {
    /// Finalizes the current Run Test: builds the report, writes JSON, prints the
    /// summary + PASS/FAIL, and quits the app. Idempotent via `self.finished`.
    fn finish_run_test(&mut self, cx: &mut Context<Self>) {
        if self.finished {
            return;
        }
        self.finished = true;
        let samples: Vec<FrameSample> = match &self.mode {
            Mode::RunTest { harness, .. } => harness.samples().to_vec(),
            Mode::Interactive => Vec::new(),
        };
        match poc_core::finalize("raw-gpui", &self.date, &self.commit, &samples, &self.out_dir) {
            Ok((report, path)) => {
                println!("{}", report.summary());
                println!("results written to {}", path.display());
            }
            Err(e) => eprintln!("failed to write results: {e}"),
        }
        // Return to interactive mode so a manual re-run is possible, then quit.
        self.mode = Mode::Interactive;
        cx.quit();
    }
}
