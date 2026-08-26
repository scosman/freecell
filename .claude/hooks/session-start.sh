#!/usr/bin/env bash
# Claude Code SessionStart hook — registered in .claude/settings.json.
#
# This is a shim, not the logic: it finds .config/utils/setup_startup.sh and runs it. The
# work lives there so the same preparation can be run by hand — "re-run
# .config/utils/setup_startup.sh" is the repair line half of that script prints.
#
# The walk upward means this file is also what you point at from ~/.claude/settings.json on
# a machine that does not honor project hooks; nothing generates a second copy of it.
set -u

# CLAUDE_PROJECT_DIR is set for SessionStart hooks, and is the session's working directory
# — not necessarily the root of the checkout, hence the walk upward. Resolved to an
# absolute path first, so a relative one walks the real tree. The walk stops when a
# directory is its own parent ("/" and "."), so it cannot spin.
dir="${CLAUDE_PROJECT_DIR:-$PWD}"
dir="$(cd "$dir" 2>/dev/null && pwd)" || dir="$PWD"
while [ ! -f "$dir/.config/utils/setup_startup.sh" ]; do
  parent="$(dirname "$dir")"
  [ "$parent" = "$dir" ] && exit 0
  dir="$parent"
done
script="$dir/.config/utils/setup_startup.sh"

# A SessionStart hook's stdout becomes context for the session and its stderr does not, so
# both streams are captured and printed together: a failure the agent never sees is worse
# than no hook at all. Always exit 0 — this must never be the reason a session fails to
# start. stdin is the hook's event JSON; nothing here reads it.
#
# Capped, because this text is spent from the session's context window: the normal run is
# ~15 lines, but a failing resolution or build can be enormous, and the tail of such
# output is where the reason is.
output="$(bash "$script" 2>&1 </dev/null)"
status=$?
# Substring expansion rather than `head -c`/`tail -c`: those count bytes while ${#output}
# counts characters, and a byte cut lands mid-sequence on the multibyte glyphs these
# scripts emit (✗, !, em-dashes) — garbling the very text a human is squinting at.
if [ "${#output}" -gt 4000 ]; then
  output="${output:0:1500}
[...trimmed. Re-run .config/utils/setup_startup.sh to see all of it...]
${output: -2000}"
fi
[ -n "$output" ] && printf '%s\n' "$output"
[ "$status" -ne 0 ] &&
  printf '%s\n' "($script exited $status. Fix the above before building or testing.)"
exit 0
