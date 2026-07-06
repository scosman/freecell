//! Hand-built `freecell-core` fixtures for the grid — the demo window's data and a
//! reference for Phase-7 render scenes. No engine involved (`components/grid.md`: the grid
//! is buildable + render-testable against core fixtures before the engine track lands).

use std::sync::Arc;

use arc_swap::ArcSwap;
use parking_lot::RwLock;

use freecell_core::cache::{SheetCacheBuilder, SheetCaches};
use freecell_core::color::Rgb;
use freecell_core::limits;
use freecell_core::publication::{CellKind, Publication, PublishedCell};
use freecell_core::refs::{CellRef, SheetId};
use freecell_core::style::{Align, RenderStyle};
use freecell_core::SelectionModel;

use super::GridDataSources;

/// The demo sheet id.
pub const DEMO_SHEET: SheetId = SheetId(0);

fn bold() -> RenderStyle {
    RenderStyle {
        bold: true,
        ..RenderStyle::default()
    }
}

fn italic() -> RenderStyle {
    RenderStyle {
        italic: true,
        ..RenderStyle::default()
    }
}

fn underline() -> RenderStyle {
    RenderStyle {
        underline: true,
        ..RenderStyle::default()
    }
}

fn bold_fill(fill: u32) -> RenderStyle {
    RenderStyle {
        bold: true,
        fill: Some(Rgb::from_hex(fill)),
        ..RenderStyle::default()
    }
}

fn right_number() -> RenderStyle {
    RenderStyle {
        h_align: Some(Align::Right),
        ..RenderStyle::default()
    }
}

fn cell(row: u32, col: u32, text: &str) -> PublishedCell {
    PublishedCell {
        row,
        col,
        display_text: text.to_string(),
        kind: CellKind::Text,
        text_color: None,
    }
}

/// Builds the demo data sources: an Excel-max sheet with a wide column, a tall row, and a
/// spread of styled + published cells exercising fills, bold/italic/underline, alignment,
/// and text clipping.
pub fn demo_sources() -> GridDataSources {
    let cache = SheetCacheBuilder::new(limits::MAX_ROWS, limits::MAX_COLS)
        .col_width(1, 180.0) // wide column B
        .row_height(2, 44.0) // tall row 3
        .cell_style(0, 0, bold()) // A1 bold
        .cell_style(1, 1, bold_fill(0xFFF2CC)) // B2 bold on light-yellow fill
        .cell_style(2, 2, italic()) // C3 italic
        .cell_style(3, 1, right_number()) // B4 right-aligned number
        .cell_style(4, 0, underline()) // A5 underline
        .build();

    let mut caches = SheetCaches::new();
    caches.insert(DEMO_SHEET, cache);

    let cells = vec![
        cell(0, 0, "FreeCell"),
        cell(0, 1, "Demo sheet"),
        cell(1, 1, "42.50"),
        cell(2, 2, "hello"),
        cell(3, 1, "1234.5"),
        cell(4, 0, "underlined"),
        cell(5, 3, "clipped-very-long-text-abcdefghijklmnopqrstuvwxyz"),
        cell(7, 5, "#DIV/0!"),
    ];
    let publication = Publication {
        sheet: DEMO_SHEET,
        rows: 0..40,
        cols: 0..20,
        generation: 1,
        cells,
    };

    GridDataSources {
        publication: Arc::new(ArcSwap::from_pointee(publication)),
        caches: Arc::new(RwLock::new(caches)),
    }
}

/// A demo range selection (B2:D4) so the standalone window shows the selection layer. The
/// overlay tints the range except the **active** cell, so the un-tinted "white anchor" is
/// the active cell D4 (the anchor B2 gets the 10% overlay).
pub fn demo_selection() -> SelectionModel {
    SelectionModel {
        anchor: CellRef::new(1, 1),
        active: CellRef::new(3, 3),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_sources_builds() {
        let sources = demo_sources();
        // The publication belongs to the demo sheet and carries the seeded cells.
        let publication = sources.publication.load();
        assert_eq!(publication.sheet, DEMO_SHEET);
        assert!(!publication.cells.is_empty());
        // The resident cache exists for the demo sheet with the expected geometry overrides.
        let caches = sources.caches.read();
        let cache = caches.get(DEMO_SHEET).expect("demo cache present");
        assert_eq!(cache.col_width(1), 180.0);
        assert_eq!(cache.row_height(2), 44.0);
        // A styled cell resolves; a plain cell does not.
        assert!(cache.render_style(0, 0).is_some());
        assert!(cache.render_style(30, 30).is_none());
        // The demo selection is a B2:D4 range with the active cell at D4.
        let sel = demo_selection();
        assert!(!sel.is_single());
        assert_eq!(sel.active, CellRef::new(3, 3));
    }
}
