//! The three candidate engine↔UI binding designs (architecture §5), written
//! generically over [`SpreadsheetEngine`] so *both* engines run the *same* D1/D2/D3
//! logic and the numbers compare directly:
//!
//! - **D1 Naive per-cell** — pull each visible cell via a single-cell read.
//! - **D2 Bulk/range** — pull the visible rectangle in one `read_viewport` call.
//! - **D3 Cached + changelog** — a [`BindingCache`] holds the visible window; reads
//!   hit the cache; after an edit only `dirty ∩ visible` is re-pulled and refreshed.
//!
//! Each design exposes the same operation — "produce the values for a viewport" —
//! so a scenario can time all three identically.

use std::collections::HashMap;

use crate::engine::{EngineValue, SpreadsheetEngine, Viewport};

/// The binding design under test. Selects how a viewport's values are produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Design {
    /// D1: one single-cell read per visible cell.
    NaivePerCell,
    /// D2: one bulk `read_viewport` call.
    BulkRange,
    /// D3: cached window refreshed from the changelog/dirty set.
    CachedChangelog,
}

impl Design {
    /// A stable short label used in recorded results (`"D1"`, `"D2"`, `"D3"`).
    pub fn label(self) -> &'static str {
        match self {
            Design::NaivePerCell => "D1",
            Design::BulkRange => "D2",
            Design::CachedChangelog => "D3",
        }
    }

    /// The read/cascade designs to benchmark, in order.
    pub const ALL: [Design; 3] = [
        Design::NaivePerCell,
        Design::BulkRange,
        Design::CachedChangelog,
    ];
}

/// D1: pull every visible cell one at a time via [`SpreadsheetEngine::get_value`].
pub fn read_viewport_d1(engine: &impl SpreadsheetEngine, vp: Viewport) -> Vec<EngineValue> {
    vp.addresses()
        .map(|(r, c)| engine.get_value(r, c))
        .collect()
}

/// D2: pull the whole viewport in a single [`SpreadsheetEngine::read_viewport`] call.
pub fn read_viewport_d2(engine: &impl SpreadsheetEngine, vp: Viewport) -> Vec<EngineValue> {
    engine.read_viewport(vp)
}

/// A tiny binding cache holding the currently-visible window's values. Backs the D3
/// design: reads are served from the map; after an edit only the dirty cells that
/// fall inside the current window are re-pulled from the engine.
#[derive(Debug, Default)]
pub struct BindingCache {
    window: HashMap<(u32, u32), EngineValue>,
    viewport: Option<Viewport>,
}

impl BindingCache {
    /// A fresh, empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The viewport currently primed into the cache, if any.
    pub fn viewport(&self) -> Option<Viewport> {
        self.viewport
    }

    /// (Re)primes the cache for `vp` by bulk-reading the viewport once. Called on a
    /// scroll/viewport change.
    pub fn prime(&mut self, engine: &impl SpreadsheetEngine, vp: Viewport) {
        let values = engine.read_viewport(vp);
        self.window.clear();
        for ((r, c), v) in vp.addresses().zip(values) {
            self.window.insert((r, c), v);
        }
        self.viewport = Some(vp);
    }

    /// Serves a cached value; falls back to a single-cell read on a miss (so a read
    /// is always correct even for a cell outside the primed window).
    pub fn read(&self, engine: &impl SpreadsheetEngine, row: u32, col: u32) -> EngineValue {
        match self.window.get(&(row, col)) {
            Some(v) => v.clone(),
            None => engine.get_value(row, col),
        }
    }

    /// Returns the cached values for the primed viewport in row-major order,
    /// falling back to reads for any missing cell.
    pub fn snapshot(&self, engine: &impl SpreadsheetEngine, vp: Viewport) -> Vec<EngineValue> {
        vp.addresses()
            .map(|(r, c)| self.read(engine, r, c))
            .collect()
    }

    /// Refreshes only the dirty cells that fall inside the primed viewport, by
    /// re-reading them from the engine. Dirty cells outside the window are ignored
    /// (they aren't visible).
    ///
    /// **Correctness note.** This is only sufficient when the `dirty` set already
    /// includes every *downstream* cell whose computed value changed. A plain edit
    /// changelog (both engines) reports only the **edit sites**, not the cascaded
    /// dependents — so for a change that cascades into the window, use
    /// [`BindingCache::refresh_after_edits`], which conservatively re-primes when an
    /// edit lands outside the window (a potential upstream precedent).
    pub fn apply_dirty(&mut self, engine: &impl SpreadsheetEngine, dirty: &[(u32, u32)]) {
        let Some(vp) = self.viewport else { return };
        for &(r, c) in dirty {
            if in_viewport(vp, r, c) {
                self.window.insert((r, c), engine.get_value(r, c));
            }
        }
    }

    /// Correct D3 refresh after a batch of edits whose `dirty` set is the **edit
    /// sites** (what both engines' change feeds actually report):
    ///
    /// - Edits *inside* the window are refreshed in place (cheap).
    /// - If any edit landed *outside* the window, it may be an upstream precedent of a
    ///   visible cell, so we **re-prime** the whole window (one bulk read) to pick up
    ///   the cascade. This keeps the visible values correct without a precise
    ///   downstream-dirty subscription (which neither engine exposes) — and is exactly
    ///   why a pure edit-log doesn't let D3 beat D2 on a cascade (a headline finding).
    ///
    /// Returns `true` if a full re-prime happened.
    pub fn refresh_after_edits(
        &mut self,
        engine: &impl SpreadsheetEngine,
        dirty: &[(u32, u32)],
    ) -> bool {
        let Some(vp) = self.viewport else {
            return false;
        };
        let any_offscreen_edit = dirty.iter().any(|&(r, c)| !in_viewport(vp, r, c));
        if any_offscreen_edit {
            self.prime(engine, vp);
            true
        } else {
            self.apply_dirty(engine, dirty);
            false
        }
    }
}

/// Whether `(row, col)` lies inside `vp`.
pub fn in_viewport(vp: Viewport, row: u32, col: u32) -> bool {
    row >= vp.row0 && row < vp.row0 + vp.rows && col >= vp.col0 && col < vp.col0 + vp.cols
}

/// Produces a viewport's values under the chosen [`Design`], priming a fresh cache
/// for D3. Used by scenarios that time a cold viewport pull under each design.
pub fn read_under(
    design: Design,
    engine: &impl SpreadsheetEngine,
    vp: Viewport,
) -> Vec<EngineValue> {
    match design {
        Design::NaivePerCell => read_viewport_d1(engine, vp),
        Design::BulkRange => read_viewport_d2(engine, vp),
        Design::CachedChangelog => {
            let mut cache = BindingCache::new();
            cache.prime(engine, vp);
            cache.snapshot(engine, vp)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::CellInput;
    use crate::fake::FakeEngine;

    fn seeded_engine() -> FakeEngine {
        let mut e = FakeEngine::new_blank_impl();
        for r in 0..8 {
            for c in 0..8 {
                e.set_value(r, c, EngineValue::Number((r * 100 + c) as f64));
            }
        }
        e
    }

    #[test]
    fn d1_d2_d3_agree() {
        let e = seeded_engine();
        let vp = Viewport::new(2, 2, 4, 4);
        let d1 = read_under(Design::NaivePerCell, &e, vp);
        let d2 = read_under(Design::BulkRange, &e, vp);
        let d3 = read_under(Design::CachedChangelog, &e, vp);
        assert_eq!(d1, d2);
        assert_eq!(d2, d3);
        assert_eq!(d1.len(), vp.cell_count());
    }

    #[test]
    fn binding_cache_reads_after_prime() {
        let e = seeded_engine();
        let vp = Viewport::new(0, 0, 3, 3);
        let mut cache = BindingCache::new();
        cache.prime(&e, vp);
        assert_eq!(cache.viewport(), Some(vp));
        assert_eq!(cache.read(&e, 1, 1), EngineValue::Number(101.0));
    }

    #[test]
    fn binding_cache_apply_dirty_refreshes_only_visible() {
        let mut e = seeded_engine();
        let vp = Viewport::new(0, 0, 3, 3);
        let mut cache = BindingCache::new();
        cache.prime(&e, vp);

        // Edit a visible cell (1,1) and an offscreen cell (5,5).
        e.set_batch(&[
            (1, 1, CellInput::Value(EngineValue::Number(999.0))),
            (5, 5, CellInput::Value(EngineValue::Number(888.0))),
        ]);
        cache.apply_dirty(&e, &[(1, 1), (5, 5)]);

        // Visible cell refreshed from cache.
        assert_eq!(cache.read(&e, 1, 1), EngineValue::Number(999.0));
        // The offscreen dirty cell was never inserted into the window.
        assert!(!cache.window.contains_key(&(5, 5)));
    }

    #[test]
    fn refresh_after_edits_reprimes_on_offscreen_edit() {
        // A linear chain: head (0,0) is offscreen; the visible window is at the tail.
        let mut e = FakeEngine::new_blank_impl();
        for r in 0..10u32 {
            let f = if r == 0 {
                "=1".to_string()
            } else {
                format!("=A{}+1", r) // A<r> is the previous row in 1-based A1
            };
            e.set_formula(r, 0, &f);
        }
        e.enable_change_tracking();
        let _ = e.drain_dirty();

        let vp = Viewport::new(7, 0, 3, 1); // rows 7..10, offscreen from head
        let mut cache = BindingCache::new();
        cache.prime(&e, vp);
        assert_eq!(cache.read(&e, 9, 0), EngineValue::Number(10.0));

        // Edit the offscreen head; the cache must re-prime to reflect the cascade.
        e.set_value(0, 0, EngineValue::Number(100.0));
        let dirty = e.drain_dirty();
        let reprimed = cache.refresh_after_edits(&e, &dirty);
        assert!(reprimed, "an offscreen edit should force a re-prime");
        // Tail now reflects the cascade: head(100) + 9 == 109.
        assert_eq!(cache.read(&e, 9, 0), EngineValue::Number(109.0));
    }

    #[test]
    fn refresh_after_edits_in_place_when_only_visible() {
        let mut e = seeded_engine();
        let vp = Viewport::new(0, 0, 3, 3);
        let mut cache = BindingCache::new();
        cache.prime(&e, vp);
        e.set_value(1, 1, EngineValue::Number(777.0));
        let reprimed = cache.refresh_after_edits(&e, &[(1, 1)]);
        assert!(!reprimed, "a purely in-window edit should refresh in place");
        assert_eq!(cache.read(&e, 1, 1), EngineValue::Number(777.0));
    }

    #[test]
    fn in_viewport_bounds() {
        let vp = Viewport::new(10, 10, 5, 5);
        assert!(in_viewport(vp, 10, 10));
        assert!(in_viewport(vp, 14, 14));
        assert!(!in_viewport(vp, 15, 10));
        assert!(!in_viewport(vp, 9, 10));
    }
}
