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
# It also runs the §9 bordered-viewport gate: one frame of a ≥500-bordered-cell fixture
# (cache-resident borders) must build under the buffered CI frame budget.
#
# Requires: cargo + the pinned toolchain; xvfb (xvfb-run); mesa-vulkan-drivers (the
# lavapipe ICD). See app/README.md for the apt list.
#
# Usage:
#   perf.sh                # calibrate: build + run, print p50/p99, write results JSON
#   perf.sh gate           # CI gate: build + run --gate (non-zero exit on a breach)
#
# Env:
#   FREECELL_CARGO_UNLOCKED=1   # opt OUT of `--locked` (see below). Only for a local run
#                               #   taken mid-dependency-edit, before Cargo.lock is updated.
# ---------------------------------------------------------------------------------
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"   # app/
mode="${1:-run}"

# `--locked` is UNCONDITIONAL (A2 — projects/architecture-review-remediation.md), same rule and
# same opt-out as render_tests.sh: a cargo invocation that resolves this workspace must build
# the COMMITTED lockfile (the one cargo-deny audited). Strict is the default so perf-gates.yml
# cannot lose the guarantee by dropping an `env:` line. The value below is compared, never
# spliced into argv, so no value of it can become a cargo positional argument.
case "${FREECELL_CARGO_UNLOCKED:-}" in
    "")  cargo_locked=(--locked) ;;
    1)   cargo_locked=()
         echo "perf.sh: FREECELL_CARGO_UNLOCKED=1 — running WITHOUT --locked;" \
              "cargo may rewrite Cargo.lock. Never set this in CI." >&2 ;;
    *)   echo "perf.sh: FREECELL_CARGO_UNLOCKED must be unset or exactly '1'" \
              "(got '${FREECELL_CARGO_UNLOCKED}'); refusing to guess." >&2
         exit 2 ;;
esac

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
cargo build "${cargo_locked[@]}" --manifest-path "$here/Cargo.toml" -p render-tests --release --bin perf_harness

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
