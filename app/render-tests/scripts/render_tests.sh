#!/usr/bin/env bash
# ---------------------------------------------------------------------------------
# Run the FreeCell render-test suite (or regenerate its baselines) under software
# Vulkan (Mesa lavapipe) — the Phase-7 pixel gate (components/render_test_harness.md,
# architecture.md §9).
#
# The suite renders each case in its OWN Xvfb display sized to the case viewport (the
# harness manages that internally — a small window on a large screen won't present under
# lavapipe, so each case's Xvfb matches its window). So this wrapper does NOT wrap the
# whole run in a single xvfb-run; it only sets FREECELL_RENDER=1 (the opt-in that turns
# the pixel suite on) and runs cargo. The harness self-discovers the lavapipe ICD.
#
# Requires: cargo + the pinned toolchain; xvfb (xvfb-run), x11-utils (xwininfo),
#           x11-xserver-utils (xrefresh), imagemagick (import), mesa-vulkan-drivers
#           (lavapipe ICD). See app/README.md for the full apt list.
#
# Usage:
#   render_tests.sh                 # run the whole suite (assert every case matches its baseline)
#   render_tests.sh test            # same
#   render_tests.sh test <filter>   # run ONLY cases whose #[test] name matches <filter> — a fast
#                                   #   subset for iterating on a specific rendering change (e.g.
#                                   #   `test cell_` or `test border_`). The full suite is slow;
#                                   #   see CLAUDE.md "Render tests" for when to run which.
#   render_tests.sh generate [--only <prefix>]   # (re)write baselines/, print a summary
#
# Env:
#   FREECELL_CARGO_UNLOCKED=1   # opt OUT of `--locked` (see below). Only for a local run
#                               #   taken mid-dependency-edit, before Cargo.lock is updated.
# ---------------------------------------------------------------------------------
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"   # app/
mode="${1:-test}"
export FREECELL_RENDER=1

# `--locked` is UNCONDITIONAL (A2 — projects/architecture-review-remediation.md): every cargo
# invocation that resolves this workspace must prove it built the COMMITTED lockfile — the one
# cargo-deny audited. Strict is the default so CI cannot lose the guarantee by forgetting an
# `env:` line, and so a new workflow calling this script inherits it (the earlier
# CARGO_LOCKED-opt-IN shape was fail-open; see phase_plans/phase_2.md).
#
# The single legitimate local exception — iterating mid-dependency-edit, before Cargo.lock is
# regenerated — opts out explicitly with FREECELL_CARGO_UNLOCKED=1. Note this is a *flag*, not
# a place to put cargo arguments: the value is compared, never spliced into argv, so a wrong
# value can never become a cargo positional (e.g. a stray test-name filter that would make the
# pixel gate pass green having run zero cases).
case "${FREECELL_CARGO_UNLOCKED:-}" in
    "")  cargo_locked=(--locked) ;;
    1)   cargo_locked=()
         echo "render_tests.sh: FREECELL_CARGO_UNLOCKED=1 — running WITHOUT --locked;" \
              "cargo may rewrite Cargo.lock. Never set this in CI." >&2 ;;
    *)   echo "render_tests.sh: FREECELL_CARGO_UNLOCKED must be unset or exactly '1'" \
              "(got '${FREECELL_CARGO_UNLOCKED}'); refusing to guess." >&2
         exit 2 ;;
esac

# Assert the capture stack is present BEFORE invoking cargo. Setting FREECELL_RENDER=1 means the
# operator wants the real pixel gate; if the tooling is missing we must fail loudly rather than let
# the suite skip to green (a "required" gate that tests zero pixels). This mirrors the hard-failure
# the render suite itself now enforces (tests/render_suite.rs `gate`).
require_tools() {
    local missing=0 tool
    for tool in xvfb-run xrefresh xwininfo import; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "render_tests.sh: required tool '$tool' not found on PATH" >&2
            missing=1
        fi
    done
    # Software-Vulkan ICD (Mesa lavapipe) — the harness renders through it.
    if ! ls /usr/share/vulkan/icd.d/lvp_icd*.json >/dev/null 2>&1; then
        echo "render_tests.sh: no lavapipe ICD (/usr/share/vulkan/icd.d/lvp_icd*.json);" \
             "install mesa-vulkan-drivers" >&2
        missing=1
    fi
    if [ "$missing" -ne 0 ]; then
        echo "render_tests.sh: the FREECELL_RENDER pixel gate needs the capture tooling above;" \
             "refusing to run a zero-pixel gate (see render-tests/README.md for the apt list)." >&2
        exit 1
    fi
}

case "$mode" in
    test)
        require_tools
        shift || true   # drop the "test" arg; forward any remaining as a cargo test name filter
        exec cargo test "${cargo_locked[@]}" --manifest-path "$here/Cargo.toml" -p render-tests "$@"
        ;;
    generate)
        require_tools
        shift || true
        # `cargo run --bin generate_baselines` does not build its sibling render_scene,
        # which it shells out to — build both bins first.
        cargo build "${cargo_locked[@]}" --manifest-path "$here/Cargo.toml" -p render-tests \
            --bin render_scene --bin generate_baselines
        exec cargo run "${cargo_locked[@]}" --manifest-path "$here/Cargo.toml" -p render-tests \
            --bin generate_baselines -- "$@"
        ;;
    *)
        echo "usage: $0 [test [filter] | generate [--only <prefix>]]" >&2
        exit 2
        ;;
esac
