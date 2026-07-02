//! A trivial in-crate [`SpreadsheetEngine`] used **only** by `binding_common`'s own
//! tests. It keeps the `common` crate engine-neutral (no real engine dependency)
//! while still letting us assert that scenarios, binding designs, and the report
//! writer behave correctly end-to-end.
//!
//! It is deliberately minimal: a `HashMap` value store plus enough formula support
//! (`=<addr>+1` linear-chain links and `=SUM(A1:B2)` rectangles) to make the cascade
//! scenarios produce real, checkable numbers. It is **not** a spreadsheet engine and
//! is never used for recorded benchmarks — those run against Formualizer / IronCalc.

use std::collections::HashMap;

use crate::engine::{CellInput, EngineCaps, EngineValue, SpreadsheetEngine, Viewport};

/// A cell's stored definition.
#[derive(Debug, Clone)]
enum Def {
    Value(EngineValue),
    Formula(String),
}

/// A tiny HashMap-backed engine for `common`'s tests. Supports `=<A1>+1` and
/// `=SUM(<A1>:<B2>)` so linear-chain and wide-fanout cascades evaluate.
#[derive(Debug, Default)]
pub struct FakeEngine {
    defs: HashMap<(u32, u32), Def>,
    values: HashMap<(u32, u32), EngineValue>,
    dirty: Vec<(u32, u32)>,
    tracking: bool,
}

impl FakeEngine {
    /// A fresh engine (concrete constructor so tests don't need the trait in scope).
    pub fn new_blank_impl() -> Self {
        Self::default()
    }

    fn mark_dirty(&mut self, row: u32, col: u32) {
        if self.tracking {
            self.dirty.push((row, col));
        }
    }

    /// Recomputes every formula cell in dependency-agnostic passes until values
    /// stabilise (fine for the small acyclic shapes the tests use).
    fn recompute_all(&mut self) {
        // Seed literal values.
        for (addr, def) in &self.defs {
            if let Def::Value(v) = def {
                self.values.insert(*addr, v.clone());
            }
        }
        // Iterate formula evaluation to a fixed point (chains resolve in order).
        let formula_addrs: Vec<(u32, u32)> = self
            .defs
            .iter()
            .filter(|(_, d)| matches!(d, Def::Formula(_)))
            .map(|(a, _)| *a)
            .collect();
        for _ in 0..formula_addrs.len().max(1) {
            let mut changed = false;
            for &addr in &formula_addrs {
                if let Some(Def::Formula(f)) = self.defs.get(&addr).cloned().as_ref() {
                    let v = self.eval_formula(f);
                    if self.values.get(&addr) != Some(&v) {
                        self.values.insert(addr, v);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
    }

    fn eval_formula(&self, formula: &str) -> EngineValue {
        let body = formula.strip_prefix('=').unwrap_or(formula);
        if let Some(rest) = body.strip_prefix("SUM(") {
            let inner = rest.trim_end_matches(')');
            return self.eval_sum(inner);
        }
        // Literal number, e.g. "=1" or "=42".
        if let Ok(n) = body.parse::<f64>() {
            return EngineValue::Number(n);
        }
        // "<addr>+1" linear-chain link.
        if let Some((lhs, rhs)) = body.split_once('+') {
            let base = self.value_at_a1(lhs.trim()).and_then(|v| v.as_number());
            let add = rhs.trim().parse::<f64>().ok();
            if let (Some(b), Some(a)) = (base, add) {
                return EngineValue::Number(b + a);
            }
        }
        // Bare cell reference.
        if let Some(v) = self.value_at_a1(body.trim()) {
            return v;
        }
        EngineValue::Error("#PARSE".into())
    }

    fn eval_sum(&self, range: &str) -> EngineValue {
        let (start, end) = match range.split_once(':') {
            Some((s, e)) => (s, e),
            None => (range, range),
        };
        let Some((r0, c0)) = parse_a1(start) else {
            return EngineValue::Error("#REF".into());
        };
        let Some((r1, c1)) = parse_a1(end) else {
            return EngineValue::Error("#REF".into());
        };
        let mut sum = 0.0;
        for r in r0.min(r1)..=r0.max(r1) {
            for c in c0.min(c1)..=c0.max(c1) {
                if let Some(EngineValue::Number(n)) = self.values.get(&(r, c)) {
                    sum += n;
                }
            }
        }
        EngineValue::Number(sum)
    }

    fn value_at_a1(&self, a1: &str) -> Option<EngineValue> {
        let (r, c) = parse_a1(a1)?;
        self.values.get(&(r, c)).cloned()
    }
}

/// Parses an "A1" reference to 0-based `(row, col)`, e.g. `"A1" -> (0,0)`,
/// `"B3" -> (2,1)`.
fn parse_a1(a1: &str) -> Option<(u32, u32)> {
    let a1 = a1.trim();
    let split = a1.find(|ch: char| ch.is_ascii_digit())?;
    let (letters, digits) = a1.split_at(split);
    if letters.is_empty() || digits.is_empty() {
        return None;
    }
    let mut col: u32 = 0;
    for ch in letters.chars() {
        let d = (ch.to_ascii_uppercase() as u8).checked_sub(b'A')? as u32 + 1;
        col = col * 26 + d;
    }
    let row: u32 = digits.parse().ok()?;
    Some((row.checked_sub(1)?, col.checked_sub(1)?))
}

impl SpreadsheetEngine for FakeEngine {
    fn name(&self) -> &'static str {
        "fake"
    }

    fn new_blank() -> Self {
        Self::default()
    }

    fn set_value(&mut self, row: u32, col: u32, v: EngineValue) {
        self.defs.insert((row, col), Def::Value(v.clone()));
        self.values.insert((row, col), v);
        self.mark_dirty(row, col);
        self.recompute_all();
    }

    fn set_formula(&mut self, row: u32, col: u32, formula: &str) {
        self.defs
            .insert((row, col), Def::Formula(formula.to_string()));
        self.mark_dirty(row, col);
        self.recompute_all();
    }

    fn set_batch(&mut self, cells: &[(u32, u32, CellInput)]) {
        for (r, c, input) in cells {
            match input {
                CellInput::Value(v) => {
                    self.defs.insert((*r, *c), Def::Value(v.clone()));
                    self.values.insert((*r, *c), v.clone());
                }
                CellInput::Formula(f) => {
                    self.defs.insert((*r, *c), Def::Formula(f.clone()));
                }
            }
            self.mark_dirty(*r, *c);
        }
        self.recompute_all();
    }

    fn get_value(&self, row: u32, col: u32) -> EngineValue {
        self.values
            .get(&(row, col))
            .cloned()
            .unwrap_or(EngineValue::Empty)
    }

    fn evaluate_cell(&mut self, row: u32, col: u32) -> EngineValue {
        self.recompute_all();
        self.get_value(row, col)
    }

    fn read_viewport(&self, vp: Viewport) -> Vec<EngineValue> {
        vp.addresses().map(|(r, c)| self.get_value(r, c)).collect()
    }

    fn recompute(&mut self) {
        self.recompute_all();
    }

    fn enable_change_tracking(&mut self) {
        self.tracking = true;
    }

    fn drain_dirty(&mut self) -> Vec<(u32, u32)> {
        std::mem::take(&mut self.dirty)
    }

    fn caps(&self) -> EngineCaps {
        EngineCaps {
            native_range_read: false,
            incremental_recalc: true,
            parallel_eval: false,
            change_log: true,
            styles_on_read: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_a1_mapping() {
        assert_eq!(parse_a1("A1"), Some((0, 0)));
        assert_eq!(parse_a1("B3"), Some((2, 1)));
        assert_eq!(parse_a1("AA1"), Some((0, 26)));
        assert_eq!(parse_a1(""), None);
        assert_eq!(parse_a1("A"), None);
    }

    #[test]
    fn linear_chain_cascades() {
        let mut e = FakeEngine::new_blank_impl();
        e.set_formula(0, 0, "=1");
        e.set_formula(1, 0, "=A1+1");
        e.set_formula(2, 0, "=A2+1");
        assert_eq!(e.get_value(2, 0), EngineValue::Number(3.0));
        // Edit head -> cascade.
        e.set_formula(0, 0, "=10");
        assert_eq!(e.get_value(2, 0), EngineValue::Number(12.0));
    }

    #[test]
    fn sum_evaluates() {
        let mut e = FakeEngine::new_blank_impl();
        e.set_value(0, 0, EngineValue::Number(2.0));
        e.set_value(0, 1, EngineValue::Number(3.0));
        e.set_formula(0, 2, "=SUM(A1:B1)");
        assert_eq!(e.get_value(0, 2), EngineValue::Number(5.0));
    }

    #[test]
    fn change_tracking_reports_dirty() {
        let mut e = FakeEngine::new_blank_impl();
        e.enable_change_tracking();
        e.set_value(3, 4, EngineValue::Number(1.0));
        e.set_value(7, 2, EngineValue::Number(2.0));
        let dirty = e.drain_dirty();
        assert!(dirty.contains(&(3, 4)));
        assert!(dirty.contains(&(7, 2)));
        // Draining clears the set.
        assert!(e.drain_dirty().is_empty());
    }
}
