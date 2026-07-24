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
use render_tests::{capture_available, render_all, render_charts};

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

/// The chart scenes' one-time render, independent of the grid's [`RENDERED`] so that filtering to
/// the `chart_` tests renders **only** the chart scenes (the grid `OnceLock` stays cold).
static CHART_RENDERED: OnceLock<Rendered> = OnceLock::new();

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

/// Render every chart scene once (guarded exactly like the grid [`rendered`]), returning where
/// the captures live (or why they don't). Kept separate from [`rendered`] so the two suites are
/// independent — a `chart_`-filtered run never triggers the grid render, and vice-versa.
fn chart_rendered() -> &'static Rendered {
    CHART_RENDERED.get_or_init(|| {
        let want_render = std::env::var("FREECELL_RENDER").ok().as_deref() == Some("1");
        match gate(want_render, capture_available()) {
            Gate::Skip(reason) => Rendered::Skipped(reason),
            Gate::Fail(reason) => Rendered::Failed(reason),
            Gate::Render => {
                let bin = Path::new(env!("CARGO_BIN_EXE_render_scene"));
                let out = target_subdir("chart-render-actual");
                match render_charts(bin, &out, None) {
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

/// Diff one chart scene's fresh capture against its committed baseline — the chart analogue of
/// [`check_case`], reusing the same baseline / diff / failure-artifact machinery over the chart
/// [`chart_rendered`] captures.
fn check_chart_case(name: &str) {
    let dir = match chart_rendered() {
        Rendered::Skipped(reason) => {
            eprintln!("chart render suite skipped ({name}): {reason}");
            return;
        }
        Rendered::Failed(err) => panic!("chart render-all failed for the suite: {err}"),
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
                "{name}: chart render differs from baseline — {} (artifacts in {})",
                report.summary(),
                target_subdir("render-failures").display()
            );
        }
        Err(err) => {
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
    cell_strikethrough, cell_strikethrough_underline,
    // Fill
    cell_fill_red, cell_fill_yellow, cell_fill_dark_text_contrast, cell_fill_none_explicit,
    cell_bold_fill_yellow, cell_bold_italic_underline_fill_blue, cell_fill_covers_gridlines,
    cell_fill_block_boundaries,
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
    cell_wrap_multiline_clipped,
    cell_valign_top, cell_valign_middle, cell_valign_bottom, cell_wrap_valign_bottom,
    cell_valign_top_large_font, cell_valign_bottom_large_font,
    // Auto-grow rows (Phase 7): wrap-driven growth, column-width response, manual-wins, cap clip,
    // and the retained large-font regression.
    autogrow_wrap_grows, autogrow_narrow_col_more_lines, autogrow_wide_col_fewer_lines,
    autogrow_manual_row_unchanged, autogrow_cap_clip, autogrow_large_font_grows,
    // Text spill / overflow (Phase 3): direction-aware spill, stop conditions, non-spill types
    spill_right_over_empties, spill_left_right_aligned, spill_center_both,
    spill_stop_at_nonempty, spill_over_fill_only_neighbor, spill_wrap_on_no_spill,
    spill_number_no_spill, spill_stop_at_coverage_edge,
    // Whole-grid scenes
    grid_empty_origin, grid_headers_scrolled_deep, grid_selection_single, grid_selection_range,
    grid_selection_range_spans_edge, grid_selection_shift_extended, grid_selection_drag_extended,
    grid_selection_scrolled, grid_variable_geometry, grid_loading_overlay,
    grid_scrollbars_visible, grid_mixed_content,
    // Fill handle + drag preview (gaps_closing_7_15 §3): the handle square on a range + the live
    // drag's target-region preview rectangle.
    fill_handle_multicell, fill_drag_preview,
    // Hidden rows & columns (gaps_closing_7_15 §4): a hidden row + hidden col collapse to zero size.
    hidden_row_and_col,
    // Freeze panes (freeze-panes `architecture.md §7`): the four-quadrant render — a pinned top row
    // / row band, a pinned first column / column band, the full four-quadrant split, the frozen
    // bands still showing VALUES with the body scrolled deep (the Phase-4 band-publishing proof),
    // and the freeze divider drawn even unscrolled.
    freeze_top_row, freeze_rows_band, freeze_first_col, freeze_cols_band,
    freeze_four_quadrant, freeze_scrolled_body, freeze_divider,
    // In-grid charts (P8): the ChartLayer painted over cells — a line chart in place, the Degraded
    // corner badge, the Unsupported placeholder, and a scrolled/clipped chart.
    grid_chart_line, grid_chart_degraded_badge, grid_chart_unsupported_placeholder,
    grid_chart_scrolled_clipped,
    // In-grid column chart (P22): the ChartLayer painting a real clustered column over cells.
    grid_chart_column,
    // In-grid area chart (P23): the ChartLayer painting a real standard area over cells.
    grid_chart_area,
    // In-grid pie chart (P24): the ChartLayer painting a real varyColors pie over cells.
    grid_chart_pie,
    // In-grid scatter chart (P25): the ChartLayer painting a real marker scatter over cells.
    grid_chart_scatter,
    // In-grid bubble chart (P26): the ChartLayer painting a real area-encoded bubble over cells.
    grid_chart_bubble,
    // Manipulate (P18): a selected chart with its selection outline + resize handles.
    grid_chart_selected,
    // Insert (P17/P21): the near-empty AUTHORED chart the insert flow produces (authored → in-grid).
    grid_chart_authored_inserted,
    // Editing feel (Phase 2): live mirror + in-cell editor overlay + its grow-right / grow-down
    cell_mirror_typing, incell_editor_open, incell_editor_grow_right, incell_editor_grow_wrap,
    // Fonts (Phase 5): family + size + row auto-grow
    font_family_serif, font_size_24_row_grown, font_missing_family_fallback,
    // Borders (Phase 6): edge paint, presets, shared-edge precedence
    border_all_thin, border_outer_medium, border_heavier_edge_wins, border_over_fill,
    border_shared_edge_adjacent, border_none_clear,
    // Border line patterns (Phase 2): dashed + double
    border_dashed_all, border_double_all, border_pattern_mixed,
    // Border pen (Phase 3): a pen-applied dashed + non-default-colour outer border
    border_pen_outer_dashed_red,
    // Structure (Phase 7): resized geometry + header selection
    col_resized_narrow_clips_text, row_resized_tall,
    header_full_column_selected, header_full_row_selected,
    // Chrome / formatting (Phase 8): explicit text colour + the macOS titlebar row
    text_color_red, titlebar_row,
    // Conditional formatting (P10): value-dependent CF folded into the cache (P3) — a numeric
    // highlight, a 3-color scale gradient, and a text highlight.
    cf_highlight_greater_than, cf_color_scale_3, cf_highlight_text_contains,
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

/// Generate one `#[test]` per chart scene (calling [`check_chart_case`]) + a `CHART_SCENE_NAMES`
/// slice used to guard drift against `chart_scene::all()`. Named `chart_*` so `render_tests.sh
/// test chart_` runs only these (and thus renders only the chart scenes).
macro_rules! chart_render_cases {
    ($($name:ident),+ $(,)?) => {
        const CHART_SCENE_NAMES: &[&str] = &[$(stringify!($name)),+];
        $(
            #[test]
            fn $name() { check_chart_case(stringify!($name)); }
        )+
    };
}

chart_render_cases! {
    // P4 — the make-or-break multi-series line scene that proves the chart render → capture →
    // diff path.
    chart_line_multi,
    // P5 — production line coverage: single-series, a zero-crossing nice-tick value axis,
    // legend-off (plot uses full width), and title/axis-title collapse (legend still shown).
    chart_line_single, chart_line_negative, chart_line_no_legend, chart_line_no_titles,
    // P6 — line P1 fidelity: theme colors + per-series markers + currency numFmt ticks
    // (chart_line_markers), and a smooth curve + percent numFmt ticks (chart_line_smooth).
    chart_line_markers, chart_line_smooth,
    // P12 — data labels: value labels with a currency numFmt (chart_line_value_labels), percent
    // labels / share-of-total (chart_line_percent_labels), and composed series+category+value
    // labels with a legend-key swatch (chart_line_named_labels).
    chart_line_value_labels, chart_line_percent_labels, chart_line_named_labels,
    // P13 — axis breadth & line styling: reversed category axis (chart_line_reversed), explicit
    // value-axis min/max scaling (chart_line_scaled), gridlines-off (chart_line_no_gridlines),
    // `a:ln` width/color/alpha styling (chart_line_styled), and a bottom-placed legend
    // (chart_line_legend_bottom). These also carry the tuned fonts + the true-rotated value-axis
    // title (P13 observations A/B), which move every existing chart_line_* baseline.
    chart_line_reversed, chart_line_scaled, chart_line_no_gridlines, chart_line_styled,
    chart_line_legend_bottom,
    // P22 — column & bar: clustered / stacked / 100%-stacked columns, a horizontal bar (proving the
    // reversed Excel category order), a non-default gapWidth/overlap geometry, and theme-schemeClr fills.
    chart_column_clustered, chart_column_stacked, chart_column_percent, chart_bar_clustered,
    chart_column_gap_overlap, chart_column_theme_fills,
    // P23 — area: standard (overlapping filled polygons), stacked (cumulative bands), 100%-stacked
    // (0–100% normalized), and theme-schemeClr area fills.
    chart_area_standard, chart_area_stacked, chart_area_percent, chart_area_theme_fills,
    // P24 — pie & doughnut: a varyColors pie (per-slice palette + legend), a doughnut (holeSize
    // annulus), on-slice percent labels, and a rotated + exploded slice with a c:dPt custom color.
    chart_pie_vary_colors, chart_doughnut_hole, chart_pie_percent_labels, chart_pie_exploded,
    // P25 — scatter (XY): a marker-only two-series scatter over two numeric axes, a lineMarker scatter
    // (dots + connecting straight segments), and a scatter with a non-trivial numeric X axis (X not 1..n).
    chart_scatter_markers, chart_scatter_line_markers, chart_scatter_wide_x,
    // P26 — bubble (XY + size): a multi-series area-encoded bubble over two numeric axes, and a
    // single-series bubble spanning a wide size range (proving the min/max radius clamp).
    chart_bubble_multi, chart_bubble_size_clamp,
}

/// The `chart_render_cases!` list must stay in lockstep with `chart_scene::all()` — same drift
/// guard as `case_names_match_table`, for the chart fixtures.
#[test]
fn chart_scene_names_match_table() {
    let mut table: Vec<&str> = render_tests::chart_scene::all()
        .into_iter()
        .map(|s| s.name)
        .collect();
    table.sort_unstable();
    let mut names: Vec<&str> = CHART_SCENE_NAMES.to_vec();
    names.sort_unstable();
    assert_eq!(
        names, table,
        "the chart_render_cases! macro list and chart_scene::all() have drifted — keep them in sync"
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
