---
status: draft
---

# Component: Cell-Render Test Harness (`render-tests/`)

The automated pixel-truth suite for the grid: renders the **real** grid component
offscreen on macOS, captures PNGs, and compares against committed baselines with a
perceptual diff. Mechanism validated end-to-end in round-3 C
(`experiments/round-3/C-ci-rendering/findings.md`; demo GATE closed on macOS
2026-07-02). This is a first-class deliverable of the MVP (per the overview): big
suite, reusable infra, names a human can read.

## Purpose and scope

**Does:** per-feature cell rendering tests (`cell_bold`, …) + whole-grid scene tests;
baseline generation tooling; the perceptual diff; failure artifacts (actual/diff PNGs)
uploaded on CI failure; README documenting the human baseline workflow.

**Does not:** perf measurement (perf harness's job); chrome/dialog screenshots (P2 if
ever); non-macOS operation (headless GPUI capture is Metal/macOS-only at our rev —
confirmed, don't fight it).

## Mechanism (locked by round-3 C — do not redesign)

- Offscreen capture: GPUI's visual-test path — `show: false` window,
  `current_headless_renderer()` → capture to image → PNG. Exactly Zed's own
  `visual_test_runner.rs` approach at the pinned rev; the ported harness code from
  `experiments/round-3/C-ci-rendering/render-grid/` is the starting point (port, not
  path-dependency).
- Perceptual diff (GPUI-free, ported from C's `ci_rendering` crate): per-channel
  tolerance **12/255**, failing-pixel fraction threshold **0.5%**; both constants live
  in one place and get re-tuned once real-grid baselines exist (C's guidance).
- Baselines are **committed PNGs** captured on the **same runner class CI uses**
  (pin `macos-14` or the chosen image; record in the README). Local baselines from a
  dev Mac are for eyeballing only unless the machine matches. Cell text renders in
  the **bundled Inter** font (UI round decision), so font-version drift — C's top
  flakiness risk — is out of the picture; residual cross-machine variance is Metal
  AA only.

## Test definition model (the extensibility requirement)

One declarative table; adding a rendering feature = adding rows.

```rust
pub struct RenderCase {
    pub name: &'static str,              // snake_case, IS the baseline filename
    pub scene: Scene,                    // fixture: cells + styles + geometry + selection + scroll
    pub viewport: (u32, u32),            // px, small & tight (e.g. 480×160 for cell cases)
}

pub struct Scene { /* built via a fluent fixture builder: */ }
Scene::cells(4, 3)                        // rows, cols (small grids)
    .cell(1, 1, "42.50")                  // literal input → engine
    .style(1, 1, bold())                  // via the real engine + real style cache
    .col_width(1, 140.0)
    .selection(Sel::single(1, 1))         // optional
```

Scenes run through the **real stack**: an in-memory workbook via `freecell-engine`
(new empty → apply inputs/styles), real `SheetCaches`, real `Publication`, real
`GridView` — so a pixel test failing means the product is wrong, not a stub.

### Case inventory (initial suite — every MVP feature + meaningful permutations)

Text attrs: `cell_plain`, `cell_bold`, `cell_italic`, `cell_underline`,
`cell_bold_italic`, `cell_bold_underline`, `cell_italic_underline`,
`cell_bold_italic_underline`.
Fill: `cell_fill_red`, `cell_fill_yellow`, `cell_fill_dark_text_contrast`,
`cell_fill_none_explicit`, `cell_bold_fill_yellow`,
`cell_bold_italic_underline_fill_blue`, `cell_fill_covers_gridlines`.
Values/formats (engine-owned display): `cell_number_plain`, `cell_number_thousands`,
`cell_number_currency`, `cell_number_percent`, `cell_number_negative_red`
(format-color path), `cell_date_default`, `cell_boolean`, `cell_text_plain`.
Errors: `cell_error_div0`, `cell_error_name`, `cell_error_circ`.
Layout: `cell_align_left_text`, `cell_align_right_number`, `cell_align_center_explicit`,
`cell_align_explicit_overrides_default`, `cell_text_clipped`,
`cell_text_exact_fit`, `cell_empty_styled` (fill on empty cell), `cell_tall_row`,
`cell_wide_column`, `cell_narrow_column_clipped_number`.
Grid scenes (from `components/grid.md`): `grid_empty_origin`,
`grid_headers_scrolled_deep`, `grid_selection_single`, `grid_selection_range`,
`grid_selection_range_spans_edge`, `grid_variable_geometry`, `grid_loading_overlay`,
`grid_mixed_content` (a busy realistic scene — the canary that catches "everything
subtly moved").

~45 cases initially. Every future rendering feature (borders, fonts, wrap, …) must add
its rows in the same table — stated in the README as a review requirement.

## Runner & tooling

- `cargo test -p render-tests` (macOS): one `#[test]` per case via a small macro over
  the table; each renders → captures → diffs → on failure writes
  `target/render-failures/<name>.{actual,baseline,diff}.png` and fails with the diff
  stats. CI uploads `render-failures/` as an artifact.
- `cargo run -p render-tests --bin generate_baselines [-- --only <prefix>]`:
  re-renders every (or filtered) case into `render-tests/baselines/`, prints a
  changed/unchanged summary.
- `render-tests/README.md` (required content): the human process — run
  `generate_baselines` on the pinned runner class (or accept the risk locally),
  **visually inspect every changed PNG** (open the folder, look), commit baselines
  together with the code change that moved pixels; never regenerate to "make CI
  green" without eyeballing; how to read a failure artifact; how to add a case; the
  tolerance constants and when re-tuning is allowed.

## Dependencies

Depends on: `freecell-app` (GridView), `freecell-engine`, gpui (visual test context),
`image`/`png`. Nothing depends on it. macOS-only crate (excluded from Linux CI
targets).

## Test plan (tests about the harness itself)

- `diff_identical_passes` / `diff_real_change_fails`: C's proven cases ported (the
  3.47%-change scene must FAIL at 0.5%).
- `diff_tolerance_boundary`: synthetic images at exactly the tolerance edge.
- `baseline_missing_fails_actionably`: a case without a baseline fails telling the
  human to run generate_baselines (not a panic).
- Linux-safe unit tests for the diff math live in a tiny GPUI-free module so the
  perceptual-diff logic itself is CI-tested everywhere.
