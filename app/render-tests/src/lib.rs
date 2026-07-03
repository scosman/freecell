//! `render-tests` — the cell-render snapshot suite (skeleton).
//!
//! Full design: `specs/projects/mvp/components/render_test_harness.md`. This crate
//! renders the **real** grid over tiny fixtures, captures PNGs, and perceptually diffs
//! them against committed baselines under `baselines/`, running in Linux CI via Xvfb +
//! Mesa lavapipe (software Vulkan).
//!
//! Phase 1 ships only this skeleton. Phase 7 wires the real dependencies
//! (`freecell-app`'s `GridView`, `freecell-engine`, `gpui`, `image`), ports the
//! round-3 C perceptual diff (per-channel tolerance 12/255, fail fraction 0.5%), builds
//! the declarative `RenderCase` table + `generate_baselines`, and lands the initial
//! ~45-case suite. See `README.md` for the human baseline process.

/// Placeholder marker so the skeleton crate compiles and is a real workspace member.
/// Replaced by the harness API (`RenderCase`, `Scene`, the diff) in Phase 7.
pub const PHASE_1_SKELETON: &str = "render-tests wired in Phase 7";
