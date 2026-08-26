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
CREATE_STARTUP_SCRIPT=false  # --create-startup-script
HUMAN=false                  # --human

# --- Constants --------------------------------------------------------------------
REPO_URL="${FREECELL_REPO_URL:-https://github.com/scosman/freecell}"
REPO_REF="${FREECELL_SETUP_REF:-main}"

# Machine-global state, outside every checkout, so it survives the environment snapshot.
# KEEP IN SYNC with the identical block in .config/utils/setup_startup.sh — the two
# scripts cannot share code (this one gets fetched and run standalone), so these paths
# are a duplicated, documented interface between them.
VM_SETUP_DIR="${FREECELL_VM_SETUP_DIR:-/opt/freecell-vm-setup}"
# The _v1 is a CONTRACT VERSION, not decoration: when what this script provides changes
# incompatibly, bump it here and in setup_startup.sh, so machines provisioned by the
# older script report as not-set-up rather than ready.
VM_SETUP_MARKER="$VM_SETUP_DIR/.setup_for_freecell_repo_v1"
SCCACHE_DISK_DIR="$VM_SETUP_DIR/sccache"
STARTUP_HOOK_SHIM="$VM_SETUP_DIR/freecell_session_start_hook.sh"
# CLAUDE_CONFIG_DIR is what Claude Code itself honors; respecting it is both correct and
# what makes hook registration testable without touching a real ~/.claude.
CLAUDE_USER_SETTINGS="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/settings.json"

# sccache's disk cache is content-addressed and immutable, so sharing one directory
# across every session on this machine is safe by construction, and it is
# path-independent — unlike a warm target/, whose fingerprints embed absolute paths.
# The cap matters: this directory is baked into the image and comes out of the same
# fixed disk allowance a session's target/ (up to ~25 GB) has to fit in.
SCCACHE_MAX_SIZE="${FREECELL_SCCACHE_CACHE_SIZE:-8G}"

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
fail() { echo "error: $*" >&2; FAILED=1; }
note() { echo "  ! $*" >&2; }
step() { echo "==> $*"; }

finish() {
  if [ "$FAILED" -ne 0 ]; then
    if [ "$BEST_EFFORT" = true ]; then
      echo "setup_env.sh: finished with errors (best effort, exiting 0)." >&2
      exit 0
    fi
    exit 1
  fi
  echo "${1:-Environment setup complete.}"
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
  --create-startup-script   Write a SessionStart hook shim under the machine-global
                            directory and register it in Claude Code's USER settings.
                            OFF by default, and not used by the cloud wrapper: this repo
                            ships a committed project-level hook instead. Belt-and-braces
                            for a machine where the project hook is not trusted.
  --human                   Contributor mode for a personal machine: install the pinned
                            toolchain and warm the dependency sources, offer the optional
                            render-test system dependencies, and write nothing outside
                            the checkout and ~/.cargo.
  -h, --help                This text.

Environment overrides (all exist so this is testable without writing to /opt or cloning
over the network):
  FREECELL_VM_SETUP_DIR        default /opt/freecell-vm-setup
  FREECELL_SETUP_REF           ref to clone for --warm-cache (default main)
  FREECELL_REPO_URL            default https://github.com/scosman/freecell
  FREECELL_SCCACHE_CACHE_SIZE  default 8G
  FREECELL_SKIP_WARM_BUILD     set to skip --warm-cache's workspace build (the dependency
                               source caches are still warmed)
  CLAUDE_CONFIG_DIR            default $HOME/.claude
  FREECELL_SETTINGS_MERGE_TOOL jq | python3 | auto (default) — which implementation
                               merges the user settings.json for --create-startup-script
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
    --create-startup-script) CREATE_STARTUP_SCRIPT=true ;;
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
  CREATE_STARTUP_SCRIPT=false
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
  local url="$1" binary="$2" tmp install_dir found
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
  [ -w "$install_dir" ] || install_dir="$HOME/.local/bin"
  mkdir -p "$install_dir" || { rm -rf "$tmp"; return 1; }
  install -m 755 "$found" "$install_dir/$binary" || { rm -rf "$tmp"; return 1; }
  rm -rf "$tmp"
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

if [ "$HUMAN" = false ] && have sccache; then
  mkdir -p "$SCCACHE_DISK_DIR" || note "could not create $SCCACHE_DISK_DIR; sccache will fall back to its per-user cache."
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
  [ -f "$script" ] || { note "$RENDER_ENV_SCRIPT_REL not found in the reference tree; skipping system deps."; return 1; }
  mkdir -p "$VM_SETUP_DIR" 2>/dev/null
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

warm_sccache() {
  local log avail_gb
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
      # Stop any server started with different settings (an inherited remote-bucket
      # config, say): a running sccache server's backend is fixed at start, so the env
      # below only takes effect for a server this call starts.
      SCCACHE_DIR="$SCCACHE_DISK_DIR" sccache --stop-server >/dev/null 2>&1
      if warm_sccache; then
        SCCACHE_WARMED_THIS_RUN=true
        SCCACHE_DIR="$SCCACHE_DISK_DIR" sccache --show-stats 2>/dev/null |
          grep -E '^(Compile requests|Cache hits|Cache misses|Cache size)' | sed 's/^/    /'
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

# --- (h) Optional: a user-level SessionStart hook ---------------------------------
# Off by default and unused by the cloud wrapper. This repo's SessionStart hook is
# COMMITTED (.claude/settings.json + .claude/hooks/session-start.sh), which is strictly
# better here: it is version-controlled with the script it runs, it cannot hit the "a shim
# on disk is not a registered hook" trap, and it is immune to an image that builds as one
# user and runs sessions as another ($HOME resolves at build time; on this image HOME is
# /root while the checkout lives under /home/user).
#
# The flag exists for a machine where the project hook is not trusted or not present.
STARTUP_HOOK_REGISTERED=false

write_startup_shim() {
  mkdir -p "$VM_SETUP_DIR" || { note "could not create $VM_SETUP_DIR; not writing the hook shim."; return 1; }
  local staged="$STARTUP_HOOK_SHIM.$$"
  # Staged and renamed: a session starting mid-write would otherwise run half a shell
  # script. KEEP IN SYNC with .claude/hooks/session-start.sh — same behavior, different
  # home (that one lives in the checkout, this one has to find a checkout).
  cat >"$staged" <<'SHIM'
#!/usr/bin/env bash
# Claude Code SessionStart hook. Written by .config/utils/setup_env.sh
# --create-startup-script and registered in the user settings.json.
#
# The hook fires at the start of every session on this machine, whatever repo the session
# is for, so this does nothing unless it can find a FreeCell checkout.
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
if [ "${#output}" -gt 4000 ]; then
  output="$(printf '%s' "$output" | head -c 1500)
[...trimmed. Re-run .config/utils/setup_startup.sh to see all of it...]
$(printf '%s' "$output" | tail -c 2000)"
fi
[ -n "$output" ] && printf '%s\n' "$output"
[ "$status" -ne 0 ] &&
  printf '%s\n' "($script exited $status. Fix the above before building or testing.)"
exit 0
SHIM
  chmod 755 "$staged" || { rm -f "$staged"; return 1; }
  mv -f "$staged" "$STARTUP_HOOK_SHIM" || { rm -f "$staged"; return 1; }
  return 0
}

# Merge our one entry into the user's settings.json. NEVER overwrite, never with sed: that
# file holds machine-wide settings, and replacing it disables them silently.
register_startup_hook() {
  local shim_base merged current settings_dir staged
  shim_base="$(basename "$STARTUP_HOOK_SHIM")"
  settings_dir="$(dirname "$CLAUDE_USER_SETTINGS")"
  mkdir -p "$settings_dir" || { note "could not create $settings_dir; hook not registered."; return 1; }
  current='{}'
  if [ -e "$CLAUDE_USER_SETTINGS" ]; then
    if [ ! -r "$CLAUDE_USER_SETTINGS" ]; then
      note "$CLAUDE_USER_SETTINGS is not readable; refusing to touch it. Hook not registered."
      return 1
    fi
    current="$(cat "$CLAUDE_USER_SETTINGS")"
    # An empty file is not valid JSON, and feeding jq an empty stream yields an empty
    # result with exit 0 — which would read as "already registered" and install nothing.
    [ -n "$(printf '%s' "$current" | tr -d '[:space:]')" ] || current='{}'
  fi

  if ! merged="$(printf '%s' "$current" | merge_hook_entry "$STARTUP_HOOK_SHIM" "$shim_base")"; then
    # Repairing someone's malformed settings is not this script's business, and
    # truncating them is worse than not installing the hook.
    note "refusing to modify $CLAUDE_USER_SETTINGS (see the reason above). Hook not registered."
    return 1
  fi

  if [ "$merged" = "$current" ]; then
    echo "    already registered in $CLAUDE_USER_SETTINGS"
    return 0
  fi

  # Staged sibling + rename, preserving the original mode. A redirect onto the file itself
  # truncates before it writes, so a failed write would leave the machine's settings empty.
  staged="$(mktemp "$settings_dir/settings.json.XXXXXX")" || return 1
  printf '%s\n' "$merged" >"$staged" || { rm -f "$staged"; return 1; }
  if [ -f "$CLAUDE_USER_SETTINGS" ]; then
    chmod --reference="$CLAUDE_USER_SETTINGS" "$staged" 2>/dev/null || chmod 644 "$staged"
  else
    chmod 644 "$staged"
  fi
  mv -f "$staged" "$CLAUDE_USER_SETTINGS" || { rm -f "$staged"; return 1; }
  echo "    registered in $CLAUDE_USER_SETTINGS"
  return 0
}

# The python3 implementation of the settings merge (see merge_hook_entry below for the
# rules it enforces). Held in a variable so it can be handed to `python3 -c`.
# `read -d ''` always ends at EOF, so it returns 1 even on success; `|| true` keeps that
# from looking like a failure to anyone reading with `set -x`.
read -r -d '' MERGE_SETTINGS_PY <<'PY' || true
import json, os, sys

entry = json.loads(os.environ["FREECELL_HOOK_ENTRY"])
base = os.environ["FREECELL_HOOK_BASE"]
raw = sys.stdin.read()

try:
    settings = json.loads(raw)
except Exception as exc:
    print("%s is not valid JSON: %s" % ("settings.json", exc), file=sys.stderr)
    sys.exit(1)
if not isinstance(settings, dict):
    print("settings root is not a JSON object", file=sys.stderr)
    sys.exit(1)
hooks = settings.get("hooks", {})
if not isinstance(hooks, dict):
    print('"hooks" is not an object', file=sys.stderr)
    sys.exit(1)
groups = hooks.get("SessionStart", [])
if not isinstance(groups, list):
    print('"hooks.SessionStart" is not an array', file=sys.stderr)
    sys.exit(1)

kept_groups = []
for group in groups:
    if not isinstance(group, dict) or not isinstance(group.get("hooks"), list):
        kept_groups.append(group)
        continue
    kept = [h for h in group["hooks"]
            if base not in str(h.get("command", "") if isinstance(h, dict) else "")]
    if len(kept) == len(group["hooks"]):
        kept_groups.append(group)
    elif kept:
        group["hooks"] = kept
        kept_groups.append(group)
    # else: a group we emptied goes with our entry.
kept_groups.append(entry)

hooks["SessionStart"] = kept_groups
settings["hooks"] = hooks
# ensure_ascii=False: the contract is that this touches nothing but our entry, and the
# default would rewrite every non-ASCII character in someone else's setting as an escape.
sys.stdout.write(json.dumps(settings, indent=2, ensure_ascii=False) + "\n")
PY

# Reads settings JSON on stdin, writes the merged JSON on stdout, exits non-zero (with a
# reason on stderr) rather than emitting anything when the input is not something we are
# willing to edit. jq preferred; python3 as the fallback; neither present is a skip.
#
# The rules, all of which matter:
#   - Identify our own entry by the shim's repo-specific BASENAME appearing in the command
#     string, so re-running is idempotent and a moved VM_SETUP_DIR replaces its old entry
#     rather than adding a second one.
#   - Drop our entry wherever it is, then append exactly one.
#   - A group WE emptied goes with it; a group holding other hooks keeps them; a group
#     that was already empty is left alone (we touch nothing but our own entry).
#   - Key order is preserved and non-ASCII is emitted raw, so the diff is our entry only.
merge_hook_entry() {
  local shim="$1" base="$2"
  # FREECELL_SETTINGS_MERGE_TOOL forces one implementation. Both are maintained, and an
  # untested fallback is a liability, so .config/utils/test_setup_scripts.sh runs the same
  # cases through each and asserts they agree.
  local merge_tool="${FREECELL_SETTINGS_MERGE_TOOL:-auto}"
  local entry command_json
  if [ "$merge_tool" != python3 ] && have jq; then
    command_json="$(printf '%s' "$shim" | jq -Rs .)"
  else
    command_json="$(printf '%s' "$shim" | python3 -c 'import json,sys; print(json.dumps(sys.stdin.read()))')"
  fi
  # matcher deliberately excludes "clear" and "compact": SessionStart fires on those too,
  # and this hook's output is spent from the context window — re-running it on every
  # compaction of a long session re-injects it for no gain, since the checkout is already
  # prepared. "fork" is included because a forked session can start on a filesystem this
  # has not run on. The generous timeout covers a VM with a cold cache.
  entry="$(printf '{"matcher":"startup|resume|fork","hooks":[{"type":"command","command":%s,"timeout":600,"statusMessage":"Preparing the FreeCell checkout"}]}' "$command_json")"

  if [ "$merge_tool" != python3 ] && have jq; then
    local jq_err
    jq_err="$(mktemp)" || return 1
    jq --argjson entry "$entry" --arg base "$base" '
      if type != "object" then error("settings root is not a JSON object")
      elif (has("hooks") and (.hooks | type) != "object") then error("\"hooks\" is not an object")
      elif ((.hooks // {}) | has("SessionStart")) and ((.hooks.SessionStart | type) != "array")
        then error("\"hooks.SessionStart\" is not an array")
      else
        (.hooks //= {})
        | (.hooks.SessionStart //= [])
        | .hooks.SessionStart = ([
            .hooks.SessionStart[]
            | . as $group
            | (if (($group | type) == "object") and (($group.hooks | type) == "array")
               then [ $group.hooks[] | select(((.command? // "") | tostring | contains($base)) | not) ]
               else null end) as $kept
            | if $kept == null then $group
              elif ($kept | length) == ($group.hooks | length) then $group
              elif ($kept | length) == 0 then empty
              else $group | .hooks = $kept
              end
          ] + [$entry])
      end
    ' 2>"$jq_err"
    local status=$? reason
    if [ "$status" -ne 0 ]; then
      reason="$(head -2 "$jq_err" 2>/dev/null)"
      note "${reason:-jq could not parse $CLAUDE_USER_SETTINGS}"
      rm -f "$jq_err"
      return 1
    fi
    rm -f "$jq_err"
    return 0
  fi

  if [ "$merge_tool" != jq ] && have python3; then
    # `python3 - <<'PY'` would redirect stdin to the heredoc, so the settings JSON piped
    # into this function would never reach sys.stdin — the program has to arrive by -c.
    FREECELL_HOOK_ENTRY="$entry" FREECELL_HOOK_BASE="$base" python3 -c "$MERGE_SETTINGS_PY"
    return $?
  fi

  note "neither jq nor python3 is available; cannot safely merge JSON settings."
  return 1
}

if [ "$CREATE_STARTUP_SCRIPT" = true ]; then
  step "SessionStart hook shim"
  if write_startup_shim; then
    echo "    wrote $STARTUP_HOOK_SHIM"
    # A shim on disk is NOT a hook. Only flag it installed once registration succeeded —
    # the marker reports this flag, and naming the shim on a VM where registration was
    # refused sends whoever is debugging to the wrong file.
    register_startup_hook && STARTUP_HOOK_REGISTERED=true
  else
    note "could not write $STARTUP_HOOK_SHIM."
  fi
fi

# --- (i) The provisioning marker, written last ------------------------------------
# It is for a human reading a broken VM, not for parsing. Its PRESENCE is what
# setup_startup.sh checks; its contents are what a person needs when that check fires.
write_marker() {
  [ "$HUMAN" = true ] && return 0
  mkdir -p "$VM_SETUP_DIR" || { note "could not create $VM_SETUP_DIR; no marker written."; return 1; }

  local sccache_present=false sccache_populated=false warm_commit warmed
  if [ -d "$SCCACHE_DISK_DIR" ]; then
    sccache_present=true
    # An empty directory this run merely created is not a warm cache, and reporting it as
    # one of unknown provenance would send a reader looking for a problem that is not there.
    [ -n "$(find "$SCCACHE_DISK_DIR" -mindepth 1 -print -quit 2>/dev/null)" ] && sccache_populated=true
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

  # Tool versions are probed live rather than carried forward: what is on PATH now is the
  # whole truth, and a re-run must not report a tool that was uninstalled since.
  #
  # cargo is probed through the PINNED toolchain, not the default one: that is the cargo
  # every build in this repo runs, and reporting the default (on a stock image, an older
  # version) is the kind of half-truth that sends a reader down the wrong path. Note also
  # that everything inside the heredoc below is literal — a shell comment placed in there
  # lands in the marker file.
  local content pinned_cargo_version
  pinned_cargo_version="$(version_of rustup run "$TOOLCHAIN_PIN" cargo --version)"
  [ -n "$pinned_cargo_version" ] || pinned_cargo_version="(toolchain $TOOLCHAIN_PIN not installed)"

  content="$(cat <<MARKER
# Written by .config/utils/setup_env.sh. Its presence means this machine was provisioned
# for the FreeCell repo (github.com/scosman/freecell). Delete it to force setup to be
# re-run, or to make setup_startup.sh report this VM as unprovisioned.
setup_version=1
written_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
repo=$REPO_URL
ref=$REPO_REF
toolchain_pin=$TOOLCHAIN_PIN
rustup_version=$(version_of rustup --version)
cargo_version=$pinned_cargo_version
sccache_version=$(version_of sccache --version)
sccache_dir=$SCCACHE_DISK_DIR
sccache_dir_present=$sccache_present
sccache_max_size=$SCCACHE_MAX_SIZE
sccache_warmed=$warmed
sccache_warm_commit=${warm_commit:-none}
cargo_sources_warmed=$CARGO_CACHE_WARMED
cargo_deny_version=$(version_of cargo-deny --version)
render_env_installed=$RENDER_ENV_INSTALLED
session_start_hook=$([ "$STARTUP_HOOK_REGISTERED" = true ] && echo "$STARTUP_HOOK_SHIM" || echo "(project-level .claude/hooks/session-start.sh)")
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

# --- (j) Close ---------------------------------------------------------------------
echo ""
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
