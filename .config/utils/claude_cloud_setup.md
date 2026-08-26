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
| [`test_setup_scripts.sh`](test_setup_scripts.sh) | Tests for all of the above. Safe to run on your own machine: it works in scratch directories and disables the two machine-mutating steps (no `apt-get`, no writes into a system `bin` directory), then asserts at the end that `/opt`, your `~/.claude`, your installed tools, and your package list are all unchanged. |

The hook itself is committed: [`.claude/settings.json`](../../.claude/settings.json) points
at [`.claude/hooks/session-start.sh`](../../.claude/hooks/session-start.sh), which is a shim
that finds and runs `setup_startup.sh`. Nothing needs registering by hand.

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
- `SCCACHE_BUCKET`, `SCCACHE_ENDPOINT`, `SCCACHE_REGION` — delete if present. `setup_startup.sh`
  unsets them anyway before starting the cache server (a running sccache server's backend is
  fixed at start), but leaving them configured is confusing for the next reader.

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
| `✗ cargo fetch --locked failed` | `Cargo.lock` does not match the `Cargo.toml`s on this branch. | `cd app && cargo update` |
| `✗ cargo in …/app resolves to 1.94.x, below the pinned 1.95.0` | The pinned toolchain is not installed, so a stale default is being used. gpui needs `std::hint::cold_path()` (stable since 1.95.0). | `rustup toolchain install 1.95.0 --profile minimal -c rustfmt -c clippy` |
| `✗ No FreeCell checkout found` | The hook fired somewhere that is not a FreeCell checkout. | `cd` into the checkout and re-run `bash .config/utils/setup_startup.sh` |
| Nothing at all | The hook did not fire. | Check `.claude/settings.json` is present on the branch you are on. |

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
- **The cache directory is capped** (`FREECELL_SCCACHE_CACHE_SIZE`, default `8G`) because it
  is baked into the image and comes out of the same disk allowance a session's `target/`
  (which can reach ~25 GB) has to fit into.
- **`$HOME` at build time is not `$HOME` at session time on this image** (`/root` versus the
  checkout under `/home/user`). That is exactly why the SessionStart hook is committed to the
  repo rather than registered in a user `settings.json`. If you ever do use
  `setup_env.sh --create-startup-script`, set `CLAUDE_CONFIG_DIR` explicitly.

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

That is safe to run anywhere — see the table at the top of this document for what it does
and does not touch.

## Local machines

Contributors do not need any of the above. The per-session script refuses to run outside a
container, and:

```bash
bash .config/utils/setup_env.sh --human
```

installs the pinned toolchain, warms `~/.cargo`, and offers (never imposes) the GPUI/render
system packages. It writes nothing under `/opt` and does not touch your Claude Code settings.
