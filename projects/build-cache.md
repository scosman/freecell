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
                   └─ /opt/freecell-vm-setup/.setup_for_freecell_repo_v2  (marker)

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

The `_vN` in the marker filename is a **contract version** (currently `_v2`): bump it in both scripts when
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

## Verifying the cache rather than trusting it

Two claims could each make a machine report a healthy cache while holding nothing, so
neither is taken on faith:

- **`SCCACHE_BUCKET` beats `SCCACHE_DIR`.** Setting the directory does not select the disk
  backend. A cloud environment build inherits the environment's variables, so a leftover
  bucket would send the whole warm build to a remote store (or, with its token revoked,
  nowhere) while the image's directory stayed empty. Both scripts clear the remote-backend
  variables at script scope — for their own process, not machine-wide; nothing about the
  clearing survives into the image — and `setup_env.sh` additionally reads back the *running
  server's* `Cache location` before building, refusing to warm if it is not the image's
  directory. `setup_startup.sh` writes the same `unset` into the session environment file on
  **every** path, since its own `unset` dies with the process while the agent's later shells
  inherit whatever the container started with; when the cache ends up off it drops an
  inherited `RUSTC_WRAPPER` too, so "off" cannot quietly mean "sccache against the old
  bucket".
- **A green build is not a warm cache.** `sccache_warmed=true` is written only after the
  cache directory is confirmed to hold at least one **file**, and the marker's carry-forward
  path applies the identical rule, so a directory of empty subdirectories cannot resurrect a
  stale `true`. `setup_startup.sh` re-checks the same thing every session — reporting a cold
  cache, and calling out the contradiction when the marker claims otherwise. Existence,
  provenance and writability all pass on an image that warmed the wrong store; only the
  contents distinguish it.

The change of meaning is why the marker's contract version moved from `_v1` to `_v2`: in
`_v1`, `sccache_warmed=true` meant "the warm build exited 0". Retrofitting the stricter
reading onto images built by the older script would have made the marker say something those
images never checked, so the filename moved instead and a `v1` machine now reports as
unprovisioned — a warning and a slow session, never a wrong answer.

The suite proves the underlying behaviour rather than only the messages, and proves it end to
end: one case runs `setup_startup.sh` for real against an empty cache fixture, then has a
*later shell* — one given no cache environment of its own, exactly like the agent's — source
the generated `CLAUDE_ENV_FILE` and compile a scratch crate. It asserts the wrapper arrived,
an object landed in the image's cache directory, and sccache's compile-request *count* rose.
Deleting `export RUSTC_WRAPPER` from the env-file block turns three of those red, so the case
is sensitive to the wiring it claims to cover rather than to a re-implementation of it.

The env file is load-bearing for the same reason: `export RUSTC_WRAPPER=sccache` inside the
hook reaches the hook's own process and nothing else. When `CLAUDE_ENV_FILE` is unset — or is
set to a path the write fails on — `setup_startup.sh` says so and degrades its status line to
`(THIS PROCESS ONLY …)` rather than reporting a cache the session will never use.

### One bash trap both scripts now depend on

Checking that a redirection actually opened is not as simple as it looks. Measured on bash
5.2.21, for `>>`/`>` to a path that cannot be opened:

| form | result |
|---|---|
| `{ echo a; } >>/bad` (bare) | `$?` is 1 — **detected** |
| `if { echo a; } >>/bad; then` | else branch — **detected** |
| `{ echo a; } >>/bad \|\| …` | fires — **detected** |
| `if ! { echo a; } >>/bad; then` | **SWALLOWED** |
| `if ! ( echo a ) >>/bad; then` | **SWALLOWED** |
| `if ! printf x >>/bad; then` | detected |
| `if ! some_function >>/bad; then` | detected |

The anomaly is **`!` applied to a compound command** — a group or a subshell — not groups in
general: without `!`, every form reports correctly, and `!` in front of a simple command or a
function call is fine too. So anywhere a compound command's redirection must be checked, these
scripts run it bare and test `$?` (`warm_sccache` in `setup_env.sh`, the `cargo fetch` in
`setup_startup.sh`), and where a single write is enough they use a simple command
(`persist_cache_env`). Note also that `2>/dev/null` does not suppress the message: redirections
are applied left to right, so `>>` fails before the stderr redirection is installed.

The same rule governs the build-time script's closing line. `setup_env.sh` counts its own
warnings and closes with `WITH WARNINGS` when there were any, because that log is an
operator's only view of an image build and an unqualified "complete." after three degradations
is a success signal that outruns what was done. Invariant 1 forbids *exiting* non-zero; it
does not forbid saying so.

## What the per-session script does *not* do

No `cargo build`. A full workspace build at session start would cost minutes even fully
cached, and the agent's first real build pays that anyway. `cargo fetch --locked` against
the image's warm registry/git caches is ~1 s, and `--locked` turns "this branch changed its
dependencies" into a clear message naming `cargo update` rather than a silent lockfile
rewrite.

There is also no background pre-fetch. The previous design detached a `cargo fetch` at
session start, which raced the agent's first build every time; a warm image makes it
unnecessary.

## Hook registration is version control's job, not a script's

`.claude/settings.json` and `.claude/hooks/session-start.sh` are **committed source** here, so
the SessionStart hook is registered by checking the repo out. Nothing in `.config/utils/`
writes to `~/.claude`.

This is worth stating because the design brief this work came from specifies a
`--create-startup-script` flag that writes a machine-global copy of the hook and merges an
entry into the user's `settings.json`. That flag was carried over from a repo where `.claude/`
is *generated and gitignored*, so a script is the only thing that can register the hook. It
was implemented here, then deleted: keeping it meant maintaining two implementations of one
JSON-merge contract (jq and python3), plus a duplicated copy of the committed hook kept in
agreement by roughly 150 lines of tests — none of it on any path this repo actually uses.

On a machine that does not honor project hooks, add the entry by hand: a `SessionStart` group
in `~/.claude/settings.json` with `"matcher": "startup|resume|fork"` whose `command` is
`<checkout>/.claude/hooks/session-start.sh`. That file walks up to find the checkout, so it
works from anywhere.

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
throwaway-clone lifecycle, and the compile cache actually caching.

What it will not do to the machine it runs on, asserted in its closing cases rather than
assumed: no `apt-get` (`FREECELL_SKIP_SYSTEM_DEPS`, plus the absence of the render-env log),
no binary written into a system `bin` directory (`FREECELL_SKIP_TOOL_INSTALL`), nothing under
`/opt/freecell-vm-setup`, no temp files outside its scratch dir,
and no interference with a running sccache server (it uses a private `SCCACHE_SERVER_PORT`).

What it *does* do, because it runs the real scripts: `rustup toolchain install` for the pin,
and `cargo fetch --locked` — so on a cold machine it downloads a toolchain into `~/.rustup`
and populates `~/.cargo` (~1 GB here). That is what a first `cargo build` in this repo would
do anyway, which is why it is documented rather than guarded. The suite's header comment is
the authoritative version of this list.
