# Shared Remote Compile Cache (sccache + Cloudflare R2)

**Status:** Implemented (2026-07-24) — dev containers only.

## Problem

Every fresh dev container recompiles the FreeCell workspace's huge pinned dependency
tree — gpui/zed, gpui-component, the ironcalc fork — from scratch (~15–25 min for a
full build). Container-local caches die with the container, so nothing carries over
between sessions.

## Design

[sccache](https://github.com/mozilla/sccache) (v0.8.2, prebuilt musl binary) as a
`RUSTC_WRAPPER`, backed by a shared **Cloudflare R2** bucket via sccache's S3 backend.
rustc outputs are content-addressed uploads: the first container to compile a given
(rustc, flags, source) tuple pays the cost; every later container — any session, any
day — downloads the object instead of recompiling.

Wiring (all in [`app/scripts/setup_sccache.sh`](../app/scripts/setup_sccache.sh)):

- **Bucket/endpoint:** `SCCACHE_BUCKET=freecell-dev`,
  `SCCACHE_ENDPOINT=https://<accountid>.r2.cloudflarestorage.com`,
  `SCCACHE_REGION=auto` (R2 requires `auto`). Env vars override the baked-in defaults.
- **Credentials:** the container provides `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY`
  as secrets; the script maps them to `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY`
  (the standard AWS cred chain sccache reads). This mapping deliberately **overrides
  the placeholder `AWS_ACCESS_KEY_ID=proxy-injected`** that the agent proxy sets.
  Secrets are persisted to the session env file **by reference** (`"${R2_ACCESS_KEY_ID}"`),
  never by value, and the script never prints them.
- **Auto-activation:** a `SessionStart` hook
  ([`.claude/hooks/session-start.sh`](../.claude/hooks/session-start.sh), registered in
  [`.claude/settings.json`](../.claude/settings.json)) runs the setup script in every
  fresh web container, so sessions are cache-ready with no manual step. Local checkouts
  opt in with `source app/scripts/setup_sccache.sh`.
- **Graceful degradation:** without the `R2_*` secrets the script leaves
  `RUSTC_WRAPPER` unset and exits 0 — contributors without creds build exactly as
  before, just uncached. Same for install/download failures.

## Scope: dev containers ONLY — CI deliberately untouched

GitHub CI keeps `Swatinem/rust-cache`, which already works well there (persistent
Actions cache, no secret distribution to fork PRs, no egress dependency). No workflow
file references sccache; keep it that way unless a separate project revisits CI
caching on its own merits.

## Limits / notes

- sccache caches **rustc invocations**, not the link step or build scripts' execution,
  so a "fully cached" cold build still pays linking + `cargo` orchestration — big
  savings, not zero-cost.
- The cache key includes the exact rustc version: a toolchain bump
  (`rust-toolchain.toml`) naturally starts a fresh keyspace; stale objects just age out.
- The proxy-injected placeholder AWS creds mean **anything else** in a dev session
  that reads the AWS cred chain now sees the R2 token instead of the placeholder;
  nothing in this repo does today.
- Seeding: the bucket warms organically as containers build. A deliberate one-off
  full-workspace build in a throwaway session is the fastest way to seed it.

## Rotating the R2 token

1. Cloudflare dashboard → R2 → **Manage R2 API Tokens** → create a new token scoped
   **Object Read & Write** on the `freecell-dev` bucket only.
2. Update the `R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` secrets in the Claude Code
   **environment settings** (where the container env vars are configured).
3. Revoke the old token. Fresh sessions pick the new values up automatically; running
   sessions keep the old env until restarted.

No FreeCell code change is needed for rotation — the script only ever reads the env.
