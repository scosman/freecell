#!/usr/bin/env bash
# Build and run a UI PoC variant interactively (macOS/Metal).
#
# Usage:
#   ./build_and_run.sh [raw-gpui|gpui-component]
#
# Defaults to raw-gpui. Opens the app; scroll/pan to judge feel, then use the app menu's
# "Run Test" item to run the measured perf test (or use ./run_test.sh for a one-shot).
#
# macOS only: the gpui/gpui-component crates need a GPU + display and will not build on
# the headless Linux CI container. See ../findings.md "HUMAN RUN REQUIRED".
set -euo pipefail

VARIANT="${1:-raw-gpui}"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
POC_DIR="$(dirname "$SCRIPT_DIR")"

case "$VARIANT" in
  raw-gpui|gpui-component) ;;
  *)
    echo "unknown variant '$VARIANT' (expected raw-gpui or gpui-component)" >&2
    exit 2
    ;;
esac

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "WARNING: this PoC targets macOS/Metal; a non-macOS build will likely fail." >&2
fi

echo "==> building + running '$VARIANT' (interactive)"
cd "$POC_DIR/$VARIANT"
exec cargo run --release
