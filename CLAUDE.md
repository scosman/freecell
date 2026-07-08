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

- FreeCell's `app/Cargo.toml` pins `ironcalc`/`ironcalc_base` via `[patch.crates-io]` → the fork's
  **`freecell-fixes`** branch (the sum of our not-yet-upstreamed fixes).
- Fork branches: `main` = clean mirror of upstream; `fix/<slug>` = one branch per fix (off `main`,
  with upstream-style tests) = one clean PR; `freecell-fixes` = integration branch FreeCell builds
  against. Sync `main` from upstream periodically (rebase `fix/*` + `freecell-fixes`); expect
  incidental drift to reconcile on the FreeCell side.
- An agent can work both repos in one container (FreeCell here; fork at `/workspace/ironcalc` via
  `add_repo scosman/ironcalc`). **Full process + the per-issue loop:**
  [`specs/projects/ironcalc-upstreaming/implementation_plan.md`](specs/projects/ironcalc-upstreaming/implementation_plan.md)
  §Operating model.

## Conventions

- **Benchmarks:** run FOREGROUND with `timeout` (never `nohup`/`&`/background monitors);
  **force + assert** the measured op so it can't measure a no-op; report **p50/p99**,
  environment-stamped; **adversarially review** surprising numbers before trusting them.
- **Commit + push regularly** — the working container is ephemeral.

## Render tests — agent-driven (no automatic every-push gate)

The pixel render suite (Xvfb + Mesa **lavapipe**) is a **manual** gate: it runs only when
the `render` workflow (`.github/workflows/render.yml`, required-check context
`render (Xvfb + lavapipe)`) is dispatched — **not** on every push. The fast `checks` job
compiles render-tests and runs its GPUI-free unit tests, but the actual **pixel diffs are
not covered automatically**. So the **agent must decide when render coverage is needed and
drive it** — there is no safety net.

**Scope — what the suite actually covers.** Every render case is the real `GridView` rendered
over an engine-driven scene: **cell / row / column / sheet rendering** (text, numbers, fonts,
alignment, borders, fills, colors, selection overlay, in-cell editor, loading overlay,
scrollbars, variable geometry) **plus the standalone macOS titlebar row**. That is the whole
baseline inventory. It does **NOT** cover the **welcome window**, the **About window**, or the
rest of the chrome (**action row, data/formula row, sheet tabs**) — none of those have
baselines. A change **confined to those surfaces cannot move any baseline**, so **do not run
the pixel suite for it**; validate it instead with the crate's gpui view tests + the Xvfb
smoke launch (`xvfb-run -a cargo run -p freecell-app` opens the welcome window). If one of
those surfaces ever gains its own baseline, update this scope note.

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

**1. Run it locally when a change could move *grid/cell/sheet or titlebar* pixels** —
grid-render code / the `GridView`, fonts, layout, borders, fills, styles, the titlebar row,
the render harness, or baselines (per the Scope above — not welcome/About/other chrome):
- first time: `app/render-tests/scripts/setup_render_env.sh` (installs the capture stack)
- subset while iterating: `app/render-tests/scripts/render_tests.sh test <prefix>`
- full suite (only at the late validation phase): `app/render-tests/scripts/render_tests.sh
  test` (asserts every case == baseline; wrap in a `timeout` + watchdog)

If the change **intentionally** alters rendering, regenerate + **eyeball** baselines
(`app/render-tests/scripts/render_tests.sh generate`) and commit them *with* the change.
Never land a rendering change without either a green local run or refreshed, eyeballed
baselines.

**2. Validate in CI before merge.** The required truth is the CI `render` gate. For any
**in-scope** change (grid/cell/sheet or titlebar — see Scope) that could regress or alter
rendering, get a green CI render run on the branch before merge:
- **Preferred — the agent triggers it:** dispatch the `render` workflow on the branch
  (GitHub Actions MCP, or `gh workflow run render.yml --ref <branch>`), poll to completion,
  confirm it passed. (Dispatchable once `render.yml` is on `main`.)
- **Fallback:** if the agent can't dispatch, ask the user to kick off `render` and report
  the result back.

**3. Bake it into plans as its OWN late phase.** When a plan makes **in-scope**
(grid/cell/sheet or titlebar) rendering changes, put render validation in a **dedicated phase
AFTER all coding + commits are done** — do **not** intermingle full runs per phase (too slow;
breaks flow + cache). The earlier coding phases verify with the relevant **subset** only
(`render_tests.sh test <prefix>`); the final render phase then, once: runs the **full** suite
(with a ~10-min watchdog), refreshes + **eyeballs** baselines if the change is intentional,
commits any baseline updates, and **dispatches the CI `render` gate** and confirms it passes.
Decide this at planning time — don't leave render validation implicit. (Welcome/About/other-
chrome changes are out of scope for the pixel suite — plan gpui view tests + a smoke launch
for those instead.)
