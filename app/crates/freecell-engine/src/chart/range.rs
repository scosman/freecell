//! Data-range → `c:f` layout (P19, charts/implementation_plan). When the edit panel sets an
//! authored chart's **data range** (a rectangular cell block), this module turns that block into the
//! per-series `c:f` references the chart binds against — the structural half of "shape a near-empty
//! chart into a real one". It is pure (gpui-free, IronCalc-free): a `CellRange` in, a
//! [`SeriesRefs`](super::write::SeriesRefs) list out.
//!
//! **Block interpretation** (a deterministic, documented rule — not Excel's header heuristics): for a
//! block ≥ 2×2, the **first row** holds series-name headers, the **first column** holds the
//! category (or scatter x) labels, and **each remaining column is one series**. A degenerate block
//! (single row / column / cell) falls back to **one value series over the whole block** with no
//! header/categories. Emitted refs are **absolute + sheet-qualified** (`Name!$A$2:$A$5`), the shape
//! Excel/LibreOffice expect, so the ranged chart round-trips through the write path and lives-binds
//! like any loaded chart.

use freecell_core::refs::column_label;
use freecell_core::CellRange;

use super::write::SeriesRefs;

/// The `c:f` references derived from a data block, one [`SeriesRefs`] per series (in column order).
/// `categories` carries the domain ref (`c:cat` for category/value, `c:xVal` for scatter); `values`
/// the value ref; `name` the series-name cell ref. The worker builds the render [`Series`] +
/// [`ChartBinding`] from these and hands them to the write path verbatim on save.
///
/// [`Series`]: freecell_chart_model::Series
/// [`ChartBinding`]: super::binding::ChartBinding
pub fn series_refs_from_block(sheet_name: &str, block: CellRange) -> Vec<SeriesRefs> {
    let (r0, r1) = (block.start.row, block.end.row);
    let (c0, c1) = (block.start.col, block.end.col);
    let qualified = |a1: String| qualify(sheet_name, &a1);

    // A block with both a header row and a category column: first row = names, first column =
    // categories/x, each remaining column a series over the data rows (r0+1..=r1).
    if r1 > r0 && c1 > c0 {
        let cats = qualified(abs_col_range(c0, r0 + 1, r1));
        return (c0 + 1..=c1)
            .map(|col| SeriesRefs {
                name: Some(qualified(abs_cell(col, r0))),
                categories: Some(cats.clone()),
                values: Some(qualified(abs_col_range(col, r0 + 1, r1))),
            })
            .collect();
    }

    // Degenerate (single row / single column / single cell): one value series over the whole block,
    // no header, no categories (the renderer auto-indexes the domain).
    vec![SeriesRefs {
        name: None,
        categories: None,
        values: Some(qualified(abs_block(block))),
    }]
}

/// An absolute single-cell A1 ref (`$B$2` for `col=1,row=1`).
fn abs_cell(col: u32, row: u32) -> String {
    format!("${}${}", column_label(col), row + 1)
}

/// An absolute single-column A1 range (`$B$2:$B$5`) spanning `row0..=row1` inclusive.
fn abs_col_range(col: u32, row0: u32, row1: u32) -> String {
    let label = column_label(col);
    format!("${}${}:${}${}", label, row0 + 1, label, row1 + 1)
}

/// An absolute A1 range spanning a whole block (`$A$1:$D$5`), or a single `$A$1` for a 1×1 block.
fn abs_block(block: CellRange) -> String {
    let start = abs_cell(block.start.col, block.start.row);
    if block.is_single() {
        start
    } else {
        format!("{}:{}", start, abs_cell(block.end.col, block.end.row))
    }
}

/// Prefix an A1 ref with its worksheet name (`Data!$A$1`), quoting the name when it is not a bare
/// identifier — the same convention Excel writes (`'My Data'!$A$1`), with embedded `'` doubled. Our
/// own [`parse_cf`](super::binding::parse_cf) round-trips both forms.
fn qualify(sheet_name: &str, a1: &str) -> String {
    if needs_quoting(sheet_name) {
        format!("'{}'!{}", sheet_name.replace('\'', "''"), a1)
    } else {
        format!("{sheet_name}!{a1}")
    }
}

/// A sheet name is a bare identifier (no quoting) iff it is non-empty, starts with a letter or
/// underscore, and is all ASCII alphanumerics / underscores. Anything else (spaces, punctuation,
/// a leading digit) is quoted — the conservative side of Excel's rule.
fn needs_quoting(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => true,
        Some(first) if !(first.is_ascii_alphabetic() || first == '_') => true,
        _ => name
            .chars()
            .any(|c| !(c.is_ascii_alphanumeric() || c == '_')),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use freecell_core::{CellRef, SheetId};

    fn range(a1: &str) -> CellRange {
        CellRange::from_a1(a1).unwrap()
    }

    #[test]
    fn fixture_block_layout_maps_headers_categories_and_series() {
        // The shared `Data` grid: row 1 = headers (B1/C1/D1), col A rows 2..5 = categories,
        // cols B/C/D rows 2..5 = the three series (matches `authoring::write_fixture`).
        let refs = series_refs_from_block("Data", range("A1:D5"));
        assert_eq!(refs.len(), 3, "three series columns (B, C, D)");
        // Every series shares the category column A2:A5.
        for r in &refs {
            assert_eq!(r.categories.as_deref(), Some("Data!$A$2:$A$5"));
        }
        assert_eq!(refs[0].name.as_deref(), Some("Data!$B$1"));
        assert_eq!(refs[0].values.as_deref(), Some("Data!$B$2:$B$5"));
        assert_eq!(refs[1].name.as_deref(), Some("Data!$C$1"));
        assert_eq!(refs[1].values.as_deref(), Some("Data!$C$2:$C$5"));
        assert_eq!(refs[2].name.as_deref(), Some("Data!$D$1"));
        assert_eq!(refs[2].values.as_deref(), Some("Data!$D$2:$D$5"));
    }

    #[test]
    fn single_column_degrades_to_one_value_series() {
        let refs = series_refs_from_block("Sheet1", range("B2:B5"));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].name, None, "no header row in a single column");
        assert_eq!(refs[0].categories, None, "no category column");
        assert_eq!(refs[0].values.as_deref(), Some("Sheet1!$B$2:$B$5"));
    }

    #[test]
    fn single_cell_degrades_to_a_one_cell_value_series() {
        let refs = series_refs_from_block("Sheet1", CellRange::single(CellRef::new(0, 0)));
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].values.as_deref(), Some("Sheet1!$A$1"));
    }

    #[test]
    fn sheet_names_needing_quotes_are_quoted() {
        let refs = series_refs_from_block("My Data", range("A1:B3"));
        assert_eq!(refs[0].categories.as_deref(), Some("'My Data'!$A$2:$A$3"));
        assert_eq!(refs[0].values.as_deref(), Some("'My Data'!$B$2:$B$3"));
        // A name with an embedded quote doubles it.
        let refs = series_refs_from_block("O'Brien", range("A1:B3"));
        assert_eq!(refs[0].values.as_deref(), Some("'O''Brien'!$B$2:$B$3"));
        // A leading-digit name is quoted too.
        assert!(needs_quoting("2024"));
        assert!(!needs_quoting("Data_1"));
    }

    /// The emitted refs re-parse through the binding layer against the same `SheetId` — the tie to the
    /// live re-resolve path (a ranged authored chart binds exactly like a loaded one).
    #[test]
    fn emitted_refs_reparse_for_live_binding() {
        let refs = series_refs_from_block("Data", range("A1:C5"));
        let binding = super::super::binding::binding_from_refs(&refs);
        assert_eq!(binding.series.len(), 2);
        let cat = binding.series[0].cat.as_ref().unwrap();
        assert_eq!(cat.areas[0].sheet.as_deref(), Some("Data"));
        assert_eq!(cat.areas[0].range, range("A2:A5"));
        let val = binding.series[1].val.as_ref().unwrap();
        assert_eq!(val.areas[0].range, range("C2:C5"));
        // The sheet name resolves through a name→id closure just like the worker's.
        let resolve = |name: &str| (name == "Data").then_some(SheetId(0));
        assert_eq!(
            resolve(cat.areas[0].sheet.as_deref().unwrap()),
            Some(SheetId(0))
        );
    }
}
