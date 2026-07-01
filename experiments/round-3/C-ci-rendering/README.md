# C — CI snapshot rendering

FreeCell's north star includes **rendering tests vs known-good PNGs**, which depends on
being able to **capture a snapshot of the GPUI grid in CI** and **perceptually diff** it
against a baseline (fuzzy match: anti-aliasing / font differences must not false-fail; a
real change must fail). Investigation C confirms the mechanism.

This folder has **two halves** (see `findings.md` for the graded result):

- **`src/lib.rs` + `tests/` — the in-container, authoritative half.** A pure-Rust,
  GPUI-independent, tolerance-based **perceptual-diff harness**. Builds, tests, and runs in
  the headless Linux container. This is the same metric shape Zed's own visual tests use
  (per-channel tolerance + a differing-pixel fraction). **Proven here** (6 tests).
- **`render-grid/` — the macOS/human-run half.** A GPUI program that renders a minimal
  grid **offscreen** (no visible window) via GPUI's headless capture surface and writes a
  PNG, then diffs it via the parent crate. GPUI's headless renderer is **macOS/Metal-only**
  in the pinned Zed rev, so this is a **separate package** whose GPUI git dependency is only
  ever resolved/built on macOS — the in-container crate never references it.

## Layout

```
C-ci-rendering/
  Cargo.toml            # ci_rendering lib (image + anyhow only; NO gpui) -- builds in-container
  src/lib.rs            # the perceptual-diff harness (diff_images / diff_png_files)
  tests/perceptual_diff.rs   # 6 discriminating-power tests (identical/within-tol/changed/...)
  render-grid/          # SEPARATE macOS package (its own Cargo.toml with the gpui git dep)
    Cargo.toml
    src/main.rs         # offscreen render -> PNG + --diff subcommand
  scripts/render_and_diff.sh   # the macOS run that closes the GATE end-to-end
  results/              # baseline.png etc. committed here by the human after the Mac run
  findings.md
```

## In-container (what runs here, authoritative)

```sh
# from experiments/round-3/C-ci-rendering/
cargo test        # 6 perceptual-diff tests pass; NO gpui is built
```

The `render-grid/` package does **not** build here: the container proxy denies GitHub git
access (HTTP 403) so the pinned Zed `gpui` source cannot even be fetched, and GPUI needs a
GPU/display regardless. That failure is a **finding**, not a blocker — see `findings.md`.

## HUMAN RUN REQUIRED (macOS)

The end-to-end render→PNG→diff demonstration that fully closes the GATE runs on a
**macOS/Metal** machine. Please run it and report back.

**0. One-time setup.** Rust (recent stable) + Xcode CLT (`xcode-select --install`). The
first build compiles the pinned Zed `gpui` from source — a long one-time build, **not a
hang**.

**1. Render + diff (closes the GATE):** from `experiments/round-3/C-ci-rendering/`:

```sh
./scripts/render_and_diff.sh
```

This renders offscreen three times and runs two perceptual diffs:

- `results/baseline.png`  — the known-good snapshot (commit this the first time).
- `results/rerender.png`  — a re-render of the SAME scene; `diff(baseline, rerender)`
  **MUST PASS** (stable render within tolerance).
- `results/changed.png`   — a deliberately-changed scene (one cell recolored/relabeled);
  `diff(baseline, changed)` **MUST FAIL** (proving the diff has real discriminating power).

The script prints each diff's summary line (`WxH px: N differing (x%), max channel delta M
-> PASS/FAIL`) and exits non-zero if either expectation is violated.

**2. Report back:**
- **Did `render-grid` build on macOS?** (paste the first Cargo error if not — likely a
  minor gpui API drift or a font-resolution issue; see Risks in `findings.md`).
- The two printed diff lines (the PASS and the FAIL).
- Commit `results/baseline.png` (and optionally `changed.png`) so CI has a baseline.
- Confirm the offscreen render needed **no visible window / display** (it should not).

**What we conclude:** the confirmed CI mechanism (macOS runner, offscreen Metal capture,
perceptual diff within tolerance) — folded into `findings.md`, closing the C GATE.

## Manual commands (if you want to drive it by hand)

```sh
cd render-grid
cargo run --release --bin render_grid -- --scene baseline --out ../results/baseline.png
cargo run --release --bin render_grid -- --scene changed  --out ../results/changed.png
cargo run --release --bin render_grid -- --diff ../results/baseline.png ../results/changed.png  # exits 1 (FAIL) -- expected
```
