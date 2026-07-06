#!/usr/bin/env bash
# ---------------------------------------------------------------------------------
# One-stop setup for the FreeCell render-test suite on Linux — the SINGLE SOURCE OF
# TRUTH for "what must be installed to render + capture baselines". Run it on a fresh
# Ubuntu 24.04 box (the pinned runner class, see render-tests/README.md) before
# generating or running the pixel suite. CI (`.github/workflows/checks.yml`) invokes
# this same script, so the dev/CI environments cannot drift.
#
#   app/render-tests/scripts/setup_render_env.sh
#
# It installs:
#   1. The GPUI Linux (blade/Vulkan) build deps.
#   2. The software-Vulkan capture stack: Xvfb + Mesa lavapipe + ImageMagick + the
#      x11 window tools (xrefresh/xwininfo) the harness captures with.
#   3. One system font — **DejaVu Serif** (fonts-dejavu-core) — used ONLY by the
#      explicit-serif render case (`font_family_serif`, which asks for "DejaVu Serif").
#
# NOTE on the UI font: the app's grid + chrome render in **Inter**, which is BUNDLED
# in the binary and registered at startup via `add_fonts` (see
# crates/freecell-app/src/shell/fonts.rs + assets/fonts/inter/). So NO system UI font
# is required, and bold/italic render correctly on every platform. This deliberately
# replaced an earlier, fragile reliance on GPUI's platform-default UI font (macOS
# ".SystemUIFont"; on Linux that resolved to "IBM Plex Sans", and when absent to a
# single regular face — which made bold/italic silently render as regular).
#
# Idempotent: safe to re-run.
# ---------------------------------------------------------------------------------
set -euo pipefail

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
    SUDO="sudo"
fi

echo "==> apt-get update"
$SUDO apt-get update

echo "==> installing build deps + capture stack + the explicit-serif case font"
$SUDO apt-get install -y --no-install-recommends \
    build-essential pkg-config cmake \
    libx11-dev libxcb1-dev libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
    libvulkan-dev libgbm-dev \
    libfontconfig1-dev libfreetype-dev libasound2-dev \
    mesa-vulkan-drivers vulkan-tools \
    xvfb x11-apps x11-xserver-utils x11-utils imagemagick \
    fonts-dejavu-core

echo "==> rebuilding fontconfig cache"
$SUDO fc-cache -f >/dev/null

# Sanity: the explicit-serif case needs DejaVu Serif on the system (the UI font Inter
# is bundled in the binary, so it is NOT checked here).
if ! fc-list | grep -q "DejaVu Serif:"; then
    echo "setup_render_env.sh: WARNING — 'DejaVu Serif' not found; the font_family_serif" \
         "render case will fall back to another face." >&2
fi

echo "==> render-env setup complete:"
echo "    capture stack (Xvfb + lavapipe + ImageMagick) + GPUI build deps + DejaVu Serif."
echo "    (The UI font, Inter, is bundled in the app binary — no system UI font needed.)"
