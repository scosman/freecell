# Phase 7: Build, dependencies, licensing & shipping posture

Reviewed as an incoming owner: is this thing buildable, reproducible, shippable, and legally
distributable by someone who is not its author? Verified against `app/Cargo.toml`,
`app/Cargo.lock` (875 packages, 9 git sources), `app/deny.toml`, `app/rust-toolchain.toml`,
`app/vendor/`, all six workflows, `app/scripts/`, `app/PACKAGING.md`, and the `projects/` notes —
not against the repo's own claims about itself. Where the docs and the tree disagree, I went with
the tree.

**Scope note on licensing:** everything below is an assessment of *engineering* risk — whether a
mechanism does what it says, whether it fails loudly, whether it survives maintenance. Nothing
here is legal advice, and the question of whether the GPL position is *legally* sound is for
counsel, not for me.

## What's Good

- **The lockfile is real and the git pins are honest.** `app/Cargo.lock` is committed and every
  one of the 9 git sources resolves to a concrete SHA — including the branch-tracked engine
  (`git+https://github.com/scosman/ironcalc?branch=freecell-fixes#cee2859dceda65ff64e52192be4ec47a259870e1`).
  The `gpui`/`gpui_platform`/`gpui-component` trio is pinned by `rev` in the manifest, not by
  branch, and the workspace comment correctly identifies the zed rev and the gpui-component SHA as
  a *known-good pair that must be bumped together*. That is the right mental model and it is
  written down where the next person will find it.

- **The GPL patch-out actually works, and I verified it rather than taking the README's word.**
  In `app/Cargo.lock`, `ztracing` and `ztracing_macro` appear with **no `source` field** (i.e.
  resolved to the local `app/vendor/` paths), and `zlog` is **absent from the lock entirely** —
  the stub correctly drops the `zlog` dependency, which is the whole trick. The stubs themselves
  (`app/vendor/ztracing/src/lib.rs`, `app/vendor/ztracing_macro/src/lib.rs`) are ~60 lines,
  faithfully no-op, and reproduce the upstream surface rather than guessing at it. They are
  `[workspace].exclude`d so they don't inherit workspace lints/edition. `app/deny.toml` now
  carries `exceptions = []` with an explicit comment that a reappearing GPL crate must *fail*
  rather than be waved through. Replacing a license exception with an actual removal is the
  strictly better engineering answer, and it was done properly.

- **`cargo-deny` is enforced, pinned, and the ignores are individually justified with
  provenance.** `checks.yml` runs `cargo deny check` (all four checks) with cargo-deny pinned to
  `0.19.9` via `taiki-e/install-action`, so the gate can't drift under a tool bump. Every entry in
  `deny.toml`'s `[advisories] ignore` names the crate, the path it enters by, and why there's no
  safe upgrade. `projects/pre-distribution-security-audit.md` goes further and traces quick-xml's
  provenance to build-time/desktop-protocol parsers and explicitly establishes it is **not** on
  the `.xlsx` open path. That is a materially better standard of dependency documentation than
  most commercial codebases manage.

- **Packaging is genuinely done, not gestured at.** One `[package.metadata.packager]` config
  drives macOS `.app`/`.dmg`, Linux `.deb`/`.AppImage` (native x64 **and** arm64 runners, not
  cross-compiled), and Windows NSIS — and CI calls the *same* `scripts/package.sh` /
  `package.ps1` a developer runs locally, so there is no CI-only packaging path to rot. The
  AppStream metainfo, the deb `package-name` override, the `file-associations` block, and the
  Linux `app_id`-must-equal-binary-name gotcha are all handled with the reasoning recorded.

- **The macOS signing script is unusually well-reasoned.** `scripts/sign_macos.sh` asserts on
  `codesign -d` *output* rather than trusting `codesign --verify`, on the correct grounds that
  every arm64 Mach-O is ad-hoc linker-signed and `--verify --strict` therefore passes on an
  unsigned bundle. It also gets `--force`, `--timestamp`, `--options runtime`, not-`--deep`, and
  the `ditto -c -k` submit-vs-staple distinction right. Most people learn each of those from a
  failed release; this was reasoned out in advance. Windows Authenticode via Azure Trusted Signing
  is wired into CI, signs the core exe *before* NSIS packs it (the part people get wrong), and
  no-ops cleanly when unconfigured so forks still build.

- **The release workflow deliberately refuses to publish.** `release.yml` uploads run artifacts
  and explicitly does *not* create a GitHub Release, gated on
  `projects/pre-distribution-security-audit.md`. Self-imposed shipping gates that the author has
  not yet quietly removed are a good sign.

- **Asset licensing is handled.** `assets/fonts/inter/OFL.txt` and `assets/icons/LICENSE` ship
  alongside the vendored font and Lucide glyphs. Easy to forget; not forgotten.

- **CI cost engineering is thoughtful and its reasoning is recorded.** `cache-on-failure: true`
  (with the correct explanation of why the default is self-perpetuating), `CARGO_INCREMENTAL=0`,
  `CARGO_PROFILE_{DEV,TEST}_DEBUG=line-tables-only` with the measured 14 GB → 6.3 GB effect, and
  the inlined runner disk-prune (inlined specifically so the `rm` set stays auditable). These are
  the notes of someone who debugged real CI failures rather than copy-pasting a template.

## Critical (must fix)

### C1. The engine is pinned to a **mutable branch ref** on a personal fork, and the project's own documented maintenance procedure destroys it

```toml
[patch.crates-io]
ironcalc      = { git = "https://github.com/scosman/ironcalc", branch = "freecell-fixes" }
ironcalc_base = { git = "https://github.com/scosman/ironcalc", branch = "freecell-fixes" }
```

`app/Cargo.lock` resolves this to `cee2859dceda65ff64e52192be4ec47a259870e1`, so a lock-respecting
build is deterministic *today*. That is not the risk. The risk is that `freecell-fixes` is, by
design, a **rebased integration branch**. `CLAUDE.md` §Engine states the standing procedure:
"Sync `main` from upstream periodically (**rebase** `fix/*` + `freecell-fixes`)." A rebase +
force-push orphans `cee2859d`; GitHub garbage-collects unreachable objects on its own schedule.
The moment that happens, **every historical commit of FreeCell becomes unbuildable** — not just
`main`, but any tag, any bisect point, any release you would need to hotfix. A pinned SHA cannot
fetch an object that no longer exists, and there is no vendored copy, no mirror, no immutable tag,
and no `[patch]` fallback in the tree.

Compounding it: the entire calculation engine — the thing the product is — hangs off one personal
GitHub account. Account deletion, rename, or a private-repo flip is a total build outage with no
in-repo recovery path.

This is the one dependency shape in the workspace that is *not* SHA-pinned in the manifest, and
it happens to be both the most load-bearing and the one whose maintenance procedure is
history-rewriting. Every other git dep in the tree — including four of zed's own forks — uses
`rev = "..."`. Fix: pin `rev = "cee2859d…"` (matching the existing style), push an immutable
`freecell-v<n>` tag on the fork per adopted state so the objects are permanently reachable, and
for release builds strongly consider `cargo vendor`ing the engine into the repo. All three are
cheap; none is done.

### C2. Nothing in CI builds with `--locked`, so the committed lockfile is not the lockfile that gets tested

Grepping all six workflows and both packaging scripts for `--locked|--frozen|--offline` returns
**only** `cargo install cargo-packager --locked` and `cargo install trusted-signing-cli --locked`.
Every `cargo build`, `cargo test`, `cargo clippy`, `cargo deny check`, and every packaging build
runs unlocked.

Consequences, in order of how much they'd annoy me as the owner:

1. **`cargo deny check` may be auditing a graph nobody reviewed.** The license and advisory gate
   is only meaningful if the lock it inspects is the lock that was committed and read. Unlocked,
   cargo is free to rewrite `Cargo.lock` in the CI workspace before deny reads it.
2. **Manifest/lock drift never fails.** A PR that edits a dependency in `Cargo.toml` without
   regenerating the lock passes green; CI silently re-resolves, and the committed lock and the
   tested graph diverge with no signal.
3. **Reproducibility of a release artifact is unverified.** `release.yml` — the workflow whose
   entire output is a signed binary you will hand to strangers — builds unlocked. You cannot
   currently assert that the `.dmg` you notarized was built from the dependency set in the
   repository.

For a supply chain of 875 crates across 9 git remotes, with a documented policy of *not*
controlling the upstream lockfiles, `--locked` is the cheapest integrity control available and it
is absent everywhere it matters. One word per step.

### C3. FreeCell rides unreleased upstream `main` plus two patches that have **never been submitted**, with no scheduled exit

The manifest says `ironcalc = "=0.7.1"`. What actually compiles is fork `main` — which
`specs/projects/ironcalc-upstreaming/fork_audit.md` documents as "well ahead of the crates.io
`0.7.1` we pin," including a **breaking style-color API change** (`Fill { fg_color: Option<String> }`
→ `Fill { color: Color }`) — plus two local fixes on top. The version string in the manifest is
actively misleading about what is in the binary.

The upstreaming plan's own status is worse than the narrative suggests.
`specs/projects/ironcalc-upstreaming/implementation_plan.md` is `status: draft`, and:

- **Phase 5 — "open one PR per fix (E2, E5) against `ironcalc/IronCalc:main`" — is unchecked.**
  The patches exist only as `patches/0001-e2-numfmt.patch` and `patches/0002-e5-indexed.patch` in
  a specs directory. Upstream has never seen them. There is no PR to track, no maintainer
  feedback, no merge ETA.
- **Phase 4 (the validation gate that proves removing the old workarounds was correct) is also
  unchecked** — yet Phase 3, which *deleted* `open_fixups.rs` / `open_repair.rs`, is marked DONE.
  The cleanup shipped ahead of the proof that it was safe.

So the distance to a released crates.io pin is not "two small PRs." It is: upstream must cut a
release containing the whole unreleased `main` delta, **and** accept two patches nobody has
proposed, **and** FreeCell must then reconcile whatever else moved. `projects/ironcalc-upgrade.md`
describes this exit but is `Status: Future` and gated on a release that has not been requested.

CLAUDE.md frames "fix upstream, don't hack FreeCell" as a virtue, and as a *principle* it is
correct — a compensating hack in the app is worse than a fix in the engine. But the principle has
been executed only through the half that adds coupling (build against the fork) and not the half
that removes it (land the PRs). Right now the project owns an indefinite private fork of its
calculation engine, and the single-fix-per-branch discipline that would make upstreaming tractable
is being spent on branches that are never sent.

### C4. The only automated gate on the product's core differentiator does not run automatically

Every other gate auto-runs on `app/**` pushes/PRs: `checks` (fmt/clippy/build/test/deny),
`perf-gates`, `roundtrip`. The **pixel render suite** — the only thing that verifies the custom
GPU-rendered grid actually draws correctly, which is the entire product thesis — is
`workflow_dispatch:`-only (`render.yml`), triggered by a human remembering to trigger it.
`CLAUDE.md` states this plainly: "the **agent must decide when render coverage is needed and
drive it** — there is no safety net."

Three things make this worse than a defensible cost trade-off:

1. **Whether it is enforced at all is unverifiable from the repository.** `render.yml`'s header
   says it "MUST be a required status check" under the exact context `render (Xvfb + lavapipe)`,
   but branch protection lives in GitHub settings, outside the tree. A new owner cloning this repo
   cannot determine whether the gate is enforced, and a rename of the job `name:` silently drops
   the requirement with no in-repo signal.
2. **The docs already contradict the tree.** `checks.yml`'s own header comment claims the job runs
   "the render suite (Xvfb + lavapipe)"; `app/README.md` §CI likewise lists **checks** as
   including "the render suite (Xvfb + lavapipe)" and omits `roundtrip` entirely. Both are wrong —
   render was split out. If the *author's own docs* have already lost track of which gate covers
   pixels, a hand-driven gate is not going to be reliably driven.
3. **The scope hole is real, not theoretical.** Per CLAUDE.md, the suite covers grid/cell/sheet,
   the macOS titlebar row, and chart scenes — and explicitly **not** the welcome window, About
   window, action row, formula row, or sheet tabs. So a meaningful fraction of the visible UI has
   no pixel coverage under *any* gate, automatic or manual.

I accept that a multi-minute lavapipe suite should not sit on every push. The answer is a
scheduled/nightly run on `main` plus a merge-queue run, not "a convention in a markdown file."
As it stands, `main` can go visually red with every automatic gate green.

## Moderate (should fix)

### M1. There is no update mechanism of any kind, and it is not on the roadmap

Grepping the workspace for `sparkle|autoupdate|auto.update|velopack|updater` returns nothing.
`projects/release-signing-and-distribution.md` covers signing and "switch the workflow to attach
GitHub Release assets" — and stops there. For a downloadable desktop app shipping `.dmg` /
`.deb` / `.AppImage` / NSIS `.exe`, that means the day you publish v0.1.0 you have committed to a
world where every future security fix reaches users only if they independently notice and
re-download. Given the dependency posture in this same review (transitive advisories acknowledged
as unfixable at the pinned revs), the inability to push a fix to installed users is the thing I'd
be most uncomfortable owning. It should at minimum be a tracked `projects/` note with a decision
recorded, not an absence.

### M2. No panic hook and no crash reporting in the app process

The engine worker is well defended — `catch_unwind` in `worker/run.rs` degrades a panicking
recompute rather than killing the process, which is the right call and clearly deliberate. But
`freecell-app` installs no `std::panic::set_hook`. A panic on the GPUI main thread closes the
window with no message, no log written to a discoverable path, and no artifact the user could
attach to a bug report.

Shipping **no telemetry** is a deliberate and defensible product stance — the packager
`long-description` promises "no cloud, no analytics, completely private," and I would keep that.
But "no analytics" and "no local crash log" are different decisions, and only the first one looks
chosen. A panic hook that writes a timestamped backtrace next to the recents file
(`dirs::data_dir()/FreeCell/`) costs ~20 lines, violates nothing the product promises, and is the
difference between a diagnosable bug report and "it just closed."

### M3. `zip 0.6.6` and `roxmltree 0.19` are now **direct runtime deps of `freecell-engine`, on the untrusted-input path** — and both project notes about them are stale

`crates/freecell-engine/Cargo.toml` lists `zip.workspace = true` and `roxmltree.workspace = true`
under `[dependencies]` (not dev-deps) for the chart file layer, which parses user-supplied `.xlsx`
— i.e. an attacker-controlled zip containing attacker-controlled XML.

- `zip = "0.6"` resolves to `0.6.6`. That is a stale major line; the crate has since moved through
  1.x/2.x under different maintenance. Nothing in the tree stages an upgrade.
- `roxmltree = "0.19"` resolves to `0.19.0` while the lock **already contains `roxmltree 0.20.0`**
  for another consumer — so the newer version is being compiled anyway and the pin buys nothing.

Both `projects/` notes that should describe this are wrong. `projects/ironcalc-upgrade.md` and the
upstreaming plan's Phase 3 state that the migration "dropped `roxmltree`, moved `zip` to
dev-deps"; the charts work re-added both as runtime deps and nobody updated the notes.
`projects/pre-distribution-security-audit.md` still frames these as "**ironcalc's** xlsx stack,"
which understates the exposure — it is now FreeCell's own first-party parsing code. cargo-deny is
clean on them today, and that is the *only* thing between this and a live finding: an advisory
landing on `zip 0.6.x` puts it squarely on the file-open path with no upgrade prepared.

### M4. The GPL patch-out has a silent-failure mode, and the direct check for it is documented but never run

`[patch]` matches by name **and version**. If a gpui/zed rev bump moves `ztracing` to `0.2.0`,
cargo emits a *warning* — "patch … was not used in the crate graph" — and links the real GPL
crate. Nothing in CI fails on cargo warnings (`clippy -D warnings` covers rustc/clippy lints, not
cargo diagnostics), so the substitution can lapse without a red build.

The backstop is real: `deny.toml` has `exceptions = []`, so the license check *would* then fail.
Credit for that — it's the reason this is Moderate and not Critical. But it is an indirect catch
that depends on cargo-deny's license detection and on the reintroduced crate still declaring
GPL-3.0. `app/vendor/README.md` already names the direct assertion —
`cargo tree -i ztracing` / `cargo tree -i zlog` — and prescribes running it on every gpui bump.
That instruction lives only in a markdown file addressed to a human. Three lines in `checks.yml`
turning it into an assertion (`! cargo tree -i zlog` must fail; `cargo tree -i ztracing` must
resolve to the local path) makes the position airtight and removes a load-bearing manual step from
the most consequential licensing control in the repo.

### M5. The stated primary platform is not gated on any PR

Every document calls macOS/Metal the primary design target. The only workflow that compiles or
tests on macOS is `macos-verify`, which is **non-required, weekly cron + manual dispatch**. So the
required-per-PR compile/test coverage is Linux only, while the tree carries real platform-
conditional code — `[target.'cfg(windows)'.dependencies] windows`, the macOS CoreFoundation/
CoreServices FFI in `shell::default_app`, `shell::open_files`'s macOS `on_open_urls` path, the
macOS titlebar row. A macOS-only break lands on `main` and is found up to seven days later, by
which point several commits are stacked on it.

I understand the runner-cost argument, and I don't think macOS belongs on every push. But
"primary target, discovered broken weekly" is the wrong end of the trade. A macOS `cargo check
--workspace` (not build+test) on PRs is minutes, catches the overwhelming majority of cfg
breakage, and would close this.

### M6. Path-filtered triggers create a documented deadlock the workflows describe as intentional

`checks.yml` and `perf-gates.yml` use trigger-level `paths: ['app/**', …]`. Both files contain a
long comment acknowledging the consequence: a PR touching no `app/**` file **skips the whole
workflow, so the context is never reported**, and under strict branch protection with `checks`
required, such a PR sits at "Expected — waiting for status" and can never merge. Given how much of
this repo is `specs/`, `projects/`, `experiments/`, and root markdown, that is not an edge case —
it is most PRs.

The comment calls this "the intended budget trade-off." A state where a docs PR cannot merge isn't
a trade-off, it's a defect, and the comment itself names the standard fix (always-run job +
`dorny/paths-filter` gating the heavy steps, so one context always posts). Documenting a known
broken state at length is not the same as accepting it.

### M7. Build cost is a symptom of the dependency choice, and the mitigation does not reach new contributors

875 lock packages; 84 crate names duplicated across versions; 9 git remotes. Essentially all of it
arrives because `gpui` is consumed from the zed monorepo rather than as a published crate, so a
spreadsheet's dependency graph contains `async-std`, a `reqwest` fork, a `wgpu` fork, `scap`
(screen capture), `rav1e`, `accesskit`, and the full `zbus`/`atspi` accessibility stack.
`projects/build-cache.md` puts a cold full build at 15–25 min; `CLAUDE.md` puts `target/` at
~25 GB.

The sccache/R2 work is good engineering and honestly documented — it states plainly that it caches
rustc invocations but not linking or build-script execution, so "fully cached" is a big saving,
not zero. My concern is who it serves. It is (a) explicitly dev-container-only, with CI
deliberately left on `Swatinem/rust-cache`; (b) gated on `R2_ACCESS_KEY_ID`/`R2_SECRET_ACCESS_KEY`
secrets that only the author holds; (c) a graceful no-op without them. So **an outside contributor
gets none of it**: ~12 apt packages, then 15–25 minutes and ~25 GB of disk before they can see a
window on screen.

To answer the question directly — plaster or fix? It is a genuine fix for the *author's* iteration
loop and a plaster for the project's bus factor. The underlying wound (depending on an unpublished
UI framework out of a 1M-line editor monorepo) is not fixable by this project, and I don't think
it should be relitigated — gpui is the bet. But the contributor-onboarding cost should be stated
honestly in `app/README.md` (it currently isn't), because right now the first-run experience is
the single largest barrier to this project having a second contributor.

## Mild (consider fixing)

- **`deny.toml` `[sources] unknown-git = "allow"`.** This is the one permissive setting with real
  supply-chain consequence: *any* git dependency from *any* host passes the gate silently. The
  file's own comment defers enumeration to "P13 hardening," which has no date. Listing the four
  known hosts (`zed-industries`, `longbridge`, `scosman`, `proptest-rs`) under `allow-git` is a
  handful of lines and would make an unexpected new git remote fail loudly. `multiple-versions`/
  `wildcards = "allow"` I'd leave alone — deduping zed's tree isn't winnable.

- **Doc drift on what CI actually gates.** `app/README.md` §CI lists `checks` as running "the
  render suite (Xvfb + lavapipe)" (it doesn't — split into `render.yml`) and omits `roundtrip`
  entirely. `checks.yml`'s header still says "cargo-deny (licenses/advisories, incl. the
  documented GPL ztracing exception)" though `deny.toml` now has `exceptions = []` precisely so
  there is no exception. These are the first files a new owner reads to learn what protects
  `main`, and both currently misdescribe it.

- **`vendor/ztracing*` make a licensing claim without carrying the license text.** Both declare
  `license = "MIT OR Apache-2.0"` in `Cargo.toml` but neither directory contains a `LICENSE` file
  — unlike the vendored assets, which get this right (`OFL.txt`, `icons/LICENSE`). These two
  crates exist *for the sole purpose* of making a licensing assertion; the assertion should be
  self-contained in the directory, not inferred from a workspace two levels up.

- **MSRV floor == pinned toolchain, with no forward-looking build.** `rust-version = "1.95"` and
  `channel = "1.95.0"` are identical, which is fine policy for an app (`publish = false`
  throughout) and the reason is documented (`std::hint::cold_path()`). The cost is that the
  project permanently rides the newest stable with no early-warning signal: there is no
  beta/nightly job, so a toolchain regression is always discovered as a broken build rather than
  a yellow warning a week earlier. A non-required weekly `cargo check` on beta, alongside
  `macos-verify`, is cheap insurance for a project that has committed to newest-stable.

- **`setup_sccache.sh` publishes the R2 bucket name and account-id endpoint in a public repo, and
  its `AWS_*` override is a quiet footgun.** The credentials themselves are handled correctly —
  env-only, written to the session env file by *reference* (`"${R2_ACCESS_KEY_ID}"`), never
  printed. But the bucket/endpoint being committed narrows an attacker's target, and the script's
  note that the R2 token overrides `AWS_ACCESS_KEY_ID` **for everything else in the session**
  deserves a louder warning than a bullet in `projects/build-cache.md`.

- **`PACKAGING.md` §Verification status is refreshingly honest and should stay that way.** It
  states outright that `.dmg`, `.AppImage`, the arm64 legs, and the NSIS `.exe` have never been
  assembled, and that `sign_macos.sh` "has never been run at all — its first real run is its first
  validation." That candour is a strength; the ask is only that it be kept current, because the
  moment it goes stale it becomes the most dangerous document in the repo.

## Phase Summary

**Counts:** Critical 4 · Moderate 7 · Mild 6.

The dependency and licensing *analysis* in this repo is better than most commercial codebases I've
reviewed — every advisory ignore is traced to its entry path, the GPL removal is a real removal
that I verified against the lock rather than a paper exception, and the packaging/signing work is
genuinely finished and correctly reasoned down to details most teams learn from a failed release.
The problem is not thinking; it is that several of the controls protecting all that thinking are
manual, and the one dependency the author controls most directly is pinned in the most fragile way
available.

**The most important finding is C1**, and it is the one I would fix this week: the calculation
engine — the product's core — is pinned to a *branch*, not a SHA, on a personal fork whose
documented maintenance procedure is rebasing. One force-push garbage-collecting `cee2859d` makes
every historical commit of FreeCell permanently unbuildable, including any tag you would need to
hotfix. Every other git dependency in the tree, including four of zed's own forks, is `rev`-pinned;
this one is not. Paired with C3 — the two engine patches have **never been submitted upstream**
(`implementation_plan.md` Phase 5 unchecked) while the code that depended on the old workarounds
has already been deleted (Phase 4, the validation gate, also unchecked) — the "fix upstream, don't
hack FreeCell" strategy has so far executed only the half that adds coupling. Add C2 (nothing in
CI builds `--locked`, so the audited lock and the built lock are not provably the same, including
in the release job that produces signed binaries) and the reproducibility story is weaker than the
quality of the surrounding work would lead you to expect. All three are cheap to fix and none
requires a design change.
