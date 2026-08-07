# FreeCell

A GPU-rendered (GPUI, à la Zed/Ghostty), Rust, Excel-compatible spreadsheet built to be
**stupid-fast on huge sheets** (Excel-max = 1,048,576 rows × 16,384 cols). Engine =
**IronCalc**; UI = **GPUI** (custom raw-gpui grid + gpui-component for chrome).

Built agentically in **staged de-risking rounds**. There is **no production app yet** —
current work is experiments + specs that decide whether/how to build it.

## Layout

- **`specs/projects/`** — spec-driven **planning + build** artifacts per phase (overview
  → functional spec → architecture → implementation plan → phase plans), managed via the
  `spec` skill.
- **`experiments/`** — de-risking experiments. Phase 1 = `00`–`06` + `SYNTHESIS.md`;
  Phase 2 = `round-2/` (`SP1`–`SP5` + `SYNTHESIS.md`). Each is an independent Cargo
  project with a `findings.md` + committed `results/`. `experiments/shared/` and
  `experiments/round-2/harness/` are **frozen** (read-only) shared crates.
- **`experiments/round-2/SYNTHESIS.md`** — the current Stage-3 recommendation, **adopted
  baseline decisions**, and Round-3 agenda (the closest thing to a real-app plan of
  record).

## Projects backlog — `PROJECTS.md` + `projects/`

`PROJECTS.md` (root) and the `projects/` folder are our **"save for later" list**. When
we spot an optimization, feature, or goal we want but that is **off the critical path /
not needed for MVP**, we capture it here instead of building it now or losing track of
it:

- Add a short entry to the list in **`PROJECTS.md`**.
- Write a design/goal note as **`projects/<name>.md`** (status: `Future`).

This keeps good ideas tracked without dragging them onto the critical path. It is
distinct from `specs/projects/`, which holds *active* spec-driven build planning.

## Engine: we ride our IronCalc fork (fix upstream, don't hack FreeCell)

FreeCell depends on **our fork** `scosman/ironcalc`, not crates.io directly. When you hit an
IronCalc bug or missing capability, **fix it in the fork and contribute it back upstream**
(`ironcalc/IronCalc`) as a clean single-fix PR — do **not** add a compensating workaround in
FreeCell. This is the standing way of working, not a one-off.

- **The fork is permanent — there is no exit and we are not working toward one.** We keep it, keep
  upstreaming fixes as clean single-fix PRs, and keep re-syncing from upstream `main`. It carries
  more than fixes: merged cells is a whole feature upstream does not have. Upstreaming keeps the
  carried delta small; it is not an attempt to reach zero. (`projects/ironcalc-upgrade.md`)
- FreeCell's `app/Cargo.toml` patches `ironcalc`/`ironcalc_base` via `[patch.crates-io]` to a
  **`rev = "<sha>"` on the fork's `freecell-fixes` branch** — pinned by SHA, never by branch, since
  `freecell-fixes` is rebased and reverted in place (v05-cleanup-1/A1). **Bumping the fork is a
  deliberate one-line edit of both revs + `cargo update -p ironcalc -p ironcalc_base`, in its own
  commit, with the workspace tests run** — never a drive-by, and never left to a stray
  `cargo update`.
- Fork branches: `main` = clean mirror of upstream; `fix/<slug>` = one branch per fix (off `main`,
  with upstream-style tests) = one clean PR; `freecell-fixes` = integration branch FreeCell builds
  against. Sync `main` from upstream periodically (rebase `fix/*` + `freecell-fixes`); expect
  incidental drift to reconcile on the FreeCell side — upstream may also *reshape* what it merged
  (it renamed our `set_worksheet_index` to `move_sheet`), so a sync can break a FreeCell call site.
- **Current state of the delta** (re-derived 2026-08-07 at `ecbf6226`, patch-id verified against
  upstream): of **11** changes we authored, **8 are merged upstream** and 3 are fork-only; one more
  (DOLLAR) was reverted as wrong. Per-fix table, upstream PR numbers, and the method to re-derive
  it: [`specs/projects/ironcalc-upstreaming/implementation_plan.md`](specs/projects/ironcalc-upstreaming/implementation_plan.md)
  §Status table. Don't trust a stale summary — re-run the classification, **including its
  cross-check step**: enumerating the branch's merges alone silently under-counts, because a fix
  stops being a fork-only merge once upstream takes it and the fork re-syncs `main`.
- **One fix = one branch = one focused single-feature upstream PR. Never fold multiple fork fixes
  into a single `fix/` branch (or a single FreeCell phase).** Upstream wants independent,
  single-feature PRs they can review + merge in isolation; a bundled branch is not acceptable
  upstream and is harder to revert. If a FreeCell phase needs two unrelated fork capabilities, each
  gets its own `fix/<slug>` branch + PR.
- An agent can work both repos in one container (FreeCell here; fork at `/workspace/ironcalc` via
  `add_repo scosman/ironcalc`). **Full process + the per-issue loop:**
  [`specs/projects/ironcalc-upstreaming/implementation_plan.md`](specs/projects/ironcalc-upstreaming/implementation_plan.md)
  §Operating model.
- **Autonomous-run gotchas** (detail in §Operating model → "Agent operating notes"): call
  `add_repo` **upfront while the user is present** — it needs interactive approval and fails
  mid-run once they leave; if it's unavailable, the container's git-proxy already routes
  `scosman/ironcalc`, so clone/push via `http://local_proxy@127.0.0.1:<port>/git/scosman/ironcalc`
  (port from FreeCell's `git remote -v`). For **read-only** work (inventorying the fork, comparing
  against upstream) neither is needed — a plain `git clone --bare --filter=blob:none
  https://github.com/scosman/ironcalc` works through the outbound proxy, and upstream
  `ironcalc/IronCalc` can be added as a second remote and fetched the same way (verified in
  v05-cleanup-1/A4). Prefer that over `add_repo` when you only need to read.
  The agent **can't open upstream `ironcalc/IronCalc`
  PRs** — it prepares a compare link (`.../compare/main...scosman:ironcalc:fix/<slug>`) + title +
  description for the owner to open. Before branching a `fix/*`, check the capability isn't
  **already in** `freecell-fixes` (`git merge-base --is-ancestor <sha> origin/freecell-fixes`).

## Conventions

- **Benchmarks:** run FOREGROUND with `timeout` (never `nohup`/`&`/background monitors);
  **force + assert** the measured op so it can't measure a no-op; report **p50/p99**,
  environment-stamped; **adversarially review** surprising numbers before trusting them.
- **Icons: use lucide.** The app renders **lucide** icons via gpui-component's `Icon`
  (`Icon::empty().path("icons/<name>.svg")`). Prefer an icon already in the gpui-component Lucide
  bundle (it resolves for free); only when the bundle lacks one, vendor that single glyph under
  `app/crates/freecell-app/assets/icons/` in the same tintable `stroke="currentColor"` form and
  register it in `shell/assets.rs` (see that file's `AppAssets` composition). Don't introduce a
  second icon set.
- **Commit + push regularly** — the working container is ephemeral.
- **Build/check efficiency — scope the work; don't full-workspace everything.** Fresh web
  containers are **cache-ready automatically**: a SessionStart hook runs
  `app/scripts/setup_sccache.sh`, wiring sccache to a shared Cloudflare R2 bucket so rustc
  outputs (including the huge pinned dep tree — gpui/zed, gpui-component, ironcalc fork) are
  served from the remote cache instead of recompiled (design:
  `projects/build-cache.md`; without the R2 secrets it no-ops → old cold-build timings apply).
  That takes the worst "cold container rebuilds the world" pain away, but full-workspace runs
  are **still not free** — linking, build scripts, and cargo orchestration aren't cached, first
  compiles of *changed* code always run for real, and the pixel suite is slower still — so
  keep matching the check to the change instead of rebuilding the world each iteration:
  - **Single-crate change → crate-scoped checks:** `cargo build -p <crate>` + `cargo test -p
    <crate> --lib` (add `-p freecell-engine` when the engine is touched) — minutes, not tens of
    minutes. Reserve `--workspace` build/test for genuinely cross-crate changes or a single
    final pre-merge validation, not every iteration.
  - **Always run `cargo fmt --all --check` (whole workspace).** It's cheap (no compile), and a
    crate-scoped `cargo fmt -p <crate>` does **not** format sibling crates — a `render-tests`
    (or other sibling-crate) edit can otherwise slip a fmt violation past a crate-scoped check
    and fail the CI `checks` gate.
  - **Pixel render suite: subset while iterating, full suite once.** A full run is many minutes
    (software lavapipe) and busts the prompt cache. Run only the relevant `render_tests.sh test
    <prefix>` subset per change; defer the **full** suite + CI `render` gate to one late
    validation (see the Render-tests section). Never full-run per coding step.
  - **Code review can be diff-only.** A reviewer that trusts the author's crate-scoped-green
    result and reads the `git diff` (compiling only the one affected crate if it needs to) is
    far cheaper than one that rebuilds the whole workspace to re-verify.
  - Run cargo from **`app/`** (the pinned toolchain activates there). If `render-tests` hits an
    intermittent `ld` bus error under full parallelism, build/test it with `-j 2`. Disk is a
    fixed per-session allowance; `target/` grows large (~25 GB) — deleting stale build dirs
    frees space immediately.

## Render tests — off the PR path, with a weekly backstop

The pixel render suite (Xvfb + Mesa **lavapipe**) is **deliberately not on the PR path** —
most PRs cannot move a pixel and the suite is far too slow to earn its cost on them. The fast
`checks` job compiles render-tests and runs its GPUI-free unit tests, but **diffs no pixels**
(the pixel cases self-skip without `FREECELL_RENDER`).

`.github/workflows/render.yml` runs on **three** triggers (D1, 2026-07-28):

| Trigger | When | What it's for |
|---|---|---|
| `schedule` | **weekly on `main`** | The backstop. Catches what slipped through merges, at a cadence where a regression is still cheap to bisect. |
| `workflow_call` | **at release** | `release.yml`'s packaging jobs `needs:` it — nothing ships without a green suite. |
| `workflow_dispatch` | on demand, any branch | **Yours.** Confirming a rendering change before merge. |

**What this means for you:** dispatching is still your job when you change in-scope pixels, and
the weekly run is a backstop, not a substitute — a regression you merge is found up to a week
later by someone who has to bisect it. What changed is that forgetting no longer means *nobody*
checked. It is **not** a required per-PR status check and never was one in practice: a
dispatch-only workflow reports no context on a PR, so requiring it would just block every merge.

**Scope — what the suite actually covers.** Most render cases are the real `GridView` rendered
over an engine-driven scene: **cell / row / column / sheet rendering** (text, numbers, fonts,
alignment, borders, fills, colors, selection overlay, in-cell editor, loading overlay,
scrollbars, variable geometry) **plus the standalone macOS titlebar row**. On top of that, the
suite also baselines **chart render scenes** (`chart_*` cases) — the real `freecell_app::chart`
widgets rendered **standalone** from chart-model fixtures (no grid, no engine; charts project
P4+). So a change to the **chart render code** (`freecell_app::chart`, from P5 onward) is
**in-scope** and follows the same run-it/validate rules below as a grid/cell/sheet or titlebar
change. Together, the grid + titlebar + chart scenes are the whole baseline inventory. It does
**NOT** cover the **welcome window**, the **About window**, or the rest of the chrome (**action
row, data/formula row, sheet tabs**) — none of those have baselines. A change **confined to
those surfaces cannot move any baseline**, so **do not run the pixel suite for it**; validate it
instead with the crate's gpui view tests + the Xvfb smoke launch (`xvfb-run -a cargo run -p
freecell-app` opens the welcome window). If one of those surfaces ever gains its own baseline,
update this scope note.

**Cost — it's slow; time it strategically.** The suite software-renders every case under
lavapipe: a **full** run takes **many minutes**, blocks your turn, and **busts the prompt
cache**. Do **not** intermingle full runs in every coding phase. Instead:
- **While coding a specific rendering change, run only the relevant cases** — the wrapper
  forwards a `#[test]`-name filter: `app/render-tests/scripts/render_tests.sh test <prefix>`
  (e.g. `… test cell_`, `… test border_`). Fast, keeps you in flow.
- **Defer the full-suite run to a dedicated late phase** (item 3), not per phase.
- **Always set a ~10-minute watchdog** when you kick off a full run: run it foreground under a
  `timeout` and/or with a Monitor check-in so a slow/hung run is caught and you re-check —
  never background-and-forget it (a detached render job dies at the turn boundary and leaves
  you parked, as happened before).

**1. Run it locally when a change could move *grid/cell/sheet, titlebar or chart-render*
pixels** — grid-render code / the `GridView`, fonts, layout, borders, fills, styles, the
titlebar row, the chart widgets (`freecell_app::chart`), the render harness, or baselines
(per the Scope above — not welcome/About/other chrome):
- first time: `app/render-tests/scripts/setup_render_env.sh` (installs the capture stack)
- subset while iterating: `app/render-tests/scripts/render_tests.sh test <prefix>`
- full suite (only at the late validation phase): `app/render-tests/scripts/render_tests.sh
  test` (asserts every case == baseline; wrap in a `timeout` + watchdog)

If the change **intentionally** alters rendering, regenerate + **eyeball** baselines
(`app/render-tests/scripts/render_tests.sh generate`) and commit them *with* the change.
Never land a rendering change without either a green local run or refreshed, eyeballed
baselines.

**2. Validate in CI before merge.** For any **in-scope** change (grid/cell/sheet, titlebar or
chart-render — see Scope) that could regress or alter rendering, get a green CI render run on
the branch before merge. The weekly `main` run does not cover you here: it fires after the
merge, on someone else's watch.
- **Preferred — the agent triggers it:** dispatch the `render` workflow on the branch
  (GitHub Actions MCP `actions_run_trigger` with `workflow_id: render.yml`, or
  `gh workflow run render.yml --ref <branch>`), poll to completion, confirm it passed.
- **Fallback:** if the agent can't dispatch, ask the user to kick off `render` and report
  the result back.
- A run on a **feature branch** uses that branch's `render.yml`, so a change to the workflow
  itself is testable before merge — but only its steps/env: a dispatch exercises neither the
  `schedule` trigger (fires only from the default branch) nor `workflow_call` (only exercised
  by a release), so changes to those two are verifiable only after merge / at the next tag.

**3. Bake it into plans as its OWN late phase.** When a plan makes **in-scope**
(grid/cell/sheet, titlebar or chart-render) rendering changes, put render validation in a
**dedicated phase AFTER all coding + commits are done** — do **not** intermingle full runs per
phase (too slow; breaks flow + cache). The earlier coding phases verify with the relevant
**subset** only (`render_tests.sh test <prefix>`); the final render phase then, once: runs the
**full** suite (with a ~10-min watchdog), refreshes + **eyeballs** baselines if the change is
intentional, commits any baseline updates, and **dispatches the CI `render` gate** and confirms
it passes. Decide this at planning time — don't leave render validation implicit.
(Welcome/About/other-chrome changes are out of scope for the pixel suite — plan gpui view
tests + a smoke launch for those instead.)
