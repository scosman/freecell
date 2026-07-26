#!/usr/bin/env bash
# setup_sccache.sh — wire up sccache backed by Cloudflare R2 as a shared remote
# compile cache for the FreeCell Rust workspace (dev containers / dev shells ONLY).
#
# Why: fresh dev containers otherwise recompile the huge pinned dep tree (gpui/zed,
# gpui-component, ironcalc fork) from scratch. With sccache + R2, rustc outputs are
# fetched from the shared bucket instead. CI is deliberately NOT touched — it keeps
# Swatinem/rust-cache (see projects/build-cache.md for design + token rotation).
#
# What it does:
#   1. Installs the prebuilt sccache binary (v0.8.2, x86_64 linux musl) if missing.
#   2. Exports SCCACHE_REGION=auto, SCCACHE_BUCKET / SCCACHE_ENDPOINT (defaults below,
#      env overrides respected) and maps R2_* credentials -> AWS_* (the standard AWS
#      cred chain sccache's S3 client reads; this also overrides the placeholder
#      AWS_ACCESS_KEY_ID the agent proxy injects).
#   3. Sets RUSTC_WRAPPER=sccache and starts the sccache server.
#
# Credentials (provided as container env-var secrets, never printed by this script):
#   R2_ACCESS_KEY_ID / R2_SECRET_ACCESS_KEY — R2 API token, Object Read+Write on the
#   bucket. If R2_ACCESS_KEY_ID is ABSENT the script no-ops gracefully: RUSTC_WRAPPER
#   stays unset and builds behave exactly as before, just uncached.
#
# Usage:
#   source app/scripts/setup_sccache.sh   # activate in the current shell
#   app/scripts/setup_sccache.sh          # run by the SessionStart hook — exports are
#                                         # persisted for the session via $CLAUDE_ENV_FILE

SCCACHE_VERSION="0.8.2"
SCCACHE_URL="https://github.com/mozilla/sccache/releases/download/v${SCCACHE_VERSION}/sccache-v${SCCACHE_VERSION}-x86_64-unknown-linux-musl.tar.gz"
DEFAULT_BUCKET="freecell-dev"
DEFAULT_ENDPOINT="https://c52d4051d39243f8a706b3e9bb2c8da1.r2.cloudflarestorage.com"

# Detect sourced-vs-executed so a no-op never kills the caller's shell.
_sccache_sourced=0
if (return 0 2>/dev/null); then _sccache_sourced=1; fi

_sccache_log() { echo "[setup_sccache] $*" >&2; }

# Export NAME=VALUE in this shell and, when running under the SessionStart hook,
# persist it to $CLAUDE_ENV_FILE so every shell in the session gets it.
# VALUE is written to the env file verbatim (single-quoted heredoc semantics via
# printf %q), so pass literal values — secrets are passed as deferred expansions.
_sccache_export() {
  local name="$1" value="$2" literal="${3:-}"
  export "$name"="$value"
  if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
    if [ -n "$literal" ]; then
      # Deferred expansion: the env file references the variable by name, so the
      # secret value itself is never written to disk.
      printf 'export %s="${%s}"\n' "$name" "$literal" >>"$CLAUDE_ENV_FILE"
    else
      printf 'export %s=%q\n' "$name" "$value" >>"$CLAUDE_ENV_FILE"
    fi
  fi
}

_sccache_main() {
  if [ -z "${R2_ACCESS_KEY_ID:-}" ] || [ -z "${R2_SECRET_ACCESS_KEY:-}" ]; then
    _sccache_log "R2_ACCESS_KEY_ID/R2_SECRET_ACCESS_KEY not set — leaving RUSTC_WRAPPER unset (uncached builds)."
    return 0
  fi

  # Install sccache if missing (prebuilt binary only — never compile it).
  if ! command -v sccache >/dev/null 2>&1; then
    if [ "$(uname -sm)" != "Linux x86_64" ]; then
      _sccache_log "no prebuilt sccache for $(uname -sm); skipping (uncached builds)."
      return 0
    fi
    local tmp install_dir
    tmp="$(mktemp -d)" || return 0
    _sccache_log "installing sccache v${SCCACHE_VERSION}..."
    if ! curl -fsSL -o "$tmp/sccache.tar.gz" "$SCCACHE_URL"; then
      _sccache_log "download failed (proxy/egress?); skipping (uncached builds)."
      rm -rf "$tmp"
      return 0
    fi
    tar -xzf "$tmp/sccache.tar.gz" -C "$tmp"
    install_dir="/usr/local/bin"
    [ -w "$install_dir" ] || install_dir="$HOME/.local/bin"
    mkdir -p "$install_dir"
    install -m 755 "$tmp"/sccache-v*/sccache "$install_dir/sccache"
    rm -rf "$tmp"
    case ":$PATH:" in
      *":$install_dir:"*) ;;
      *) _sccache_export PATH "$install_dir:$PATH" ;;
    esac
  fi

  _sccache_export SCCACHE_BUCKET "${SCCACHE_BUCKET:-$DEFAULT_BUCKET}"
  _sccache_export SCCACHE_ENDPOINT "${SCCACHE_ENDPOINT:-$DEFAULT_ENDPOINT}"
  _sccache_export SCCACHE_REGION "auto"
  # R2 creds via the standard AWS chain; deferred expansion keeps secrets off disk
  # and overrides the agent proxy's placeholder AWS_ACCESS_KEY_ID.
  _sccache_export AWS_ACCESS_KEY_ID "$R2_ACCESS_KEY_ID" R2_ACCESS_KEY_ID
  _sccache_export AWS_SECRET_ACCESS_KEY "$R2_SECRET_ACCESS_KEY" R2_SECRET_ACCESS_KEY
  _sccache_export RUSTC_WRAPPER "sccache"

  # Start the server now so the first cargo call doesn't pay startup, and verify the
  # backend actually resolved to R2 (not the local-disk fallback).
  sccache --stop-server >/dev/null 2>&1 || true
  SCCACHE_ERROR_LOG=/tmp/sccache.log sccache --start-server >/dev/null 2>&1 || true
  local location
  location="$(sccache --show-stats 2>/dev/null | grep '^Cache location' || true)"
  if echo "$location" | grep -q 's3'; then
    _sccache_log "ready — ${location}"
  else
    _sccache_log "WARNING: backend is not R2 (${location:-server not running}); check /tmp/sccache.log."
  fi
  return 0
}

_sccache_main
_sccache_rc=$?
unset -f _sccache_main _sccache_export _sccache_log
if [ "$_sccache_sourced" = 0 ]; then exit "$_sccache_rc"; fi
