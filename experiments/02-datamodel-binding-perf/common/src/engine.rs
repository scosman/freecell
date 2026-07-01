//! The engine-abstraction trait — the "binding surface" FreeCell's UI needs from a
//! spreadsheet engine, plus the neutral value/coordinate types the scenarios speak.
//!
//! Both the Formualizer and IronCalc adapters implement [`SpreadsheetEngine`] so the
//! *identical* scenarios (`crate::scenario`) and binding designs (`crate::binding`)
//! run against both engines and produce directly comparable numbers (architecture
//! §5, functional_spec §6.C). This module is engine-neutral by construction: it
//! depends on no spreadsheet engine.
//!
//! ## Coordinate convention
//!
//! All trait coordinates are **0-based** `(row, col)` — the same space
//! [`datagen`](../../../shared/datagen) uses. Real engines are 1-based (Formualizer)
//! or 1-based `i32` (IronCalc); each adapter owns the `+1` conversion internally so
//! both engines evaluate the *same logical sheet* datagen produces.

use serde::{Deserialize, Serialize};

/// A neutral cell value the scenarios speak, so they never touch an engine's own
/// value type. Adapters convert to/from their engine's value enum.
#[derive(Debug, Clone, PartialEq)]
pub enum EngineValue {
    /// No value.
    Empty,
    /// A numeric value.
    Number(f64),
    /// A text value.
    Text(String),
    /// A boolean value.
    Bool(bool),
    /// An error value (carries the engine's error label for reporting).
    Error(String),
}

impl EngineValue {
    /// The numeric content, if this value is a [`EngineValue::Number`]. Used by
    /// correctness checks that assert a cascade produced the expected number.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            EngineValue::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// One logical cell input: either a literal value or a formula string (with the
/// leading `=`). Used by [`SpreadsheetEngine::set_batch`] and the scenario builders.
#[derive(Debug, Clone, PartialEq)]
pub enum CellInput {
    /// A literal value.
    Value(EngineValue),
    /// A formula string, e.g. `"=A1+1"`.
    Formula(String),
}

/// A rectangular viewport in 0-based coordinates: the top-left `(row0, col0)` plus a
/// height/width in cells. Models the visible window the UI reads as it scrolls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Viewport {
    pub row0: u32,
    pub col0: u32,
    pub rows: u32,
    pub cols: u32,
}

impl Viewport {
    /// Creates a viewport.
    pub const fn new(row0: u32, col0: u32, rows: u32, cols: u32) -> Self {
        Self {
            row0,
            col0,
            rows,
            cols,
        }
    }

    /// The number of cells the viewport covers (`rows * cols`).
    pub fn cell_count(&self) -> usize {
        self.rows as usize * self.cols as usize
    }

    /// Iterates the viewport's `(row, col)` addresses in row-major order.
    pub fn addresses(&self) -> impl Iterator<Item = (u32, u32)> + '_ {
        (self.row0..self.row0 + self.rows)
            .flat_map(move |r| (self.col0..self.col0 + self.cols).map(move |c| (r, c)))
    }
}

/// What an engine can do natively, so scenarios and the findings doc can state
/// plainly which operations are native versus emulated in the adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineCaps {
    /// A native 2D range/bulk read (Formualizer `read_range`); if `false` the adapter
    /// emulates [`SpreadsheetEngine::read_viewport`] with a per-cell loop (IronCalc).
    pub native_range_read: bool,
    /// Incremental/dirty recalc after an edit; if `false` the engine re-evaluates the
    /// whole workbook on recompute (IronCalc `Model::evaluate`).
    pub incremental_recalc: bool,
    /// Parallel evaluation is available (Formualizer `EvalConfig`).
    pub parallel_eval: bool,
    /// A change log / diff mechanism the binding can poll for dirty cells.
    pub change_log: bool,
    /// Cell styles are exposed on the read path (IronCalc `get_style_for_cell`;
    /// Formualizer 0.7 hard-codes `style: None`).
    pub styles_on_read: bool,
}

/// The binding surface both engines implement. Every operation FreeCell's engine↔UI
/// binding needs: build/load, writes (single + batched), reads/eval (single + bulk
/// viewport), recompute, and edit→dirty tracking. All coordinates are 0-based
/// (datagen space).
pub trait SpreadsheetEngine {
    /// A short, stable engine identifier (`"formualizer"` / `"ironcalc"`) used in
    /// recorded results.
    fn name(&self) -> &'static str;

    /// A fresh, empty single-sheet workbook.
    fn new_blank() -> Self
    where
        Self: Sized;

    /// Sets a literal value at `(row, col)`.
    fn set_value(&mut self, row: u32, col: u32, v: EngineValue);

    /// Sets a formula (with leading `=`) at `(row, col)`.
    fn set_formula(&mut self, row: u32, col: u32, formula: &str);

    /// Writes a batch of inputs with a **single** recompute at the end (the batched
    /// write path — challenges the per-edit recompute cost).
    fn set_batch(&mut self, cells: &[(u32, u32, CellInput)]);

    /// Loads a **dense `rows × cols` rectangle of literal values** produced by `cell`,
    /// using the engine's **fastest native bulk-ingest path** for base values.
    ///
    /// This is the fair way to benchmark a large literal load: it lets each engine use
    /// its optimal loader — Formualizer's columnar Arrow bulk-ingest
    /// (`begin_bulk_ingest_arrow`, ~O(cells), no graph vertices / no overlay / no chunk
    /// rebuilds), IronCalc's direct `set_user_input` into its `HashMap` — instead of
    /// Formualizer's slow interactive `write_range` overlay path (which is super-linear
    /// and was the source of an earlier unfair measurement). The block occupies rows
    /// `0..rows` and cols `0..cols`; `cell(r, c)` yields each literal.
    ///
    /// The default implementation falls back to chunked [`SpreadsheetEngine::set_batch`]
    /// for engines (and the test `FakeEngine`) that have no special path.
    fn bulk_load_block(&mut self, rows: u32, cols: u32, cell: &dyn Fn(u32, u32) -> EngineValue) {
        let chunk_rows: u32 = (40_000 / cols.max(1)).max(1);
        let mut r0 = 0;
        while r0 < rows {
            let r1 = (r0 + chunk_rows).min(rows);
            let mut batch = Vec::with_capacity(((r1 - r0) as usize) * (cols as usize));
            for r in r0..r1 {
                for c in 0..cols {
                    let v = cell(r, c);
                    if v != EngineValue::Empty {
                        batch.push((r, c, CellInput::Value(v)));
                    }
                }
            }
            self.set_batch(&batch);
            r0 = r1;
        }
    }

    /// Reads the stored/cached value at `(row, col)` (no recompute).
    fn get_value(&self, row: u32, col: u32) -> EngineValue;

    /// Evaluates a single cell, pulling precedents. On engines without incremental
    /// recalc the adapter falls back to a full recompute first.
    fn evaluate_cell(&mut self, row: u32, col: u32) -> EngineValue;

    /// Reads an entire viewport in one call. Adapters use a native range API where
    /// available; otherwise a per-cell loop (the difference is a headline finding).
    fn read_viewport(&self, vp: Viewport) -> Vec<EngineValue>;

    /// Recomputes after edits. Incremental engines recompute only what changed;
    /// non-incremental engines re-evaluate the whole workbook.
    fn recompute(&mut self);

    /// Turns on change tracking (Formualizer changelog / IronCalc `UserModel`
    /// diff-list) so [`SpreadsheetEngine::drain_dirty`] can report edited cells.
    fn enable_change_tracking(&mut self);

    /// Returns the `(row, col)` addresses touched since the last drain, clearing the
    /// pending set. Best-effort per engine; drives the D3 cached binding.
    fn drain_dirty(&mut self) -> Vec<(u32, u32)>;

    /// This engine's native capabilities.
    fn caps(&self) -> EngineCaps;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_value_as_number() {
        assert_eq!(EngineValue::Number(3.0).as_number(), Some(3.0));
        assert_eq!(EngineValue::Text("x".into()).as_number(), None);
        assert_eq!(EngineValue::Empty.as_number(), None);
    }

    #[test]
    fn viewport_cell_count_and_addresses() {
        let vp = Viewport::new(10, 5, 3, 2);
        assert_eq!(vp.cell_count(), 6);
        let addrs: Vec<_> = vp.addresses().collect();
        assert_eq!(addrs.len(), 6);
        // Row-major: first row fully, then next.
        assert_eq!(addrs[0], (10, 5));
        assert_eq!(addrs[1], (10, 6));
        assert_eq!(addrs[2], (11, 5));
        assert_eq!(addrs[5], (12, 6));
    }
}
