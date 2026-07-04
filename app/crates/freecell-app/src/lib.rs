//! `freecell-app` library surface.
//!
//! The GPUI application is a binary (`src/main.rs`), but the custom [`grid`] component is
//! also exposed as a library so `render-tests` (Phase 7) and the perf harness (Phase 12)
//! can render the **real** grid over hand-built fixtures (`architecture.md §1` crate
//! dependency rule: `render-tests → freecell-app (grid)`). Nothing engine-specific lives
//! here; the grid reads only `freecell-core` read models (`components/grid.md`).

pub mod chrome;
pub mod grid;
pub mod shell;
