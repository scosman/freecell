#!/bin/bash
# SessionStart hook — make fresh web dev containers cache-ready.
#
# Runs the sccache + Cloudflare R2 compile-cache setup (app/scripts/setup_sccache.sh)
# so cargo builds in this session hit the shared remote cache with no manual step.
# The setup script no-ops gracefully when the R2_* credential env vars are absent,
# so sessions without creds still build (just uncached). Dev-only: CI is untouched.
set -euo pipefail

# Web/remote sessions only — local checkouts opt in manually by sourcing the script.
if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

"$CLAUDE_PROJECT_DIR/app/scripts/setup_sccache.sh"
