//! Tolerance-based perceptual image diff — a **faithful refactor** of round-3 C
//! (`experiments/round-3/C-ci-rendering/src/lib.rs`), whose 6-assertion discriminating-power
//! proof closed the C GATE. The metric and both tolerance constants are identical; the refactor
//! extracts the per-pixel `pixel_delta` helper and adds `diff_image` (the magenta failure
//! visualization). Pure Rust (`image` only), GPUI-free, so it builds, tests, and runs in the
//! headless Linux container with no GPU/display.
//!
//! # Why a two-part metric (tolerance AND fraction)
//!
//! A rendering test must tolerate **anti-aliasing / font-rasterization** wiggle (a few levels
//! of sub-pixel difference on glyph/gridline edges is not a regression) while still catching a
//! **genuine change** (a moved/recolored cell). Neither half alone is enough:
//!
//! - A pure changed-pixel **count** false-fails on AA edges (every edge pixel "differs").
//! - A pure per-pixel **tolerance** can pass a large, low-amplitude global shift.
//!
//! So a pixel counts as *differing* only if some channel differs by more than
//! `per_channel_tolerance`, and the images PASS only if the **fraction** of differing pixels is
//! `<= fail_fraction`. AA/font noise stays under both bars; a real regression trips the
//! fraction. This is the same metric shape Zed's own visual tests use.

use std::path::Path;

use anyhow::{bail, Context, Result};
use image::{Rgba, RgbaImage};

/// Tolerance knobs for the perceptual diff. Both constants live **here, in one place**, and
/// get re-tuned only with real baselines in hand (see `render-tests/README.md`); if lavapipe
/// proves bit-exact, tighten rather than loosen (`components/render_test_harness.md`).
#[derive(Debug, Clone, Copy)]
pub struct DiffOptions {
    /// A pixel counts as *differing* only if some channel's absolute delta exceeds this
    /// (0..=255). Absorbs anti-aliasing / font-rasterization sub-pixel wiggle.
    pub per_channel_tolerance: u8,
    /// The images PASS only if the fraction of differing pixels is `<=` this (0.0..=1.0).
    /// Absorbs a small number of genuinely-changed edge pixels without letting a real content
    /// change through.
    pub fail_fraction: f64,
}

impl Default for DiffOptions {
    /// Defaults tuned for a spreadsheet-grid snapshot: ~5% per-channel wiggle tolerated per
    /// pixel, and up to 0.5% of pixels allowed to differ before we call it a regression
    /// (`components/render_test_harness.md §Mechanism`: 12/255, 0.5%).
    fn default() -> Self {
        Self {
            per_channel_tolerance: 12,
            fail_fraction: 0.005,
        }
    }
}

/// The outcome of a perceptual diff — enough to explain a pass/fail, not just a boolean.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiffReport {
    pub width: u32,
    pub height: u32,
    pub total_pixels: u64,
    /// Pixels whose max channel delta exceeded `per_channel_tolerance`.
    pub differing_pixels: u64,
    /// `differing_pixels / total_pixels`.
    pub differing_fraction: f64,
    /// The largest single-channel delta seen anywhere (diagnostic; not a gate on its own).
    pub max_channel_delta: u8,
    /// PASS iff `differing_fraction <= opts.fail_fraction`.
    pub passed: bool,
}

impl DiffReport {
    /// A one-line human summary for logs / CI output.
    pub fn summary(&self) -> String {
        format!(
            "{}x{} px: {} differing ({:.4}%), max channel delta {} -> {}",
            self.width,
            self.height,
            self.differing_pixels,
            self.differing_fraction * 100.0,
            self.max_channel_delta,
            if self.passed { "PASS" } else { "FAIL" },
        )
    }
}

/// Whether a pixel pair differs beyond `per_channel_tolerance` on some channel, and by how much.
fn pixel_delta(pa: &Rgba<u8>, pb: &Rgba<u8>) -> u8 {
    let mut max = 0u8;
    for ch in 0..4 {
        let delta = pa.0[ch].abs_diff(pb.0[ch]);
        if delta > max {
            max = delta;
        }
    }
    max
}

/// Perceptually diffs two in-memory RGBA images with the tolerance metric above.
///
/// Errors if the images differ in dimensions (a size change is a hard failure the caller must
/// handle explicitly, not a fuzzy one).
pub fn diff_images(a: &RgbaImage, b: &RgbaImage, opts: &DiffOptions) -> Result<DiffReport> {
    if a.dimensions() != b.dimensions() {
        bail!(
            "image dimensions differ: {:?} vs {:?}",
            a.dimensions(),
            b.dimensions()
        );
    }

    let (width, height) = a.dimensions();
    let total_pixels = (width as u64) * (height as u64);

    let mut differing_pixels: u64 = 0;
    let mut max_channel_delta: u8 = 0;

    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let pixel_max_delta = pixel_delta(pa, pb);
        if pixel_max_delta > max_channel_delta {
            max_channel_delta = pixel_max_delta;
        }
        if pixel_max_delta > opts.per_channel_tolerance {
            differing_pixels += 1;
        }
    }

    let differing_fraction = if total_pixels == 0 {
        0.0
    } else {
        differing_pixels as f64 / total_pixels as f64
    };
    let passed = differing_fraction <= opts.fail_fraction;

    Ok(DiffReport {
        width,
        height,
        total_pixels,
        differing_pixels,
        differing_fraction,
        max_channel_delta,
        passed,
    })
}

/// Loads two PNGs from disk and perceptually diffs them (the file-based CI entry point).
pub fn diff_png_files(a: &Path, b: &Path, opts: &DiffOptions) -> Result<DiffReport> {
    let img_a = image::open(a)
        .with_context(|| format!("loading baseline image {}", a.display()))?
        .to_rgba8();
    let img_b = image::open(b)
        .with_context(|| format!("loading comparison image {}", b.display()))?
        .to_rgba8();
    diff_images(&img_a, &img_b, opts)
}

/// A failure-artifact visualization: the actual image dimmed to grey, with every pixel that
/// exceeds `per_channel_tolerance` painted solid magenta — so a human opening `<name>.diff.png`
/// sees exactly *where* the pixels moved (`components/render_test_harness.md §Runner`).
/// Falls back to a plain copy of `actual` on a dimension mismatch (which the caller already
/// reports as a hard failure).
pub fn diff_image(baseline: &RgbaImage, actual: &RgbaImage, opts: &DiffOptions) -> RgbaImage {
    if baseline.dimensions() != actual.dimensions() {
        return actual.clone();
    }
    let (w, h) = actual.dimensions();
    let mut out = RgbaImage::new(w, h);
    for (x, y, pa) in actual.enumerate_pixels() {
        let pb = baseline.get_pixel(x, y);
        let out_px = if pixel_delta(pa, pb) > opts.per_channel_tolerance {
            Rgba([0xFF, 0x00, 0xFF, 0xFF]) // magenta = a differing pixel
        } else {
            // Dim the unchanged content to grey so the magenta pops.
            let g = ((pa.0[0] as u16 + pa.0[1] as u16 + pa.0[2] as u16) / 3) as u8;
            let dim = 96 + (g as u16 * 96 / 255) as u8; // 96..=192 grey ramp
            Rgba([dim, dim, dim, 0xFF])
        };
        out.put_pixel(x, y, out_px);
    }
    out
}
