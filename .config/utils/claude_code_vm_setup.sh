#!/usr/bin/env bash
# The setup script for a FreeCell Claude Code cloud environment. Paste the ENTIRE
# contents of this file into the environment's "Setup script" field.
#
# It fetches .config/utils/setup_env.sh and runs it, rather than being a copy of it.
# That is the whole point: setup_env.sh changes, and a pasted copy of it goes stale
# the moment it does — silently, until someone notices a VM missing something. This
# file is small and almost never changes.
#
# It runs at environment-build time, before any repo is checked out, so it cannot read
# anything from the repo — hence fetching over HTTPS rather than a path.
#
# FREECELL_SETUP_REF overrides the branch (or tag — not a commit SHA; the depth-1 clone
# setup_env.sh makes cannot target one), to try a branch's setup_env.sh without merging
# it. Set it by editing the line below IN THE PASTED SCRIPT, not in the environment
# variables: editing the setup script is also what forces the environment snapshot to
# rebuild, which is what makes the new setup actually run.
set -u

REF="${FREECELL_SETUP_REF:-main}"
URL="https://raw.githubusercontent.com/scosman/freecell/$REF/.config/utils/setup_env.sh"

# Nothing below exits non-zero. A cloud setup script that fails stops the session from
# ever starting, which is worse than a machine that comes up unprovisioned and says so
# — the same reason setup_env.sh is run with --best-effort.
script="$(mktemp)" || exit 0
trap 'rm -f "$script"' EXIT

echo "Fetching setup_env.sh from $REF..."
# --retry without --retry-all-errors on purpose: it retries timeouts and 5xx, and fails
# immediately on a 404, which is what a wrong ref deserves rather than four identical
# errors.
if ! curl -fsSL --retry 3 --retry-delay 2 -o "$script" "$URL"; then
  echo "" >&2
  echo "  ! Could not download $URL" >&2
  echo "    This machine is NOT set up for FreeCell. In a session, run:" >&2
  echo "        bash .config/utils/setup_env.sh --upgrade-tools" >&2
  echo "" >&2
  exit 0
fi

# A captive proxy or a renamed path answers 200 with an HTML error page, and piping that
# into bash produces pages of syntax errors instead of one clear line.
if ! head -1 "$script" | grep -q '^#!/usr/bin/env bash'; then
  echo "" >&2
  echo "  ! $URL did not return a shell script. First line:" >&2
  head -1 "$script" >&2
  echo "    Check that the ref '$REF' still exists." >&2
  echo "" >&2
  exit 0
fi

# FREECELL_SETUP_REF is exported so setup_env.sh clones the same ref it was fetched from.
# Without it, testing a branch would fetch the branch's setup_env.sh and then warm the
# caches from main — the one combination guaranteed to mislead.
export FREECELL_SETUP_REF="$REF"

# --create-startup-script is deliberately NOT passed. In this repo the SessionStart hook
# is committed source (.claude/settings.json + .claude/hooks/session-start.sh), so it needs
# no machine-global registration and cannot drift from the checkout. setup_env.sh still
# implements the flag as belt-and-braces for a machine where the project hook is not
# trusted; see .config/utils/claude_cloud_setup.md.
bash "$script" \
  --upgrade-tools \
  --best-effort \
  --warm-cache

exit 0
