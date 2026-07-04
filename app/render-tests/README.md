# `render-tests` — cell-render snapshot suite

The automated **pixel-truth** suite for the grid: renders the **real** `GridView`
(`freecell-app`) over scenes produced by the **real** engine (`freecell-engine` worker /
`DocumentClient`), captures PNGs on Linux under **Xvfb + Mesa lavapipe** (software Vulkan),
and compares them against committed baselines with a **perceptual diff** (ported from
round-3 C `ci_rendering`). A pixel test failing means the product is wrong, not a stub —
scenes run engine → `Publication` + `SheetCaches` → grid → pixels. Full design:
`../../specs/projects/mvp/components/render_test_harness.md`.

## How it runs

```sh
# From app/. Runs the full suite (assert every case matches its baseline):
render-tests/scripts/render_tests.sh test
#   ≡  FREECELL_RENDER=1 cargo test -p render-tests

# Regenerate baselines/ (then eyeball + commit — see below):
render-tests/scripts/render_tests.sh generate [--only <prefix>]
```

- **One `#[test]` per case** (via the `render_cases!` macro over the case table in
  `tests/render_suite.rs`), so a red CI line names the exact broken feature. All cases are
  rendered once into `target/render-actual/`, then each test perceptual-diffs its case.
- **Capture (the Phase-1 spike mechanism, per case).** Each case renders in its **own**
  `xvfb-run` display sized to the case viewport. This is load-bearing: gpui/lavapipe only
  *presents* a window's frame when the window nearly fills the screen (a small window on a
  large screen captures blank), so the Xvfb is sized per case. Inside it the harness
  launches `render_scene`, waits for the first paint, runs **`xrefresh`** to force the
  Expose that makes gpui present (Xvfb has no compositor to emit one), finds the grid
  window by size, and captures it with ImageMagick `import -window <id>`.
- **Gating.** The pixel render runs only when `FREECELL_RENDER=1` **and** the capture tools
  are present. So a plain `cargo test --workspace` (no env var) skips the pixel cases while
  the GPUI-free perceptual-diff unit tests (`tests/perceptual_diff.rs`) still run; CI runs
  the real gate via `render_tests.sh` (a required step in `checks.yml`).

## Pinned baseline environment (record on every re-baseline)

Baselines are captured on and validated against **this exact image**; software
rasterization + the (Phase-10) bundled Inter font make them bit-stable. Re-baseline only on
this class of runner:

| | |
|---|---|
| Runner image | `ubuntu-24.04` (GitHub Actions), locally Ubuntu 24.04.4 LTS |
| Rust toolchain | `1.95.0` (`app/rust-toolchain.toml`) |
| Mesa (lavapipe) | `mesa-vulkan-drivers` 25.2.8-0ubuntu0.24.04.2 → device `llvmpipe (LLVM 20.1.2, 256 bits)` |
| Vulkan loader | `libvulkan1` 1.3.275.0 |

Dev-machine renders (a real GPU) are for **eyeballing only** — never commit them as
baselines; their anti-aliasing differs from lavapipe.

## The human baseline process (a review requirement)

Baselines are **committed PNGs** and must be trustworthy:

1. Regenerate on the pinned runner image (CI artifact job, or a matching container):
   `render-tests/scripts/render_tests.sh generate [--only <prefix>]` — prints a
   **NEW / CHANGED / unchanged** summary.
2. **Visually inspect every NEW/CHANGED PNG.** Open `render-tests/baselines/` and look.
   Never regenerate baselines to "make CI green" without eyeballing what moved.
3. Commit the baselines **together with** the code change that moved the pixels, with a
   message saying why they changed.

## Adding a rendering feature / case

Every new rendering feature (borders, fonts, wrap, …) **must add its rows** to the
declarative case table — one axis or meaningful permutation per case, snake_case names:

1. Add the `RenderCase` to `src/cases.rs` (`cases::all()`).
2. Add the case name to the `render_cases! { … }` list in `tests/render_suite.rs`
   (`case_names_match_table` fails the build if the two drift).
3. Regenerate its baseline, **eyeball it**, and commit it in the same PR.

Scenes drive the real worker: values/formulas/errors and number formats via `SetCellInput`
(IronCalc **infers** currency/percent/thousands/date from the input string), and
bold/italic/underline/fill via `SetStyleAttr` (the real style-cache mirror). Render
features the MVP worker protocol has **no edit command** for — alignment, explicit font
colour, column/row geometry (in the product these come from an opened file) — are applied
to the real `SheetCache` the grid consumes, via its public mutators.

> **Note (`cell_number_negative_red`).** The `[Red]` number-format *colour* is not yet
> published end-to-end (the worker publishes `text_color = None`, deferred at Phase 4), so
> that baseline currently shows the negative number correctly formatted in the default
> colour. When `text_color` is wired, regenerate + eyeball it.

## Reading a CI failure

On failure the job uploads `target/render-failures/` as an artifact, with per case:
`<name>.baseline.png` (expected), `<name>.actual.png` (what rendered), and `<name>.diff.png`
(differing pixels highlighted magenta over a dimmed copy), plus the printed diff stats
(differing fraction, max channel delta).

## Tolerance constants — when to re-tune

The tolerance lives in **one place**: `DiffOptions::default()` in `src/diff.rs` —
**per-channel 12/255**, **failing-pixel fraction 0.5%**. Re-tune **only** with real
baselines in hand and a committed rationale. Lavapipe is deterministic here (the whole
suite re-renders bit-stably within tolerance), so if it proves bit-exact, **tighten** rather
than loosen. Treat observed nondeterminism as a bug to investigate, not tolerance to widen.
