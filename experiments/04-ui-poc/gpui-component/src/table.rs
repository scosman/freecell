//! The `gpui-component` variant: a `TableDelegate` over the same static provider,
//! rendered by `gpui-component`'s virtualized `DataTable`.
//!
//! This is the head-to-head counterpart to `raw-gpui/`. Two findings fall out of using
//! the component (both recorded in `../findings.md`):
//! - **Variable column widths: supported** — each [`Column`] carries its own `width`
//!   (from the provider's `col_width`), plus min/max and interactive resize.
//! - **Variable row heights: NOT supported** — `DataTable`'s vertical virtualization is
//!   built on gpui `uniform_list`, which is fixed-row-height. So this variant renders a
//!   *uniform* row height while the raw-gpui variant honours per-row heights. That gap
//!   is itself a comparison result.
//!
//! Cells are styled through the shared [`poc_core::RenderCell`] path so the two variants
//! look the same where the component allows.
//!
//! macOS/Metal only.

use gpui::{App, Context, IntoElement, ParentElement, Styled, Window, div, px, rgb};
use gpui_component::table::{Column, TableDelegate, TableState};

use datagen::{CellSource, SyntheticSheet, column_label};
use poc_core::{
    Align, PocConfig, RenderCell,
    style::{HEADER_TEXT, TEXT_DARK},
};

/// A uniform row height for the component variant (see the module note: `DataTable`
/// cannot do per-row heights). Chosen near the synthetic sheet's typical row height.
pub const UNIFORM_ROW_HEIGHT: f32 = 24.0;

/// The delegate: it owns the provider + config and answers the table's row/col/cell
/// queries. It holds no per-cell state — cells are pulled from the provider on demand,
/// exactly like the raw-gpui variant, so both exercise the same "load on scroll" path.
pub struct SheetDelegate {
    cfg: PocConfig,
    provider: SyntheticSheet,
}

impl SheetDelegate {
    pub fn new(cfg: PocConfig) -> Self {
        let provider = SyntheticSheet::new(cfg.seed, cfg.rows, cfg.cols);
        Self { cfg, provider }
    }

    /// The number of columns this variant renders. `DataTable` builds a `Column` per
    /// column up front, so materializing all 16,384 Excel-max columns is itself part of
    /// the finding (raw-gpui virtualizes columns without a per-column object). We cap to
    /// a large-but-tractable width here and note the limitation in findings.
    fn rendered_cols(&self) -> u32 {
        self.cfg.cols
    }
}

impl TableDelegate for SheetDelegate {
    fn columns_count(&self, _cx: &App) -> usize {
        self.rendered_cols() as usize
    }

    fn rows_count(&self, _cx: &App) -> usize {
        self.cfg.rows as usize
    }

    fn column(&self, col_ix: usize, _cx: &App) -> Column {
        // Variable column widths ARE supported: give each column its provider width.
        let w = self.provider.col_width(col_ix as u32);
        Column::new(
            col_ix.to_string(),
            gpui::SharedString::from(column_label(col_ix as u32)),
        )
        .width(px(w))
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        div()
            .px_1()
            .text_color(rgb(HEADER_TEXT))
            .text_xs()
            .child(column_label(col_ix as u32))
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        _cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        // Same provider + styling path as raw-gpui.
        let data = self.provider.cell(row_ix as u32, col_ix as u32);
        let rc = RenderCell::from_cell(&data);
        render_cell_div(&rc)
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _cx: &App) -> String {
        poc_core::style::format_value(&self.provider.cell(row_ix as u32, col_ix as u32).value)
    }
}

/// Builds a styled cell `div` from a [`RenderCell`] — mirrors the raw-gpui variant's
/// look (fill, text colour, bold/italic, alignment) within one table cell.
fn render_cell_div(rc: &RenderCell) -> gpui::Div {
    let mut cell = div()
        .h(px(UNIFORM_ROW_HEIGHT))
        .px_1()
        .flex()
        .items_center()
        .bg(rgb(rc.fill))
        .text_color(rgb(TEXT_DARK))
        .text_sm()
        .overflow_hidden()
        .whitespace_nowrap();

    cell = match rc.align {
        Align::Left => cell.justify_start(),
        Align::Center => cell.justify_center(),
        Align::Right => cell.justify_end(),
    };
    if rc.bold {
        cell = cell.font_weight(gpui::FontWeight::BOLD);
    }
    if rc.italic {
        cell = cell.italic();
    }
    cell.child(rc.text.clone())
}
