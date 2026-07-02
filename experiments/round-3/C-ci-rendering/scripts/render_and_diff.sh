#!/usr/bin/env bash
# FreeCell Round-3 Investigation C -- the macOS render->PNG->perceptual-diff run that
# closes the C GATE end-to-end. Run this on a **macOS/Metal** machine (GPUI's headless
# renderer is Metal-only in the pinned Zed rev; there is no in-container GPU capture path).
#
# It renders the grid OFFSCREEN (no visible window) three times and runs two perceptual
# diffs to prove the mechanism AND its discriminating power:
#
#   1. baseline  -> results/baseline.png
#   2. re-render -> results/rerender.png ; diff(baseline, rerender)  MUST PASS  (stable)
#   3. changed   -> results/changed.png  ; diff(baseline, changed)   MUST FAIL  (has power)
#
# The perceptual diff is tolerance-based (per-channel tolerance + differing-fraction), so
# anti-aliasing / font-rasterization wiggle does not false-fail; a real content change does.
# It is the SAME metric shape Zed's own visual tests use (compare_screenshots).
#
# First run compiles the pinned Zed `gpui` from source -- expect a long one-time build; it
# is NOT a hang. See ../README.md "HUMAN RUN REQUIRED (macOS)" and ../findings.md.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
C_DIR="$(dirname "$SCRIPT_DIR")"          # experiments/round-3/C-ci-rendering
PKG_DIR="$C_DIR/render-grid"
RESULTS="$C_DIR/results"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "ERROR: this must run on macOS/Metal. GPUI's headless renderer" >&2
  echo "       (gpui_platform::current_headless_renderer) returns None off-macOS in the" >&2
  echo "       pinned Zed rev, so offscreen capture is unavailable here." >&2
  exit 2
fi

mkdir -p "$RESULTS"
cd "$PKG_DIR"

run() { echo "==> $*"; "$@"; }

# 1. Baseline (commit results/baseline.png the first time as the known-good snapshot).
run cargo run --release --bin render_grid -- --scene baseline --out "$RESULTS/baseline.png"

# 2. Re-render the SAME scene; the perceptual diff vs baseline MUST PASS.
run cargo run --release --bin render_grid -- --scene baseline --out "$RESULTS/rerender.png"
echo "==> diff baseline vs re-render (expect PASS)"
if cargo run --release --bin render_grid -- --diff "$RESULTS/baseline.png" "$RESULTS/rerender.png"; then
  echo "    PASS (stable render matches baseline within tolerance) -- GOOD"
else
  echo "    FAIL: a stable re-render should match the baseline. Investigate before trusting the GATE." >&2
  exit 1
fi

# 3. Render the deliberately-changed scene; the perceptual diff vs baseline MUST FAIL.
run cargo run --release --bin render_grid -- --scene changed --out "$RESULTS/changed.png"
echo "==> diff baseline vs changed (expect FAIL -- proves discriminating power)"
if cargo run --release --bin render_grid -- --diff "$RESULTS/baseline.png" "$RESULTS/changed.png"; then
  echo "    UNEXPECTED PASS: the changed scene should NOT match -- the diff lacks power. Investigate." >&2
  exit 1
else
  echo "    FAIL as expected (changed scene detected) -- GATE mechanism proven."
fi

echo
echo "GATE CLOSED: offscreen render -> PNG -> perceptual diff works end-to-end on macOS;"
echo "stable re-render PASSES within tolerance and a deliberate change FAILS."
echo "Commit results/baseline.png and report the printed diff lines in ../findings.md."
