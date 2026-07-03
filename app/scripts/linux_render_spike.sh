#!/usr/bin/env bash
# ---------------------------------------------------------------------------------
# Phase-1 load-bearing Linux render spike (architecture.md §9,
# components/render_test_harness.md §Mechanism).
#
# Goal: prove that pixels can be captured off a GPUI window on Linux under
#   Xvfb + Mesa lavapipe (software Vulkan)
# — the capture path the Phase-7 render suite will build on. Round-3 C established that
# GPUI's OFFSCREEN headless capture is macOS/Metal-only at the pinned rev, so this spike
# validates the fallback capture variant: render into a real window under a virtual
# display and capture the X root pixels.
#
# Preference order (render_test_harness.md §Mechanism):
#   1. a GPUI capture API on the Linux backend  -> not present at the pinned rev (round-3 C)
#   2. render to a window under Xvfb + capture X pixels  <-- THIS SCRIPT
#   3. macOS offscreen-Metal fallback  -> macos-verify workflow
#
# A non-blank capture = spike PASS. A failure is a DECISION POINT, not a stopper: record
# it in DECISIONS_TO_REVIEW.md and move the render suite to macos-verify.
#
# Usage: app/scripts/linux_render_spike.sh [output.png]
# Requires: xvfb, mesa-vulkan-drivers (lavapipe ICD), imagemagick, x11-xserver-utils
#           (xrefresh — forces the Expose that makes gpui present), x11 libs.
# ---------------------------------------------------------------------------------
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # app/
out="${1:-$here/target/render-spike/linux_hello.png}"
mkdir -p "$(dirname "$out")"

# --- locate the lavapipe (software Vulkan) ICD -----------------------------------
icd=""
for cand in \
    /usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
    /usr/share/vulkan/icd.d/lvp_icd.json; do
    [ -f "$cand" ] && icd="$cand" && break
done
if [ -z "$icd" ]; then
    icd="$(find /usr/share/vulkan/icd.d -iname 'lvp_icd*.json' 2>/dev/null | head -n1 || true)"
fi
if [ -z "$icd" ]; then
    echo "SPIKE FAIL: no lavapipe ICD found (install mesa-vulkan-drivers)" >&2
    exit 3
fi
echo "using lavapipe ICD: $icd"

export VK_ICD_FILENAMES="$icd"
export LIBGL_ALWAYS_SOFTWARE=1
export ZED_ALLOW_EMULATED_GPU=1   # gpui/blade: permit software Vulkan adapters

# --- build the hello-world binary ------------------------------------------------
echo "building freecell (debug)…"
( cd "$here" && cargo build -p freecell-app --bin freecell )
bin="$here/target/debug/freecell"

# --- run under Xvfb, capture the root window -------------------------------------
run_under_xvfb() {
    # Launch the window. --exit-after-ms is a safety net (self-quit via an executor
    # timer); we ALSO kill explicitly after capturing so this never hangs even if the
    # window fails to draw. Under Xvfb there is no compositor, so we cannot rely on
    # continuous rendering — one painted frame is enough for the capture.
    "$bin" --exit-after-ms 10000 >/tmp/freecell-spike.log 2>&1 &
    local app_pid=$!
    # give it time to init Vulkan, create the window, and render the first frame
    sleep 4
    # CRITICAL: gpui's X11 backend only PRESENTS a frame when it receives an Expose
    # event (crates/gpui_linux/.../x11/client.rs — `require_presentation` is gated on
    # `expose_event_received`). Under Xvfb there is no compositor to generate one, so the
    # rendered frame never reaches the framebuffer and the capture is blank. `xrefresh`
    # repaints the root, forcing an Expose on every window → gpui presents. This is the
    # load-bearing trick that makes the Linux capture path work.
    xrefresh >/dev/null 2>&1 || true
    sleep 2
    local rc=0
    if command -v import >/dev/null 2>&1; then
        import -window root "$out" || rc=$?
    elif command -v xwd >/dev/null 2>&1; then
        xwd -root -silent | convert xwd:- "$out" || rc=$?
    else
        echo "SPIKE FAIL: neither ImageMagick 'import' nor 'xwd' available" >&2
        rc=4
    fi
    # tear the app down deterministically (do not wait on self-quit)
    kill -TERM "$app_pid" 2>/dev/null || true
    sleep 1
    kill -KILL "$app_pid" 2>/dev/null || true
    return $rc
}

echo "rendering under Xvfb + lavapipe…"
xvfb-run -a -s "-screen 0 1024x768x24" bash -c "$(declare -f run_under_xvfb); \
    bin='$bin' out='$out' run_under_xvfb"

# --- verify the capture is non-blank --------------------------------------------
if [ ! -s "$out" ]; then
    echo "SPIKE FAIL: no PNG captured at $out" >&2
    echo "---- app log ----"; cat /tmp/freecell-spike.log || true
    exit 5
fi
colors="$(convert "$out" -format '%k' info: 2>/dev/null || echo 1)"
echo "captured $out with $colors unique colors"
if [ "${colors:-1}" -gt 1 ]; then
    echo "SPIKE PASS: non-blank GPUI capture on Linux (Xvfb + lavapipe)"
    exit 0
else
    echo "SPIKE FAIL: capture is blank (1 unique color) — GPUI did not render" >&2
    echo "---- app log ----"; cat /tmp/freecell-spike.log || true
    exit 6
fi
