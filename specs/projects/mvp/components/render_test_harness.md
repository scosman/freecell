---
status: draft
---

# Component: Cell-Render Test Harness (`render-tests/`)

The automated pixel-truth suite for the grid: renders the **real** grid component,
captures PNGs, and compares against committed baselines with a perceptual diff.
**Runs in Linux CI** (architecture-round product call) via software-rendered Vulkan;
the perceptual-diff mechanism was validated end-to-end in round-3 C
(`experiments/round-3/C-ci-rendering/findings.md`), whose macOS capture harness is
the fallback path. This is a first-class deliverable of the MVP (per the overview):
big suite, reusable infra, names a human can read.

## Purpose and scope

**Does:** per-feature cell rendering tests (`cell_bold`, …) + whole-grid scene tests;
baseline generation tooling; the perceptual diff; failure artifacts (actual/diff PNGs)
uploaded on CI failure; README documenting the human baseline workflow.

**Does not:** perf measurement (perf harness's job); chrome/dialog screenshots (P2 if
ever).

## Mechanism

**Primary (Linux CI): Xvfb + Mesa lavapipe (software Vulkan) + GPUI's blade/Vulkan
backend.** Deterministic software rasterization + bundled Inter should make
baselines bit-stable across runs — better than macOS Metal-AA variance. The capture
step is the one unvalidated link (round-3 C's offscreen
`current_headless_renderer()` path is Metal/macOS-only at our rev), resolved by the
**Phase-1 spike**, in preference order:
1. A GPUI capture/screenshot API that works on the Linux backend at our rev, if one
   exists (`show:false` preferred).
2. Render to a normal window under Xvfb and capture the X root/window pixels
   (`xwd`/XGetImage-class capture) after a settle frame.
3. **Fallback if neither works:** the round-3 C macOS offscreen-Metal harness,
   verbatim (validated), run as a manual-dispatch/cron macOS workflow; Linux CI
   then runs everything except pixels. The suite's case table, diff, and baseline
   process are identical in every variant — only the capture function swaps.

- Perceptual diff (GPUI-free, ported from C's `ci_rendering` crate): per-channel
  tolerance **12/255**, failing-pixel fraction threshold **0.5%**; both constants live
  in one place and get re-tuned once real-grid baselines exist (C's guidance). If
  lavapipe proves bit-exact, tighten rather than loosen.
- Baselines are **committed PNGs** captured on the **pinned CI image + Mesa
  version** (record both in the README). Regenerate via a CI artifact job or a
  matching container locally; dev-Mac renders are for eyeballing only. Cell text
  renders in the **bundled Inter** font (UI round decision), so font-version drift —
  C's top flakiness risk — is out of the picture on every platform.

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

- `cargo test -p render-tests` (Linux CI under Xvfb+lavapipe; also runs on a dev
  machine with a real GPU for eyeballing): one `#[test]` per case via a small macro over
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
`image`/`png`. Nothing depends on it. Builds on Linux + macOS; the capture function
is the only platform-conditional code.

## Test plan (tests about the harness itself)

- `diff_identical_passes` / `diff_real_change_fails`: C's proven cases ported (the
  3.47%-change scene must FAIL at 0.5%).
- `diff_tolerance_boundary`: synthetic images at exactly the tolerance edge.
- `baseline_missing_fails_actionably`: a case without a baseline fails telling the
  human to run generate_baselines (not a panic).
- Linux-safe unit tests for the diff math live in a tiny GPUI-free module so the
  perceptual-diff logic itself is CI-tested everywhere.
