#!/usr/bin/env bash
# setup_startup.sh — PREPARE a FreeCell checkout, once per session.
#
# Runs from the repo's SessionStart hook (.claude/settings.json ->
# .claude/hooks/session-start.sh) in every cloud/container session. The machine itself was
# built once by .config/utils/setup_env.sh and snapshotted; this script is what turns that
# image plus a fresh checkout into something that can build.
#
# Its stdout becomes the session's context, so it is deliberately terse: about fifteen
# lines on a normal run, ending in one "Ready." line the agent can act on.
#
# The complement of setup_env.sh's contract: THIS script exits non-zero when something is
# actually wrong, and every failure names exactly one repair command.

# Same re-exec guard as setup_env.sh, same reason: `set -o pipefail` is not POSIX.
if [ -z "${BASH_VERSION:-}" ]; then
  if [ -r "$0" ]; then exec bash "$0" "$@"; fi
  echo "error: setup_startup.sh requires bash. Re-run it as: bash setup_startup.sh" >&2
  exit 1
fi
set -uo pipefail

usage() {
  cat <<'USAGE'
Usage: bash .config/utils/setup_startup.sh

Prepare this FreeCell checkout for a session: verify the toolchain, wire the compile
cache, and fetch the dependency sources this branch pins. Takes no options.

It runs only in a container (CLAUDE_CODE_REMOTE=true, or IS_CONTAINERIZED=true to force
it). To set up or repair a machine — including your own — use:

    bash .config/utils/setup_env.sh --human

Environment overrides (for testing; see .config/utils/claude_cloud_setup.md):
  FREECELL_VM_SETUP_DIR   default /opt/freecell-vm-setup
  IS_CONTAINERIZED        force the container gate open
USAGE
}

# Parsed rather than ignored, so a mistyped flag is not silently swallowed into a full sync.
# Every branch happens to exit today; the `shift` is what keeps that from being the only
# reason the loop terminates, so adding a non-exiting flag later cannot hang session start.
while [ $# -gt 0 ]; do
  case "$1" in
    -h | --help) usage; exit 0 ;;
    *) usage >&2; echo "" >&2; echo "error: unrecognized option: $1" >&2; exit 2 ;;
  esac
  # shellcheck disable=SC2317  # unreachable only because every branch above exits today.
  shift
done

# --- (a) Containers only -----------------------------------------------------------
# On a dev machine the environment is set up once and shared across checkouts; these steps
# would re-do that work every session and start a compile-cache server a contributor did
# not ask for.
#
# The rule is "non-empty and not falsy": someone who sets IS_CONTAINERIZED=no means no, and
# reading that as yes is the one failure mode that is impossible to notice.
is_flag_set() {
  case "$(printf '%s' "${1:-}" | tr '[:upper:]' '[:lower:]')" in
    "" | false | f | 0 | no | n | off) return 1 ;;
    *) return 0 ;;
  esac
}
if ! is_flag_set "${CLAUDE_CODE_REMOTE:-}" && ! is_flag_set "${IS_CONTAINERIZED:-}"; then
  # The notice is advice for a person, but this script's stdout is a SessionStart hook's
  # output — context spent from the window of every contributor who ever opens this repo
  # locally. So it is printed when someone is watching a terminal, and the no-op is silent
  # when it is being captured by the hook. (Piping a manual run therefore silences it too;
  # that is the trade, and `--help` says the same thing either way.)
  if [ -t 1 ]; then
    echo "Not running in a VM/container, will not run custom setup."
    echo "  To set up or repair this environment: bash .config/utils/setup_env.sh --human"
    echo "  Set IS_CONTAINERIZED=true to run this script here anyway."
  fi
  exit 0
fi

# --- Constants ---------------------------------------------------------------------
# KEEP IN SYNC with the identical block in .config/utils/setup_env.sh. The two scripts
# cannot share code (that one is fetched and run standalone at image-build time), so these
# paths are a duplicated, documented interface between them — including the _v2 contract
# version, which must be bumped in both at once.
VM_SETUP_DIR="${FREECELL_VM_SETUP_DIR:-/opt/freecell-vm-setup}"
VM_SETUP_MARKER="$VM_SETUP_DIR/.setup_for_freecell_repo_v2"
SCCACHE_DISK_DIR="$VM_SETUP_DIR/sccache"
# The cap is a property of the IMAGE, so the marker's value wins over this default: an image
# baked with FREECELL_SCCACHE_CACHE_SIZE=20G would otherwise be pruned back to 8G by its own
# first session, silently evicting the objects it was baked to carry. An explicit override in
# this session's environment still beats both. Resolved once the marker is located, below.
SCCACHE_MAX_SIZE_DEFAULT="8G"
SCCACHE_MAX_SIZE="${FREECELL_SCCACHE_CACHE_SIZE:-$SCCACHE_MAX_SIZE_DEFAULT}"

# THE R2 RETIREMENT, AT SCRIPT SCOPE. This used to live inside enable_compile_cache, behind
# four gates — so on any early return (no sccache, no marker, no directory, not writable) the
# session kept SCCACHE_BUCKET and an inherited RUSTC_WRAPPER, and every rustc call went to
# the remote bucket while the status line said "cache: off". "No marker" is exactly the state
# between merging this branch and rebuilding the image, so that was the first state a real
# user would hit. Unconditional now, and persist_cache_env() carries it into the session's
# later shells on every path.
#
# Captured FIRST, because after the unset there is no way to tell whether this container had
# anything to retire — and warning about a leftover backend that was never there names five
# no-op unsets and degrades a healthy machine's closing line to "Ready, with warnings above".
INHERITED_CACHE_ENV=false
if [ -n "${SCCACHE_BUCKET:-}${SCCACHE_ENDPOINT:-}${SCCACHE_REGION:-}${SCCACHE_S3_KEY_PREFIX:-}${RUSTC_WRAPPER:-}" ]; then
  INHERITED_CACHE_ENV=true
fi
unset SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX

REPAIR_SETUP="bash .config/utils/setup_env.sh --upgrade-tools"
RENDER_ENV_SCRIPT="app/render-tests/scripts/setup_render_env.sh"

# Extract a key from the provisioning marker written by setup_env.sh. KEEP IN SYNC with
# marker_value() there — same file, same format.
marker_value() {
  [ -f "$VM_SETUP_MARKER" ] || return 1
  sed -n "s/^$1=//p" "$VM_SETUP_MARKER" | tail -1
}

have() { command -v "$1" >/dev/null 2>&1; }
version_ge() {
  [ "$1" = "$2" ] && return 0
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -1)" = "$2" ]
}
version_of() { "$@" 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -1; }

WARNED=0
warn() { echo "  ! $*" >&2; WARNED=1; }

# --- (b) A single error shape, so every failure names exactly one repair ------------
bad_environment() {
  cat >&2 <<EOF

  ✗ $1

    To fix:

        $2

    In a Claude Code cloud sandbox, seeing this usually means the environment's VM setup
    script did not run, or ran against a different image. See
    .config/utils/claude_cloud_setup.md.

EOF
  exit 1
}

# --- (c) Locate the checkout — fatal on a miss --------------------------------------
# Same identity pair as setup_env.sh: the git root holds app/, and app/ is the cargo
# workspace root, so every cargo call below runs from "$PROJECT_ROOT/app" (which is also
# where app/rust-toolchain.toml's pin activates).
is_freecell_root() {
  [ -n "${1:-}" ] && [ -f "$1/app/Cargo.toml" ] && [ -d "$1/app/crates/freecell-engine" ]
}

find_freecell_root() {
  local candidate
  candidate="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." 2>/dev/null && pwd)"
  is_freecell_root "$candidate" && { echo "$candidate"; return 0; }
  candidate="$(git rev-parse --show-toplevel 2>/dev/null)"
  is_freecell_root "$candidate" && { echo "$candidate"; return 0; }
  candidate="$PWD"
  while :; do
    is_freecell_root "$candidate" && { echo "$candidate"; return 0; }
    [ "$candidate" = "/" ] && return 1
    candidate="$(dirname "$candidate")"
  done
}

PROJECT_ROOT="$(find_freecell_root)" || PROJECT_ROOT=""
if [ -z "$PROJECT_ROOT" ]; then
  bad_environment \
    "No FreeCell checkout found (looked for app/Cargo.toml + app/crates/freecell-engine)." \
    "cd into a FreeCell checkout and re-run: bash .config/utils/setup_startup.sh"
fi
CARGO_DIR="$PROJECT_ROOT/app"

# --- (d) Was this VM provisioned for THIS repo? -------------------------------------
# Checked BEFORE the hard dependencies, because a missing marker is usually the reason
# they are about to fail. Not fatal on its own: an unprovisioned machine with a working
# toolchain can still do everything below, just slower.
VM_WAS_PROVISIONED=true
if [ ! -f "$VM_SETUP_MARKER" ]; then
  VM_WAS_PROVISIONED=false
  cat >&2 <<EOF

  ! This VM was not set up for FreeCell: $VM_SETUP_MARKER is missing.

    The environment's VM setup script — .config/utils/claude_code_vm_setup.sh, which
    fetches and runs .config/utils/setup_env.sh — should have run when this machine was
    built, and either did not run, or was an older version of the script.

    Continuing anyway. Expect this to be slow, and expect the checks below to report
    anything the setup script was supposed to provide.

EOF
fi

if [ -z "${FREECELL_SCCACHE_CACHE_SIZE:-}" ]; then
  marker_max_size="$(marker_value sccache_max_size 2>/dev/null)"
  [ -n "${marker_max_size:-}" ] && SCCACHE_MAX_SIZE="$marker_max_size"
fi

# --- (e) Hard dependencies and version floors, before any sync ----------------------
# "Absent" gets its own repair line: setup_env.sh needs cargo in order to install cargo
# tools, so pointing there when cargo is missing sends the reader somewhere that cannot
# help.
have git || bad_environment \
  "git is not installed." \
  "apt-get install -y git    # then re-run: $REPAIR_SETUP"

have rustup || bad_environment \
  "rustup is not installed, so the toolchain pinned by app/rust-toolchain.toml cannot be selected." \
  "curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh    # then: $REPAIR_SETUP"

# rustup reads rust-toolchain.toml from `rustup toolchain install` (no argument) only from
# 1.28.0 on. An older rustup installs nothing, silently leaves the default toolchain
# active, and the failure then surfaces as the version-floor error below — pointing at the
# wrong tool. Absent and too-old have different repairs, so they are separate checks.
RUSTUP_FLOOR="1.28.0"
RUSTUP_VERSION="$(version_of rustup --version)"
if [ -n "$RUSTUP_VERSION" ] && ! version_ge "$RUSTUP_VERSION" "$RUSTUP_FLOOR"; then
  bad_environment \
    "rustup $RUSTUP_VERSION is below $RUSTUP_FLOOR, so it cannot read app/rust-toolchain.toml." \
    "rustup self update"
fi

# --- (f) The pinned toolchain -------------------------------------------------------
# rust-toolchain.toml is COMMITTED here and rustup honors it automatically, so this job is
# verification, not writing. A startup script must never write into a tracked file.
TOOLCHAIN_PIN="$(sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' \
  "$CARGO_DIR/rust-toolchain.toml" 2>/dev/null | head -1)"
[ -n "$TOOLCHAIN_PIN" ] || bad_environment \
  "app/rust-toolchain.toml has no [toolchain] channel, so there is no pin to honor." \
  "git -C $PROJECT_ROOT checkout -- app/rust-toolchain.toml"

# The one remaining `if ! ( … )` with a redirection, kept deliberately: the target is
# /dev/null, which cannot fail to open, and a swallowed failure here is caught immediately by
# the empty-CARGO_VERSION and version-floor checks below.
if ! ( cd "$CARGO_DIR" && rustup toolchain install --no-self-update ) >/dev/null 2>&1; then
  bad_environment \
    "Toolchain $TOOLCHAIN_PIN (app/rust-toolchain.toml) is not installed and could not be installed." \
    "rustup toolchain install $TOOLCHAIN_PIN --profile minimal -c rustfmt -c clippy"
fi

CARGO_VERSION="$( cd "$CARGO_DIR" && version_of cargo --version )"
RUSTC_VERSION="$( cd "$CARGO_DIR" && version_of rustc --version )"
# gpui calls std::hint::cold_path(), stabilized in 1.95.0, so a toolchain below that cannot
# build this workspace at all — and cargo below 1.78 would rewrite the version-4 Cargo.lock
# rather than read it. Checked here, before the fetch, so an old cargo cannot do the damage
# the check exists to prevent.
#
# The floor is the PIN only when the pin is a version number. rust-toolchain.toml's channel
# is a free-form string, and a perfectly valid one — "stable", "nightly-2026-01-01", a
# host-suffixed pin — is not comparable with sort -V: it would sort below any release and
# fail every session start with a repair line that cannot help. So a non-numeric channel
# falls back to the hardcoded floor, which is the constraint that actually matters.
RUST_FLOOR="1.95.0"
if printf '%s' "$TOOLCHAIN_PIN" | grep -qE '^[0-9]+\.[0-9]+(\.[0-9]+)?$'; then
  RUST_FLOOR="$TOOLCHAIN_PIN"
fi
if [ -z "$CARGO_VERSION" ]; then
  bad_environment \
    "cargo is not runnable from $CARGO_DIR." \
    "rustup toolchain install $TOOLCHAIN_PIN --profile minimal -c rustfmt -c clippy"
fi
if ! version_ge "$CARGO_VERSION" "$RUST_FLOOR"; then
  bad_environment \
    "cargo in $CARGO_DIR resolves to $CARGO_VERSION, below the required $RUST_FLOOR (app/rust-toolchain.toml pins '$TOOLCHAIN_PIN'). gpui needs std::hint::cold_path() (stable since 1.95.0), so the workspace will not build." \
    "rustup toolchain install $TOOLCHAIN_PIN --profile minimal -c rustfmt -c clippy"
fi
( cd "$CARGO_DIR" && cargo fmt --version ) >/dev/null 2>&1 ||
  warn "rustfmt is missing from $TOOLCHAIN_PIN; 'cargo fmt --all --check' (a CI gate) will fail. Fix: rustup component add rustfmt --toolchain $TOOLCHAIN_PIN"
( cd "$CARGO_DIR" && cargo clippy --version ) >/dev/null 2>&1 ||
  warn "clippy is missing from $TOOLCHAIN_PIN; the clippy CI gate will fail. Fix: rustup component add clippy --toolchain $TOOLCHAIN_PIN"

# --- (g) The compile cache -----------------------------------------------------------
# sccache against the image's local disk cache. Content-addressed and immutable, so it is
# safe to share across sessions by construction, and path-independent — which is why this
# repo caches rustc OUTPUTS rather than hardlinking a warm target/ in (cargo fingerprints
# embed absolute paths, and a hardlinked target/ would be written through into the
# pristine tree). Design + the rejected alternative: projects/build-cache.md.
CACHE_STATUS="off"
enable_compile_cache() {
  have sccache || { CACHE_STATUS="off (sccache not installed; run $REPAIR_SETUP)"; return 1; }
  # Refuse an unattributable cache: without the marker, a directory at that path was not
  # put there by this repo's setup script.
  if [ "$VM_WAS_PROVISIONED" != true ]; then
    CACHE_STATUS="off (no provisioning marker; not trusting $SCCACHE_DISK_DIR)"
    return 1
  fi
  [ -d "$SCCACHE_DISK_DIR" ] || {
    CACHE_STATUS="off ($SCCACHE_DISK_DIR is missing; run $REPAIR_SETUP)"
    return 1
  }
  # The image is built as root and a session may run as someone else, in which case sccache
  # would start, read happily, and silently store nothing. Better to say so than to report
  # a cache that only half works.
  [ -w "$SCCACHE_DISK_DIR" ] || {
    CACHE_STATUS="off ($SCCACHE_DISK_DIR is not writable by $(id -un); run: chmod 1777 $SCCACHE_DISK_DIR)"
    return 1
  }

  # A running sccache server's storage backend is fixed at start, so any server already
  # running with a different configuration has to go before this one starts. (The variables
  # that would select such a backend were cleared at script scope, above.)
  export SCCACHE_DIR="$SCCACHE_DISK_DIR"
  export SCCACHE_CACHE_SIZE="$SCCACHE_MAX_SIZE"
  sccache --stop-server >/dev/null 2>&1
  SCCACHE_ERROR_LOG="${TMPDIR:-/tmp}/sccache.log" sccache --start-server >/dev/null 2>&1

  # Exact string, not a substring: `*"$SCCACHE_DISK_DIR"*` would also accept
  # `Local disk: "…/sccache-old"`, which is precisely the kind of near-miss this checks for.
  local location
  location="$(sccache --show-stats 2>/dev/null | sed -n 's/^Cache location[[:space:]]*//p')"
  if [ "$location" != "Local disk: \"$SCCACHE_DISK_DIR\"" ]; then
    CACHE_STATUS="off (sccache resolved to '${location:-server not running}', not $SCCACHE_DISK_DIR)"
    return 1
  fi

  # Existence, provenance and writability were all checked above and would all pass on an
  # image whose warm build filled a remote bucket instead of this directory. The directory's
  # CONTENTS are the only thing that distinguishes a warm image from a cold one, and a cache
  # that reports healthy while doing nothing is the exact failure this whole design exists
  # to prevent — so it is checked here, every session, rather than trusted from the marker.
  if [ -z "$(find "$SCCACHE_DISK_DIR" -mindepth 1 -type f -print -quit 2>/dev/null)" ]; then
    local claimed
    claimed="$(marker_value sccache_warmed 2>/dev/null)"
    if [ "$claimed" = true ]; then
      warn "$SCCACHE_DISK_DIR is EMPTY although the VM marker claims it was warmed (commit $(marker_value sccache_warm_commit)) — its objects were removed after the image was built. Fix: rebuild the environment image (see .config/utils/claude_cloud_setup.md)."
    else
      warn "$SCCACHE_DISK_DIR is empty (marker: sccache_warmed=${claimed:-unknown}), so the pinned dep tree will compile from cold this session. Fix: rebuild the environment image, or run: $REPAIR_SETUP --warm-cache (15-25 min — it compiles the dependency tree from scratch, whatever this checkout has already built)"
    fi
  fi

  export RUSTC_WRAPPER=sccache
  CACHE_STATUS="sccache -> $SCCACHE_DISK_DIR"
  return 0
}

# Runs on EVERY path, cache on or off. The unsets above die with this process; the agent's
# later shells inherit whatever the container was started with, so the retirement has to be
# written into the session env file too — and when the cache ends up off, an inherited
# RUSTC_WRAPPER has to go with it, or "off" means "sccache against whatever backend this
# container was configured for" rather than off.
#
# The env file is sourced as shell — the previous, proven implementation of this wiring wrote
# `export X=Y` lines into it — so `unset` works there too. `|| true` keeps a consumer running
# under `set -e` from tripping on it. Nothing written here is a secret; that is the point of
# a local disk cache.
# Everything this script does for the session's LATER shells travels through the env file, so
# not writing it has consequences whether the cache is on or off — and they are different
# consequences, which is why both are reported. There are two ways to get here: no env file
# named, or a named one that cannot be written.
# The two hazards are INDEPENDENT, so they are reported independently rather than as an
# if/else: the cache can be on or off either way, and an inherited backend is equally live in
# both cases. Making them exclusive meant the cache-on branch described a build as "uncached"
# when it would in fact be cached against a dead bucket.
session_env_not_written() { # $1 = what went wrong, as a clause
  if [ "$CACHE_ENABLED" = true ]; then
    CACHE_STATUS="$CACHE_STATUS (THIS PROCESS ONLY — $1)"
    warn "$1, so the compile-cache wiring cannot reach the session's shells. Fix: run this from the SessionStart hook, or export RUSTC_WRAPPER=sccache SCCACHE_DIR=$SCCACHE_DISK_DIR yourself."
  fi
  # Only when something WAS inherited (see INHERITED_CACHE_ENV above). The block carries the
  # remote-backend retirement as well as the cache wiring, and leaving it unwritten leaves
  # that env live in the session's later shells whatever the cache is doing.
  if [ "$INHERITED_CACHE_ENV" = true ]; then
    CACHE_STATUS="$CACHE_STATUS; THE RETIREMENT DID NOT REACH THE SESSION — $1"
    warn "$1, so the remote-backend retirement cannot reach the session's shells: the inherited RUSTC_WRAPPER/SCCACHE_BUCKET stays live and rustc calls will go to it. Fix: unset RUSTC_WRAPPER SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX yourself."
  fi
}

persist_cache_env() {
  if [ -z "${CLAUDE_ENV_FILE:-}" ]; then
    session_env_not_written "CLAUDE_ENV_FILE is unset"
    return 0
  fi
  local signature sentinel
  if [ "$CACHE_ENABLED" = true ]; then
    signature="sccache $SCCACHE_DISK_DIR $SCCACHE_MAX_SIZE"
  else
    signature="off"
  fi
  sentinel="# freecell compile cache: $signature"
  # SessionStart fires more than once per container — startup and resume at minimum, and the
  # matcher in .claude/settings.json also lists `fork`, which the documented source list does
  # not mention and may well be inert. Either way this runs several times, and without the
  # sentinel the env file grows an identical block each time.
  if [ -f "$CLAUDE_ENV_FILE" ] && grep -qxF -- "$sentinel" "$CLAUDE_ENV_FILE" 2>/dev/null; then
    return 0
  fi
  # Assembled first, then written by ONE simple command, because the append's status has to be
  # checked: an unwritable or missing path reaches the identical end state as an unset
  # CLAUDE_ENV_FILE — a cache wired for this process only — and bash's own "No such file or
  # directory" on stderr is not a report anything acts on.
  #
  # The simple-command form is load bearing, and the reason is narrower than it looks.
  # Measured on bash 5.2.21, for a redirection that fails to open:
  #
  #     { echo a; } >>/bad          -> $? is 1                       (detected)
  #     if { echo a; } >>/bad       -> else branch                   (detected)
  #     { echo a; } >>/bad || …     -> fires                         (detected)
  #     if ! { echo a; } >>/bad     -> then branch NOT taken         (SWALLOWED)
  #     if ! ( echo a ) >>/bad      -> then branch NOT taken         (SWALLOWED)
  #     if ! if …; fi >>/bad        -> then branch NOT taken         (SWALLOWED)
  #     if ! while …; done >>/bad   -> then branch NOT taken         (SWALLOWED)
  #     if ! printf x >>/bad        -> then branch taken             (detected)
  #     if ! some_function >>/bad   -> then branch taken             (detected)
  #
  # So the anomaly is `!` applied to ANY compound command — group, subshell, if, while, for,
  # case — not groups in particular; a too-narrow reading of this is what let the subshell
  # form survive a sweep once. `!` in front of a simple command or a function call reports
  # correctly, and so does every form without `!`. Anywhere a compound command's redirection
  # must be checked, run it bare and test `$?` (see the cargo fetch below and warm_sccache in
  # setup_env.sh).
  local block
  block="$sentinel
unset SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX || true"
  if [ "$CACHE_ENABLED" = true ]; then
    block="$block
$(printf 'export SCCACHE_DIR=%q' "$SCCACHE_DISK_DIR")
$(printf 'export SCCACHE_CACHE_SIZE=%q' "$SCCACHE_MAX_SIZE")
export RUSTC_WRAPPER=sccache"
  else
    block="$block
unset RUSTC_WRAPPER || true"
  fi
  # No `2>/dev/null` here: redirections are applied left to right, so `>>` fails before the
  # stderr redirection is installed and bash's own message lands regardless. Suppressing it
  # would only look like it worked, and the message names the path, which is useful.
  if ! printf '%s\n' "$block" >>"$CLAUDE_ENV_FILE"; then
    session_env_not_written "the write to $CLAUDE_ENV_FILE failed"
    return 0
  fi
}

CACHE_ENABLED=false
if enable_compile_cache; then
  CACHE_ENABLED=true
else
  # Whatever the container handed us, it is not a cache this script is willing to vouch for.
  unset RUSTC_WRAPPER
fi
persist_cache_env

# --- (h) Sync the dependency sources this branch pins --------------------------------
BRANCH="$(git -C "$PROJECT_ROOT" branch --show-current 2>/dev/null)"
echo "Syncing dependencies for ${BRANCH:-this checkout}..."
HOST_TRIPLE="$( cd "$CARGO_DIR" && rustc -vV 2>/dev/null | sed -n 's/^host: //p' )"
FETCH_LOG="${TMPDIR:-/tmp}/freecell-cargo-fetch.log"
# --locked, so cargo refuses to update Cargo.lock rather than quietly rewriting it — the
# Rust equivalent of the lockfile-churn problem, solved by the tool instead of detected
# after the fact. Scoped to the host target so the fetch skips the Windows/macOS-only
# crates an unscoped fetch would pull.
#
# Only `cargo fetch`, deliberately: a full `cargo build` at session start would cost many
# minutes on this workspace even fully cached (linking and build scripts are not cached),
# and the agent's first real build pays that anyway. The image's warm registry/git caches
# make this step a couple of seconds.
#
# RUSTC_WRAPPER is cleared for this call (cargo treats an empty value as unset): fetching
# sources needs no compiler cache, and running under one means a broken cache — a leftover
# unreachable bucket, a dead server — surfaces here as a fetch failure whose single named
# repair would be `cargo update`. Handing an autonomous agent a lockfile-rewriting command
# for a compile-cache fault is worse than no repair line at all.
#
# Run bare with $? captured, NOT as `if ! ( … ) >"$FETCH_LOG"`: per the measurements above,
# `!` in front of a subshell swallows a redirection that fails to open, so a TMPDIR pointing
# at a directory that does not exist (or a full disk — target/ runs ~25 GB against ~29 GB
# here) would skip the fetch entirely and still reach the "Ready." line.
# Probed with a simple command, whose redirection status IS reliable, so the log's own
# failure is separated from the fetch's: nothing has run yet, so neither repair below would
# apply, and both causes can be named.
if ! : >>"$FETCH_LOG" 2>/dev/null; then
  bad_environment \
    "could not open $FETCH_LOG, so cargo fetch never ran." \
    "check TMPDIR points at a directory that exists, and that the disk is not full, then re-run: bash .config/utils/setup_startup.sh"
fi
( cd "$CARGO_DIR" && RUSTC_WRAPPER='' cargo fetch --locked ${HOST_TRIPLE:+--target "$HOST_TRIPLE"} ) \
  >"$FETCH_LOG" 2>&1
FETCH_STATUS=$?
if [ "$FETCH_STATUS" -ne 0 ]; then
  # `>&2 2>/dev/null`, in that order. The reverse points fd 2 at /dev/null FIRST and then
  # dups fd 1 from it, so the 15 lines that say WHY the fetch failed are discarded and the
  # agent gets only a path — the same left-to-right redirection trap this file documents.
  tail -15 "$FETCH_LOG" >&2 2>/dev/null
  # Belt and braces: if the cache still managed to be the cause, say so rather than blaming
  # the lockfile.
  if grep -qiE 'sccache|RUSTC_WRAPPER|cache storage' "$FETCH_LOG"; then
    bad_environment \
      "cargo fetch --locked failed with a compile-cache error (full output: $FETCH_LOG)." \
      "sccache --stop-server; unset RUSTC_WRAPPER SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION   # then: bash .config/utils/setup_startup.sh"
  fi
  bad_environment \
    "cargo fetch --locked failed (full output: $FETCH_LOG)." \
    "If you changed dependencies in a Cargo.toml, the lockfile is stale — run: cd app && cargo update"
fi

# --- (i) Verify what the environment setup was supposed to provide -------------------
# After the sync, because some of what is verified is what the sync just placed. The
# lesson these checks encode: a sandbox image is missing one native thing and the failure
# surfaces three layers away, in a link error nobody reads as "install libvulkan-dev".
MISSING_NATIVE=""
if have pkg-config; then
  for lib in x11 xkbcommon wayland-client fontconfig freetype2 vulkan gbm alsa; do
    pkg-config --exists "$lib" 2>/dev/null || MISSING_NATIVE="$MISSING_NATIVE $lib"
  done
else
  MISSING_NATIVE=" pkg-config"
fi
if [ -n "$MISSING_NATIVE" ]; then
  # A warning, not a failure: freecell-core and freecell-engine are GPUI-free and build
  # without any of this. Only freecell-app and render-tests need it.
  warn "native deps missing (${MISSING_NATIVE# }) — freecell-app and render-tests will not build. Fix: bash $RENDER_ENV_SCRIPT"
fi

if ! have cargo-deny; then
  warn "cargo-deny is not installed; 'cargo deny check' (a CI gate) will not run here. Fix: $REPAIR_SETUP"
fi

# --- (j) One closing line -------------------------------------------------------------
if [ "$WARNED" -ne 0 ]; then
  echo "Ready, with warnings above. rustc $RUSTC_VERSION, cargo $CARGO_VERSION (pin $TOOLCHAIN_PIN), cache: $CACHE_STATUS."
else
  echo "Ready. rustc $RUSTC_VERSION, cargo $CARGO_VERSION (pin $TOOLCHAIN_PIN), cache: $CACHE_STATUS."
fi
exit 0
