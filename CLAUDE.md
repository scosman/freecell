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

**1. Run it locally, in-container, whenever a change could move *grid/cell/sheet or titlebar*
pixels** — grid-render code / the `GridView`, fonts, layout, borders, fills, styles, the
titlebar row, the render harness, or baselines (per the Scope above — not welcome/About/other
chrome):
- first time: `app/render-tests/scripts/setup_render_env.sh` (installs the capture stack)
- then: `app/render-tests/scripts/render_tests.sh test` (asserts every case == baseline)

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

**3. Bake it into plans.** When an implementation plan makes **in-scope** (grid/cell/sheet or
titlebar) rendering changes that could regress or change pixels, the plan **must** include
explicit steps: refresh + eyeball baselines if the change is intentional, and a **"dispatch
the CI `render` gate and confirm it passes"** step before the phase is done. Decide this at
planning time — don't leave render validation implicit. (Welcome/About/other-chrome changes
are out of scope for the pixel suite — plan gpui view tests + a smoke launch for those
instead.)
