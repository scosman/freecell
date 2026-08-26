# Claude Code cloud environment for FreeCell

Set this up once, in about five minutes. After that every cloud session starts from a
machine image that already has the Rust toolchain, the dependency sources, the GPUI/render
system packages, and a warm compile cache — and a SessionStart hook that gets the checkout
ready before the agent reads anything.

## The pieces

| File | Job |
|---|---|
| [`claude_code_vm_setup.sh`](claude_code_vm_setup.sh) | The **only** thing you paste into the cloud UI. It fetches `setup_env.sh` from `main` and runs it. |
| [`setup_env.sh`](setup_env.sh) | **Builds a machine.** Runs once, at environment-build time, before any repo is checked out. Also the "set up my laptop" script (`--human`). |
| [`setup_startup.sh`](setup_startup.sh) | **Prepares a checkout.** Runs every session, via the repo's SessionStart hook. |
| [`test_setup_scripts.sh`](test_setup_scripts.sh) | Tests for all of the above. Runs no `apt-get`, writes no binary into a system `bin` directory, touches nothing under `/opt/freecell-vm-setup` (fingerprinted recursively, with mtimes, before and after), and uses a private sccache port so it cannot stop your running cache server — all asserted at the end, not assumed. It *does* install the toolchain pinned by `app/rust-toolchain.toml` and warm `~/.cargo` if those are cold, which is what your first `cargo build` here would do anyway. The file's header comment is the full contract. |

The hook itself is committed: [`.claude/settings.json`](../../.claude/settings.json) points
at [`.claude/hooks/session-start.sh`](../../.claude/hooks/session-start.sh), which is a shim
that finds and runs `setup_startup.sh`. Version control registers it; no script does, and
none of these scripts has a flag that would.

## 1. Connect GitHub

At [claude.ai/code](https://claude.ai/code), connect the GitHub account that can read
`scosman/freecell`.

## 2. Create the cloud environment

The cloud icon in the row above the message box → **Add cloud environment**. There is no
settings page and no direct URL for this; it is only reachable from that menu.

Name it something like `freecell`.

**Network access: Custom.** Tick **"Also include default list of common package managers"**
(that is what lets `apt-get` reach the Ubuntu archives for the GPUI/render system packages),
then add these domains, one per line:

```
raw.githubusercontent.com
github.com
objects.githubusercontent.com
codeload.github.com
static.rust-lang.org
static.rustup.rs
sh.rustup.rs
index.crates.io
static.crates.io
crates.io
```

Why each one:

- `raw.githubusercontent.com` — the pasted wrapper fetches `setup_env.sh` from there. Without
  it, nothing else happens at all.
- `github.com` + `codeload.github.com` — the throwaway clone of this repo, plus the three git
  dependencies the workspace pins: `zed-industries/zed` (gpui), `longbridge/gpui-component`,
  and `scosman/ironcalc` (our fork, `freecell-fixes` branch).
- `objects.githubusercontent.com` — GitHub release assets redirect there. This is where the
  prebuilt `sccache` and `cargo-deny` binaries actually come from; allowing `github.com`
  alone gets you a redirect into a wall.
- `static.rust-lang.org` — rustup's toolchain downloads (the pinned 1.95.0 plus `rustfmt` and
  `clippy`).
- `sh.rustup.rs` + `static.rustup.rs` — only needed if the base image has no rustup at all;
  cheap insurance.
- `index.crates.io`, `static.crates.io`, `crates.io` — the sparse registry index and the crate
  tarballs.

On the default **Trusted** network policy, expect the environment to come up looking fine and
then be slow forever: the release-asset downloads fail, so `sccache` and `cargo-deny` are
missing, and `setup_startup.sh` says so on every session start.

## 3. Environment variables and the setup script

**Environment variables: none are required.** The compile cache is a local directory on the
machine's own disk, so there is no bucket, no endpoint, and no credential.

If this environment previously used the Cloudflare R2 compile cache, **delete these now** and
revoke the token:

- `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY` — delete from the environment variables, then
  revoke the API token in the Cloudflare dashboard (R2 → *Manage R2 API Tokens*). A secret on
  a snapshotted disk outlives its own rotation, so revoking is the part that matters.
- `SCCACHE_BUCKET`, `SCCACHE_ENDPOINT`, `SCCACHE_REGION` — delete if present. Both scripts
  clear them for their own process before starting a cache server, and `setup_startup.sh`
  writes the same `unset` into the session's environment file on every path — including when
  the cache ends up off, where it drops an inherited `RUSTC_WRAPPER` too. That is a safety
  net, not a substitute: they are inherited by the environment *build*, so a leftover bucket
  is still what stops an image from being warmed (see the row below), and leaving them
  configured is confusing for the next reader.

**Setup script:** paste the *entire contents* of
[`.config/utils/claude_code_vm_setup.sh`](claude_code_vm_setup.sh).

It is a fetcher, not a copy, on purpose: this field is a blob that is pasted once and never
updated, so a pasted copy of the real setup script goes stale silently the moment the real
one changes. The wrapper is ~40 lines and almost never changes; everything that does change
lives in `setup_env.sh` on `main`.

## 4. Start a session

Pick the environment, pick `scosman/freecell`, start a session.

## 5. What to expect

The first session builds the environment — toolchain, system packages, a throwaway clone of
`main`, `cargo fetch`, and a full `cargo build --workspace` to fill the compile cache — then
snapshots the disk. That is a long first run (tens of minutes; the workspace pins gpui/zed,
gpui-component and the IronCalc fork). Every session after that starts from the snapshot.

Setup re-runs, and the snapshot is rebuilt, when you **change the setup script or the
allowed-domain list**, and when the snapshot **expires after about 7 days**.

Then, every session, the SessionStart hook runs `setup_startup.sh`: about a second on a
provisioned machine, ending in one line. That is the trade for not having to think about it.

## 6. Checking it worked

The success line looks like this:

```
Ready. rustc 1.95.0, cargo 1.95.0 (pin 1.95.0), cache: sccache -> /opt/freecell-vm-setup/sccache.
```

If it says `Ready, with warnings above`, or does not appear at all, find your line here:

| What you see | What it means | The one thing to do |
|---|---|---|
| `! This VM was not set up for FreeCell: … is missing.` | The environment's setup script never ran, or ran an older version. Sessions still work, just slowly and uncached. | Re-check the **Setup script** field, then edit it (any edit) to force a snapshot rebuild. |
| `cache: off (sccache not installed; …)` | The `sccache` download failed at build time — almost always `objects.githubusercontent.com` missing from the allowlist. | Add the domain, then force a snapshot rebuild. |
| `cache: off (no provisioning marker; …)` | Same cause as the first row. The cache directory is deliberately not trusted without the marker that says this repo's setup script put it there. | As above. |
| `cache: off (… is missing; …)` | The machine was provisioned but the cache directory is gone. | `bash .config/utils/setup_env.sh --upgrade-tools` |
| `cache: off (sccache resolved to '…', not …)` | The cache server came up on a different backend — usually an sccache server left running from an earlier configuration, or leftover `SCCACHE_BUCKET`/`SCCACHE_ENDPOINT` variables. | `sccache --stop-server`, delete those variables from the environment settings, then start a new session. |
| `cache: off (… is not writable by <user>; …)` | The image was built as one user and the session runs as another. | `chmod 1777 /opt/freecell-vm-setup/sccache` |
| `! native deps missing (…) — freecell-app and render-tests will not build.` | The apt step did not run or did not finish. `freecell-core` and `freecell-engine` still build. | `bash app/render-tests/scripts/setup_render_env.sh` |
| `! cargo-deny is not installed; …` | Same cause as the sccache row. `cargo deny check` (a CI gate) cannot run locally. | `bash .config/utils/setup_env.sh --upgrade-tools` |
| `cache: sccache -> … (THIS PROCESS ONLY — …)` | The cache is wired, but only inside the script's own process. `CLAUDE_ENV_FILE` is how the wiring reaches the session's shells; the message says whether it was **unset** or whether the **write to it failed**. Every build the agent runs would be uncached. | Expected when you run `setup_startup.sh` by hand; in a session it means the hook environment is wrong. Export `RUSTC_WRAPPER=sccache` and `SCCACHE_DIR=/opt/freecell-vm-setup/sccache` yourself, or start a new session. |
| `! … is empty (marker: sccache_warmed=false)` | The image was built, but its warm build never ran or never finished. Everything works; the pinned dependency tree just compiles from cold. | Rebuild the environment image (edit the Setup script field to force it), or `bash .config/utils/setup_env.sh --upgrade-tools --warm-cache` |
| `! … is EMPTY although the VM marker claims it was warmed` | The cache's objects were removed after the image was built (disk pressure, a manual clean). A leftover `SCCACHE_BUCKET` cannot produce this any more — since the warm build reads the server's backend back before building, that case records `sccache_warmed=false` and shows the row above instead. | Force a snapshot rebuild. |
| `! rustfmt is missing from …` / `! clippy is missing from …` | The pinned toolchain was installed without its components. Both are CI gates. | `rustup component add rustfmt --toolchain <pin>` (same for `clippy`) |
| `✗ cargo fetch --locked failed` | `Cargo.lock` does not match the `Cargo.toml`s on this branch. | `cd app && cargo update` |
| `✗ cargo in …/app resolves to 1.94.x, below the required 1.95.0 (app/rust-toolchain.toml pins '…')` | The pinned toolchain is not installed, so a stale default is being used. gpui needs `std::hint::cold_path()` (stable since 1.95.0). | `rustup toolchain install 1.95.0 --profile minimal -c rustfmt -c clippy` |
| `✗ git is not installed.` | The base image has no git. Nothing repo-shaped can work. | `apt-get install -y git`, then `bash .config/utils/setup_env.sh --upgrade-tools` |
| `✗ rustup is not installed, …` | The base image has no rustup, so the pin cannot be selected. The repair cannot be `setup_env.sh` — it needs rustup itself. | `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh` |
| `✗ rustup X is below 1.28.0, …` | Too old to read `rust-toolchain.toml`, so it silently leaves a stale default active. | `rustup self update` |
| `✗ Toolchain … is not installed and could not be installed.` | Usually `static.rust-lang.org` missing from the allowlist. | Add the domain and force a snapshot rebuild, or run the `rustup toolchain install` line it prints. |
| `✗ app/rust-toolchain.toml has no [toolchain] channel` | The pin file is truncated or edited badly on this branch. | `git checkout -- app/rust-toolchain.toml` |
| `✗ No FreeCell checkout found` | The hook fired somewhere that is not a FreeCell checkout. | `cd` into the checkout and re-run `bash .config/utils/setup_startup.sh` |
| `Environment setup complete.` followed by `! WITH WARNINGS` (in the **build** log, not a session) | The image came up, but at least one thing it was supposed to provide is missing — read the `!` lines above it. Most often no `sccache` and no `cargo-deny`, i.e. `objects.githubusercontent.com` missing from the allowlist. | Fix what the `!` lines name, then edit the Setup script field to force a snapshot rebuild. |
| Nothing at all | Expected on a **local** checkout: the startup script is a silent no-op outside a container so it costs a contributor's session nothing. In a cloud session it means the hook did not fire. | Check `.claude/settings.json` is present on the branch you are on. |

## 7. Things to know

- **`Ready, with warnings above` is still a working session.** The warnings name what is
  degraded (usually: no compile cache, or no GPUI system libraries) and each carries its own
  repair. Nothing below the `✗` shape is fatal.
- **The compile cache is not free money.** sccache caches *rustc invocations*, not linking,
  build scripts, or cargo's own orchestration, and it deliberately does not cache incremental
  compilations — which in cargo's dev profile means workspace crates are never cached and the
  pinned dependency tree always is. That tree is the whole point. A "fully cached" cold build
  still pays for linking.
- **A warm `target/` was considered and rejected.** Cargo fingerprints embed absolute paths,
  and hardlinking a shared tree in would let builds write through into it. The reasoning, and
  what would have to change to revisit it, is in
  [`projects/build-cache.md`](../../projects/build-cache.md).
- **The cache directory is world-writable (`1777`), deliberately.** The image is built as
  root and a session may run as another user, and a cache only root can write is a cache
  that silently stores nothing. The trade-off is real and worth naming: any local user can
  put objects into a content-addressed rustc-output cache that later builds link against.
  That is acceptable for a single-tenant sandbox, and would not be on a shared host — there,
  own the directory to the session user and drop the mode to `0755`.
- **The cache directory is capped** (`FREECELL_SCCACHE_CACHE_SIZE`, default `8G`) because it
  is baked into the image and comes out of the same disk allowance a session's `target/`
  (which can reach ~25 GB) has to fit into.
- **The marker's `_vN` is a contract version, and `v1` images are not `v2` images.** In `v1`,
  `sccache_warmed=true` meant "the warm build exited 0"; in `v2` it means "the cache directory
  was confirmed to hold objects". A `v1` image can carry a `true` that is not true in the `v2`
  sense, so the filename moved rather than the new check being retrofitted onto images that
  never ran it. A machine built by an older script therefore reports as *unprovisioned* — a
  warning and a slow session, never a wrong answer. Rebuild the environment image to clear it.
- **`$HOME` at build time is not `$HOME` at session time on this image** (`/root` versus the
  checkout under `/home/user`). That is one reason the SessionStart hook is committed to the
  repo rather than written into a user `settings.json` by a script: `$HOME` resolves when the
  image is built, so such a script would register the hook in the wrong home and nothing would
  notice.
- **If a machine does not honor project hooks**, add the entry by hand rather than looking for
  a flag that does it: put a `SessionStart` group in `~/.claude/settings.json` with
  `"matcher": "startup|resume|fork"` and a `command` of
  `<checkout>/.claude/hooks/session-start.sh`. That file is the same shim the project hook
  runs, and it finds the checkout by walking up from the session's directory.

## 8. Testing a change to the setup

Edit `FREECELL_SETUP_REF` **in the pasted script**, not in the environment variables:

```bash
REF="${FREECELL_SETUP_REF:-my-branch}"
```

Two reasons it has to go there. The wrapper reads it before any session environment exists,
and editing the setup script is *also* what forces the snapshot to rebuild — which is what
makes the new setup actually run. An environment variable would do neither.

A **branch or a tag**, not a commit SHA — the throwaway clone `setup_env.sh` makes is a
depth-1 clone, which cannot target one. The value is passed through to `setup_env.sh`, so the
clone it warms from is the same branch the wrapper fetched from.

To exercise the scripts without a cloud environment at all:

```bash
bash .config/utils/test_setup_scripts.sh
```

Run it anywhere, including your own machine — but read the exact contract first: the table
at the top of this document says what it will never touch, and what it *will* do (install the
pinned toolchain and warm `~/.cargo` if those are cold).

## Local machines

Contributors do not need any of the above. The per-session script refuses to run outside a
container, and:

```bash
bash .config/utils/setup_env.sh --human
```

installs the pinned toolchain, warms `~/.cargo`, and offers (never imposes) the GPUI/render
system packages. It writes nothing under `/opt` and does not touch your Claude Code settings.
