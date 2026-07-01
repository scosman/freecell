//! Discriminating-power tests for the perceptual-diff harness (Investigation C GATE:
//! the diff must PASS on identical + within-tolerance-perturbed images and FAIL on a
//! genuine change). All fixtures are **code-generated** (no committed binaries) so the
//! in-container proof is reproducible; the macOS baseline PNG is committed by the human.

use image::{Rgba, RgbaImage};

use ci_rendering::{diff_images, diff_png_files, DiffOptions};

const W: u32 = 200;
const H: u32 = 120;

/// A deterministic "grid-like" scene: white background, grey vertical gridlines every 20px
/// (a proxy for cell borders / text edges — the AA-sensitive high-frequency content), and a
/// solid highlighted block (a proxy for a filled cell). Pure function of its inputs.
fn base_scene() -> RgbaImage {
    let mut img = RgbaImage::from_pixel(W, H, Rgba([255, 255, 255, 255]));
    for x in (0..W).step_by(20) {
        for y in 0..H {
            img.put_pixel(x, y, Rgba([180, 180, 180, 255]));
        }
    }
    // A "highlighted cell" block.
    for y in 40..70 {
        for x in 60..120 {
            img.put_pixel(x, y, Rgba([255, 249, 196, 255]));
        }
    }
    img
}

/// Applies a **within-tolerance** perturbation: a small sub-`tolerance` per-channel jitter
/// on the gridline/edge pixels (the AA/font-rasterization proxy). No pixel moves; no pixel
/// changes by more than `delta` levels, so with `delta < tolerance` every pixel stays
/// under the per-channel bar and none is counted as differing.
fn perturb_subtolerance(img: &RgbaImage, delta: u8) -> RgbaImage {
    let mut out = img.clone();
    for (i, (_, _, px)) in img.enumerate_pixels().enumerate() {
        // Only jitter the darker (edge/gridline) pixels, alternating sign, so it looks like
        // AA noise on high-frequency content rather than a uniform shift.
        if px.0[0] < 200 {
            let sign = if i % 2 == 0 { 1i16 } else { -1i16 };
            let mut np = *px;
            for ch in 0..3 {
                let v = np.0[ch] as i16 + sign * delta as i16;
                np.0[ch] = v.clamp(0, 255) as u8;
            }
            let (x, y) = (i as u32 % W, i as u32 / W);
            out.put_pixel(x, y, np);
        }
    }
    out
}

/// Applies a **genuine content change**: recolor the highlighted block to a clearly
/// different colour (a real render regression proxy — e.g. a fill/style change).
fn genuine_change(img: &RgbaImage) -> RgbaImage {
    let mut out = img.clone();
    for y in 40..70 {
        for x in 60..120 {
            out.put_pixel(x, y, Rgba([40, 90, 220, 255])); // was pale yellow -> now blue
        }
    }
    out
}

#[test]
fn identical_images_pass() {
    let a = base_scene();
    let opts = DiffOptions::default();
    let report = diff_images(&a, &a, &opts).expect("same dims");
    assert!(
        report.passed,
        "identical images must pass: {}",
        report.summary()
    );
    assert_eq!(report.differing_pixels, 0);
    assert_eq!(report.max_channel_delta, 0);
}

#[test]
fn within_tolerance_perturbation_passes() {
    let a = base_scene();
    let opts = DiffOptions::default();
    // Jitter strictly below the tolerance so no pixel crosses the per-channel bar.
    let delta = opts.per_channel_tolerance - 1;
    let b = perturb_subtolerance(&a, delta);
    let report = diff_images(&a, &b, &opts).expect("same dims");
    assert!(
        report.passed,
        "AA/font-like sub-tolerance perturbation must pass: {}",
        report.summary()
    );
    // The perturbation is visible in the max delta but never exceeds tolerance, so nothing
    // is counted as differing.
    assert_eq!(report.differing_pixels, 0);
    assert!(report.max_channel_delta <= opts.per_channel_tolerance);
    assert!(
        report.max_channel_delta > 0,
        "perturbation should be non-trivial"
    );
}

#[test]
fn genuine_change_fails() {
    let a = base_scene();
    let b = genuine_change(&a);
    let opts = DiffOptions::default();
    let report = diff_images(&a, &b, &opts).expect("same dims");
    assert!(
        !report.passed,
        "a genuine recolored-block change must fail: {}",
        report.summary()
    );
    // The changed block is 60x30 = 1800 px out of 200x120 = 24000 => 7.5% >> 0.5%.
    assert!(report.differing_fraction > opts.fail_fraction * 5.0);
    assert!(report.max_channel_delta > opts.per_channel_tolerance);
}

#[test]
fn dimension_mismatch_errors() {
    let a = base_scene();
    let b = RgbaImage::from_pixel(W + 1, H, Rgba([255, 255, 255, 255]));
    let err = diff_images(&a, &b, &DiffOptions::default());
    assert!(
        err.is_err(),
        "a size change must be a hard error, not fuzzy pass"
    );
}

#[test]
fn threshold_is_discriminating() {
    // Prove the metric is not a rubber stamp: a perturbation touching just UNDER
    // fail_fraction of pixels (at supra-tolerance amplitude) passes; touching just OVER
    // fails. We construct exact differing-pixel counts around the threshold.
    let a = base_scene();
    let total = (W * H) as f64;
    let opts = DiffOptions::default();
    let threshold_pixels = (opts.fail_fraction * total).floor() as u32; // e.g. 120 px

    // Change exactly `threshold_pixels` pixels by a supra-tolerance amount => at the bar,
    // fraction == fail_fraction => PASS (`<=`).
    let mut at = a.clone();
    change_n_pixels(&mut at, threshold_pixels, 60);
    let r_at = diff_images(&a, &at, &opts).expect("same dims");
    assert_eq!(r_at.differing_pixels as u32, threshold_pixels);
    assert!(
        r_at.passed,
        "at the threshold must pass: {}",
        r_at.summary()
    );

    // One more differing pixel => over the bar => FAIL.
    let mut over = a.clone();
    change_n_pixels(&mut over, threshold_pixels + 1, 60);
    let r_over = diff_images(&a, &over, &opts).expect("same dims");
    assert_eq!(r_over.differing_pixels as u32, threshold_pixels + 1);
    assert!(
        !r_over.passed,
        "just over the threshold must fail: {}",
        r_over.summary()
    );
}

#[test]
fn png_roundtrip_diff() {
    // The file-loading path (diff_png_files) must agree with the in-memory diff.
    let dir = tempfile::tempdir().expect("tempdir");
    let a = base_scene();
    let b = genuine_change(&a);
    let pa = dir.path().join("a.png");
    let pb = dir.path().join("b.png");
    a.save(&pa).expect("save a");
    b.save(&pb).expect("save b");

    let opts = DiffOptions::default();
    let from_files = diff_png_files(&pa, &pb, &opts).expect("diff files");
    let in_memory = diff_images(&a, &b, &opts).expect("diff mem");
    assert_eq!(from_files, in_memory, "file path must match in-memory diff");
    assert!(!from_files.passed);
}

/// Sets the first `n` pixels (row-major) to a colour a fixed `amp` above the source, i.e. a
/// supra-tolerance change on exactly `n` pixels. Used to pin the threshold behavior.
fn change_n_pixels(img: &mut RgbaImage, n: u32, amp: u8) {
    let mut changed = 0u32;
    'outer: for y in 0..H {
        for x in 0..W {
            if changed >= n {
                break 'outer;
            }
            let p = *img.get_pixel(x, y);
            let np = Rgba([p.0[0].saturating_sub(amp), p.0[1], p.0[2], p.0[3]]);
            img.put_pixel(x, y, np);
            changed += 1;
        }
    }
}
