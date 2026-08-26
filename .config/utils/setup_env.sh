#!/usr/bin/env bash
# setup_env.sh — BUILD or REPAIR a machine for the FreeCell repo.
#
# Two callers, two very different worlds:
#
#   1. A Claude Code cloud environment build. Fetched and run by
#      .config/utils/claude_code_vm_setup.sh (the blob pasted into the environment's
#      "Setup script" field) ONCE, before any repo is checked out, after which the disk
#      is snapshotted and every later session starts from that image. There is no
#      checkout to read, so this script discovers or clones one, and everything it wants
#      to survive is written OUTSIDE any checkout (see VM_SETUP_DIR).
#
#   2. A contributor's own machine: `bash .config/utils/setup_env.sh --human`, which
#      installs the toolchain and warms the dependency sources and touches nothing else.
#
# Per-session preparation is a different script: .config/utils/setup_startup.sh, run by
# the repo's SessionStart hook. This one builds machines; that one prepares checkouts.
#
# Human doc (what to click in the cloud UI, what to expect, how to debug):
# .config/utils/claude_cloud_setup.md

# --- (a) Re-exec under bash -------------------------------------------------------
# `set -o pipefail` is not POSIX. If a runner ignores the shebang and starts this with
# `sh`, the shell dies on the next line — and as a cloud setup script, a non-zero exit
# means sessions never start, before --best-effort ever gets a chance to suppress it.
if [ -z "${BASH_VERSION:-}" ]; then
  if [ -r "$0" ]; then exec bash "$0" "$@"; fi
  echo "error: setup_env.sh requires bash. Re-run it as: bash setup_env.sh" >&2
  exit 1
fi
set -uo pipefail

# There is deliberately no `set -e`: --best-effort has to own the exit code, so every
# step routes its failure through fail() and the script always reaches finish().

# --- (b) CONFIGURATION ------------------------------------------------------------
# Same defaults as the flags, so this file can also be pasted whole as a fallback.
UPGRADE_TOOLS=false          # --upgrade-tools
BEST_EFFORT=false            # --best-effort
WARM_CACHE=false             # --warm-cache
HUMAN=false                  # --human

# --- Constants --------------------------------------------------------------------
# $HOME is referenced below — install_prebuilt_binary's `$HOME/.local/bin` fallback and the
# post-rustup-install `export PATH="$HOME/.cargo/bin:$PATH"`. Under `set -u` an unset HOME
# does not fail when those lines are PARSED; it kills the shell outright WHEN THEY EXECUTE,
# mid-run, bypassing finish() and exiting non-zero no matter what --best-effort says. As the
# script the pasted cloud wrapper runs, that is precisely the never-exit-non-zero failure the
# whole best-effort design exists to prevent — so HOME is normalized once here rather than
# defended at each site.
if [ -z "${HOME:-}" ]; then
  HOME="$(getent passwd "$(id -u)" 2>/dev/null | cut -d: -f6)"
  [ -n "$HOME" ] || HOME="/root"
  export HOME
  echo "warning: HOME was unset; using $HOME." >&2
fi

REPO_URL="${FREECELL_REPO_URL:-https://github.com/scosman/freecell}"
REPO_REF="${FREECELL_SETUP_REF:-main}"

# Machine-global state, outside every checkout, so it survives the environment snapshot.
# KEEP IN SYNC with the identical block in .config/utils/setup_startup.sh — the two
# scripts cannot share code (this one gets fetched and run standalone), so these paths
# are a duplicated, documented interface between them.
VM_SETUP_DIR="${FREECELL_VM_SETUP_DIR:-/opt/freecell-vm-setup}"
# The _v2 is a CONTRACT VERSION, not decoration: when what this script provides changes
# incompatibly, bump it here and in setup_startup.sh, so machines provisioned by the
# older script report as not-set-up rather than ready.
#
# v1 -> v2: `sccache_warmed=true` used to mean "the warm build exited 0". It now means "the
# cache directory was confirmed to hold objects". A v1 image can therefore carry a
# sccache_warmed=true that is not true in the v2 sense — which is exactly the ambiguity a
# contract version exists to remove, so the version moved rather than the check being
# quietly retrofitted onto images that never ran it.
VM_SETUP_MARKER="$VM_SETUP_DIR/.setup_for_freecell_repo_v2"
SCCACHE_DISK_DIR="$VM_SETUP_DIR/sccache"

# sccache's disk cache is content-addressed and immutable, so sharing one directory
# across every session on this machine is safe by construction, and it is
# path-independent — unlike a warm target/, whose fingerprints embed absolute paths.
# The cap matters: this directory is baked into the image and comes out of the same
# fixed disk allowance a session's target/ (up to ~25 GB) has to fit in.
SCCACHE_MAX_SIZE="${FREECELL_SCCACHE_CACHE_SIZE:-8G}"

# SCCACHE_BUCKET BEATS SCCACHE_DIR — setting the directory does not select the disk backend,
# it only says where the disk backend would live. A cloud environment build inherits the
# environment's variables, so an operator who upgrades without deleting a leftover
# SCCACHE_BUCKET would otherwise spend tens of minutes warming a remote bucket (or, with its
# token already revoked, warming nothing) while $SCCACHE_DISK_DIR stays empty — and every
# session for the next week would report a healthy local cache and compile from cold.
# Cleared here, once, for everything this script runs. setup_startup.sh clears the same list
# for the same reason; keep the two in sync.
unset SCCACHE_BUCKET SCCACHE_ENDPOINT SCCACHE_REGION SCCACHE_S3_KEY_PREFIX

# Version floors. Each one is a thing that silently breaks below it, so each is checked
# BEFORE the step it would break.
#   rustc/cargo 1.95.0 — gpui calls std::hint::cold_path(), stabilized in 1.95.0, so
#     1.94 cannot build this workspace at all. This is also app/rust-toolchain.toml's pin
#     and the workspace MSRV; the literal here is the fallback used when no checkout is
#     in reach to read the pin from.
#   rustup 1.28.0 — `rustup toolchain install` with no argument reads rust-toolchain.toml
#     only from 1.28.0 on; older rustup installs nothing and leaves the default toolchain
#     active, which then fails the 1.95.0 floor three layers away.
#   Cargo.lock is format version 4, which cargo < 1.78 rewrites rather than reads. The
#     1.95.0 floor subsumes it; noted so a future floor drop does not lose the constraint.
RUST_TOOLCHAIN_FALLBACK_PIN="1.95.0"
RUSTUP_FLOOR="1.28.0"
# Pinned rather than @latest: a bare latest is one broken publish away from breaking every
# session. Bump deliberately.
SCCACHE_PIN="0.8.2"
SCCACHE_URL="https://github.com/mozilla/sccache/releases/download/v${SCCACHE_PIN}/sccache-v${SCCACHE_PIN}-x86_64-unknown-linux-musl.tar.gz"
# The exact cargo-deny .github/workflows/checks.yml gates on. Same version locally means
# a local `cargo deny check` and the CI gate cannot disagree.
CARGO_DENY_PIN="0.19.9"
CARGO_DENY_URL="https://github.com/EmbarkStudios/cargo-deny/releases/download/${CARGO_DENY_PIN}/cargo-deny-${CARGO_DENY_PIN}-x86_64-unknown-linux-musl.tar.gz"

# --- Reporting --------------------------------------------------------------------
FAILED=0
# Every note() is a DEGRADATION: something this script was supposed to provide is not there
# (no compile cache, no cargo-deny, no render stack), or was deliberately skipped. They are
# counted because the closing line is the operator's only view of an image build, and an
# unqualified "complete." after three of them is a success signal that outruns what was
# actually done. Invariant 1 forbids exiting non-zero; it does not forbid saying so — the
# same trade setup_startup.sh makes with its "Ready, with warnings above." line.
NOTED=0
fail() { echo "error: $*" >&2; FAILED=1; }
note() { echo "  ! $*" >&2; NOTED=1; }
step() { echo "==> $*"; }

finish() {
  local closing="${1:-Environment setup complete.}"
  if [ "$FAILED" -ne 0 ]; then
    if [ "$BEST_EFFORT" = true ]; then
      echo "setup_env.sh: finished with errors (best effort, exiting 0)." >&2
      exit 0
    fi
    exit 1
  fi
  if [ "$NOTED" -ne 0 ]; then
    echo "$closing"
    echo "  ! WITH WARNINGS — every '!' line above is something this machine does not have."
    echo "    Read them before treating this image as provisioned."
    exit 0
  fi
  echo "$closing"
  exit 0
}

usage() {
  cat <<'USAGE'
Usage: bash .config/utils/setup_env.sh [options]

Build or repair a machine for the FreeCell repo. Run once per environment (cloud), or
once per laptop (--human). Per-session work lives in .config/utils/setup_startup.sh.

Options:
  --upgrade-tools           Install/upgrade the toolchain without asking when it is
                            missing or below the floor.
  --best-effort             Never exit non-zero. REQUIRED for cloud setup scripts: a
                            failing setup script stops sessions from ever starting.
  --warm-cache              Populate this machine's caches. With no checkout in reach,
                            clone the repo to a throwaway directory first. Fills
                            ~/.cargo/{registry,git} and the sccache disk cache, then
                            deletes the clone (its target/ included).
  --human                   Contributor mode for a personal machine: install the pinned
                            toolchain and warm the dependency sources, offer the optional
                            render-test system dependencies, and write nothing outside
                            the checkout and ~/.cargo.
  -h, --help                This text.

Environment overrides (all exist so this is testable without writing to /opt or cloning
over the network):
  FREECELL_VM_SETUP_DIR        default /opt/freecell-vm-setup
  FREECELL_SETUP_REF           branch or tag to clone for --warm-cache (default main).
                               Not a commit SHA — a depth-1 clone cannot target one.
  FREECELL_REPO_URL            default https://github.com/scosman/freecell
  FREECELL_SCCACHE_CACHE_SIZE  default 8G
  FREECELL_SKIP_WARM_BUILD     set to skip --warm-cache's workspace build (the dependency
                               source caches are still warmed)
  FREECELL_SKIP_TOOL_INSTALL   set to never download or replace a binary in a system bin
                               directory (sccache, cargo-deny)
  FREECELL_SKIP_SYSTEM_DEPS    set to never run the apt install in
                               app/render-tests/scripts/setup_render_env.sh

This script does NOT register a SessionStart hook. FreeCell's hook is committed source
(.claude/settings.json -> .claude/hooks/session-start.sh), so version control registers it.
On a machine that does not honor project hooks, add an entry to ~/.claude/settings.json by
hand: a SessionStart group with "matcher": "startup|resume|fork" whose command is
<checkout>/.claude/hooks/session-start.sh.
USAGE
}

# --- (c) Argument parsing ---------------------------------------------------------
# Unknown flags are collected rather than fatal, so --best-effort (which may be parsed
# after the bad flag) still gets to own the exit code.
UNKNOWN_ARGS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --upgrade-tools) UPGRADE_TOOLS=true ;;
    --best-effort) BEST_EFFORT=true ;;
    --warm-cache) WARM_CACHE=true ;;
    --human) HUMAN=true ;;
    -h | --help) usage; exit 0 ;;
    *) UNKNOWN_ARGS+=("$1") ;;
  esac
  shift
done
if [ "${#UNKNOWN_ARGS[@]}" -gt 0 ]; then
  usage >&2
  fail "unrecognized option(s): ${UNKNOWN_ARGS[*]}"
  finish
fi

# --human is a personal machine: nothing machine-global, nothing snapshot-shaped.
if [ "$HUMAN" = true ]; then
  WARM_CACHE=false
fi

# --- Helpers ----------------------------------------------------------------------
have() { command -v "$1" >/dev/null 2>&1; }

# True when $1 >= $2, comparing dotted versions. sort -V, not string compare: "1.9.0"
# sorts after "1.10.0" as a string and that is exactly the kind of floor check that
# passes for two years and then quietly stops working.
version_ge() {
  [ "$1" = "$2" ] && return 0
  [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | head -1)" = "$2" ]
}

# First dotted version-looking token in a `--version` line.
version_of() { "$@" 2>/dev/null | head -1 | grep -oE '[0-9]+\.[0-9]+(\.[0-9]+)?' | head -1; }

# Write $2 to $1 through a staged sibling, then rename. A truncating redirect that then
# fails leaves a zero-byte file behind — and setup_startup.sh only tests that the marker
# EXISTS, so that would turn the one provenance check into a false positive.
write_atomic() {
  local target="$1" content="$2" staged
  staged="$target.$$"
  printf '%s' "$content" >"$staged" || { rm -f "$staged"; return 1; }
  mv -f "$staged" "$target" || { rm -f "$staged"; return 1; }
}

# Extract a key from the existing marker, so a repair run can carry forward facts about a
# cache directory it did not touch (see write_marker).
marker_value() {
  [ -f "$VM_SETUP_MARKER" ] || return 1
  sed -n "s/^$1=//p" "$VM_SETUP_MARKER" | tail -1
}

# Download $1, unpack it, and install the single binary named $2 into a writable bin dir.
# Prebuilt binaries only — never compile a tool we are installing to make compiles faster.
install_prebuilt_binary() {
  local url="$1" binary="$2" tmp install_dir found existing
  # One choke point for every write into a system bin directory, so a caller that must not
  # mutate the machine (the test suite, a dry run) can opt out in one place.
  if [ -n "${FREECELL_SKIP_TOOL_INSTALL:-}" ]; then
    note "FREECELL_SKIP_TOOL_INSTALL is set; not installing $binary."
    return 1
  fi
  if [ "$(uname -sm)" != "Linux x86_64" ]; then
    note "no prebuilt $binary for $(uname -sm); skipping."
    return 1
  fi
  tmp="$(mktemp -d)" || return 1
  if ! curl -fsSL --retry 3 --retry-delay 2 -o "$tmp/dl.tar.gz" "$url"; then
    note "could not download $url (proxy/egress?); skipping $binary."
    rm -rf "$tmp"
    return 1
  fi
  tar -xzf "$tmp/dl.tar.gz" -C "$tmp" || { rm -rf "$tmp"; return 1; }
  found="$(find "$tmp" -type f -name "$binary" -perm -u+x | head -1)"
  if [ -z "$found" ]; then
    note "$url did not contain a '$binary' binary; skipping."
    rm -rf "$tmp"
    return 1
  fi
  install_dir="/usr/local/bin"
  if [ ! -w "$install_dir" ]; then
    install_dir="$HOME/.local/bin"
    # Worth saying out loud: this process gets the directory on its PATH, but a later
    # session does not inherit that, so the marker can record a version of a tool no
    # session will find.
    note "/usr/local/bin is not writable; installing $binary into $install_dir instead — add it to PATH or sessions will not see it."
  fi
  mkdir -p "$install_dir" || { rm -rf "$tmp"; return 1; }
  existing=""
  [ -e "$install_dir/$binary" ] && existing=" (replacing the one already there)"
  install -m 755 "$found" "$install_dir/$binary" || { rm -rf "$tmp"; return 1; }
  rm -rf "$tmp"
  # Announced, not silent: overwriting a binary in a system directory is not something to
  # discover later from a version string.
  echo "    installed $binary -> $install_dir/$binary$existing"
  case ":$PATH:" in
    *":$install_dir:"*) ;;
    *) export PATH="$install_dir:$PATH" ;;
  esac
  return 0
}

# --- (d) Project-root discovery, validated ----------------------------------------
# Never derive the root from $BASH_SOURCE alone: it is unset when the body is fed on
# stdin, `cd ""` then succeeds as a no-op, and the script goes on to write files two
# directories above $PWD.
#
# The identity check is two facts only this repo has. Note the shape: the GIT root holds
# app/, and the CARGO WORKSPACE root is app/ itself — so every cargo invocation in these
# scripts runs from "$PROJECT_ROOT/app", which is also where the pinned toolchain
# activates.
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

# The pin lives in app/rust-toolchain.toml; read it when a tree is in reach, so a pin bump
# needs no edit here.
read_toolchain_pin() {
  local file="$1/app/rust-toolchain.toml"
  [ -f "$file" ] || return 1
  sed -n 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "$file" | head -1
}

# --- Reference tree: a checkout to read the pin from and build ---------------------
# Clone INSIDE VM_SETUP_DIR: it keeps the multi-GB target/ this build produces on the same
# device as everything else sized for it, and makes the clone trivially removable with the
# rest of the machine-global state.
THROWAWAY_CLONE=""
# shellcheck disable=SC2317  # invoked indirectly, by the EXIT trap below.
cleanup_throwaway() {
  [ -n "$THROWAWAY_CLONE" ] && rm -rf "$THROWAWAY_CLONE"
  return 0
}
trap cleanup_throwaway EXIT

# Sets THROWAWAY_CLONE on success. Deliberately NOT a function that echoes the path: a
# command substitution runs it in a subshell, where the assignment that the EXIT trap and
# the cleanup below depend on would be discarded — and the clone (with a multi-GB target/
# in it) would be left behind on the image.
make_throwaway_clone() {
  local clone="$VM_SETUP_DIR/throwaway-clone"
  mkdir -p "$VM_SETUP_DIR" || return 1
  rm -rf "$clone"
  # --branch accepts a branch or a tag, not a commit SHA; a depth-1 clone of an arbitrary
  # SHA needs a fetch-by-object instead. Documented as "branch or tag" rather than
  # supported, because nothing in this design needs to warm from a detached commit.
  git clone --quiet --depth 1 --branch "$REPO_REF" "$REPO_URL" "$clone" || return 1
  is_freecell_root "$clone" || { rm -rf "$clone"; return 1; }
  THROWAWAY_CLONE="$clone"
  return 0
}

echo "FreeCell environment setup"
echo "  repo:        $REPO_URL @ $REPO_REF"
# --human writes nothing machine-global, so naming a directory it will not touch would just
# invite someone to go looking for it.
[ "$HUMAN" = false ] && echo "  vm state:    $VM_SETUP_DIR"
[ -n "$PROJECT_ROOT" ] && echo "  checkout:    $PROJECT_ROOT"

REFERENCE_TREE=""
WARM_TREE_COMMIT=""
if [ -n "$PROJECT_ROOT" ]; then
  REFERENCE_TREE="$PROJECT_ROOT"
  WARM_TREE_COMMIT="$(git -C "$PROJECT_ROOT" rev-parse HEAD 2>/dev/null)"
elif [ "$WARM_CACHE" = true ]; then
  if ! have git; then
    note "git is not installed, so there is no way to reach a checkout; skipping every repo-specific step."
  else
    step "Cloning $REPO_URL@$REPO_REF (throwaway; no checkout is present at build time)"
    if make_throwaway_clone; then
      REFERENCE_TREE="$THROWAWAY_CLONE"
      WARM_TREE_COMMIT="$(git -C "$REFERENCE_TREE" rev-parse HEAD 2>/dev/null)"
    else
      REFERENCE_TREE=""
      # A warning, not a failure: warming is an optimization, the machine is usable
      # without it, and setup_startup.sh re-checks everything that must actually be true.
      note "could not clone $REPO_URL@$REPO_REF; skipping every repo-specific step."
      note "if the repo is private, the environment build has no credentials for it — expect slow first builds."
    fi
  fi
fi

TOOLCHAIN_PIN="$RUST_TOOLCHAIN_FALLBACK_PIN"
if [ -n "$REFERENCE_TREE" ]; then
  TOOLCHAIN_PIN="$(read_toolchain_pin "$REFERENCE_TREE")" || TOOLCHAIN_PIN="$RUST_TOOLCHAIN_FALLBACK_PIN"
  [ -n "$TOOLCHAIN_PIN" ] || TOOLCHAIN_PIN="$RUST_TOOLCHAIN_FALLBACK_PIN"
fi

# --- (e) Toolchain floor checks, before anything that they would break -------------
step "Rust toolchain (pin: $TOOLCHAIN_PIN)"

if ! have curl; then
  fail "curl is not installed. Install it (apt-get install -y curl), then re-run this script."
fi

RUSTUP_OK=false
if ! have rustup; then
  # "Absent" and "too old" have different fixes, and the absent case cannot run the
  # repair — there is nothing to upgrade.
  if [ "$UPGRADE_TOOLS" = true ] && have curl; then
    step "Installing rustup (not present)"
    if curl -fsSL --proto '=https' --tlsv1.2 https://sh.rustup.rs |
      sh -s -- -y --no-modify-path --default-toolchain none >/dev/null 2>&1; then
      export PATH="$HOME/.cargo/bin:$PATH"
      RUSTUP_OK=true
    else
      fail "could not install rustup. Install it by hand: https://rustup.rs"
    fi
  else
    fail "rustup is not installed. Install it (https://rustup.rs), then re-run this script."
  fi
else
  rustup_version="$(version_of rustup --version)"
  if [ -n "$rustup_version" ] && ! version_ge "$rustup_version" "$RUSTUP_FLOOR"; then
    if [ "$UPGRADE_TOOLS" = true ]; then
      step "Upgrading rustup $rustup_version -> >= $RUSTUP_FLOOR"
      rustup self update >/dev/null 2>&1 ||
        fail "rustup $rustup_version is below $RUSTUP_FLOOR and 'rustup self update' failed. Upgrade it by hand."
      RUSTUP_OK=true
    else
      fail "rustup $rustup_version is below $RUSTUP_FLOOR (it cannot read rust-toolchain.toml). Run: rustup self update"
    fi
  else
    RUSTUP_OK=true
  fi
fi

if [ "$RUSTUP_OK" = true ]; then
  # `rustup toolchain install <channel> --profile minimal -c ...` rather than the bare
  # form, so this works identically with and without a checkout to read the file from.
  step "Installing toolchain $TOOLCHAIN_PIN (+ rustfmt, clippy)"
  rustup toolchain install "$TOOLCHAIN_PIN" --profile minimal \
    --component rustfmt --component clippy --no-self-update >/dev/null 2>&1 ||
    fail "could not install toolchain $TOOLCHAIN_PIN. Run: rustup toolchain install $TOOLCHAIN_PIN --profile minimal -c rustfmt -c clippy"
fi

# --- sccache: the machine's compile cache -----------------------------------------
# Local disk, NOT a remote object store. Every hit off a remote bucket is an HTTPS round
# trip through the sandbox proxy; a directory baked into the image is a local read, needs
# no credentials, and cannot leave a long-lived secret on a snapshotted disk.
# Skipped entirely under --human: that mode promises to write nothing outside the checkout
# and ~/.cargo, and the install would be pointless anyway — nothing in --human sets
# RUSTC_WRAPPER, and setup_startup.sh (which would) refuses to run outside a container.
if [ "$HUMAN" = false ]; then
  step "Compile cache (sccache $SCCACHE_PIN, local disk)"
  if have sccache; then
    sccache_version="$(version_of sccache --version)"
    if [ -n "$sccache_version" ] && ! version_ge "$sccache_version" "$SCCACHE_PIN"; then
      note "sccache $sccache_version is below $SCCACHE_PIN; replacing it."
      install_prebuilt_binary "$SCCACHE_URL" sccache || note "leaving sccache $sccache_version in place."
    fi
  elif ! install_prebuilt_binary "$SCCACHE_URL" sccache; then
    note "no sccache on this machine; builds will work, just uncached."
  fi

  if have sccache; then
    # 1777, not the default 0755: this image builds as root while sessions may run as
    # another user (see claude_cloud_setup.md), and a cache only root can write is a cache
    # that silently does nothing. Sticky, so one session cannot delete another's entries.
    if mkdir -p "$SCCACHE_DISK_DIR"; then
      chmod 1777 "$SCCACHE_DISK_DIR" 2>/dev/null ||
        note "could not make $SCCACHE_DISK_DIR world-writable; sessions running as another user will not be able to use it."
    else
      note "could not create $SCCACHE_DISK_DIR; sccache will fall back to its per-user cache."
    fi
  fi
fi

# --- cargo-deny: the licenses/advisories gate -------------------------------------
if [ "$HUMAN" = false ]; then
  step "cargo-deny $CARGO_DENY_PIN"
  cargo_deny_version="$(version_of cargo-deny --version)"
  if [ -z "$cargo_deny_version" ] || ! version_ge "$cargo_deny_version" "$CARGO_DENY_PIN"; then
    install_prebuilt_binary "$CARGO_DENY_URL" cargo-deny ||
      note "cargo-deny not installed; 'cargo deny check' will need it installed first."
  fi
fi

# --- (f) System dependencies for the GPUI/render stack ----------------------------
# app/render-tests/scripts/setup_render_env.sh is the repo's single source of truth for
# "what must be installed to build the GPUI app and capture pixels", and CI runs the same
# script — so environments cannot drift. It is idempotent, and it is a slow apt run, which
# is exactly the kind of cost that belongs in an image rather than in every session.
RENDER_ENV_INSTALLED=false
RENDER_ENV_SCRIPT_REL="app/render-tests/scripts/setup_render_env.sh"
install_render_env() {
  local script="$REFERENCE_TREE/$RENDER_ENV_SCRIPT_REL" log
  if [ -n "${FREECELL_SKIP_SYSTEM_DEPS:-}" ]; then
    note "FREECELL_SKIP_SYSTEM_DEPS is set; not running $RENDER_ENV_SCRIPT_REL."
    return 1
  fi
  [ -f "$script" ] || { note "$RENDER_ENV_SCRIPT_REL not found in the reference tree; skipping system deps."; return 1; }
  # No mkdir of VM_SETUP_DIR here: the log goes to TMPDIR, and this path is reachable under
  # --human (on a "yes" answer), which promises to write nothing under /opt.
  log="${TMPDIR:-/tmp}/freecell-render-env.log"
  step "System dependencies ($RENDER_ENV_SCRIPT_REL)"
  if bash "$script" >"$log" 2>&1; then
    RENDER_ENV_INSTALLED=true
    return 0
  fi
  note "$RENDER_ENV_SCRIPT_REL failed; last lines of $log:"
  tail -5 "$log" >&2
  note "the GPUI app and the pixel suite will not build until those packages are installed."
  return 1
}

if [ -n "$REFERENCE_TREE" ]; then
  if [ "$HUMAN" = true ]; then
    # Never apt-install on a contributor's machine without asking.
    echo ""
    echo "The GPUI build + render-test system packages can be installed with:"
    echo "    bash $RENDER_ENV_SCRIPT_REL"
    if [ -t 0 ]; then
      read -r -p "Run it now (needs sudo)? [y/N] " reply
      case "$reply" in [yY]*) install_render_env ;; *) echo "Skipped." ;; esac
    else
      echo "(not a terminal; skipping — run it yourself when you need the render stack)"
    fi
  elif [ "$(id -u)" -eq 0 ] || have sudo; then
    install_render_env
  else
    note "not root and no sudo; skipping the system dependency install."
  fi
fi

# --- (g) Warm this machine's caches -----------------------------------------------
# Everything here is a warning-and-fall-through on failure. The machine is usable without
# a warm cache, and marking the run failed would flip the closing line to "resolve the
# errors above" on a VM that is fine.
CARGO_CACHE_WARMED=false
SCCACHE_WARMED_THIS_RUN=false

warm_cargo_sources() {
  local host
  host="$(rustc -vV 2>/dev/null | sed -n 's/^host: //p')"
  step "Warming ~/.cargo/{registry,git} (cargo fetch --locked)"
  # Scoped to this host target so the fetch skips the Windows/macOS-only crates an
  # unscoped fetch would pull. --locked because a lockfile rewrite during a cache warm is
  # a change nobody asked for and nobody would see.
  ( cd "$REFERENCE_TREE/app" && cargo fetch --locked ${host:+--target "$host"} ) >/dev/null 2>&1
}

# Echoes the backend the RUNNING sccache server resolved to. The server decides this once,
# at start, from its own environment — so this is the only way to know what a build actually
# wrote to, as opposed to what the environment asked for.
sccache_cache_location() {
  SCCACHE_DIR="$SCCACHE_DISK_DIR" SCCACHE_CACHE_SIZE="$SCCACHE_MAX_SIZE" \
    sccache --show-stats 2>/dev/null | sed -n 's/^Cache location[[:space:]]*//p'
}

# Non-empty when the cache directory holds at least one file. The artifact, not the claim:
# an exit code from a build says the build succeeded, not that anything landed here.
sccache_disk_has_objects() {
  [ -n "$(find "$SCCACHE_DISK_DIR" -mindepth 1 -type f -print -quit 2>/dev/null)" ]
}

warm_sccache() {
  local log avail_gb location
  # Checked BEFORE the build, not after — the point is not to spend half an hour filling the
  # wrong store — and before the skip knob, so this stays a real check rather than one that
  # only ever runs on the expensive path. The caller started the server with the disk
  # settings; if it did not land on the disk backend, something here still overrides them.
  # Exact string, not a substring: `*"$SCCACHE_DISK_DIR"*` would also accept
  # `Local disk: "…/sccache-old"`, which is precisely the kind of near-miss this checks for.
  location="$(sccache_cache_location)"
  if [ "$location" != "Local disk: \"$SCCACHE_DISK_DIR\"" ]; then
    note "sccache resolved to '${location:-server not running}', not $SCCACHE_DISK_DIR; refusing to warm."
    note "something in this environment still selects a different backend (SCCACHE_BUCKET?). Not building."
    return 1
  fi

  # Re-provisioning a machine whose sccache is already full does not need the build again,
  # and the test suite exercises the clone/fetch/marker path without paying for it.
  if [ -n "${FREECELL_SKIP_WARM_BUILD:-}" ]; then
    note "FREECELL_SKIP_WARM_BUILD is set; not building. The sccache disk cache is unchanged."
    return 1
  fi
  # A full workspace build needs room for target/ on top of everything already on this
  # disk. Better to skip the build and keep the (already large) source-cache win than to
  # fill the image's filesystem during its own construction.
  avail_gb="$(df -BG --output=avail "$VM_SETUP_DIR" 2>/dev/null | tail -1 | tr -dc '0-9')"
  if [ -n "$avail_gb" ] && [ "$avail_gb" -lt 20 ]; then
    note "only ${avail_gb}G free; skipping the warm build. Dependency sources are still cached."
    return 1
  fi

  log="${TMPDIR:-/tmp}/freecell-warm-build.log"
  step "Filling the sccache disk cache (cargo build --workspace; this takes a while)"
  # The build runs with the SAME cargo profile a session uses. sccache keys include the
  # rustc command line, so warming with CI's debug/incremental trims would produce a cache
  # that misses on every single session build — a slower machine that looks provisioned.
  #
  # sccache refuses to cache incremental compilations, and cargo's dev profile is
  # incremental for WORKSPACE crates only; the pinned dependency tree (gpui/zed,
  # gpui-component, the ironcalc fork) is built non-incrementally and is what this fills.
  # That tree is also the entire reason this cache exists.
  ( cd "$REFERENCE_TREE/app" &&
    SCCACHE_DIR="$SCCACHE_DISK_DIR" SCCACHE_CACHE_SIZE="$SCCACHE_MAX_SIZE" \
      RUSTC_WRAPPER=sccache cargo build --workspace --locked ) >"$log" 2>&1
  local status=$?
  if [ "$status" -ne 0 ]; then
    note "the warm build failed (exit $status); last lines of $log:"
    tail -10 "$log" >&2
    return 1
  fi
  # A green build is not evidence of a warm cache. Check the directory itself, so a machine
  # can never be recorded as warmed while holding nothing.
  if ! sccache_disk_has_objects; then
    note "the build succeeded but $SCCACHE_DISK_DIR is still empty; not recording this machine as warmed."
    note "sccache reports: $(sccache_cache_location)"
    return 1
  fi
  return 0
}

if [ "$WARM_CACHE" = true ] && [ -n "$REFERENCE_TREE" ]; then
  if ! have cargo; then
    note "cargo is not on PATH, so the caches cannot be warmed."
  else
    if warm_cargo_sources; then
      CARGO_CACHE_WARMED=true
    else
      note "cargo fetch --locked failed; the first build in a session will fetch instead."
    fi
    if have sccache && [ -d "$SCCACHE_DISK_DIR" ]; then
      # A running sccache server's backend is fixed at start, so any server already up with
      # different settings has to go before this one comes up. Started explicitly rather
      # than left to the first client call, so warm_sccache's backend check below is asking
      # about a server this block configured.
      SCCACHE_DIR="$SCCACHE_DISK_DIR" sccache --stop-server >/dev/null 2>&1
      SCCACHE_DIR="$SCCACHE_DISK_DIR" SCCACHE_CACHE_SIZE="$SCCACHE_MAX_SIZE" \
        SCCACHE_ERROR_LOG="${TMPDIR:-/tmp}/sccache.log" sccache --start-server >/dev/null 2>&1
      if warm_sccache; then
        SCCACHE_WARMED_THIS_RUN=true
        # Cache location included deliberately: it is the line that reveals a warm build
        # that filled somewhere other than this image's disk.
        SCCACHE_DIR="$SCCACHE_DISK_DIR" sccache --show-stats 2>/dev/null |
          grep -E '^(Cache location|Compile requests|Cache hits|Cache misses|Cache size)' | sed 's/^/    /'
      fi
      SCCACHE_DIR="$SCCACHE_DISK_DIR" sccache --stop-server >/dev/null 2>&1
    fi
  fi
elif [ -n "$REFERENCE_TREE" ] && [ "$HUMAN" = true ]; then
  if have cargo && warm_cargo_sources; then
    CARGO_CACHE_WARMED=true
  fi
fi

# Drop the throwaway clone (and the multi-GB target/ the warm build put in it) as soon as
# the caches are filled — the caches are what survives, not the tree.
if [ -n "$THROWAWAY_CLONE" ]; then
  step "Removing the throwaway clone"
  rm -rf "$THROWAWAY_CLONE"
  THROWAWAY_CLONE=""
  REFERENCE_TREE=""
fi

# --- (h) The provisioning marker, written last ------------------------------------
# It is for a human reading a broken VM, not for parsing. Its PRESENCE is what
# setup_startup.sh checks; its contents are what a person needs when that check fires.
write_marker() {
  [ "$HUMAN" = true ] && return 0
  mkdir -p "$VM_SETUP_DIR" || { note "could not create $VM_SETUP_DIR; no marker written."; return 1; }

  local sccache_present=false sccache_populated=false warm_commit warmed max_size
  if [ -d "$SCCACHE_DISK_DIR" ]; then
    sccache_present=true
    # The same predicate the warm build gates on, called rather than restated: a directory
    # holding nothing but empty subdirectories must not carry a previous sccache_warmed=true
    # forward, and two copies of that rule is how one of them drifts.
    sccache_disk_has_objects && sccache_populated=true
  fi

  # Describe the cache that is on disk NOW, not what this run did. Repairing an
  # environment means re-running WITHOUT --warm-cache; the cache it does not touch
  # outlives it, and a marker rewritten as "warmed=false, commit=none" would be lying
  # about the provenance of the cache every session inherits — the one fact it exists to
  # record. So carry the previous values forward whenever the directory survives.
  if [ "$SCCACHE_WARMED_THIS_RUN" = true ]; then
    warmed=true
    warm_commit="$WARM_TREE_COMMIT"
  elif [ "$sccache_populated" = true ]; then
    warmed="$(marker_value sccache_warmed 2>/dev/null)"
    warm_commit="$(marker_value sccache_warm_commit 2>/dev/null)"
    [ -n "$warmed" ] || warmed=unknown
    [ -n "$warm_commit" ] || warm_commit=unknown
  else
    warmed=false
    warm_commit=none
  fi

  # Same reasoning, same interface, other half: the cap belongs to the cache that is on disk.
  # `setup_startup.sh` reads this key precisely so an image baked with a bigger cap is not
  # pruned back to the default by its own first session — and re-deriving it here would
  # reintroduce that through a plain `setup_env.sh --upgrade-tools`, which is the repair line
  # this script and four rows of claude_cloud_setup.md tell people to run. An explicit
  # override in this run's environment still wins.
  max_size="$SCCACHE_MAX_SIZE"
  if [ -z "${FREECELL_SCCACHE_CACHE_SIZE:-}" ] && [ "$sccache_populated" = true ]; then
    local previous_max_size
    previous_max_size="$(marker_value sccache_max_size 2>/dev/null)"
    [ -n "$previous_max_size" ] && max_size="$previous_max_size"
  fi

  # Tool versions are probed live rather than carried forward: what is on PATH now is the
  # whole truth, and a re-run must not report a tool that was uninstalled since.
  #
  # cargo is probed through the PINNED toolchain, not the default one: that is the cargo
  # every build in this repo runs, and reporting the default (on a stock image, an older
  # version) is the kind of half-truth that sends a reader down the wrong path. Note also
  # that everything inside the heredoc below is literal — a shell comment placed in there
  # lands in the marker file.
  local content pinned_cargo_version rustup_v sccache_v cargo_deny_v
  pinned_cargo_version="$(version_of rustup run "$TOOLCHAIN_PIN" cargo --version)"
  [ -n "$pinned_cargo_version" ] || pinned_cargo_version="(toolchain $TOOLCHAIN_PIN not installed)"
  # A blank value is the least useful thing to show the one reader this file has: someone
  # staring at a VM where a tool is missing is exactly the case it exists for.
  rustup_v="$(version_of rustup --version)"; [ -n "$rustup_v" ] || rustup_v="(not installed)"
  sccache_v="$(version_of sccache --version)"; [ -n "$sccache_v" ] || sccache_v="(not installed)"
  cargo_deny_v="$(version_of cargo-deny --version)"; [ -n "$cargo_deny_v" ] || cargo_deny_v="(not installed)"

  content="$(cat <<MARKER
# Written by .config/utils/setup_env.sh. Its presence means this machine was provisioned
# for the FreeCell repo (github.com/scosman/freecell). Delete it to force setup to be
# re-run, or to make setup_startup.sh report this VM as unprovisioned.
setup_version=2
written_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
repo=$REPO_URL
ref=$REPO_REF
toolchain_pin=$TOOLCHAIN_PIN
rustup_version=$rustup_v
cargo_version=$pinned_cargo_version
sccache_version=$sccache_v
sccache_dir=$SCCACHE_DISK_DIR
sccache_dir_present=$sccache_present
sccache_max_size=$max_size
sccache_warmed=$warmed
sccache_warm_commit=${warm_commit:-none}
cargo_sources_warmed=$CARGO_CACHE_WARMED
cargo_deny_version=$cargo_deny_v
render_env_installed=$RENDER_ENV_INSTALLED
session_start_hook=(committed: .claude/settings.json -> .claude/hooks/session-start.sh)
MARKER
)"
  write_atomic "$VM_SETUP_MARKER" "$content
" || { note "could not write $VM_SETUP_MARKER."; return 1; }
  echo "    wrote $VM_SETUP_MARKER"
  return 0
}

if [ "$HUMAN" = false ]; then
  step "Provisioning marker"
  write_marker
fi

# --- (i) Close ---------------------------------------------------------------------
echo ""
# The no-checkout case comes FIRST for --human, because almost nothing repo-local happened:
# ~/.cargo was never warmed, the render-package offer was never made, and the toolchain
# installed above is this script's fallback pin rather than the one the repo pins. Closing
# with "Local setup complete. Build with: cd app && ..." would name a directory that does not
# exist. README.md and app/README.md both point contributors at this command, so getting the
# wrong-directory case right is not a corner.
if [ "$HUMAN" = true ] && [ -z "$PROJECT_ROOT" ]; then
  echo "  The toolchain above was installed, but nothing repo-local was done: no checkout was" >&2
  echo "  found from $PWD, so the dependency sources were not warmed, the system-package offer" >&2
  echo "  was not made, and the pin used was this script's fallback ($RUST_TOOLCHAIN_FALLBACK_PIN)," >&2
  echo "  not app/rust-toolchain.toml's." >&2
  echo "" >&2
  echo "  Re-run from inside a FreeCell checkout:" >&2
  echo "      git clone $REPO_URL && cd freecell && bash .config/utils/setup_env.sh --human" >&2
  echo "" >&2
  fail "no FreeCell checkout found from $PWD; local setup is incomplete."
  finish
fi

if [ "$HUMAN" = true ]; then
  finish "Local setup complete. Build with: cd app && cargo build --workspace"
fi

if [ -n "$PROJECT_ROOT" ]; then
  finish "Environment setup complete. Per-session preparation runs from .config/utils/setup_startup.sh."
fi

# Two readers end up here, and they need different next steps: the cloud setup log (where
# there was never going to be a checkout) and a human who ran this from the wrong
# directory.
echo "No FreeCell checkout was present, so nothing repo-local was written."
echo "  In a Claude Code session: the SessionStart hook runs .config/utils/setup_startup.sh"
echo "  for you, which prepares the checkout against what was installed above."
echo "  On your own machine: re-run this from inside a FreeCell checkout."
finish "Environment setup complete."
