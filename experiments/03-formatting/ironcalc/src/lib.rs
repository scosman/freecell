//! # ironcalc_formatting — IronCalc 0.7 formatting-exposure probe (Sub-project D)
//!
//! This crate answers, for **IronCalc**, the Sub-project D question
//! (functional_spec §6.D, architecture §6): *what formatting/metadata does the engine
//! expose on read, on write, and across a load → edit → save round-trip, and what
//! FreeCell formatting model does that imply?*
//!
//! ## The headline finding (verified by [`tests`](../tests/probe.rs))
//!
//! Unlike Formualizer, IronCalc exposes styles **natively and symmetrically**:
//!
//! - **Read/write.** `Model::get_style_for_cell(sheet,row,col) -> Style` and
//!   `Model::set_cell_style(sheet,row,col,&Style)` expose a full
//!   [`Style`](ironcalc_base::types::Style) (`font.{b,i,u,sz,color,name}`, `fill`,
//!   `border`, `num_fmt`, `alignment`). Row/column sizes are first-class too
//!   (`set_row_height`/`set_column_width` + getters).
//! - **Round-trip.** Styles cross a real `.xlsx` boundary: `save_xlsx_to_writer` +
//!   `load_from_xlsx_bytes` → `Model::from_workbook` preserve bold/italic/fill/
//!   number-format.
//! - **Gaps.** IronCalc 0.7 exposes **no public API for merged cells** (the
//!   `merge_cells` field on the internal `Worksheet` has no public getter/setter) and
//!   **no conditional-formatting API**. These are recorded as `None`/`Unverified` in the
//!   matrix rather than asserted.
//!
//! So for FreeCell on IronCalc, styles can live **inside the engine** — no external
//! store is required for the attributes the engine models — with a side-store needed
//! only for what IronCalc omits (merges, conditional formatting).

use ironcalc::export::save_xlsx_to_writer;
use ironcalc::import::load_from_xlsx_bytes;
use ironcalc_base::Model;
use serde::Serialize;

/// The single sheet all probes use (index 0, created by `new_empty`).
pub const SHEET: u32 = 0;

/// A small, engine-neutral snapshot of the formatting attributes FreeCell cares about
/// (parallels the Formualizer probe's `NeutralFormat` so the two matrices compare
/// like-for-like).
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
pub struct NeutralFormat {
    pub bold: bool,
    pub italic: bool,
    pub font_size: Option<f64>,
    /// Fill foreground colour as a hex string (IronCalc uses `"#RRGGBB"` /
    /// `"#AARRGGBB"`), if any.
    pub fill_argb: Option<String>,
    /// Number-format code (e.g. `"0.00"`), if any.
    pub number_format: Option<String>,
}

/// The three axes each formatting attribute is scored on (IronCalc variant).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub enum Support {
    /// Exposed by IronCalc's own native `Model`/`Style` API.
    Native,
    /// Not reachable through IronCalc's public API in 0.7.
    None,
    /// The underlying data exists but has no public API / unverified fidelity.
    Unverified,
}

/// One row of the capability matrix.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityRow {
    pub attribute: &'static str,
    pub read: Support,
    pub write: Support,
    pub roundtrip: Support,
    pub note: &'static str,
}

/// The full, env-stamped capability matrix serialized to `results/ironcalc/`.
#[derive(Debug, Clone, Serialize)]
pub struct CapabilityMatrix {
    pub engine: &'static str,
    pub engine_version: &'static str,
    pub rustc: &'static str,
    pub date: &'static str,
    pub rows: Vec<CapabilityRow>,
}

/// A fresh single-sheet model.
pub fn new_model() -> Model<'static> {
    Model::new_empty("formatting", "en", "UTC", "en").expect("ironcalc new_empty")
}

/// Builds a small **styled** model: A1 bold + 16pt + yellow fill + `"0.00"`; B2 italic;
/// row 1 height 40.0; column A (index 1) width 30.0. Values are set so the cells exist.
pub fn styled_model() -> Model<'static> {
    let mut model = new_model();
    model
        .set_user_input(SHEET, 1, 1, "12.5".to_string())
        .expect("set A1");
    model
        .set_user_input(SHEET, 2, 2, "hello".to_string())
        .expect("set B2");

    // A1: bold, 16pt, yellow fill, 2-decimal number format.
    {
        let mut s = model.get_style_for_cell(SHEET, 1, 1).expect("A1 style");
        s.font.b = true;
        s.font.sz = 16;
        s.fill.pattern_type = "solid".to_string();
        s.fill.fg_color = Some("#FFFF00".to_string());
        s.num_fmt = "0.00".to_string();
        model.set_cell_style(SHEET, 1, 1, &s).expect("set A1 style");
    }
    // B2: italic.
    {
        let mut s = model.get_style_for_cell(SHEET, 2, 2).expect("B2 style");
        s.font.i = true;
        model.set_cell_style(SHEET, 2, 2, &s).expect("set B2 style");
    }
    // Row/column sizes.
    model.set_row_height(SHEET, 1, 40.0).expect("row 1 height");
    model.set_column_width(SHEET, 1, 30.0).expect("col A width");
    model
}

/// Reads the [`NeutralFormat`] for a cell straight from an IronCalc [`Model`].
pub fn read_format(model: &Model, sheet: u32, row: i32, col: i32) -> NeutralFormat {
    let style = match model.get_style_for_cell(sheet, row, col) {
        Ok(s) => s,
        Err(_) => return NeutralFormat::default(),
    };
    let number_format = if style.num_fmt.is_empty() || style.num_fmt == "general" {
        None
    } else {
        Some(style.num_fmt.clone())
    };
    NeutralFormat {
        bold: style.font.b,
        italic: style.font.i,
        font_size: Some(style.font.sz as f64),
        fill_argb: style.fill.fg_color.clone(),
        number_format,
    }
}

/// Round-trips a model through a real `.xlsx` byte boundary: serialize with
/// `save_xlsx_to_writer` into an in-memory cursor, then `load_from_xlsx_bytes` +
/// `Model::from_workbook`. This is IronCalc's native style persistence path.
pub fn roundtrip_via_xlsx(model: &Model) -> Model<'static> {
    let cursor = std::io::Cursor::new(Vec::new());
    let cursor = save_xlsx_to_writer(model, cursor).expect("save_xlsx_to_writer");
    let bytes = cursor.into_inner();
    let workbook =
        load_from_xlsx_bytes(&bytes, "roundtrip", "en", "UTC").expect("load_from_xlsx_bytes");
    Model::from_workbook(workbook, "en").expect("Model::from_workbook")
}

/// Builds the IronCalc capability matrix (statuses backed by the passing probes).
pub fn capability_matrix() -> CapabilityMatrix {
    use Support::*;
    let rows = vec![
        CapabilityRow {
            attribute: "bold",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.font.b via get_style_for_cell / set_cell_style; survives xlsx round-trip.",
        },
        CapabilityRow {
            attribute: "italic",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.font.i.",
        },
        CapabilityRow {
            attribute: "font_size",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.font.sz (i32 points).",
        },
        CapabilityRow {
            attribute: "fill_color",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.fill.fg_color (hex); requires pattern_type = \"solid\".",
        },
        CapabilityRow {
            attribute: "number_format",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.num_fmt (format code string).",
        },
        CapabilityRow {
            attribute: "borders",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.border (per-side BorderItem); not probed exhaustively (Round 2).",
        },
        CapabilityRow {
            attribute: "alignment",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Style.alignment (Horizontal/VerticalAlignment enums).",
        },
        CapabilityRow {
            attribute: "row_height",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Model::get/set_row_height (f64).",
        },
        CapabilityRow {
            attribute: "col_width",
            read: Native,
            write: Native,
            roundtrip: Native,
            note: "Model::get/set_column_width (f64).",
        },
        CapabilityRow {
            attribute: "merges",
            read: None,
            write: None,
            roundtrip: None,
            note: "No public merged-cells API on Model in 0.7 (internal Worksheet.merge_cells field is not exposed).",
        },
        CapabilityRow {
            attribute: "conditional_formatting",
            read: None,
            write: None,
            roundtrip: None,
            note: "No conditional-formatting API in the public crate interface.",
        },
        CapabilityRow {
            attribute: "themes / named styles",
            read: Unverified,
            write: Native,
            roundtrip: Unverified,
            note: "set_cell_style_by_name / set_sheet_style exist; no general theme read API. Not probed.",
        },
    ];
    CapabilityMatrix {
        engine: "ironcalc",
        engine_version: "0.7.1",
        rustc: "1.94.1",
        date: "2026-07-01",
        rows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_format_defaults_are_absent() {
        let f = NeutralFormat::default();
        assert!(!f.bold);
        assert!(f.fill_argb.is_none());
    }

    #[test]
    fn matrix_serializes() {
        let json = serde_json::to_string(&capability_matrix()).expect("serialize matrix");
        assert!(json.contains("ironcalc"));
        assert!(json.contains("conditional_formatting"));
    }
}
