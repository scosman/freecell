//! FreeCell Round-3 Investigation A (the crux) — style/geometry cache sync +
//! structural editing. Scaffolding stub only.
//!
//! Phase A implements (see specs/projects/freecell-phase-3 architecture §4):
//!   - a `UserModel` probe (insert/delete rows/cols, undo/redo, copy/paste, diff-list,
//!     `Send`-ness — does the SP1 worker seam still hold?),
//!   - a correctness harness asserting structural edits shift references + band styles +
//!     row/col sizes (and `.xlsx` round-trip),
//!   - a resident-cache-shift prototype that shifts on insert/delete and provably agrees
//!     with IronCalc's re-read authoritative state, reversible for undo,
//!   - structural-edit + cache-shift cost at 10^5–10^6 (foreground, force+assert).

fn main() {
    println!("TODO(Phase A): cache-sync + structural-editing investigation. Scaffolding stub.");
}
