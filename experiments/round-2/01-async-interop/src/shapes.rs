//! DAG-shape builders for the `evaluate()` latency matrix (functional_spec SP1,
//! architecture §4.4).
//!
//! Each shape populates an [`ironcalc_base::Model`] with a known dependency graph and
//! reports the **tail cell** whose value the matrix runner force-reads and asserts
//! changed after each `evaluate()`. Inputs are written via `set_user_input` **without**
//! evaluating (deferred, exactly like the harness adapter) — the matrix times a single
//! full `evaluate()` separately from build.
//!
//! Coordinates are datagen 0-based; IronCalc is 1-based `i32` (+ a `u32` sheet), so the
//! builders add `+1` on write, matching `round2_harness::ironcalc`.

use datagen::{linear_chain, wide_fanout, CellAddress, EXCEL_MAX_ROWS};
use ironcalc_base::cell::CellValue;
use ironcalc_base::Model;

/// The primary sheet all single-sheet shapes populate.
const SHEET: u32 = 0;

/// Maps a 0-based linear cell index to a `(row, col)` that wraps into the next column
/// every `EXCEL_MAX_ROWS` cells, so shapes can reach 10⁷ populated cells without
/// exceeding Excel's 1,048,576-row limit (which IronCalc enforces — a single column
/// would panic past the limit; that ceiling is itself an SP1 finding).
fn wrapped(index: u32) -> (u32, u32) {
    (index % EXCEL_MAX_ROWS, index / EXCEL_MAX_ROWS)
}

/// The five DAG shapes the SP1 matrix measures (functional_spec SP1 "DAG shapes").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// ~1% formula density over a square region; the rest are literals.
    Sparse,
    /// Wide fan-out: 1000 sources, 1000 dependents each summing all sources.
    WideFanout,
    /// Deep serial `=PREV+1` chain of the requested length (the 1M variant is the
    /// known ~2 s FAIL vs the <100 ms target).
    DeepSerial,
    /// Cross-sheet: literals on a second sheet, formulas on sheet 0 referencing them.
    CrossSheet,
    /// Volatile: `=RAND()` cells whose values genuinely change every eval.
    Volatile,
}

impl Shape {
    /// A short, stable identifier used in recorded result filenames/fields.
    pub fn id(self) -> &'static str {
        match self {
            Shape::Sparse => "sparse",
            Shape::WideFanout => "wide_fanout",
            Shape::DeepSerial => "deep_serial",
            Shape::CrossSheet => "cross_sheet",
            Shape::Volatile => "volatile",
        }
    }

    /// Parses an identifier back to a [`Shape`] (for the binary's `--shape` filter).
    pub fn from_id(s: &str) -> Option<Shape> {
        match s {
            "sparse" => Some(Shape::Sparse),
            "wide_fanout" => Some(Shape::WideFanout),
            "deep_serial" => Some(Shape::DeepSerial),
            "cross_sheet" => Some(Shape::CrossSheet),
            "volatile" => Some(Shape::Volatile),
            _ => None,
        }
    }

    /// All five shapes, in matrix order.
    pub fn all() -> [Shape; 5] {
        [
            Shape::Sparse,
            Shape::WideFanout,
            Shape::DeepSerial,
            Shape::CrossSheet,
            Shape::Volatile,
        ]
    }
}

/// A built model plus the metadata the matrix runner needs to force+assert progress.
pub struct BuiltShape {
    /// Which shape this was built from (recorded on results).
    pub shape: Shape,
    /// The populated model, inputs written but **not yet evaluated**.
    pub model: Model<'static>,
    /// Number of populated (non-empty) cells across all sheets — the honest
    /// `input_size` for the recorded result (an eval is O(all cells)).
    pub populated_cells: u64,
    /// The tail cell whose value the runner reads and asserts changed after eval.
    pub tail: (u32, u32, u32),
    /// The tail's `A1` label (for reporting).
    pub tail_a1: String,
    /// How the runner should re-arm the model between timed samples so each eval does
    /// real work (see [`ReArm`]).
    pub rearm: ReArm,
    /// Whether the tail value is expected to change between two consecutive evals with
    /// the same inputs (only volatile), vs only after a re-arm edit.
    pub changes_without_rearm: bool,
}

/// How to make the *next* `evaluate()` produce a different tail value, so the matrix
/// measures real recompute work sample-to-sample rather than a no-op re-eval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReArm {
    /// Bump a head/source literal by +1 (sparse, chain, cross-sheet, fanout): the tail
    /// then changes by a deterministic amount.
    BumpSeed { sheet: u32, row: u32, col: u32 },
    /// No re-arm needed — the shape is volatile, so every eval changes the tail.
    None,
}

/// Writes an input at datagen 0-based `(row,col)` on `sheet` **without** evaluating.
fn put(model: &mut Model<'static>, sheet: u32, row: u32, col: u32, input: &str) {
    model
        .set_user_input(sheet, (row + 1) as i32, (col + 1) as i32, input.to_string())
        .expect("ironcalc set_user_input");
}

/// Reads the numeric value of a cell (for tail assertions). Non-number → `None`.
pub fn read_number(model: &Model<'static>, sheet: u32, row: u32, col: u32) -> Option<f64> {
    match model.get_cell_value_by_index(sheet, (row + 1) as i32, (col + 1) as i32) {
        Ok(CellValue::Number(n)) => Some(n),
        _ => None,
    }
}

/// Builds the requested shape at (approximately) `n` populated cells.
///
/// `n` is the *target* populated-cell count (the matrix sizes 10⁴…10⁷). Shapes hit it
/// as closely as their geometry allows; `populated_cells` records the exact figure.
pub fn build(shape: Shape, n: u64) -> BuiltShape {
    match shape {
        Shape::Sparse => build_sparse(n),
        Shape::WideFanout => build_wide_fanout(n),
        Shape::DeepSerial => build_deep_serial(n),
        Shape::CrossSheet => build_cross_sheet(n),
        Shape::Volatile => build_volatile(n),
    }
}

fn new_model() -> Model<'static> {
    Model::new_empty("sp1", "en", "UTC", "en").expect("ironcalc new_empty")
}

/// Sparse ~1% formula density: a square-ish grid of literals with ~1% of cells being
/// `=<cell-above>+1`, so an edit to the top seed cascades down each formula column.
///
/// Layout: `rows × cols ≈ n`. Column 0 is a chain seed column (`row 0 = 1`, then
/// `=A<r>+1`) so the tail (bottom of column 0) depends on the seed. The other ~1% of
/// formula cells reference their up-neighbor; the ~99% remainder are literals.
fn build_sparse(n: u64) -> BuiltShape {
    let side = (n as f64).sqrt().ceil() as u32;
    let side = side.max(2);
    let mut model = new_model();
    let mut populated: u64 = 0;

    // Seed chain down column 0: the tail's precedent path.
    put(&mut model, SHEET, 0, 0, "=1");
    populated += 1;
    for r in 1..side {
        let prev = CellAddress::new(r - 1, 0).a1();
        put(&mut model, SHEET, r, 0, &format!("={prev}+1"));
        populated += 1;
    }

    // Fill the rest: ~1% formulas (reference up-neighbor), ~99% literals.
    for r in 0..side {
        for c in 1..side {
            if populated as u128 >= n as u128 {
                break;
            }
            // ~1% of the non-seed cells are formulas.
            let is_formula =
                ((r as u64).wrapping_mul(side as u64) + c as u64).is_multiple_of(100) && r > 0;
            if is_formula {
                let up = CellAddress::new(r - 1, c).a1();
                put(&mut model, SHEET, r, c, &format!("={up}+1"));
            } else {
                put(&mut model, SHEET, r, c, &format!("{}", (r + c) % 97));
            }
            populated += 1;
        }
    }

    let tail = (SHEET, side - 1, 0);
    BuiltShape {
        shape: Shape::Sparse,
        model,
        populated_cells: populated,
        tail,
        tail_a1: CellAddress::new(side - 1, 0).a1(),
        rearm: ReArm::BumpSeed {
            sheet: SHEET,
            row: 0,
            col: 0,
        },
        changes_without_rearm: false,
    }
}

/// Wide fan-out (functional_spec "wide fan-out 1000×1000"): 1000 source literals and
/// 1000 dependents each `=SUM(sources)`. `n` is informational — the shape is fixed at
/// 1000×1000 (2000 populated cells) to match the spec; the recorded `populated_cells`
/// reflects that. Editing any source changes every dependent (fan-out).
fn build_wide_fanout(_n: u64) -> BuiltShape {
    let sources = 1000u32;
    let dependents = 1000u32;
    let cells = wide_fanout(sources, dependents);
    let mut model = new_model();
    for fc in &cells {
        put(&mut model, SHEET, fc.addr.row, fc.addr.col, &fc.formula);
    }
    // Last dependent is the tail; it sums all sources.
    let tail_addr = CellAddress::new(dependents - 1, sources);
    BuiltShape {
        shape: Shape::WideFanout,
        model,
        populated_cells: cells.len() as u64,
        tail: (SHEET, tail_addr.row, tail_addr.col),
        tail_a1: tail_addr.a1(),
        // Bumping source A1 (=1) changes every dependent's SUM.
        rearm: ReArm::BumpSeed {
            sheet: SHEET,
            row: 0,
            col: 0,
        },
        changes_without_rearm: false,
    }
}

/// Deep serial `=PREV+1` chain of length `n` down column 0 (functional_spec "deep-serial
/// 1M `=PREV+1` chain"). Tail = last cell (value == n). The 10⁶ variant (~2 s) is the
/// known-FAIL vs the <100 ms target.
fn build_deep_serial(n: u64) -> BuiltShape {
    let len = n.min(u32::MAX as u64) as u32;
    let len = len.max(2);
    let mut model = new_model();
    for fc in linear_chain(len, 0) {
        put(&mut model, SHEET, fc.addr.row, fc.addr.col, &fc.formula);
    }
    let tail_addr = CellAddress::new(len - 1, 0);
    BuiltShape {
        shape: Shape::DeepSerial,
        model,
        populated_cells: len as u64,
        tail: (SHEET, tail_addr.row, tail_addr.col),
        tail_a1: tail_addr.a1(),
        // Bumping the head literal (=1 at A1) shifts the whole chain's values.
        rearm: ReArm::BumpSeed {
            sheet: SHEET,
            row: 0,
            col: 0,
        },
        changes_without_rearm: false,
    }
}

/// Cross-sheet: `n/2` literals on sheet 2 (`Sheet2`), and `n/2` formulas on sheet 0
/// each referencing the matching sheet-2 cell (`=Sheet2!A<r>+1`). Editing a sheet-2
/// literal changes its sheet-0 dependent (cross-sheet propagation).
fn build_cross_sheet(n: u64) -> BuiltShape {
    let per = (n / 2).max(1) as u32;
    let mut model = new_model();
    // Model starts with one sheet ("Sheet1", index 0). Add a second ("Sheet2", index 1).
    let (name2, sheet2) = model.new_sheet();

    // Literals on sheet 2, wrapped into columns at the Excel row limit.
    for i in 0..per {
        let (r, c) = wrapped(i);
        put(&mut model, sheet2, r, c, &format!("{}", i + 1));
    }
    // Formulas on sheet 0, each referencing the matching wrapped sheet-2 cell.
    for i in 0..per {
        let (r, c) = wrapped(i);
        let ref_cell = CellAddress::new(r, c).a1();
        put(&mut model, SHEET, r, c, &format!("={name2}!{ref_cell}+1"));
    }
    let (tail_row, tail_col) = wrapped(per - 1);
    let tail_addr = CellAddress::new(tail_row, tail_col);
    BuiltShape {
        shape: Shape::CrossSheet,
        model,
        populated_cells: (per as u64) * 2,
        tail: (SHEET, tail_row, tail_col),
        tail_a1: tail_addr.a1(),
        // The tail references Sheet2!<same cell>, so re-arm that exact precedent literal
        // — bumping it changes the tail's cross-sheet dependent.
        rearm: ReArm::BumpSeed {
            sheet: sheet2,
            row: tail_row,
            col: tail_col,
        },
        changes_without_rearm: false,
    }
}

/// Volatile: `n` `=RAND()` cells, wrapped into columns at the Excel row limit. Every
/// `evaluate()` re-rolls them, so the tail changes value with no re-arm edit needed.
fn build_volatile(n: u64) -> BuiltShape {
    let len = n.min(u32::MAX as u64) as u32;
    let len = len.max(2);
    let mut model = new_model();
    for i in 0..len {
        let (r, c) = wrapped(i);
        put(&mut model, SHEET, r, c, "=RAND()");
    }
    let (tail_row, tail_col) = wrapped(len - 1);
    let tail_addr = CellAddress::new(tail_row, tail_col);
    BuiltShape {
        shape: Shape::Volatile,
        model,
        populated_cells: len as u64,
        tail: (SHEET, tail_row, tail_col),
        tail_a1: tail_addr.a1(),
        rearm: ReArm::None,
        changes_without_rearm: true,
    }
}

/// Applies a re-arm edit (bump a seed literal by +1) so the next eval changes the tail.
/// Returns the delta applied to the seed (always 1.0 here) for the caller's records.
pub fn rearm(model: &mut Model<'static>, rearm: ReArm, sample: u64) -> f64 {
    match rearm {
        ReArm::None => 0.0,
        ReArm::BumpSeed { sheet, row, col } => {
            // Set the seed to a fresh value each sample so the cascade re-derives. Start
            // at 2 and step monotonically so it never collides with the warm-up seed
            // (=1) nor with the previous sample's value — guaranteeing the tail changes.
            let v = (sample % 1_000_000) as f64 + 2.0;
            put(model, sheet, row, col, &format!("{v}"));
            v
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn eval_tail(built: &mut BuiltShape) -> Option<f64> {
        built.model.evaluate();
        read_number(&built.model, built.tail.0, built.tail.1, built.tail.2)
    }

    #[test]
    fn sparse_tail_is_chain_end() {
        let mut built = build(Shape::Sparse, 400);
        assert!(built.populated_cells >= 400);
        let v = eval_tail(&mut built).expect("sparse tail is a number");
        // Column-0 seed chain: value at row r is (r+1) since head =1. Tail = side.
        let side = (400f64).sqrt().ceil() as u32;
        assert_eq!(v, side as f64);
    }

    #[test]
    fn wide_fanout_tail_sums_sources() {
        let mut built = build(Shape::WideFanout, 0);
        assert_eq!(built.populated_cells, 2000);
        let v = eval_tail(&mut built).expect("fanout tail is a number");
        // Sources are =1..=1000, so SUM = 1000*1001/2.
        assert_eq!(v, (1000.0 * 1001.0) / 2.0);
    }

    #[test]
    fn deep_serial_tail_equals_len() {
        let mut built = build(Shape::DeepSerial, 500);
        assert_eq!(built.populated_cells, 500);
        let v = eval_tail(&mut built).expect("chain tail is a number");
        assert_eq!(v, 500.0); // head=1, +1 each of 499 steps.
    }

    #[test]
    fn cross_sheet_tail_references_other_sheet() {
        let mut built = build(Shape::CrossSheet, 100);
        assert_eq!(built.populated_cells, 100); // 50 literals + 50 formulas
        let v = eval_tail(&mut built).expect("cross-sheet tail is a number");
        // Sheet2!A50 = 50; sheet0 tail = Sheet2!A50 + 1 = 51.
        assert_eq!(v, 51.0);
    }

    #[test]
    fn volatile_tail_is_unit_random_and_changes() {
        let mut built = build(Shape::Volatile, 50);
        assert_eq!(built.populated_cells, 50);
        let a = eval_tail(&mut built).expect("volatile tail is a number");
        assert!((0.0..1.0).contains(&a), "RAND() in [0,1): {a}");
        // A second eval re-rolls it (with overwhelming probability, differs).
        let b = eval_tail(&mut built).expect("volatile tail is a number");
        assert!((0.0..1.0).contains(&b));
        assert_ne!(a, b, "RAND() should change across evals");
    }

    #[test]
    fn rearm_bumps_seed_and_changes_tail() {
        let mut built = build(Shape::DeepSerial, 10);
        let before = eval_tail(&mut built).unwrap();
        assert_eq!(before, 10.0);
        let seed = rearm(&mut built.model, built.rearm, 4); // seed = 4 + 2 = 6
        assert_eq!(seed, 6.0);
        let after = eval_tail(&mut built).unwrap();
        // Head became 6, chain of length 10 -> tail = 6 + 9 = 15.
        assert_eq!(after, 15.0);
        assert_ne!(before, after);
    }

    #[test]
    fn shape_id_roundtrips() {
        for s in Shape::all() {
            assert_eq!(Shape::from_id(s.id()), Some(s));
        }
        assert_eq!(Shape::from_id("nope"), None);
    }
}
