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
# paths are a duplicated, documented interface between them — including the _v1 contract
# version, which must be bumped in both at once.
VM_SETUP_DIR="${FREECELL_VM_SETUP_DIR:-/opt/freecell-vm-setup}"
VM_SETUP_MARKER="$VM_SETUP_DIR/.setup_for_freecell_repo_v1"
SCCACHE_DISK_DIR="$VM_SETUP_DIR/sccache"
SCCACHE_MAX_SIZE="${FREECELL_SCCACHE_CACHE_SIZE:-8G}"

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
  # running with a different configuration has to go before this one starts. This is also
  # what retires the old Cloudflare-R2-backed setup on a container that still has the
  # SCCACHE_BUCKET/AWS_* variables in its environment.
  unset SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX
  export SCCACHE_DIR="$SCCACHE_DISK_DIR"
  export SCCACHE_CACHE_SIZE="$SCCACHE_MAX_SIZE"
  sccache --stop-server >/dev/null 2>&1
  SCCACHE_ERROR_LOG="${TMPDIR:-/tmp}/sccache.log" sccache --start-server >/dev/null 2>&1

  local location
  location="$(sccache --show-stats 2>/dev/null | sed -n 's/^Cache location[[:space:]]*//p')"
  case "$location" in
    *"$SCCACHE_DISK_DIR"*) ;;
    *)
      CACHE_STATUS="off (sccache resolved to '${location:-server not running}', not $SCCACHE_DISK_DIR)"
      return 1
      ;;
  esac

  # Existence, provenance and writability were all checked above and would all pass on an
  # image whose warm build filled a remote bucket instead of this directory. The directory's
  # CONTENTS are the only thing that distinguishes a warm image from a cold one, and a cache
  # that reports healthy while doing nothing is the exact failure this whole design exists
  # to prevent — so it is checked here, every session, rather than trusted from the marker.
  if [ -z "$(find "$SCCACHE_DISK_DIR" -mindepth 1 -type f -print -quit 2>/dev/null)" ]; then
    local claimed
    claimed="$(marker_value sccache_warmed 2>/dev/null)"
    if [ "$claimed" = true ]; then
      warn "$SCCACHE_DISK_DIR is EMPTY although the VM marker claims it was warmed (commit $(marker_value sccache_warm_commit)) — the image build filled something else. Fix: rebuild the environment image (see .config/utils/claude_cloud_setup.md)."
    else
      warn "$SCCACHE_DISK_DIR is empty (marker: sccache_warmed=${claimed:-unknown}), so the pinned dep tree will compile from cold this session. Fix: rebuild the environment image, or run: $REPAIR_SETUP --warm-cache"
    fi
  fi

  export RUSTC_WRAPPER=sccache
  # Persist to the session so every later shell — and every cargo call the agent makes —
  # sees the same wrapper. Written by value; nothing here is a secret, which is the point
  # of a local disk cache.
  # The env file is sourced as shell — the previous, proven implementation of this wiring
  # wrote `export X=Y` lines into it — so `unset` works there too. `|| true` keeps a
  # consumer running under `set -e` from tripping on it. The variable list is the same one
  # unset live above; letting the two drift is how a stale bucket comes back.
  if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
    {
      printf 'unset SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX || true\n'
      printf 'export SCCACHE_DIR=%q\n' "$SCCACHE_DISK_DIR"
      printf 'export SCCACHE_CACHE_SIZE=%q\n' "$SCCACHE_MAX_SIZE"
      printf 'export RUSTC_WRAPPER=sccache\n'
    } >>"$CLAUDE_ENV_FILE"
  fi
  CACHE_STATUS="sccache -> $SCCACHE_DISK_DIR"
  return 0
}
enable_compile_cache || true

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
if ! ( cd "$CARGO_DIR" && cargo fetch --locked ${HOST_TRIPLE:+--target "$HOST_TRIPLE"} ) \
  >"$FETCH_LOG" 2>&1; then
  tail -15 "$FETCH_LOG" >&2
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
