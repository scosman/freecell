//! Engine-neutral formula-pattern generators for cascade/propagation benchmarks
//! (architecture §5, functional_spec §6.C).
//!
//! These emit `(address, formula-string)` pairs so any engine the Sub-project A
//! gate selects can be fed the same well-known dependency shapes. Nothing here
//! parses or evaluates formulas — that is the engine's job in the perf sub-project.

use crate::cell::CellAddress;

/// A single formula-bearing cell: where it lives and the A1-style formula text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormulaCell {
    pub addr: CellAddress,
    /// The formula text, including the leading `=`, e.g. `"=A1+1"`.
    pub formula: String,
}

/// Builds a linear dependency chain of length `len` down a single column: the head
/// cell is a literal seed (`=1`) and every subsequent cell is `=<prev>+1`
/// (functional_spec §5.4 "1,000,000-cell `=PREV+1` chain").
///
/// Returned lazily so callers can drive a very long (1M+) chain without holding it
/// all in memory. Cells are yielded top-to-bottom starting at row 0 of `col`.
///
/// A `len` of `0` yields nothing.
pub fn linear_chain(len: u32, col: u32) -> impl Iterator<Item = FormulaCell> {
    (0..len).map(move |row| {
        let addr = CellAddress::new(row, col);
        let formula = if row == 0 {
            "=1".to_string()
        } else {
            let prev = CellAddress::new(row - 1, col).a1();
            format!("={prev}+1")
        };
        FormulaCell { addr, formula }
    })
}

/// Builds a wide fan-out shape: `dependents` cells that each sum the same block of
/// `sources` source cells. Exercises a single edit propagating to many dependents
/// (architecture §5 "wide fan-out").
///
/// Sources are laid out across row 0 (columns `0..sources`). Dependents are laid
/// out down column `sources` (rows `0..dependents`), each computing
/// `=SUM(A1:<lastSource>1)`. Returns sources first, then dependents.
///
/// `sources == 0` yields only the (degenerate, empty-range) dependents; callers
/// that need a non-trivial shape should pass `sources >= 1`.
pub fn wide_fanout(sources: u32, dependents: u32) -> Vec<FormulaCell> {
    let mut cells = Vec::with_capacity(sources as usize + dependents as usize);

    // Source literals across the top row.
    for c in 0..sources {
        cells.push(FormulaCell {
            addr: CellAddress::new(0, c),
            formula: format!("={}", c + 1),
        });
    }

    // Each dependent sums the whole source range.
    let range = if sources == 0 {
        // Degenerate: reference the first source cell that would exist.
        CellAddress::new(0, 0).a1()
    } else {
        let first = CellAddress::new(0, 0).a1();
        let last = CellAddress::new(0, sources - 1).a1();
        format!("{first}:{last}")
    };
    for d in 0..dependents {
        cells.push(FormulaCell {
            addr: CellAddress::new(d, sources),
            formula: format!("=SUM({range})"),
        });
    }

    cells
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_chain_formulas() {
        let cells: Vec<FormulaCell> = linear_chain(5, 0).collect();
        assert_eq!(cells.len(), 5);
        // Head is a seed literal, not a reference.
        assert_eq!(cells[0].addr, CellAddress::new(0, 0));
        assert_eq!(cells[0].formula, "=1");
        // Subsequent cells reference the previous cell in A1 form.
        assert_eq!(cells[1].formula, "=A1+1");
        assert_eq!(cells[2].formula, "=A2+1");
        assert_eq!(cells[4].formula, "=A4+1");
        // Chain runs down the requested column.
        assert!(cells.iter().all(|c| c.addr.col == 0));
    }

    #[test]
    fn linear_chain_uses_requested_column() {
        let cells: Vec<FormulaCell> = linear_chain(3, 2).collect();
        assert_eq!(cells[0].addr, CellAddress::new(0, 2));
        assert_eq!(cells[1].formula, "=C1+1");
    }

    #[test]
    fn linear_chain_zero_len_is_empty() {
        assert_eq!(linear_chain(0, 0).count(), 0);
    }

    #[test]
    fn linear_chain_is_lazy_and_cheap_for_large_len() {
        // Should not allocate a 1M vector; just count the first few.
        let mut it = linear_chain(1_000_000, 0);
        assert_eq!(it.next().unwrap().formula, "=1"); // row 0 consumed
        // After consuming row 0, nth(999_998) skips rows 1..=999_998 and returns
        // row 999_999 — the last cell of a 1,000,000-long chain.
        assert_eq!(it.nth(999_998).unwrap().formula, "=A999999+1");
        assert!(it.next().is_none());
    }

    #[test]
    fn wide_fanout_shape() {
        let cells = wide_fanout(4, 3);
        // 4 sources + 3 dependents.
        assert_eq!(cells.len(), 7);
        // Sources across the top row.
        assert_eq!(cells[0].addr, CellAddress::new(0, 0));
        assert_eq!(cells[3].addr, CellAddress::new(0, 3));
        // Dependents sum the full source range and sit in the next column.
        let dep = &cells[4];
        assert_eq!(dep.addr, CellAddress::new(0, 4));
        assert_eq!(dep.formula, "=SUM(A1:D1)");
        assert!(cells[5..].iter().all(|c| c.formula == "=SUM(A1:D1)"));
        assert_eq!(cells[6].addr, CellAddress::new(2, 4));
    }

    #[test]
    fn wide_fanout_single_source() {
        let cells = wide_fanout(1, 2);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[1].formula, "=SUM(A1:A1)");
    }
}
