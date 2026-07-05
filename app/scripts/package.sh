#!/usr/bin/env bash
# ---------------------------------------------------------------------------------
# Build distributable FreeCell packages on the CURRENT platform with cargo-packager.
#
# This is the single packaging entry point for BOTH local iteration and CI (the
# `release` workflow just calls this script) — so a green run here means CI is green.
#
#   macOS  -> .app bundle + .dmg
#   Linux  -> .deb + .AppImage
#   (Windows uses the sibling `package.ps1`.)
#
# The packager config lives in `crates/freecell-app/Cargo.toml`
# (`[package.metadata.packager]`); see `../PACKAGING.md` for the full story. Builds are
# UNSIGNED dev builds (macOS Gatekeeper will need right-click -> Open); signing is future
# work (PACKAGING.md + projects/pre-distribution-security-audit.md).
#
# cargo-packager does NOT build the app itself, and it looks for the binary under the
# profile dir matching its `--release`/`--profile` flag — so this script builds the
# release binary FIRST and packages with `--release` (same profile) to avoid a mismatch.
# Icon paths in the config are resolved relative to the CWD, so we always run from `app/`.
#
# Usage:
#   scripts/package.sh                      # build + package the platform defaults
#   scripts/package.sh --verbose            # extra flags pass through to cargo-packager
#   FREECELL_PACKAGE_FORMATS=dmg scripts/package.sh     # override formats (comma list)
#   FREECELL_PACKAGE_OUT_DIR=/tmp/pkgs scripts/package.sh
#
# Requires: the pinned Rust toolchain, this platform's build deps (see ../README.md),
#   and `cargo-packager` (install: cargo install cargo-packager --locked --version 0.11.8).
#   Linux .AppImage also needs `file` + `patchelf` on PATH (used by linuxdeploy) and network
#   access (linuxdeploy/AppRun are downloaded on first run).
# ---------------------------------------------------------------------------------
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"   # app/
cd "$here"

# --- choose per-OS package formats (overridable) ---------------------------------
case "$(uname -s)" in
    Darwin) default_formats="app,dmg" ;;
    Linux)  default_formats="deb,appimage" ;;
    *)
        echo "package.sh: unsupported OS '$(uname -s)'." \
             "Use scripts/package.ps1 on Windows." >&2
        exit 2
        ;;
esac
formats="${FREECELL_PACKAGE_FORMATS:-$default_formats}"
out_dir="${FREECELL_PACKAGE_OUT_DIR:-$here/target/packages}"

# --- require the cargo-packager subcommand ---------------------------------------
if ! command -v cargo-packager >/dev/null 2>&1; then
    echo "package.sh: 'cargo-packager' not found on PATH. Install the pinned version:" >&2
    echo "    cargo install cargo-packager --locked --version 0.11.8" >&2
    exit 3
fi

# --- build the release binary (profile MUST match the packager's --release) -------
echo "package.sh: building freecell (release)…"
cargo build --release -p freecell-app --bin freecell

# --- package ----------------------------------------------------------------------
mkdir -p "$out_dir"
echo "package.sh: packaging formats '$formats' -> $out_dir"
cargo packager \
    --release \
    --packages freecell-app \
    --formats "$formats" \
    --out-dir "$out_dir" \
    "$@"

echo
echo "package.sh: done. Packages in $out_dir:"
ls -la "$out_dir"
