#!/usr/bin/env bash
# ---------------------------------------------------------------------------------
# Phase-12 perf harness runner (`architecture.md §4, §9`, `CLAUDE.md` benchmark
# conventions). Builds the harness in RELEASE and runs the POC "Run Test" scenario
# against the real GridView over a 1M×100 styled engine-backed fixture, under a
# virtual display (Xvfb) + software Vulkan (Mesa lavapipe).
#
# Run FOREGROUND (this script blocks) — never background it (`CLAUDE.md`). The harness
# itself times the CPU render-build path + the engine-call counter (representative under
# lavapipe); it does not gate on GPU present (not representative under software Vulkan).
#
# Requires: cargo + the pinned toolchain; xvfb (xvfb-run); mesa-vulkan-drivers (the
# lavapipe ICD). See app/README.md for the apt list.
#
# Usage:
#   perf.sh                # calibrate: build + run, print p50/p99, write results JSON
#   perf.sh gate           # CI gate: build + run --gate (non-zero exit on a breach)
# ---------------------------------------------------------------------------------
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"   # app/
mode="${1:-run}"

require_tools() {
    local missing=0
    if ! command -v xvfb-run >/dev/null 2>&1; then
        echo "perf.sh: required tool 'xvfb-run' not found on PATH" >&2
        missing=1
    fi
    if ! ls /usr/share/vulkan/icd.d/lvp_icd*.json >/dev/null 2>&1; then
        echo "perf.sh: no lavapipe ICD (/usr/share/vulkan/icd.d/lvp_icd*.json);" \
             "install mesa-vulkan-drivers" >&2
        missing=1
    fi
    if [ "$missing" -ne 0 ]; then
        echo "perf.sh: the perf harness needs a virtual display + software Vulkan;" \
             "see app/README.md for the apt list." >&2
        exit 1
    fi
}

require_tools

# Build the harness in release (the perf numbers must be optimized — CLAUDE.md).
cargo build --manifest-path "$here/Cargo.toml" -p render-tests --release --bin perf_harness

bin="$here/target/release/perf_harness"

case "$mode" in
    run)
        exec xvfb-run -a "$bin"
        ;;
    gate)
        exec xvfb-run -a "$bin" --gate
        ;;
    *)
        echo "usage: $0 [run | gate]" >&2
        exit 2
        ;;
esac
