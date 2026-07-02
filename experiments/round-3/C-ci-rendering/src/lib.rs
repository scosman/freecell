//! FreeCell Round-3 Investigation C — CI snapshot rendering.
//!
//! This library is the **GPUI-independent, in-container-authoritative** half of C: a
//! **tolerance-based perceptual image diff** used to compare a freshly-rendered grid PNG
//! against a committed baseline (functional_spec §6-C, architecture §3 / §5-C). It is pure
//! Rust (`image` crate only) so it builds, tests, and runs in the headless Linux container
//! with no GPU/display.
//!
//! The other half — actually rendering the GPUI grid to a PNG — needs a real GPU and (in
//! the pinned Zed rev) is **macOS/Metal-only**, so it lives in the `render_grid` bin behind
//! the `mac-render` feature and is run by a human on macOS. See `README.md` and
//! `findings.md`.
//!
//! # Why a two-part metric (tolerance AND fraction)
//!
//! A rendering test must tolerate **anti-aliasing / font-rasterization** wiggle (a few
//! levels of sub-pixel difference on glyph/gridline edges is not a regression) while still
//! catching a **genuine change** (a moved/recolored cell). Neither half alone is enough:
//!
//! - A pure changed-pixel **count** false-fails on AA edges (every edge pixel "differs").
//! - A pure per-pixel **tolerance** can pass a large, low-amplitude global shift.
//!
//! So a pixel is counted as *differing* only if some channel differs by more than
//! `per_channel_tolerance`, and the images PASS only if the **fraction** of differing
//! pixels is `<= fail_fraction`. AA/font noise stays under both bars; a real regression
//! trips the fraction. This is the discriminating power the C GATE requires.

use std::path::Path;

use anyhow::{bail, Context, Result};
use image::RgbaImage;

/// Tolerance knobs for the perceptual diff.
#[derive(Debug, Clone, Copy)]
pub struct DiffOptions {
    /// A pixel counts as *differing* only if some channel's absolute delta exceeds this
    /// (0..=255). Absorbs anti-aliasing / font-rasterization sub-pixel wiggle.
    pub per_channel_tolerance: u8,
    /// The images PASS only if the fraction of differing pixels is `<=` this (0.0..=1.0).
    /// Absorbs a small number of genuinely-changed edge pixels without letting a real
    /// content change through.
    pub fail_fraction: f64,
}

impl Default for DiffOptions {
    /// Defaults tuned for a spreadsheet-grid snapshot: ~5% per-channel wiggle tolerated
    /// per pixel, and up to 0.5% of pixels allowed to differ before we call it a
    /// regression. These are the starting point the macOS run confirms/tunes against real
    /// Metal AA (findings.md notes this).
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

/// Perceptually diffs two in-memory RGBA images with the tolerance metric above.
///
/// Errors if the images differ in dimensions (a size change is a hard failure the caller
/// must handle explicitly, not a fuzzy one).
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

    // Zip the raw channel buffers (RGBA8 => 4 bytes/pixel). Both buffers are the same
    // length because dimensions match.
    for (pa, pb) in a.pixels().zip(b.pixels()) {
        let mut pixel_max_delta: u8 = 0;
        for ch in 0..4 {
            let delta = pa.0[ch].abs_diff(pb.0[ch]);
            if delta > pixel_max_delta {
                pixel_max_delta = delta;
            }
        }
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
