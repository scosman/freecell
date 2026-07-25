#!/bin/bash
# SessionStart hook — make fresh web dev containers cache-ready.
#
# Runs the sccache + Cloudflare R2 compile-cache setup (app/scripts/setup_sccache.sh)
# so cargo builds in this session hit the shared remote cache with no manual step,
# then kicks off a background pre-fetch of the dependency SOURCE cache (see below).
# The setup script no-ops gracefully when the R2_* credential env vars are absent,
# so sessions without creds still build (just uncached). Dev-only: CI is untouched.
set -euo pipefail

# Web/remote sessions only — local checkouts opt in manually by sourcing the script.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

"$CLAUDE_PROJECT_DIR/app/scripts/setup_sccache.sh"

# Pre-warm the dependency SOURCE cache in the background: download the crates.io
# tarballs + git checkouts pinned in Cargo.lock into ~/.cargo so the session's first
# `cargo build` skips the registry/git fetch phase (~100s of "Updating git repository
# …/Downloaded …") before compilation even starts. This is the source cache — separate
# from, and complementary to, the sccache compile cache above (dependency *sources* vs.
# compiled *rustc outputs*, which are pulled from R2 during the build itself). It never
# compiles, so it can't fail on in-progress code and is a fast no-op once warm. Scoped
# to this container's target to skip the Windows/macOS-only crates an unscoped fetch
# would pull. Detached via setsid + `&` so it never delays session start; a failed or
# killed fetch is harmless (the first build just fetches whatever is still missing).
setsid bash -c "cd '$CLAUDE_PROJECT_DIR/app' && exec cargo fetch --locked --target x86_64-unknown-linux-gnu" \
  </dev/null >/tmp/freecell-prewarm-fetch.log 2>&1 &
