# Dev-container Build Cache (baked VM image + per-session startup script)

**Status:** Implemented (2026-08-26) — dev containers only. Supersedes the sccache +
Cloudflare R2 design (implemented 2026-07-24, removed here).

## Problem

Every fresh dev container starts from a stock image and pays for the same four things
before it can do any work:

1. **The toolchain.** rustup's default is a stock `stable`; `app/rust-toolchain.toml`
   pins `1.95.0`, so the first `cargo` call under `app/` downloads a whole toolchain.
2. **Dependency sources.** `~/.cargo/git` alone is ~630 MB once the pinned zed and
   IronCalc-fork checkouts land.
3. **rustc outputs.** The pinned dependency tree — gpui/zed, gpui-component, the
   IronCalc fork — is ~15–25 min of compilation.
4. **System packages.** `app/render-tests/scripts/setup_render_env.sh` is a heavy apt
   install (X11/Wayland/xkb, Vulkan + lavapipe, fontconfig/freetype, gbm, ALSA, Xvfb,
   ImageMagick, DejaVu Serif).

Container-local caches die with the container, so nothing carried over.

## Design

Two scripts, two lifetimes. Everything expensive moves into the **environment image**,
which is built once and snapshotted; the per-session script only does what genuinely
depends on the checkout.

```
environment build (once, then the disk is snapshotted)
  cloud UI "Setup script" field
    └─ .config/utils/claude_code_vm_setup.sh   (pasted; a fetcher, not a copy)
         └─ curl raw.githubusercontent.com/.../main/.config/utils/setup_env.sh
              └─ bash setup_env.sh --upgrade-tools --best-effort --warm-cache
                   ├─ pinned toolchain 1.95.0 + rustfmt + clippy
                   ├─ prebuilt sccache + cargo-deny (pinned versions)
                   ├─ system packages, via the repo's own setup_render_env.sh
                   ├─ throwaway clone → cargo fetch --locked → cargo build --workspace
                   │    (fills ~/.cargo/{registry,git} and the sccache disk cache,
                   │     then the clone and its target/ are deleted)
                   └─ /opt/freecell-vm-setup/.setup_for_freecell_repo_v1  (marker)

every session (fresh checkout, snapshot-restored disk)
  SessionStart hook  (.claude/settings.json → .claude/hooks/session-start.sh)
    └─ .config/utils/setup_startup.sh
         ├─ containers only; identity-check the checkout; check the marker
         ├─ verify the pinned toolchain + version floors
         ├─ wire sccache to /opt/freecell-vm-setup/sccache
         ├─ cargo fetch --locked
         └─ one "Ready." line   (~1 s)
```

`/opt/freecell-vm-setup/` is machine-global state deliberately outside every checkout, so
it survives the snapshot. Its paths are a **documented interface between the two scripts**,
duplicated in both with a "keep in sync" comment — they cannot share code, because
`setup_env.sh` is fetched and run standalone before any checkout exists.

The `_v1` in the marker filename is a **contract version**: bump it in both scripts when
what the image provides changes incompatibly, so machines built by the older script report
as unprovisioned rather than as ready.

Full operator documentation — the cloud UI steps, the allowed-domain list, and a repair
line for every message the scripts emit — is
[`.config/utils/claude_cloud_setup.md`](../.config/utils/claude_cloud_setup.md).

## Warm-cache strategy: local-disk sccache

`RUSTC_WRAPPER=sccache` with `SCCACHE_DIR=/opt/freecell-vm-setup/sccache`, filled at image
build time and capped by `SCCACHE_CACHE_SIZE` (default `8G`, `FREECELL_SCCACHE_CACHE_SIZE`
to override). Content-addressed and immutable, so sharing one directory across every
session is safe by construction; local disk, so a hit is a read rather than an HTTPS round
trip through the sandbox proxy; and path-independent, so it does not care that the image
was warmed in `/opt/.../throwaway-clone` and used from `/home/user/freecell`.

### Why not a warm `target/` hardlinked into the checkout

This is the obvious bigger win, and it was **rejected deliberately**:

- **Cargo writes through hardlinks.** `cp -al`-style seeding means a write through one path
  is a write to the other, and cargo rewrites `.fingerprint/*/lib-*.json`, `.cargo-lock`,
  and dep-info files scattered deep in the tree. The npm equivalent works because npm
  mutates exactly one top-level file, which can be un-shared afterwards; there is no
  equivalent short list for cargo, so a pristine warm tree would degrade silently across
  sessions.
- **Fingerprints embed absolute paths.** A `target/` built anywhere but the session's own
  checkout path is largely not reused anyway.
- **Disk.** `target/` runs ~25 GB against ~29 GB of session disk allowance.

To revisit it, the thing to measure is what a same-absolute-path prebuild actually saves
over the sccache route, against the cost of un-sharing every path cargo mutates in place.
Capture that as its own `projects/*.md` note rather than reopening this one.

### What is still paid per session

sccache caches **rustc invocations** — not linking, not build scripts, not cargo's own
orchestration — and it refuses to cache incremental compilations. In cargo's dev profile
that means workspace crates are never served from the cache and the pinned dependency tree
always is. The dependency tree is the entire point; a "fully cached" cold build still pays
for linking.

Two consequences worth knowing:

- The image is warmed with the **same cargo profile a session uses**. Warming with CI's
  trims (`CARGO_INCREMENTAL=0`, `CARGO_PROFILE_DEV_DEBUG=line-tables-only`) would change
  the rustc command lines that sccache keys on and produce a cache that misses on every
  session build — a slow machine that looks provisioned.
- The cache key includes the exact rustc version, so a `rust-toolchain.toml` bump naturally
  starts a fresh keyspace and the old objects age out under the size cap.

## What the per-session script does *not* do

No `cargo build`. A full workspace build at session start would cost minutes even fully
cached, and the agent's first real build pays that anyway. `cargo fetch --locked` against
the image's warm registry/git caches is ~1 s, and `--locked` turns "this branch changed its
dependencies" into a clear message naming `cargo update` rather than a silent lockfile
rewrite.

There is also no background pre-fetch. The previous design detached a `cargo fetch` at
session start, which raced the agent's first build every time; a warm image makes it
unnecessary.

## Scope: dev containers ONLY — CI deliberately untouched

GitHub CI keeps `Swatinem/rust-cache`, which works well there (persistent Actions cache, no
secret distribution to fork PRs, no egress dependency). No workflow file references
sccache; keep it that way unless a separate project revisits CI caching on its own merits.

Local machines are untouched too, by two independent guards: `setup_startup.sh` refuses to
run outside a container, and `setup_env.sh --human` writes nothing under `/opt` and never
edits a contributor's Claude Code settings.

## Migration from the R2 design (2026-07-24 → 2026-08-26)

Removed: `app/scripts/setup_sccache.sh`, the R2 bucket backend, and the
`R2_ACCESS_KEY_ID` / `R2_SECRET_ACCESS_KEY` environment secrets. Rotation is no longer a
thing that exists — there is no credential.

**The R2 API token should be revoked** in the Cloudflare dashboard (R2 → *Manage R2 API
Tokens*), and the two variables deleted from the cloud environment's settings. A secret
written onto a snapshotted disk outlives its own rotation, which is the general reason
nothing in these scripts touches secrets at all.

`setup_startup.sh` unsets `SCCACHE_BUCKET` / `SCCACHE_ENDPOINT` / `SCCACHE_REGION` before
starting the cache server, so a container that still carries the old variables gets the
disk backend regardless.

## Tests

`bash .config/utils/test_setup_scripts.sh` covers the parts that took real reasoning: the
container gate's falsy-value handling, the marker's effect on whether an unattributable
cache directory is trusted, the marker's carry-forward rules across a repair run, the
throwaway-clone lifecycle, and every rule of the optional user-`settings.json` merge — run
through both the jq and python3 implementations and asserted to agree.

It is safe to run on a personal machine: it works in scratch directories and sets
`FREECELL_SKIP_TOOL_INSTALL` + `FREECELL_SKIP_SYSTEM_DEPS` so `setup_env.sh`'s two
machine-mutating steps (downloading binaries into a system `bin` directory, and the
`setup_render_env.sh` apt install) cannot fire. Its closing cases assert that — `/opt`, the
user's `~/.claude/settings.json`, the installed tool binaries, and the dpkg package list are
all fingerprinted before and after.
