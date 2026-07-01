//! FreeCell Round-3 Investigation C — CI snapshot rendering. Scaffolding stub only.
//!
//! This crate is authored for a **macOS / human run** (GPUI needs a real GPU). No GPUI
//! or GPU dependencies are wired at scaffolding time — building GPUI in-container is out
//! of scope and *whether a headless capture path even exists* is part of C's own
//! investigation. See README.md and specs/projects/freecell-phase-3 architecture §5, §7.
//!
//! Phase C implements: (in-container) investigate GPUI's offscreen/headless capture
//! surface and attempt it — expected to fail with no GPU, the failure mode is the
//! finding; (macOS/human-run) render the raw-gpui grid to a PNG, commit a baseline, and
//! run a tolerance-based perceptual diff of a re-render (must pass) and a
//! deliberately-changed scene (must fail) to prove discriminating power.

fn main() {
    println!("TODO(Phase C): CI snapshot rendering (render -> PNG -> perceptual diff). Authored for macOS/human run; GPUI deps added in Phase C. Scaffolding stub.");
}
