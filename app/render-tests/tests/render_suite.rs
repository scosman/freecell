//! The pixel-truth suite: render each case through the real engine + real grid, capture, and
//! perceptually diff against its committed baseline (`components/render_test_harness.md §Runner`).
//!
//! One `#[test]` per case (via the `render_cases!` macro over the table) so a red CI line names
//! the exact broken feature. All cases are rendered **once** (a `OnceLock`) into a temp dir, then
//! each test diffs its case. On a diff failure the case writes
//! `target/render-failures/<name>.{actual,baseline,diff}.png` + the diff stats.
//!
//! Gating: the render step runs only when `FREECELL_RENDER=1` **and** the capture tooling
//! (`xvfb-run` + lavapipe) is present — so `cargo test --workspace` (no env var) skips it while
//! the GPUI-free diff unit tests still run, and the dedicated CI step
//! (`scripts/render_tests.sh`) runs the real pixel gate.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use render_tests::diff::{diff_image, diff_png_files, DiffOptions};
use render_tests::{capture_available, render_all};

/// The outcome of the one-time render-all.
enum Rendered {
    /// The render step was intentionally skipped (reason for the log).
    Skipped(String),
    /// Every case was captured into this directory as `<name>.png`.
    Ok(PathBuf),
    /// The render-all itself failed (all cases fail with this message).
    Failed(String),
}

static RENDERED: OnceLock<Rendered> = OnceLock::new();

fn baselines_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("baselines")
}

/// `<target>/<sub>` — derived from `CARGO_TARGET_TMPDIR` (`<target>/tmp`), so failure artifacts
/// land at the specced `target/render-failures` for CI upload.
fn target_subdir(sub: &str) -> PathBuf {
    let tmp = PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    let target = tmp.parent().unwrap_or(&tmp);
    target.join(sub)
}

/// What to do with the render step, decided from the two independent signals. Factored out of
/// [`rendered`] so the gate policy is unit-testable without the process-global env var + `OnceLock`.
#[derive(Debug, PartialEq)]
enum Gate {
    /// Don't render; skip to green (the implicit `cargo test --workspace` / macOS path).
    Skip(String),
    /// FAIL the suite — the operator asked for the pixel gate but it can't run.
    Fail(String),
    /// Render every case.
    Render,
}

/// The gate policy: `want_render` = `FREECELL_RENDER=1` (operator explicitly wants the pixel
/// suite); `capture` = [`capture_available`]. A required gate that can't test any pixels must
/// **fail**, never silently skip to green — so a missing capture stack is only tolerated (skipped)
/// when the pixel suite was not requested.
fn gate(want_render: bool, capture: bool) -> Gate {
    match (want_render, capture) {
        (false, _) => Gate::Skip(
            "set FREECELL_RENDER=1 (see scripts/render_tests.sh) to run the pixel suite".into(),
        ),
        (true, false) => Gate::Fail(
            "FREECELL_RENDER=1 but capture tooling is unavailable (needs xvfb-run + a lavapipe \
             ICD) — a required pixel gate must not silently skip; install xvfb + \
             mesa-vulkan-drivers"
                .into(),
        ),
        (true, true) => Gate::Render,
    }
}

/// Render every case once (guarded), returning where the captures live (or why they don't).
fn rendered() -> &'static Rendered {
    RENDERED.get_or_init(|| {
        let want_render = std::env::var("FREECELL_RENDER").ok().as_deref() == Some("1");
        match gate(want_render, capture_available()) {
            Gate::Skip(reason) => Rendered::Skipped(reason),
            Gate::Fail(reason) => Rendered::Failed(reason),
            Gate::Render => {
                let bin = Path::new(env!("CARGO_BIN_EXE_render_scene"));
                let out = target_subdir("render-actual");
                match render_all(bin, &out, None) {
                    Ok(_) => Rendered::Ok(out),
                    Err(err) => Rendered::Failed(format!("{err:#}")),
                }
            }
        }
    })
}

/// The committed baseline path for `name`, or an **actionable** error (telling the human to
/// regenerate) when it is missing — not a panic deep in the diff. Pure (no display), so the
/// missing-baseline contract is testable without rendering.
fn require_baseline(name: &str) -> Result<PathBuf, String> {
    let baseline = baselines_dir().join(format!("{name}.png"));
    if baseline.exists() {
        Ok(baseline)
    } else {
        Err(format!(
            "no committed baseline for `{name}` — run `scripts/render_tests.sh generate` on the \
             pinned runner image, eyeball the PNG, and commit it (render-tests/README.md)"
        ))
    }
}

/// Diff one case's fresh capture against its committed baseline, writing failure artifacts + the
/// diff stats on a mismatch.
fn check_case(name: &str) {
    let dir = match rendered() {
        Rendered::Skipped(reason) => {
            eprintln!("render suite skipped ({name}): {reason}");
            return;
        }
        Rendered::Failed(err) => panic!("render-all failed for the suite: {err}"),
        Rendered::Ok(dir) => dir,
    };

    let actual = dir.join(format!("{name}.png"));
    let baseline = require_baseline(name).unwrap_or_else(|msg| panic!("{msg}"));

    let opts = DiffOptions::default();
    match diff_png_files(&baseline, &actual, &opts) {
        Ok(report) if report.passed => {}
        Ok(report) => {
            write_failure_artifacts(name, &baseline, &actual, &opts);
            panic!(
                "{name}: render differs from baseline — {} (artifacts in {})",
                report.summary(),
                target_subdir("render-failures").display()
            );
        }
        Err(err) => {
            // A dimension mismatch (a size change) is a hard failure; still surface the pair.
            write_failure_artifacts(name, &baseline, &actual, &opts);
            panic!(
                "{name}: {err:#} (artifacts in {})",
                target_subdir("render-failures").display()
            );
        }
    }
}

/// Copy the baseline + actual and write a magenta-highlighted diff into `target/render-failures/`.
fn write_failure_artifacts(name: &str, baseline: &Path, actual: &Path, opts: &DiffOptions) {
    let dir = target_subdir("render-failures");
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let _ = std::fs::copy(baseline, dir.join(format!("{name}.baseline.png")));
    let _ = std::fs::copy(actual, dir.join(format!("{name}.actual.png")));
    if let (Ok(b), Ok(a)) = (image::open(baseline), image::open(actual)) {
        let diff = diff_image(&b.to_rgba8(), &a.to_rgba8(), opts);
        let _ = diff.save(dir.join(format!("{name}.diff.png")));
    }
}

/// Generate one `#[test]` per case name + a `CASE_NAMES` slice used to guard table drift.
macro_rules! render_cases {
    ($($name:ident),+ $(,)?) => {
        const CASE_NAMES: &[&str] = &[$(stringify!($name)),+];
        $(
            #[test]
            fn $name() { check_case(stringify!($name)); }
        )+
    };
}

render_cases! {
    // Text attributes
    cell_plain, cell_bold, cell_italic, cell_underline,
    cell_bold_italic, cell_bold_underline, cell_italic_underline, cell_bold_italic_underline,
    // Fill
    cell_fill_red, cell_fill_yellow, cell_fill_dark_text_contrast, cell_fill_none_explicit,
    cell_bold_fill_yellow, cell_bold_italic_underline_fill_blue, cell_fill_covers_gridlines,
    // Values & engine-owned number formats
    cell_number_plain, cell_number_thousands, cell_number_currency, cell_number_percent,
    cell_number_negative_red, cell_date_default, cell_boolean, cell_text_plain,
    // Formula errors
    cell_error_div0, cell_error_name, cell_error_circ,
    // Layout / alignment / geometry
    cell_align_left_text, cell_align_right_number, cell_number_align_left,
    cell_align_center_explicit,
    cell_align_explicit_overrides_default, cell_text_clipped, cell_text_exact_fit,
    cell_empty_styled, cell_tall_row, cell_wide_column, cell_narrow_column_clipped_number,
    // Whole-grid scenes
    grid_empty_origin, grid_headers_scrolled_deep, grid_selection_single, grid_selection_range,
    grid_selection_range_spans_edge, grid_selection_shift_extended, grid_selection_drag_extended,
    grid_selection_scrolled, grid_variable_geometry, grid_loading_overlay,
    grid_scrollbars_visible, grid_mixed_content,
    // Editing feel (Phase 2): live mirror + in-cell editor overlay
    cell_mirror_typing, incell_editor_open,
    // Fonts (Phase 5): family + size + row auto-grow
    font_family_serif, font_size_24_row_grown, font_missing_family_fallback,
    // Borders (Phase 6): edge paint, presets, shared-edge precedence
    border_all_thin, border_outer_medium, border_heavier_edge_wins, border_over_fill,
    border_shared_edge_adjacent, border_none_clear,
    // Structure (Phase 7): resized geometry + header selection
    col_resized_narrow_clips_text, row_resized_tall,
    header_full_column_selected, header_full_row_selected,
    // Chrome / formatting (Phase 8): explicit text colour + the macOS titlebar row
    text_color_red, titlebar_row,
}

/// The `#[test]` name list must stay in lockstep with the case table — a new case added to
/// `cases::all()` without a test row (or vice-versa) fails here rather than silently skipping.
#[test]
fn case_names_match_table() {
    let mut table: Vec<&str> = render_tests::cases::all()
        .into_iter()
        .map(|c| c.name)
        .collect();
    table.sort_unstable();
    let mut names: Vec<&str> = CASE_NAMES.to_vec();
    names.sort_unstable();
    assert_eq!(
        names, table,
        "the render_cases! macro list and cases::all() have drifted — keep them in sync"
    );
}

/// The gate policy (`gate`) — a required pixel gate must never silently skip to green. Runs with
/// no display (pure function), so it guards the policy on every `cargo test --workspace`.
#[test]
fn gate_skips_only_when_render_not_requested() {
    // FREECELL_RENDER unset → skip regardless of tooling (the implicit / macOS path).
    assert!(matches!(gate(false, false), Gate::Skip(_)));
    assert!(matches!(gate(false, true), Gate::Skip(_)));
}

#[test]
fn gate_fails_when_requested_but_tooling_missing() {
    // The regression this guards: FREECELL_RENDER=1 + no capture stack used to Skip (pass) — a
    // "required" gate testing zero pixels. It must now FAIL.
    match gate(true, false) {
        Gate::Fail(msg) => assert!(msg.contains("must not silently skip"), "message: {msg}"),
        other => panic!("expected a hard failure, got {other:?}"),
    }
}

#[test]
fn gate_renders_when_requested_and_available() {
    assert_eq!(gate(true, true), Gate::Render);
}

/// A missing baseline fails **actionably** (telling the human to regenerate), not with a panic
/// deep in the diff. Exercises the real `require_baseline` path with a bogus name — no display
/// needed, so it runs even when the pixel suite is skipped. A committed case (`cell_plain`)
/// resolves, proving the check is not vacuously erroring.
#[test]
fn baseline_missing_fails_actionably() {
    let err = require_baseline("definitely_not_a_real_case_zzz")
        .expect_err("a missing baseline must be reported, not resolved");
    assert!(err.contains("no committed baseline"), "message: {err}");
    assert!(
        err.contains("render_tests.sh generate"),
        "message must be actionable: {err}"
    );
    assert!(
        require_baseline("cell_plain").is_ok(),
        "a committed baseline must resolve"
    );
}
