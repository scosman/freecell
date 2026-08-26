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
#     - no edit to the real ~/.claude/settings.json (CLAUDE_CONFIG_DIR points at scratch).
#       Where no such file exists, the assertion narrows to "and none was created", which is
#       the only half of that claim with any content
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
# cache is trusted, and every rule of the settings.json merge.
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

# Inherited by every setup_env.sh invocation below. Without these the merge cases would
# curl binaries into /usr/local/bin, and the --warm-cache case (which produces a real
# reference tree) would run a ~20-package apt install on whatever box the suite is on.
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

# Guard rails. Everything below writes only under $SCRATCH; the closing isolation case
# proves the machine's own settings and /opt were untouched.
if [ -z "$SCRATCH" ] || [ ! -d "$SCRATCH" ]; then
  echo "could not create a scratch dir" >&2
  exit 1
fi
REAL_USER_SETTINGS="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/settings.json"
REAL_USER_SETTINGS_BEFORE="(absent)"
[ -f "$REAL_USER_SETTINGS" ] && REAL_USER_SETTINGS_BEFORE="$(cksum <"$REAL_USER_SETTINGS")"
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
# must mean the checkout is NOT prepared.
for falsy in false FALSE 0 no n off ""; do
  out="$(env -u CLAUDE_CODE_REMOTE IS_CONTAINERIZED="$falsy" bash "$SETUP_STARTUP" 2>&1)"
  expect_not_contains "IS_CONTAINERIZED='$falsy' is treated as no" "$out" "Syncing dependencies"
done

start "gate/truthy"
out="$(env -u CLAUDE_CODE_REMOTE IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$SCRATCH/vm-empty" \
  bash "$SETUP_STARTUP" 2>&1)"
expect_not_contains "IS_CONTAINERIZED=true opens the gate" "$out" "Not running in a VM/container"

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
printf 'setup_version=1\nsccache_warmed=true\nsccache_warm_commit=abc123\n' \
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
printf 'setup_version=1\nsccache_warmed=false\n' >"$COLD_VM/$MARKER_NAME"
out="$(IS_CONTAINERIZED=true FREECELL_VM_SETUP_DIR="$COLD_VM" bash "$SETUP_STARTUP" 2>&1)"
expect_status "still exits 0 — a cold cache is slow, not broken" 0 "$?"
expect_contains "an empty cache is reported" "$out" "is empty (marker: sccache_warmed=false)"
expect_contains "with the repair that fills it" "$out" "--warm-cache"

start "provisioned/lying-marker"
# The nastier variant: the image build reported success, so the marker says warmed, but the
# objects went somewhere else (a leftover SCCACHE_BUCKET). The directory is the evidence.
LIAR_VM="$SCRATCH/vm-liar"
mkdir -p "$LIAR_VM/sccache"
printf 'setup_version=1\nsccache_warmed=true\nsccache_warm_commit=deadbeef\n' \
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
  env_file_content="$(cat "$ENV_FILE")"
  expect_contains "exports the wrapper to the session" "$env_file_content" "export RUSTC_WRAPPER=sccache"
  expect_contains "exports the disk cache dir" "$env_file_content" "export SCCACHE_DIR="
  expect_contains "retires any inherited remote-bucket config" "$env_file_content" "unset SCCACHE_BUCKET"
  expect_not_contains "writes no credentials into the session env" "$env_file_content" "AWS_SECRET"
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
printf 'setup_version=1\nsccache_warmed=true\nsccache_max_size=20G\n' >"$CAP_VM/$MARKER_NAME"
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
  skip "root bypasses the permission check this case exercises"
else
  RO_VM="$SCRATCH/vm-readonly"
  mkdir -p "$RO_VM/sccache"
  printf 'setup_version=1\n' >"$RO_VM/$MARKER_NAME"
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
# case that proves the actual behaviour the whole design rests on: wire sccache exactly the
# way enable_compile_cache does, compile something through it, and watch the disk cache go
# from empty to holding an object. A two-line crate, so it costs about a second.
if ! command -v sccache >/dev/null 2>&1 || ! command -v cargo >/dev/null 2>&1; then
  skip "needs both cargo and sccache"
else
  PROBE_CACHE="$SCRATCH/probe-cache"
  PROBE_CRATE="$SCRATCH/probe-crate"
  mkdir -p "$PROBE_CACHE" "$PROBE_CRATE/src"
  cat >"$PROBE_CRATE/Cargo.toml" <<'PROBE'
[package]
name = "freecell-cache-probe"
version = "0.0.0"
edition = "2021"
PROBE
  # Unique content, so this compiles for real rather than hitting an object left by an
  # earlier run of the suite.
  printf 'pub fn probe() -> u64 { %s }\n' "$$" >"$PROBE_CRATE/src/lib.rs"

  expect "the probe cache starts empty" \
    test -z "$(find "$PROBE_CACHE" -mindepth 1 -type f -print -quit 2>/dev/null)"

  # Same environment enable_compile_cache builds: remote-backend variables cleared, disk dir
  # exported, server restarted so it picks them up.
  probe_env() {
    env -u SCCACHE_BUCKET -u SCCACHE_ENDPOINT -u SCCACHE_REGION -u SCCACHE_S3_KEY_PREFIX \
      SCCACHE_DIR="$PROBE_CACHE" SCCACHE_CACHE_SIZE=1G "$@"
  }
  probe_env sccache --stop-server >/dev/null 2>&1
  probe_env sccache --start-server >/dev/null 2>&1
  probe_location="$(probe_env sccache --show-stats 2>/dev/null | sed -n 's/^Cache location[[:space:]]*//p')"
  expect "sccache resolved to the probe's disk cache" \
    test "$probe_location" = "Local disk: \"$PROBE_CACHE\""

  # CARGO_INCREMENTAL=0 because sccache refuses to cache incremental compilations — the same
  # reason workspace crates are never served from the image's cache and the dependency tree
  # always is.
  ( cd "$PROBE_CRATE" && probe_env env RUSTC_WRAPPER=sccache CARGO_INCREMENTAL=0 \
      CARGO_TARGET_DIR="$SCRATCH/probe-target" cargo build --quiet ) >"$TMPDIR/probe-build.log" 2>&1
  expect_status "the probe crate builds through the wrapper" 0 "$?"
  expect "and an object landed in the disk cache" \
    test -n "$(find "$PROBE_CACHE" -mindepth 1 -type f -print -quit 2>/dev/null)"
  probe_stats="$(probe_env sccache --show-stats 2>/dev/null)"
  expect_contains "sccache recorded a compile request" "$probe_stats" "Compile requests"
  probe_env sccache --stop-server >/dev/null 2>&1
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

start "setup_env/human"
# README.md and app/README.md now point every contributor at --human, and its whole contract
# is what it does NOT do.
HUMAN_VM="$SCRATCH/vm-human"
human_out="$(FREECELL_VM_SETUP_DIR="$HUMAN_VM" CLAUDE_CONFIG_DIR="$SCRATCH/cfg-human" \
  bash "$SETUP_ENV" --human 2>&1)"
expect_status "exits 0" 0 "$?"
expect "writes nothing under the machine-global directory" test ! -e "$HUMAN_VM"
expect "writes no provisioning marker" test ! -e "$HUMAN_VM/$MARKER_NAME"
expect "creates no Claude Code settings" test ! -e "$SCRATCH/cfg-human/settings.json"
expect_not_contains "does not name a machine-global directory it will not touch" "$human_out" "vm state:"
expect_not_contains "does not install or replace the compile cache binary" "$human_out" "Compile cache"
expect_contains "does warm the dependency sources" "$human_out" "cargo fetch --locked"
expect_contains "and closes with a local next step" "$human_out" "Local setup complete"

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
    CLAUDE_CONFIG_DIR="$SCRATCH/cfg-warm" \
    SCCACHE_BUCKET=a-bucket-that-must-not-win SCCACHE_REGION=auto \
    SCCACHE_ENDPOINT=https://example.invalid \
    bash "$WARM_ISOLATED/setup_env.sh" --warm-cache --best-effort 2>&1 )"
expect_status "exits 0" 0 "$?"
expect_contains "clones because no checkout is in reach" "$warm_out" "throwaway"
expect_contains "warms the cargo source caches from the clone" "$warm_out" "cargo fetch --locked"
expect "the throwaway clone is gone afterwards" test ! -e "$WARM_VM/throwaway-clone"
# The reference tree exists here, so this is the one case that WOULD apt-install. Assert
# the guard fired, not just that the outcome looked unchanged: on a box that already has
# the packages, a real apt run is indistinguishable from no run at all.
expect_contains "the system-dependency guard fires" "$warm_out" "FREECELL_SKIP_SYSTEM_DEPS is set"
expect_contains "and the warm build reaches its own guard" "$warm_out" "FREECELL_SKIP_WARM_BUILD is set"
expect_not_contains "having resolved to the image's disk cache" "$warm_out" "refusing to warm"
expect "so the apt step left no log behind" test ! -e "$TMPDIR/freecell-render-env.log"
expect "a marker was written" test -f "$WARM_VM/$MARKER_NAME"
marker="$(cat "$WARM_VM/$MARKER_NAME")"
expect_contains "recording the source-cache warm" "$marker" "cargo_sources_warmed=true"
expect_contains "and the pinned toolchain's cargo, not the default one" "$marker" "cargo_version=${TOOLCHAIN_PIN%%-*}"

start "marker/carry-forward"
# Repairing an environment means re-running WITHOUT --warm-cache. The marker must keep
# describing the cache that is on disk, not be rewritten to claim it has no provenance.
CARRY_VM="$SCRATCH/vm-carry"
# A real object, not just a subdirectory: "warmed" means files on disk everywhere it is
# tested, and a fixture that is only a directory would bake the looser rule into the suite.
mkdir -p "$CARRY_VM/sccache/x"
printf 'object\n' >"$CARRY_VM/sccache/x/cached-object"
cat >"$CARRY_VM/$MARKER_NAME" <<'MARKER'
setup_version=1
sccache_warmed=true
sccache_warm_commit=deadbeefcafe
MARKER
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$CARRY_VM" CLAUDE_CONFIG_DIR="$SCRATCH/cfg-carry" \
    bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
marker="$(cat "$CARRY_VM/$MARKER_NAME")"
expect_contains "the warm flag survives a repair run" "$marker" "sccache_warmed=true"
expect_contains "so does the commit the cache came from" "$marker" "sccache_warm_commit=deadbeefcafe"

start "marker/carry-forward-needs-objects"
# The same "real objects" rule the other two call sites use. With a looser "any entry" test, a
# directory holding nothing but empty subdirectories carried a stale sccache_warmed=true
# forward — reinstating exactly the lie the round-3 checks exist to catch.
HOLLOW_VM="$SCRATCH/vm-hollow"
mkdir -p "$HOLLOW_VM/sccache/empty-subdir"
cat >"$HOLLOW_VM/$MARKER_NAME" <<'MARKER'
setup_version=1
sccache_warmed=true
sccache_warm_commit=cafef00d
MARKER
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$HOLLOW_VM" CLAUDE_CONFIG_DIR="$SCRATCH/cfg-hollow" \
    bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
marker="$(cat "$HOLLOW_VM/$MARKER_NAME")"
expect_contains "a directory with no files does not carry 'warmed' forward" "$marker" "sccache_warmed=false"
expect_not_contains "nor the commit it claimed" "$marker" "cafef00d"

start "marker/empty-cache"
# The opposite trap: a directory this run merely created is not a cache of unknown origin.
EMPTY_VM="$SCRATCH/vm-empty-cache"
( cd "$WARM_ISOLATED" && FREECELL_VM_SETUP_DIR="$EMPTY_VM" CLAUDE_CONFIG_DIR="$SCRATCH/cfg-empty" \
    bash "$WARM_ISOLATED/setup_env.sh" --best-effort ) >/dev/null 2>&1
marker="$(cat "$EMPTY_VM/$MARKER_NAME" 2>/dev/null)"
expect_contains "an empty cache dir reports as not warmed" "$marker" "sccache_warmed=false"
expect_contains "with no commit claimed" "$marker" "sccache_warm_commit=none"

# ---------------------------------------------------------------------------------
echo ""
echo "setup_env.sh --create-startup-script — the settings.json merge"
# ---------------------------------------------------------------------------------
# Run from a copy outside any checkout, so PROJECT_ROOT is empty and setup_env skips its
# repo-specific steps (system deps, cargo fetch). This isolates the hook registration.
ISOLATED="$SCRATCH/isolated"
mkdir -p "$ISOLATED"
cp "$SETUP_ENV" "$ISOLATED/setup_env.sh"

run_register() { # run_register <config-dir> <vm-dir> [merge-tool]
  ( cd "$ISOLATED" &&
    CLAUDE_CONFIG_DIR="$1" FREECELL_VM_SETUP_DIR="$2" \
      FREECELL_SETTINGS_MERGE_TOOL="${3:-auto}" \
      bash "$ISOLATED/setup_env.sh" --create-startup-script --best-effort 2>&1 )
}
entry_count() { # entry_count <settings-file>
  # grep -c prints 0 AND exits 1 when nothing matches, so a `|| echo 0` fallback would
  # emit two lines and break every numeric comparison downstream.
  local n
  n="$(grep -c 'freecell_session_start_hook.sh' "$1" 2>/dev/null)"
  printf '%s' "${n:-0}"
}

for tool in jq python3; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    start "merge[$tool]"
    skip "$tool is not installed"
    continue
  fi

  start "merge[$tool]/fresh"
  CFG="$SCRATCH/cfg-$tool-fresh"
  VM="$SCRATCH/vm-$tool-fresh"
  run_register "$CFG" "$VM" "$tool" >/dev/null
  expect "creates the settings file" test -f "$CFG/settings.json"
  expect "the result is valid JSON" python3 -c "import json,sys; json.load(open('$CFG/settings.json'))"
  expect "registers exactly one entry" test "$(entry_count "$CFG/settings.json")" -eq 1
  registered="$(cat "$CFG/settings.json")"
  expect_contains "with the context-preserving matcher" "$registered" '"matcher": "startup|resume|fork"'
  expect_contains "and a generous timeout" "$registered" '"timeout": 600'
  expect "the shim is executable" test -x "$VM/freecell_session_start_hook.sh"

  start "merge[$tool]/idempotent"
  before="$(cat "$CFG/settings.json")"
  out="$(run_register "$CFG" "$VM" "$tool")"
  after="$(cat "$CFG/settings.json")"
  expect "a second run leaves the file byte-identical" test "$before" = "$after"
  expect_contains "and says so" "$out" "already registered"

  start "merge[$tool]/coexists"
  CFG2="$SCRATCH/cfg-$tool-other"
  mkdir -p "$CFG2"
  cat >"$CFG2/settings.json" <<'JSON'
{
  "model": "opusmagnum",
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          { "type": "command", "command": "/usr/local/bin/somebody-elses-hook.sh" }
        ]
      }
    ],
    "PreToolUse": [
      {
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": "/usr/local/bin/audit.sh" }]
      }
    ]
  },
  "note": "ünïcode stays raw"
}
JSON
  run_register "$CFG2" "$VM" "$tool" >/dev/null
  merged="$(cat "$CFG2/settings.json")"
  expect_contains "another SessionStart hook survives" "$merged" "somebody-elses-hook.sh"
  expect_contains "an unrelated hook group survives" "$merged" "audit.sh"
  expect_contains "unrelated settings survive" "$merged" "opusmagnum"
  expect_contains "non-ASCII is not re-escaped" "$merged" "ünïcode"
  expect "our entry is added once" test "$(entry_count "$CFG2/settings.json")" -eq 1

  start "merge[$tool]/moved"
  # A moved VM_SETUP_DIR must REPLACE the old entry, not add a second one, and the group
  # left empty by that removal goes with it.
  VM_MOVED="$SCRATCH/vm-$tool-moved"
  run_register "$CFG2" "$VM_MOVED" "$tool" >/dev/null
  expect "still exactly one entry after the move" test "$(entry_count "$CFG2/settings.json")" -eq 1
  moved="$(cat "$CFG2/settings.json")"
  expect_contains "pointing at the new location" "$moved" "$VM_MOVED/freecell_session_start_hook.sh"
  expect_not_contains "and not the old one" "$moved" "$VM/freecell_session_start_hook.sh"
  expect_contains "the unrelated SessionStart hook is still there" "$moved" "somebody-elses-hook.sh"

  start "merge[$tool]/malformed"
  CFG3="$SCRATCH/cfg-$tool-bad"
  mkdir -p "$CFG3"
  printf '{ this is not json\n' >"$CFG3/settings.json"
  before="$(cat "$CFG3/settings.json")"
  out="$(run_register "$CFG3" "$VM" "$tool")"
  expect "malformed settings are left exactly as they were" test "$before" = "$(cat "$CFG3/settings.json")"
  expect_contains "and the refusal is reported" "$out" "refusing to modify"

  start "merge[$tool]/wrong-shape"
  CFG4="$SCRATCH/cfg-$tool-shape"
  mkdir -p "$CFG4"
  printf '{"hooks": {"SessionStart": "not-an-array"}}\n' >"$CFG4/settings.json"
  before="$(cat "$CFG4/settings.json")"
  run_register "$CFG4" "$VM" "$tool" >/dev/null
  expect "a non-array hooks.SessionStart is refused, not rewritten" \
    test "$before" = "$(cat "$CFG4/settings.json")"

  start "merge[$tool]/empty-file"
  CFG5="$SCRATCH/cfg-$tool-empty"
  mkdir -p "$CFG5"
  : >"$CFG5/settings.json"
  run_register "$CFG5" "$VM" "$tool" >/dev/null
  expect "an empty settings file gets the entry" test "$(entry_count "$CFG5/settings.json")" -eq 1
done

if command -v jq >/dev/null 2>&1 && command -v python3 >/dev/null 2>&1; then
  start "merge/agreement"
  # Two implementations of one contract only stay in agreement if something checks.
  A="$SCRATCH/cfg-agree-jq"; B="$SCRATCH/cfg-agree-py"
  mkdir -p "$A" "$B"
  cat >"$A/settings.json" <<'JSON'
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "/opt/old/freecell_session_start_hook.sh" }] },
      { "hooks": [{ "type": "command", "command": "/usr/local/bin/keep-me.sh" }] }
    ]
  }
}
JSON
  cp "$A/settings.json" "$B/settings.json"
  VM_AGREE="$SCRATCH/vm-agree"
  run_register "$A" "$VM_AGREE" jq >/dev/null
  run_register "$B" "$VM_AGREE" python3 >/dev/null
  expect "jq and python3 produce identical output" \
    test "$(cat "$A/settings.json")" = "$(cat "$B/settings.json")"
  expect_contains "both drop the group they emptied and keep the other" \
    "$(cat "$A/settings.json")" "keep-me.sh"
  expect_not_contains "neither keeps the stale shim path" \
    "$(cat "$A/settings.json")" "/opt/old/freecell_session_start_hook.sh"
fi

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

start "shim/parity"
# .claude/hooks/session-start.sh and the heredoc in setup_env.sh must not drift: they are
# the same hook in two homes, and only one of them is exercised on any given machine.
generated="$SCRATCH/shim-generated.sh"
sed -n "/^  cat >\"\$staged\" <<'SHIM'\$/,/^SHIM\$/p" "$SETUP_ENV" | sed '1d;$d' >"$generated"
expect "the generated shim is non-empty" test -s "$generated"
# The shebang is kept, not stripped as a comment: a generated shim that lost it would
# otherwise pass parity and then be run by whatever shell happened to pick it up.
shim_signature() {
  head -1 "$1"
  tail -n +2 "$1" | grep -v '^#' | grep -v '^$'
}
expect "both shims share one implementation" \
  test "$(shim_signature "$generated")" = "$(shim_signature "$PROJECT_ROOT/.claude/hooks/session-start.sh")"
expect "and the generated one keeps its shebang" \
  test "$(head -1 "$generated")" = "#!/usr/bin/env bash"

# ---------------------------------------------------------------------------------
echo ""
echo "Nothing escaped the scratch directory"
# ---------------------------------------------------------------------------------
start "isolation"
# Not "does not exist" — this machine may legitimately be provisioned. What must hold is
# that the suite did not touch it.
expect "the machine-global setup dir is exactly as the suite found it" \
  test "$REAL_VM_DIR_BEFORE" = "$(vm_dir_fingerprint)"
real_after="(absent)"
[ -f "$REAL_USER_SETTINGS" ] && real_after="$(cksum <"$REAL_USER_SETTINGS")"
if [ "$REAL_USER_SETTINGS_BEFORE" = "(absent)" ]; then
  # Said out loud: with no such file to begin with, "unchanged" is a weaker statement than it
  # sounds, and the meaningful half is that the suite did not CREATE one.
  expect "no user settings file existed, and none was created" test ! -e "$REAL_USER_SETTINGS"
else
  expect "this machine's own $REAL_USER_SETTINGS is unchanged" \
    test "$REAL_USER_SETTINGS_BEFORE" = "$real_after"
fi
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
