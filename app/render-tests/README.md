# `render-tests` — cell-render snapshot suite

The automated pixel-truth suite for the grid: renders the **real** `GridView` over tiny
fixtures, captures PNGs, and compares them against committed baselines with a perceptual
diff. Full design: `../../specs/projects/mvp/components/render_test_harness.md`.

> **Phase 1 status:** skeleton only. The gpui/grid dependencies, the ported round-3 C
> perceptual diff, the declarative `RenderCase` table, `generate_baselines`, and the
> initial ~45-case suite land in **Phase 7**. This README documents the process now so it
> is in place when the suite arrives; the capture path it depends on is validated by the
> Phase-1 render spike (`../scripts/linux_render_spike.sh`).

## How it runs (Phase 7)

- `cargo test -p render-tests` — one `#[test]` per case: render → capture → perceptual
  diff → on failure, write `target/render-failures/<name>.{actual,baseline,diff}.png`.
- **Linux CI:** under Xvfb + Mesa lavapipe (software Vulkan) — deterministic software
  rasterization. **macOS:** offscreen Metal capture (round-3 C's validated fallback).
- The perceptual diff (per-channel tolerance **12/255**, failing-pixel fraction
  **0.5%**) is GPUI-free and unit-tested everywhere.

## The human baseline process (a review requirement)

Baselines are **committed PNGs** and must be trustworthy:

1. Regenerate on the **pinned CI runner image + Mesa/lavapipe version** (via a CI
   artifact job or a matching local container). Dev-machine renders are for eyeballing
   only — never commit them as baselines.
   ```sh
   cargo run -p render-tests --bin generate_baselines [-- --only <prefix>]
   ```
2. **Visually inspect every changed PNG.** Open the folder and look. Never regenerate
   baselines to "make CI green" without eyeballing what moved.
3. Commit the baselines **together with** the code change that moved the pixels, with a
   message saying why they changed.

## Adding a rendering feature

Every new rendering feature (borders, fonts, wrap, …) **must add its rows** to the
declarative `RenderCase` table (one axis or meaningful permutation per case, snake_case
names so a red CI line names the exact broken feature) and its baselines in the same PR.

## Reading a CI failure

On failure the job uploads `render-failures/` as an artifact:
`<name>.actual.png` (what rendered), `<name>.baseline.png` (expected),
`<name>.diff.png` (highlighted differences), plus the printed diff stats
(differing fraction, max channel delta).

## Tolerance constants — when to re-tune

The tolerance (12/255) and fraction (0.5%) live in one place. Re-tune **only** with real
baselines in hand and a committed rationale. If lavapipe proves bit-exact, **tighten**
rather than loosen. Pinned runner image + Mesa version and the bundled Inter font keep
baselines bit-stable; treat observed nondeterminism as a bug, not a reason to widen
tolerance.
