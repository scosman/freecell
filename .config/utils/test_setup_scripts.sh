#!/usr/bin/env bash
# Tests for the cloud-VM setup scripts in this directory.
#
#     bash .config/utils/test_setup_scripts.sh
#
# What this suite does and does not do to the machine it runs on — stated precisely, because
# an isolation claim that is broader than its guards is worse than no claim:
#
#   Never, and asserted at the end rather than assumed:
#     - no apt (FREECELL_SKIP_SYSTEM_DEPS; the render-env log's absence is checked too)
#     - no binary written into /usr/local/bin or ~/.local/bin (FREECELL_SKIP_TOOL_INSTALL)
#     - nothing under /opt/freecell-vm-setup (FREECELL_VM_SETUP_DIR points at scratch, and
#       that one path — not all of /opt — is what the closing case fingerprints)
#     - no temp files outside the scratch dir (TMPDIR points at scratch)
#     - your running sccache server is not stopped (SCCACHE_SERVER_PORT is a private port,
#       and the EXIT trap stops the server this suite starts on it)
#
#   DOES happen, as an unavoidable side effect of testing the real scripts:
#     - `rustup toolchain install` for the pin in app/rust-toolchain.toml, which DOWNLOADS a
#       full toolchain into ~/.rustup if you do not already have it
#     - `cargo fetch --locked`, which populates ~/.cargo/registry and ~/.cargo/git (~1 GB for
#       this workspace) if they are cold
#     - an sccache server started and stopped on the private port above
#
# Both of the latter are exactly what your first `cargo build` in this repo would do anyway,
# which is why they are documented rather than guarded — but they are not nothing, and this
# comment is the contract.
#
# A case that cannot run here (no jq, no sccache, running as root) reports `skip` and is
# counted separately, so a green run on a machine that has none of the tools cannot be
# mistaken for a green run that proved anything.
#
# The cases are the ones that took real reasoning to get right: the container gate's
# falsy-value handling, the provisioning marker's effect on whether an unattributable
# cache is trusted, and the compile cache actually caching.
if [ -z "${BASH_VERSION:-}" ]; then
  if [ -r "$0" ]; then exec bash "$0" "$@"; fi
  echo "error: this test suite requires bash." >&2
  exit 1
fi
set -uo pipefail

UTILS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
PROJECT_ROOT="$(cd "$UTILS_DIR/../.." && pwd)"
SETUP_ENV="$UTILS_DIR/setup_env.sh"
SETUP_STARTUP="$UTILS_DIR/setup_startup.sh"

# Read from the scripts under test rather than hardcoded, so bumping the marker contract or
# the toolchain pin does not break this suite for reasons that have nothing to do with it.
# shellcheck disable=SC2016  # $VM_SETUP_DIR is literal text being matched in the source.
MARKER_NAME="$(sed -n 's|^VM_SETUP_MARKER="$VM_SETUP_DIR/\(.*\)"$|\1|p' "$SETUP_STARTUP" | head -1)"
# shellcheck disable=SC2016  # as above.
MARKER_NAME_ENV="$(sed -n 's|^VM_SETUP_MARKER="$VM_SETUP_DIR/\(.*\)"$|\1|p' "$SETUP_ENV" | head -1)"
TOOLCHAIN_PIN="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
  "$PROJECT_ROOT/app/rust-toolchain.toml" | head -1)"

SCRATCH="$(mktemp -d)"
# The sccache server this suite starts on its private port is stopped structurally here, not
# as a side effect of whichever case happens to run last.
cleanup() {
  command -v sccache >/dev/null 2>&1 && sccache --stop-server >/dev/null 2>&1
  rm -rf "$SCRATCH"
}
trap cleanup EXIT

# Inherited by every setup_env.sh invocation below. Without these, any of them could curl a
# binary into /usr/local/bin, and the --warm-cache case (which produces a real reference
# tree) would run a ~20-package apt install on whatever box the suite is on.
export FREECELL_SKIP_TOOL_INSTALL=1
export FREECELL_SKIP_SYSTEM_DEPS=1
# Temp files the scripts write (build logs, the render-env log, mktemp scratch) land under
# the scratch dir too — which is also what makes "the apt step never ran" provable by the
# absence of its log rather than by a package listing that is unchanged either way.
export TMPDIR="$SCRATCH/tmp"
mkdir -p "$TMPDIR"
# sccache has one server per port, and both scripts stop and start it. Without a private
# port the suite would kill the compile cache out from under a contributor's concurrent
# build; with one, it starts and stops a daemon of its own.
export SCCACHE_SERVER_PORT="${FREECELL_TEST_SCCACHE_PORT:-14227}"

PASSED=0
FAILED=0
SKIPPED=0
CURRENT=""

start() { CURRENT="$1"; }
ok() { PASSED=$((PASSED + 1)); echo "  ok   $CURRENT — $1"; }
bad() { FAILED=$((FAILED + 1)); echo "  FAIL $CURRENT — $1" >&2; }
# Counted separately and reported in the summary. A case that cannot run on this machine
# printing "ok" is how a green run stops meaning anything.
skip() { SKIPPED=$((SKIPPED + 1)); echo "  skip $CURRENT — $1"; }
expect() { # expect <description> <condition...>
  local description="$1"; shift
  if "$@"; then ok "$description"; else bad "$description"; fi
}
expect_contains() { # expect_contains <description> <haystack> <needle>
  if printf '%s' "$2" | grep -qF -- "$3"; then ok "$1"; else
    bad "$1 (expected to find: $3)"
  fi
}
expect_not_contains() {
  if printf '%s' "$2" | grep -qF -- "$3"; then bad "$1 (unexpectedly found: $3)"; else
    ok "$1"
  fi
}
expect_status() { # expect_status <description> <expected> <actual>
  if [ "$2" = "$3" ]; then ok "$1"; else bad "$1 (expected exit $2, got $3)"; fi
}
expect_equal() { # expect_equal <description> <expected> <actual>
  if [ "$2" = "$3" ]; then
    ok "$1"
  else
    # Prints the difference, because an exact-match assertion replaces several substring ones
    # and would otherwise be the least debuggable failure in the suite.
    bad "$1"
    diff <(printf '%s\n' "$2") <(printf '%s\n' "$3") >&2 || true
  fi
}

# Guard rails. Everything below writes only under $SCRATCH; the closing isolation case
# fingerprints the machine-global directory to prove it.
if [ -z "$SCRATCH" ] || [ ! -d "$SCRATCH" ]; then
  echo "could not create a scratch dir" >&2
  exit 1
fi
vm_dir_fingerprint() {
  # Full depth, and mtime as well as size: a same-size edit deep inside sccache/ is exactly
  # the change a shallow, size-only fingerprint would miss.
  find /opt/freecell-vm-setup -printf '%p %s %T@\n' 2>&1 | sort | cksum
}
REAL_VM_DIR_BEFORE="$(vm_dir_fingerprint)"

# The machine's own tools and packages. Size + mtime catches a re-install even when the
# replacement bytes are identical, which is exactly the case that looks like a no-op.
system_tools_fingerprint() {
  stat -c '%n %s %Y' /usr/local/bin/sccache /usr/local/bin/cargo-deny "$HOME/.local/bin/sccache" \
    "$HOME/.local/bin/cargo-deny" 2>&1 | sort | cksum
}
apt_state_fingerprint() {
  dpkg --get-selections 2>/dev/null | cksum
}
SYSTEM_TOOLS_BEFORE="$(system_tools_fingerprint)"
APT_STATE_BEFORE="$(apt_state_fingerprint)"

echo "Scratch: $SCRATCH"
echo ""

# ---------------------------------------------------------------------------------
echo "setup_startup.sh — the container gate"
# ---------------------------------------------------------------------------------
start "gate/absent"
out="$(env -u CLAUDE_CODE_REMOTE -u IS_CONTAINERIZED bash "$SETUP_STARTUP" 2>&1)"
expect_status "exits 0 outside a container" 0 "$?"
# Captured (which is how the SessionStart hook runs it), the no-op must cost a local
# contributor's session nothing at all.
expect "says nothing when its output is being captured" test -z "$out"

start "gate/absent-tty"
if command -v script >/dev/null 2>&1; then
  # At a terminal the same no-op is advice for a person, so it does speak.
  out="$(script -qec "env -u CLAUDE_CODE_REMOTE -u IS_CONTAINERIZED bash '$SETUP_STARTUP'" /dev/null 2>&1)"
  expect_contains "says it is not running in a container" "$out" "Not running in a VM/container"
  expect_contains "names the local repair" "$out" "setup_env.sh --human"
else
  skip "'script' is not installed, so no pty is available"
fi

start "gate/falsy"
# The one failure mode impossible to notice: reading IS_CONTAINERIZED=no as yes. Asserted on
# behavior rather than the notice, because the notice is silent when captured: a falsy value
# must mean the checkout is NOT prepared. All seven are listed because each pins one
# alternative of is_flag_set's case pattern — drop or mistype any one of them and exactly one
# of these fails, so none of them is decoration. `f` is in the list for that reason: without
# it, deleting `f` from the pattern turned nothing red (FALSE pins the lowercasing, not `f`).
for falsy in false FALSE f 0 no n off ""; do
  out="$(env -u CLAUDE_CODE_REMOTE IS_CONTAINERIZED="$falsy" bash "$SETUP_STARTUP" 2>&1)"
  expect_not_contains "IS_CONTAINERIZED='$falsy' is treated as no" "$out" "Syncing dependencies"
done

start "gate/truthy"
# Asserted on behavior for the same reason gate/falsy is: the "Not running in a VM/container"
# notice is printed only at a terminal, so under command substitution it is absent whether
# the gate opened or not — an assertion on its absence passes even with the gate shut.
out="$(env -u CLAUDE_CODE_REMOTE IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-empty" \
  bash "$SETUP_STARTUP" 2>&1)"
expect_contains "IS_CONTAINERIZED=true opens the gate" "$out" "Syncing dependencies"

start "args"
out="$(env -u CLAUDE_CODE_REMOTE bash "$SETUP_STARTUP" --no-such-flag 2>&1)"
expect_status "a mistyped flag is rejected, not swallowed" 2 "$?"
expect_contains "and names the flag" "$out" "--no-such-flag"

# ---------------------------------------------------------------------------------
echo ""
echo "setup_startup.sh — an unprovisioned VM"
# ---------------------------------------------------------------------------------
start "unprovisioned"
UNPROVISIONED_OUT="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-empty" \
  bash "$SETUP_STARTUP" 2>&1)"
UNPROVISIONED_STATUS=$?
expect_status "warns and continues rather than failing" 0 "$UNPROVISIONED_STATUS"
expect_contains "names the missing marker" "$UNPROVISIONED_OUT" "was not set up for FreeCell"
expect_contains "reaches the Ready line anyway" "$UNPROVISIONED_OUT" "Ready"
expect_contains "refuses the unattributable cache" "$UNPROVISIONED_OUT" "no provisioning marker"
expect "writes nothing under the scratch VM dir" test ! -d "$SCRATCH/vm-empty"

# ---------------------------------------------------------------------------------
echo ""
echo "setup_startup.sh — a provisioned VM"
# ---------------------------------------------------------------------------------
start "provisioned"
VM_OK="$SCRATCH/vm-ok"
mkdir -p "$VM_OK/sccache"
# A non-empty cache: emptiness is now itself a reported condition, so the "healthy" fixture
# has to look healthy.
printf 'object\n' >"$VM_OK/sccache/some-cached-object"
printf 'setup_version=2\nsccache_warmed=true\nsccache_warm_commit=abc123\n' \
  >"$VM_OK/$MARKER_NAME"
PROVISIONED_OUT="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" \
  bash "$SETUP_STARTUP" 2>&1)"
PROVISIONED_STATUS=$?
expect_status "exits 0" 0 "$PROVISIONED_STATUS"
expect_not_contains "does not warn about the marker" "$PROVISIONED_OUT" "was not set up for FreeCell"
expect_contains "reports the pinned toolchain" "$PROVISIONED_OUT" "pin $TOOLCHAIN_PIN"
if command -v sccache >/dev/null 2>&1; then
  expect_contains "wires sccache to the image's disk cache" "$PROVISIONED_OUT" "cache: sccache -> $VM_OK/sccache"
else
  skip "sccache is not installed, so cache wiring cannot be asserted"
fi
expect "the output stays short enough to spend on context" \
  test "$(printf '%s\n' "$PROVISIONED_OUT" | wc -l)" -le 20

start "provisioned/cold-cache"
# The failure this whole design exists to prevent: a cache that reports healthy while
# holding nothing. Existence, provenance and writability all pass here — only the contents
# distinguish a warm image from a cold one.
COLD_VM="$SCRATCH/vm-cold"
mkdir -p "$COLD_VM/sccache"
printf 'setup_version=2\nsccache_warmed=false\n' >"$COLD_VM/$MARKER_NAME"
out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$COLD_VM" bash "$SETUP_STARTUP" 2>&1)"
expect_status "still exits 0 — a cold cache is slow, not broken" 0 "$?"
expect_contains "an empty cache is reported" "$out" "is empty (marker: sccache_warmed=false)"
expect_contains "with the repair that fills it" "$out" "--warm-cache"

start "provisioned/lying-marker"
# The nastier variant: the image build reported success, so the marker says warmed, but the
# objects went somewhere else (a leftover SCCACHE_BUCKET). The directory is the evidence.
LIAR_VM="$SCRATCH/vm-liar"
mkdir -p "$LIAR_VM/sccache"
printf 'setup_version=2\nsccache_warmed=true\nsccache_warm_commit=deadbeef\n' \
  >"$LIAR_VM/$MARKER_NAME"
out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$LIAR_VM" bash "$SETUP_STARTUP" 2>&1)"
expect_contains "the contradiction is called out, not the claim believed" "$out" "EMPTY although the VM marker claims it was warmed"
expect_contains "naming the commit the marker claims" "$out" "deadbeef"

start "provisioned/env-file"
ENV_FILE="$SCRATCH/session-env"
: >"$ENV_FILE"
IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" CLAUDE_ENV_FILE="$ENV_FILE" \
  bash "$SETUP_STARTUP" >/dev/null 2>&1
if command -v sccache >/dev/null 2>&1; then
  # The WHOLE file, not substrings of it: this is the one artifact that reaches every shell
  # the agent opens, so an extra line — a credential, a stray export, a second block — has to
  # fail something. A canary matching a specific forbidden string would only ever catch the
  # forbidden string someone thought of, and per-line substring checks cannot fail unless
  # this does.
  expected_env_file="# freecell compile cache: sccache $VM_OK/sccache 8G
unset SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX || true
export SCCACHE_DIR=$VM_OK/sccache
export SCCACHE_CACHE_SIZE=8G
export RUSTC_WRAPPER=sccache"
  expect_equal "the session env file is exactly the wiring block, and nothing else" \
    "$expected_env_file" "$(cat "$ENV_FILE")"
else
  skip "sccache is not installed, so env-file wiring cannot be asserted"
fi

start "cache/retirement-on-every-path"
# The critical regression: the remote-backend retirement used to sit behind four gates, so an
# unprovisioned machine kept SCCACHE_BUCKET and an inherited RUSTC_WRAPPER — every rustc call
# went to the old bucket while the status line said "cache: off". "No marker" is exactly the
# state between merging this and rebuilding the image, so it is the first state a user hits.
RETIRE_ENV="$SCRATCH/retire-env"
: >"$RETIRE_ENV"
retire_out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-nonexistent" \
  CLAUDE_ENV_FILE="$RETIRE_ENV" \
  SCCACHE_BUCKET=a-bucket-that-must-not-survive SCCACHE_REGION=auto \
  SCCACHE_ENDPOINT=https://example.invalid RUSTC_WRAPPER=sccache \
  bash "$SETUP_STARTUP" 2>&1)"
expect_status "an unprovisioned session still exits 0" 0 "$?"
expect_contains "and reports the cache as off" "$retire_out" "cache: off"
retire_content="$(cat "$RETIRE_ENV")"
expect_contains "the bucket is retired for the session's later shells" "$retire_content" "unset SCCACHE_BUCKET"
expect_contains "and an inherited wrapper goes with it, so off means off" "$retire_content" "unset RUSTC_WRAPPER"
expect_not_contains "no wrapper is exported when there is no cache" "$retire_content" "export RUSTC_WRAPPER"

start "cache/cap-comes-from-the-image"
# The cap is a property of the image. Re-deriving the default here would have the first
# session prune a 20G-baked cache back to 8G, evicting exactly what the image was built to
# carry. An explicit override in this session's environment still wins over both.
CAP_VM="$SCRATCH/vm-cap"
mkdir -p "$CAP_VM/sccache"
printf 'object\n' >"$CAP_VM/sccache/some-cached-object"
printf 'setup_version=2\nsccache_warmed=true\nsccache_max_size=20G\n' >"$CAP_VM/$MARKER_NAME"
CAP_ENV="$SCRATCH/cap-env"
: >"$CAP_ENV"
IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$CAP_VM" CLAUDE_ENV_FILE="$CAP_ENV" \
  bash "$SETUP_STARTUP" >/dev/null 2>&1
if command -v sccache >/dev/null 2>&1; then
  expect_contains "the marker's cap is honoured, not the default" "$(cat "$CAP_ENV")" "SCCACHE_CACHE_SIZE=20G"
else
  skip "sccache is not installed, so the cache is never wired"
fi
: >"$CAP_ENV"
IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$CAP_VM" CLAUDE_ENV_FILE="$CAP_ENV" \
  FREECELL_SCCACHE_CACHE_SIZE=3G bash "$SETUP_STARTUP" >/dev/null 2>&1
if command -v sccache >/dev/null 2>&1; then
  expect_contains "an explicit override still wins" "$(cat "$CAP_ENV")" "SCCACHE_CACHE_SIZE=3G"
else
  skip "sccache is not installed, so the cache is never wired"
fi

start "cache/leftover-bucket-loses"
# The in-process half of the same fix. SCCACHE_BUCKET beats SCCACHE_DIR, so with the script-
# scope unset removed the server this script starts would come up on the remote backend and
# the wiring would be refused — observable right here in the status line.
if ! command -v sccache >/dev/null 2>&1; then
  skip "sccache is not installed"
else
  out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" \
    SCCACHE_BUCKET=a-bucket-that-must-not-win SCCACHE_REGION=auto \
    SCCACHE_ENDPOINT=https://example.invalid \
    bash "$SETUP_STARTUP" 2>&1)"
  expect_contains "a leftover bucket does not win the session's cache" "$out" "cache: sccache -> $VM_OK/sccache"
  expect_not_contains "and the backend readback has nothing to complain about" "$out" "resolved to"
fi

start "sync/log-unopenable"
# `if ! ( … ) >"$LOG"` swallows a redirection that fails to open — `!` in front of a COMPOUND
# command reports the wrong status in bash (see the measurement table in setup_startup.sh) —
# so the obvious shape here skips cargo fetch entirely and still reaches "Ready.". A TMPDIR
# pointing at a directory that does not exist is the everyday way in; a full disk is the other.
logfail_out="$(TMPDIR="$SCRATCH/no-such-tmpdir" IS_CONTAINERIZED=true \
  FREECELL_VM_SETUP_DIR="$VM_OK" bash "$SETUP_STARTUP" 2>&1)"
expect_status "a fetch that never ran is a failure, not a Ready" 1 "$?"
expect_contains "and says the log could not be opened" "$logfail_out" "so cargo fetch never ran"
# "Ready", not "Ready.": with the bug present the script prints "Ready, with warnings above",
# which does not contain the literal "Ready." — so the dot made this pass with the bug.
expect_not_contains "never reaching the success line" "$logfail_out" "Ready"

start "sync/failure-excerpt"
# A `✗` line and a log PATH is not a diagnosis. `tail -15 "$LOG" 2>/dev/null >&2` discards the
# excerpt entirely — redirections apply left to right, so fd 2 goes to /dev/null and then fd 1
# is duped from it — and no assertion covered the excerpt, which is why that stayed green.
FAKE_CARGO_BIN="$SCRATCH/fake-cargo-bin"
mkdir -p "$FAKE_CARGO_BIN"
cat >"$FAKE_CARGO_BIN/cargo" <<'FAKE_CARGO'
#!/bin/sh
case "$1" in
  fetch) echo "SENTINEL_WHY_THE_FETCH_FAILED"; exit 101 ;;
  *) exec "$REAL_CARGO" "$@" ;;
esac
FAKE_CARGO
chmod +x "$FAKE_CARGO_BIN/cargo"
excerpt_out="$(REAL_CARGO="$(command -v cargo)" PATH="$FAKE_CARGO_BIN:$PATH" \
  IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" bash "$SETUP_STARTUP" 2>&1)"
expect_status "a failing fetch fails the session" 1 "$?"
expect_contains "and the log excerpt reaches the agent, not just the log path" \
  "$excerpt_out" "SENTINEL_WHY_THE_FETCH_FAILED"
expect_contains "alongside the repair" "$excerpt_out" "cargo update"

start "cache/env-file-unwritable"
# The adjacent door to cache/no-env-file: CLAUDE_ENV_FILE is set, but the write fails. Same
# end state — a cache wired for this process only — so it has to produce the same report.
# Bash does not make a failed redirection on a `{ …; }` group return non-zero, so the obvious
# implementation swallows this silently while printing "Ready." and a healthy status line.
unwritable_out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" \
  CLAUDE_ENV_FILE="$SCRATCH/no-such-directory/env" bash "$SETUP_STARTUP" 2>&1)"
expect_status "still exits 0" 0 "$?"
if command -v sccache >/dev/null 2>&1; then
  expect_contains "the failed write degrades the status line" "$unwritable_out" "THIS PROCESS ONLY"
  expect_contains "naming the file it could not write" "$unwritable_out" "the write to $SCRATCH/no-such-directory/env failed"
  expect_contains "and the closing line stops saying plain Ready" "$unwritable_out" "Ready, with warnings above"
else
  skip "sccache is not installed, so the cache is never wired"
fi

start "cache/retirement-write-failure"
# The block carries the R2 retirement as well as the cache wiring, so a failed write matters
# even when the cache is off: the session keeps an inherited RUSTC_WRAPPER + SCCACHE_BUCKET
# and every rustc call goes to the retired bucket while the status line says "cache: off".
retirefail_out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-nonexistent" \
  CLAUDE_ENV_FILE="$SCRATCH/no-such-directory/env" \
  SCCACHE_BUCKET=a-bucket-that-must-not-survive RUSTC_WRAPPER=sccache \
  bash "$SETUP_STARTUP" 2>&1)"
expect_status "still exits 0" 0 "$?"
expect_contains "the cache is still reported off" "$retirefail_out" "cache: off"
expect_contains "and the unretired session is called out too" "$retirefail_out" "THE RETIREMENT DID NOT REACH THE SESSION"
expect_contains "so the closing line stops saying plain Ready" "$retirefail_out" "Ready, with warnings above"

start "cache/retirement-write-failure-with-cache-on"
# The two hazards are independent: with the cache ON and the block unwritten, an inherited
# bucket is equally live. An if/else made them exclusive, and this branch then described the
# builds as "uncached" when they would in fact be cached against a dead bucket.
bothfail_out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" \
  CLAUDE_ENV_FILE="$SCRATCH/no-such-directory/env" \
  SCCACHE_BUCKET=a-bucket-that-must-not-survive RUSTC_WRAPPER=sccache \
  bash "$SETUP_STARTUP" 2>&1)"
if command -v sccache >/dev/null 2>&1; then
  expect_contains "the cache half is reported" "$bothfail_out" "THIS PROCESS ONLY"
  expect_contains "and the retirement half too, not instead" "$bothfail_out" "THE RETIREMENT DID NOT REACH THE SESSION"
else
  skip "sccache is not installed, so the cache is never wired"
fi

start "cache/retirement-quiet-when-nothing-inherited"
# The mirror of the case above. With no remote-backend variables in the environment there is
# nothing to retire, so warning about one names five no-op unsets and drags a healthy
# machine's closing line down to "Ready, with warnings above".
quiet_out="$(env -u RUSTC_WRAPPER -u SCCACHE_BUCKET -u SCCACHE_ENDPOINT -u SCCACHE_REGION \
  -u SCCACHE_S3_KEY_PREFIX -u CLAUDE_ENV_FILE \
  IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-nonexistent" \
  bash "$SETUP_STARTUP" 2>&1)"
expect_not_contains "nothing inherited means nothing to retire" "$quiet_out" "THE RETIREMENT DID NOT REACH"
expect_contains "and the closing line stays clean" "$quiet_out" "Ready. "

start "cache/env-file-idempotent"
# SessionStart fires on startup, resume and fork; without a guard one container accumulates
# a duplicate block per event.
IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-nonexistent" \
  CLAUDE_ENV_FILE="$RETIRE_ENV" bash "$SETUP_STARTUP" >/dev/null 2>&1
expect "a second run appends nothing" \
  test "$(grep -c 'unset SCCACHE_BUCKET' "$RETIRE_ENV")" -eq 1

start "cache/unwritable"
# The image builds as root and a session may run as someone else; a cache directory the
# session cannot write is a cache that silently stores nothing.
if [ "$(id -u)" -eq 0 ]; then
  skip "running as root, which bypasses [ -w ] entirely — note that the cloud sandbox is also root, so this guard is untested AND unreachable there; it earns its place on an image that runs sessions as a non-root user"
else
  RO_VM="$SCRATCH/vm-readonly"
  mkdir -p "$RO_VM/sccache"
  printf 'setup_version=2\n' >"$RO_VM/$MARKER_NAME"
  chmod 555 "$RO_VM/sccache"
  out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$RO_VM" bash "$SETUP_STARTUP" 2>&1)"
  chmod 755 "$RO_VM/sccache"
  expect_contains "an unwritable cache is reported, not used" "$out" "is not writable by"
  expect_contains "with the repair that fixes it" "$out" "chmod 1777"
fi

start "repo-identity"
# A directory that is not this repo must not be mistaken for it.
mkdir -p "$SCRATCH/not-freecell/app/crates/other"
: >"$SCRATCH/not-freecell/app/Cargo.toml"
mkdir -p "$SCRATCH/not-freecell/.config/utils"
cp "$SETUP_STARTUP" "$SCRATCH/not-freecell/.config/utils/setup_startup.sh"
out="$(cd "$SCRATCH/not-freecell" && IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$VM_OK" \
  bash "$SCRATCH/not-freecell/.config/utils/setup_startup.sh" 2>&1)"
status=$?
expect_status "a look-alike tree without freecell-engine is rejected" 1 "$status"
expect_contains "and says what it looked for" "$out" "freecell-engine"

# ---------------------------------------------------------------------------------
echo ""
echo "The marker contract, and a cache that really caches"
# ---------------------------------------------------------------------------------
start "contract/version"
# The marker path is the interface between two scripts that cannot share code, and the _vN
# in it is a contract version. Drifting halves would mean setup_env.sh provisions a machine
# setup_startup.sh reports as unprovisioned, forever.
expect "the marker name is non-empty" test -n "$MARKER_NAME"
expect "both scripts name the same marker" test "$MARKER_NAME" = "$MARKER_NAME_ENV"

start "cache/really-caches"
# Everything else about the cache is asserted from fixtures and messages. This is the one
# case that proves the behaviour the whole design rests on, and it proves it END TO END:
# setup_startup.sh wires the cache for real, writes the session env file, and a LATER SHELL
# sourcing that file compiles something whose object then appears in the image's cache
# directory. Driving it through the real script rather than a local re-implementation is
# what makes it sensitive to enable_compile_cache regressing (a dropped
# `export RUSTC_WRAPPER`, a SCCACHE_DIR that never reaches the session). A two-line crate,
# so the whole case costs about a second.
if ! command -v sccache >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
  skip "needs both cargo and sccache"
else
  PROBE_VM="$SCRATCH/vm-probe"
  PROBE_CACHE="$PROBE_VM/sccache"
  PROBE_ENV_FILE="$SCRATCH/probe-session-env"
  PROBE_CRATE="$SCRATCH/probe-crate"
  mkdir -p "$PROBE_CACHE" "$PROBE_CRATE/src"
  printf 'setup_version=2\nsccache_warmed=false\n' >"$PROBE_VM/$MARKER_NAME"
  : >"$PROBE_ENV_FILE"
  cat >"$PROBE_CRATE/Cargo.toml" <<'PROBE'
[package]
name = "freecell-cache-probe"
version = "0.0.0"
edition = "2021"
PROBE
  # Unique content, so this compiles for real rather than hitting an object left behind by an
  # earlier run of the suite.
  printf 'pub fn probe() -> u64 { %s }\n' "$$" >"$PROBE_CRATE/src/lib.rs"

  probe_out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$PROBE_VM" \
    CLAUDE_ENV_FILE="$PROBE_ENV_FILE" bash "$SETUP_STARTUP" 2>&1)"
  expect_contains "the real script wires the cache to it" "$probe_out" "cache: sccache -> $PROBE_CACHE"
  expect_contains "and reports it cold, because it is" "$probe_out" "is empty (marker: sccache_warmed=false)"

  # The session's later shells get the wiring by sourcing this file and nothing else — which
  # is exactly what is being tested, so the build below is given no cache environment of its
  # own. CARGO_INCREMENTAL=0 because sccache refuses to cache incremental compilations, the
  # same reason workspace crates are never served from the image's cache and the pinned
  # dependency tree always is.
  # shellcheck disable=SC2016  # the single-quoted script is expanded by the child shell.
  probe_build_env() {
    env -u RUSTC_WRAPPER -u SCCACHE_DIR -u SCCACHE_CACHE_SIZE \
      -u SCCACHE_BUCKET -u SCCACHE_ENDPOINT -u SCCACHE_REGION \
      bash -c '. "$1" >/dev/null 2>&1; shift; exec "$@"' _ "$PROBE_ENV_FILE" "$@"
  }
  # shellcheck disable=SC2016  # deliberately expanded by the child shell, after sourcing.
  probe_wrapper="$(probe_build_env sh -c 'printf %s "${RUSTC_WRAPPER:-<unset>}"')"
  expect "a later shell picks up the wrapper from the env file alone" \
    test "$probe_wrapper" = "sccache"

  probe_before="$(probe_build_env sccache --show-stats 2>/dev/null |
    sed -n 's/^Compile requests[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  ( cd "$PROBE_CRATE" && probe_build_env env CARGO_INCREMENTAL=0 \
      CARGO_TARGET_DIR="$SCRATCH/probe-target" cargo build --quiet ) >"$TMPDIR/probe-build.log" 2>&1
  expect_status "the probe crate builds through that environment" 0 "$?"
  expect "and an object landed in the image's cache directory" \
    test -n "$(find "$PROBE_CACHE" -mindepth 1 -type f -print -quit 2>/dev/null)"
  probe_after="$(probe_build_env sccache --show-stats 2>/dev/null |
    sed -n 's/^Compile requests[[:space:]]*\([0-9][0-9]*\).*/\1/p' | head -1)"
  # The COUNT, not the presence of the row: `sccache --show-stats` always prints
  # "Compile requests", including as 0 on a server that has never compiled anything, so
  # matching the label is an assertion that cannot fail.
  expect "sccache's compile-request count went up" \
    test "${probe_after:-0}" -gt "${probe_before:-0}"
  probe_build_env sccache --stop-server >/dev/null 2>&1
fi

start "cache/no-env-file"
# The other half of the same contract. With CLAUDE_ENV_FILE unset, `export RUSTC_WRAPPER`
# reaches the hook's own process and nothing else, so a plain "cache: sccache -> …" would be
# a success line asserting something it never checked.
NOENV_VM="$SCRATCH/vm-noenv"
mkdir -p "$NOENV_VM/sccache"
printf 'object\n' >"$NOENV_VM/sccache/some-cached-object"
printf 'setup_version=2\nsccache_warmed=true\n' >"$NOENV_VM/$MARKER_NAME"
noenv_out="$(env -u CLAUDE_ENV_FILE IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$NOENV_VM" \
  bash "$SETUP_STARTUP" 2>&1)"
expect_status "still exits 0" 0 "$?"
if command -v sccache >/dev/null 2>&1; then
  expect_contains "the status line says the wiring did not reach the session" "$noenv_out" "THIS PROCESS ONLY"
  expect_contains "and warns with a repair" "$noenv_out" "CLAUDE_ENV_FILE is unset"
else
  skip "sccache is not installed, so the cache is never wired"
fi

# ---------------------------------------------------------------------------------
echo ""
echo "setup_env.sh — flags and exit codes"
# ---------------------------------------------------------------------------------
start "setup_env/help"
bash "$SETUP_ENV" --help >/dev/null 2>&1
expect_status "--help exits 0" 0 "$?"

start "setup_env/unknown"
out="$(bash "$SETUP_ENV" --no-such-flag 2>&1)"
expect_status "an unknown flag fails" 1 "$?"
expect_contains "and names it" "$out" "--no-such-flag"

start "setup_env/best-effort"
# The invariant that keeps a cloud environment bootable: a build-time script must never
# exit non-zero, whatever went wrong.
bash "$SETUP_ENV" --no-such-flag --best-effort >/dev/null 2>&1
expect_status "--best-effort owns the exit code even for a bad flag" 0 "$?"

start "setup_env/upgrade-is-not-a-warning"
# An sccache below the floor is replaced, and a replacement that SUCCEEDS leaves the machine
# with what it was supposed to have. Reporting that as a '!' flips the whole build log to
# "WITH WARNINGS" for a success, which is the same defect as an unqualified success line,
# inverted. The install itself is blocked here by FREECELL_SKIP_TOOL_INSTALL, so what this
# pins is which channel the "replacing" line uses.
STUB_BIN="$SCRATCH/stub-bin"
mkdir -p "$STUB_BIN"
printf '#!/bin/sh\necho "sccache 0.5.0"\n' >"$STUB_BIN/sccache"
chmod +x "$STUB_BIN/sccache"
upgrade_out="$(PATH="$STUB_BIN:$PATH" FREECELL_VM_SETUP_DIR="$SCRATCH/vm-upgrade" \
  bash "$SETUP_ENV" --best-effort 2>&1)"
expect_contains "the replacement is announced as progress" "$upgrade_out" "==> replacing sccache 0.5.0"
expect_not_contains "not as a warning about something missing" "$upgrade_out" "! sccache 0.5.0 is below"

start "setup_env/human"
# README.md and app/README.md now point every contributor at --human, and its whole contract
# is what it does NOT do.
HUMAN_VM="$SCRATCH/vm-human"
human_out="$(FREECELL_VM_SETUP_DIR="$HUMAN_VM" \
  bash "$SETUP_ENV" --human 2>&1)"
expect_status "exits 0" 0 "$?"
# Covers the marker too: nothing at all may appear at that path.
expect "writes nothing under the machine-global directory" test ! -e "$HUMAN_VM"
expect_not_contains "does not name a machine-global directory it will not touch" "$human_out" "vm state:"
expect_not_contains "does not install or replace the compile cache binary" "$human_out" "Compile cache"
# Again the banner: --human has no marker to record an outcome in.
expect_contains "runs the source-warming step" "$human_out" "cargo fetch --locked"
expect_contains "and closes with a local next step" "$human_out" "Local setup complete"
# The other side of the warnings check below: a run that emitted no '!' lines must not claim
# warnings either, or the qualifier means nothing.
expect_not_contains "with no warnings claimed, because none were emitted" "$human_out" "WITH WARNINGS"

start "setup_env/human-no-checkout"
# Run from a copy outside any checkout, which is what a contributor who cloned nothing yet
# gets. Almost nothing repo-local happens in that state, so closing with "Local setup
# complete. Build with: cd app && …" would name a directory that does not exist.
HUMAN_ISOLATED="$SCRATCH/human-isolated"
NOCHECKOUT_VM="$SCRATCH/vm-human-nocheckout"
mkdir -p "$HUMAN_ISOLATED"
cp "$SETUP_ENV" "$HUMAN_ISOLATED/setup_env.sh"
# FREECELL_VM_SETUP_DIR is set here like everywhere else. Without it, the only thing keeping
# this invocation off the machine's real /opt was the --human guard inside the code under
# test — so the suite header's "points at scratch" would have been false for exactly one case.
nocheckout_out="$( cd "$HUMAN_ISOLATED" &&
  FREECELL_VM_SETUP_DIR="$NOCHECKOUT_VM" bash "$HUMAN_ISOLATED/setup_env.sh" --human 2>&1 )"
nocheckout_status=$?
expect_status "exits non-zero — this is a user error, and --human is not a cloud script" 1 "$nocheckout_status"
expect_not_contains "does not claim local setup is complete" "$nocheckout_out" "Local setup complete"
expect_contains "says what was skipped" "$nocheckout_out" "nothing repo-local was done"
expect_contains "and how to get it right" "$nocheckout_out" "Re-run from inside a FreeCell checkout"
expect "writing nothing machine-global on the way out" test ! -e "$NOCHECKOUT_VM"

# ---------------------------------------------------------------------------------
echo ""
echo "setup_env.sh --warm-cache — the throwaway clone"
# ---------------------------------------------------------------------------------
start "warm-cache"
# Cloning from the local checkout rather than GitHub keeps the case offline and fast, and
# FREECELL_SKIP_WARM_BUILD keeps the multi-minute workspace build out of a test suite. What
# is under test here is the part that has failure modes: cloning with no checkout in reach,
# identity-checking the clone, warming the source caches from it, and deleting it after.
WARM_VM="$SCRATCH/vm-warm"
WARM_ISOLATED="$SCRATCH/warm-isolated"
mkdir -p "$WARM_ISOLATED"
cp "$SETUP_ENV" "$WARM_ISOLATED/setup_env.sh"
warm_branch="$(git -C "$PROJECT_ROOT" branch --show-current 2>/dev/null)"
# SCCACHE_BUCKET is set deliberately: SCCACHE_BUCKET beats SCCACHE_DIR, a cloud environment
# build inherits the environment's variables, and an operator who upgrades without deleting
# a leftover bucket would otherwise warm the bucket while the image's disk cache stayed
# empty — and the marker would record it as warmed. setup_env.sh clears the list machine-
# wide; if that ever stops happening, warm_sccache's backend check refuses and the
# assertions below flip.
warm_out="$( cd "$WARM_ISOLATED" &&
  FREECELL_VM_SETUP_DIR="$WARM_VM" FREECELL_REPO_URL="$PROJECT_ROOT" \
    FREECELL_SETUP_REF="${warm_branch:-main}" FREECELL_SKIP_WARM_BUILD=1 \
    \
    SCCACHE_BUCKET=a-bucket-that-must-not-win SCCACHE_REGION=auto \
    SCCACHE_ENDPOINT=https://example.invalid \
    bash "$WARM_ISOLATED/setup_env.sh" --warm-cache --best-effort 2>&1 )"
expect_status "exits 0" 0 "$?"
# "Cloning ", not "throwaway": the latter also appears in the removal banner, so changing the
# clone step alone would leave this green.
expect_contains "clones because no checkout is in reach" "$warm_out" "Cloning "
# The step banner, i.e. that the step RAN. Its outcome is asserted from the marker below
# (cargo_sources_warmed=true); on a machine with a warm ~/.cargo this is sub-second, so a
# green run here is not evidence that a cold fetch works.
expect_contains "runs the source-warming step against the clone" "$warm_out" "cargo fetch --locked"
expect "the throwaway clone is gone afterwards" test ! -e "$WARM_VM/throwaway-clone"
# The reference tree exists here, so this is the one case that WOULD apt-install. Assert
# the guard fired, not just that the outcome looked unchanged: on a box that already has
# the packages, a real apt run is indistinguishable from no run at all.
expect_contains "the system-dependency guard fires" "$warm_out" "FREECELL_SKIP_SYSTEM_DEPS is set"
expect_contains "and the warm build reaches its own guard" "$warm_out" "FREECELL_SKIP_WARM_BUILD is set"
# Both of those guards print '!' lines, i.e. this image is missing things. The closing line is
# an operator's only view of an image build, so it must not sign off as unqualified success.
expect_contains "so the closing line is qualified, not plain success" "$warm_out" "WITH WARNINGS"
expect_not_contains "having resolved to the image's disk cache" "$warm_out" "refusing to warm"
expect "a marker was written" test -f "$WARM_VM/$MARKER_NAME"
marker="$(cat "$WARM_VM/$MARKER_NAME")"
expect_contains "recording the source-cache warm" "$marker" "cargo_sources_warmed=true"
expect_contains "and the pinned toolchain's cargo, not the default one" "$marker" "cargo_version=${TOOLCHAIN_PIN%%-*}"

start "warm-cache/from-scratch"
# The defect this case exists for: with a checkout in reach, $REFERENCE_TREE is that
# checkout, and a checkout whose app/target is already built gives cargo almost nothing to
# compile — so almost no rustc call reaches sccache and the machine is recorded as warmed
# while holding a few percent of the dependency tree (measured: 18 objects against 612
# rlibs). sccache_disk_has_objects() cannot see that; it is a non-emptiness test. What the
# fix guarantees, and what these assertions are about, is WHERE the build wrote.
#
# A miniature checkout, not this repo's: it satisfies is_freecell_root(), and it compiles in
# about a second where the real workspace takes 20 minutes. incremental=false because
# sccache refuses to cache incremental compilations and every crate here is a workspace
# crate — the real dependency tree is non-incremental for free, this fixture is not.
if ! command -v sccache >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
  skip "needs both cargo and sccache"
else
  FS_ROOT="$SCRATCH/fake-warm-checkout"
  FS_VM="$SCRATCH/vm-from-scratch"
  mkdir -p "$FS_ROOT/app/crates/freecell-engine/src" "$FS_ROOT/.config/utils"
  cat >"$FS_ROOT/app/Cargo.toml" <<'FIXTURE_TOML'
[workspace]
members = ["crates/freecell-engine"]
resolver = "2"

[profile.dev]
incremental = false
FIXTURE_TOML
  cat >"$FS_ROOT/app/crates/freecell-engine/Cargo.toml" <<'FIXTURE_TOML'
[package]
name = "freecell-engine-fixture"
version = "0.0.0"
edition = "2021"

[lib]
path = "src/lib.rs"
FIXTURE_TOML
  echo 'pub fn warm() -> u8 { 1 }' >"$FS_ROOT/app/crates/freecell-engine/src/lib.rs"
  # The real pin, so the fixture builds with the toolchain the script selects rather than
  # whatever this machine's default happens to be.
  cp "$PROJECT_ROOT/app/rust-toolchain.toml" "$FS_ROOT/app/rust-toolchain.toml"
  # Copied into the fixture, because find_freecell_root() resolves the script's own location
  # first — run from this repo's copy it would warm THIS workspace, for twenty minutes.
  cp "$SETUP_ENV" "$FS_ROOT/.config/utils/setup_env.sh"
  ( cd "$FS_ROOT/app" && cargo generate-lockfile --offline ) >/dev/null 2>&1
  # An already-built checkout is the whole precondition. A sentinel file is enough: what is
  # asserted is that the warm build put no build output here, not what a real target/ holds.
  mkdir -p "$FS_ROOT/app/target"
  printf 'sentinel\n' >"$FS_ROOT/app/target/do-not-touch"
  # Directories, not "any new entry": the source-warming step's `cargo fetch` legitimately
  # drops target/.rustc_info.json (cargo's own rustc-version cache, ~1 KB) into the checkout,
  # and only the BUILD is under test here. Every profile a build could use — debug/, release/ —
  # is a directory, so this catches the defect without failing on the fetch's metadata file.
  fs_target_dirs() { find "$FS_ROOT/app/target" -mindepth 1 -type d 2>/dev/null | sort; }
  # The disk guard is pinned low rather than left to whatever this machine has free, so this
  # case tests the build and `warm-cache/disk-guard` below tests the guard.
  fs_out="$( cd "$FS_ROOT" &&
    FREECELL_VM_SETUP_DIR="$FS_VM" FREECELL_MIN_WARM_BUILD_GB=1 \
      bash "$FS_ROOT/.config/utils/setup_env.sh" --warm-cache --best-effort 2>&1 )"
  expect_status "exits 0" 0 "$?"
  # Without this, the two "nothing was written there" assertions below would both pass on a
  # run that compiled nothing at all.
  expect_contains "the warm build runs" "$fs_out" "Filling the sccache disk cache"
  expect_equal "and puts no build output in the checkout's own target/" "" "$(fs_target_dirs)"
  expect "having built into a throwaway target/ that is gone afterwards" \
    test ! -e "$FS_VM/warm-build-target"
  expect "the cache it filled holds objects" \
    test -n "$(find "$FS_VM/sccache" -mindepth 1 -type f -print -quit 2>/dev/null)"
  expect_contains "so the machine records as warmed" "$(cat "$FS_VM/$MARKER_NAME")" "sccache_warmed=true"
fi

start "warm-cache/disk-guard"
# The guard now stands between a guaranteed-from-scratch build and a full disk, so it needs a
# case: a threshold that always fires and one that never does look identical from outside.
if [ ! -d "${FS_ROOT:-/nonexistent}" ]; then
  skip "needs the miniature checkout the previous case builds"
else
  GUARD_VM="$SCRATCH/vm-disk-guard"
  # What a run killed with SIGKILL (or an OOM killer) leaves behind: the EXIT trap never ran,
  # so a multi-GB tree is stranded at the fixed path the next run will use. It must be swept
  # even on this path, where no build happens — otherwise it sits on the image forever and is
  # counted against the very guard that is firing.
  mkdir -p "$GUARD_VM/sccache" "$GUARD_VM/warm-build-target/stale"
  guard_out="$( cd "$FS_ROOT" &&
    FREECELL_VM_SETUP_DIR="$GUARD_VM" FREECELL_MIN_WARM_BUILD_GB=999999 \
      bash "$FS_ROOT/.config/utils/setup_env.sh" --warm-cache --best-effort 2>&1 )"
  expect_status "a disk too full is a note, not a failure" 0 "$?"
  expect_contains "the guard fires" "$guard_out" "skipping the warm build"
  expect_not_contains "so no build is started" "$guard_out" "Filling the sccache disk cache"
  expect "and a stranded throwaway target from a killed run is swept anyway" \
    test ! -e "$GUARD_VM/warm-build-target"
  expect_contains "the machine is not recorded as warmed" \
    "$(cat "$GUARD_VM/$MARKER_NAME")" "sccache_warmed=false"
fi

start "marker/carry-forward"
# Repairing an environment means re-running WITHOUT --warm-cache. The marker must keep
# describing the cache that is on disk, not be rewritten to claim it has no provenance.
CARRY_VM="$SCRATCH/vm-carry"
# A real object, not just a subdirectory: "warmed" means files on disk everywhere it is
# tested, and a fixture that is only a directory would bake the looser rule into the suite.
mkdir -p "$CARRY_VM/sccache/x"
printf 'object\n' >"$CARRY_VM/sccache/x/cached-object"
cat >"$CARRY_VM/$MARKER_NAME" <<'MARKER'
setup_version=2
sccache_warmed=true
sccache_warm_commit=deadbeefcafe
MARKER
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$CARRY_VM" \
    bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
marker="$(cat "$CARRY_VM/$MARKER_NAME")"
# First: the marker was REWRITTEN. Both assertions below grep a file this test wrote itself,
# so without this they pass identically if write_marker never runs at all. sccache_dir is not
# in the fixture, so only a real write can put it there.
expect_contains "the marker was rewritten, not merely left alone" "$marker" "sccache_dir="
expect_contains "the warm flag survives a repair run" "$marker" "sccache_warmed=true"
expect_contains "so does the commit the cache came from" "$marker" "sccache_warm_commit=deadbeefcafe"

start "marker/cap-carry-forward"
# The write side of the cap interface. `cache/cap-comes-from-the-image` covers the read side;
# without this, a plain `setup_env.sh --upgrade-tools` — the repair line the scripts and four
# rows of claude_cloud_setup.md tell people to run — silently re-derives the default and the
# next session prunes the image's own objects.
CAPW_VM="$SCRATCH/vm-cap-write"
mkdir -p "$CAPW_VM/sccache"
printf 'object\n' >"$CAPW_VM/sccache/some-cached-object"
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$CAPW_VM" \
    FREECELL_SCCACHE_CACHE_SIZE=20G bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
expect_contains "the baked cap is recorded" "$(cat "$CAPW_VM/$MARKER_NAME")" "sccache_max_size=20G"
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$CAPW_VM" \
    bash "$WARM_ISOLATED/setup_env.sh" --upgrade-tools --best-effort ) >/dev/null 2>&1
capw_marker="$(cat "$CAPW_VM/$MARKER_NAME")"
expect_contains "the repair run rewrote the marker" "$capw_marker" "written_at="
expect_contains "and carried the cap forward, not the default" "$capw_marker" "sccache_max_size=20G"

start "marker/carry-forward-needs-objects"
# The same "real objects" rule the other two call sites use. With a looser "any entry" test, a
# directory holding nothing but empty subdirectories carried a stale sccache_warmed=true
# forward — reinstating exactly the lie the round-3 checks exist to catch.
HOLLOW_VM="$SCRATCH/vm-hollow"
mkdir -p "$HOLLOW_VM/sccache/empty-subdir"
cat >"$HOLLOW_VM/$MARKER_NAME" <<'MARKER'
setup_version=2
sccache_warmed=true
sccache_warm_commit=cafef00d
MARKER
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$HOLLOW_VM" \
    bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
marker="$(cat "$HOLLOW_VM/$MARKER_NAME")"
expect_contains "a directory with no files does not carry 'warmed' forward" "$marker" "sccache_warmed=false"
expect_not_contains "nor the commit it claimed" "$marker" "cafef00d"

start "marker/empty-cache"
# The opposite trap: a directory this run merely created is not a cache of unknown origin.
EMPTY_VM="$SCRATCH/vm-empty-cache"
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$EMPTY_VM" \
    bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
marker="$(cat "$EMPTY_VM/$MARKER_NAME" 2>/dev/null)"
expect_contains "an empty cache dir reports as not warmed" "$marker" "sccache_warmed=false"
expect_contains "with no commit claimed" "$marker" "sccache_warm_commit=none"

# ---------------------------------------------------------------------------------
echo ""
echo "The SessionStart hook shim"
# ---------------------------------------------------------------------------------
start "shim/behavior"
# The shim is the only piece whose failure mode is silence, so it gets its own cases: a
# session must still start, the agent must SEE the failure (stderr and stdout together),
# and the output must not run away with the context window.
FAKE="$SCRATCH/fake-checkout/.config/utils"
mkdir -p "$FAKE"
cat >"$FAKE/setup_startup.sh" <<'FAKE_SCRIPT'
#!/usr/bin/env bash
head -c 6000 /dev/zero | tr '\0' 'o'
echo ""
echo "SENTINEL_ON_STDERR" >&2
exit 3
FAKE_SCRIPT
shim_out="$(CLAUDE_PROJECT_DIR="$SCRATCH/fake-checkout" \
  bash "$PROJECT_ROOT/.claude/hooks/session-start.sh" 2>/dev/null)"
expect_status "a failing startup script never fails the session" 0 "$?"
expect_contains "the failure is reported with its exit code" "$shim_out" "exited 3"
expect_contains "stderr reaches the agent, not just stdout" "$shim_out" "SENTINEL_ON_STDERR"
expect_contains "runaway output is trimmed" "$shim_out" "[...trimmed."
expect "and capped near 4000 characters" test "${#shim_out}" -lt 4400

start "shim/no-checkout"
# The user-level shim fires in every session on the machine, whatever repo it is for.
mkdir -p "$SCRATCH/unrelated"
shim_out="$(CLAUDE_PROJECT_DIR="$SCRATCH/unrelated" \
  bash "$PROJECT_ROOT/.claude/hooks/session-start.sh" 2>&1)"
expect_status "a session in an unrelated tree is untouched" 0 "$?"
expect "and says nothing at all" test -z "$shim_out"

# ---------------------------------------------------------------------------------
echo ""
echo "Nothing escaped the scratch directory"
# ---------------------------------------------------------------------------------
start "isolation"
# Not "does not exist" — this machine may legitimately be provisioned. What must hold is
# that the suite did not touch it.
expect "the machine-global setup dir is exactly as the suite found it" \
  test "$REAL_VM_DIR_BEFORE" = "$(vm_dir_fingerprint)"
expect "no binary was installed or replaced in a system bin directory" \
  test "$SYSTEM_TOOLS_BEFORE" = "$(system_tools_fingerprint)"
expect "no packages were installed" \
  test "$APT_STATE_BEFORE" = "$(apt_state_fingerprint)"
expect "the render-env apt script was never invoked" test ! -e "$TMPDIR/freecell-render-env.log"

echo ""
if [ "$SKIPPED" -gt 0 ]; then
  echo "$PASSED passed, $FAILED failed, $SKIPPED skipped (a skip means this machine could not run that case)."
else
  echo "$PASSED passed, $FAILED failed."
fi
[ "$FAILED" -eq 0 ]
