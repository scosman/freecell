#!/usr/bin/env bash
# Run the in-app "Run Test" harness one-shot and dump results (macOS/Metal).
#
# Usage:
#   ./run_test.sh [raw-gpui|gpui-component|both]
#
# Runs the scripted scroll / fast-scroll / horizontal / jump / random-jump sequence,
# measures per-frame render time + newly-visible-cell load latency, prints measured
# PASS/FAIL vs functional_spec §5.4, and writes JSON to ../results/<variant>-runtest.json.
# The app auto-quits when the script finishes.
#
# The run is stamped with the current date + short commit (POC_DATE / POC_COMMIT) so the
# recorded JSON is reproducible and self-describing.
#
# macOS only: see ../findings.md "HUMAN RUN REQUIRED".
set -euo pipefail

VARIANT="${1:-raw-gpui}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
POC_DIR="$(dirname "$SCRIPT_DIR")"

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "WARNING: this PoC targets macOS/Metal; a non-macOS build will likely fail." >&2
fi

run_one() {
  local variant="$1"
  echo "==> Run Test: $variant"
  cd "$POC_DIR/$variant"
  POC_DATE="$(date +%F)" \
  POC_COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)" \
    cargo run --release -- --run-test
  echo "==> results: $POC_DIR/results/${variant}-runtest.json"
}

case "$VARIANT" in
  raw-gpui|gpui-component)
    run_one "$VARIANT"
    ;;
  both)
    run_one raw-gpui
    run_one gpui-component
    echo "==> both variants done; compare $POC_DIR/results/*.json"
    ;;
  *)
    echo "unknown variant '$VARIANT' (expected raw-gpui, gpui-component, or both)" >&2
    exit 2
    ;;
esac
