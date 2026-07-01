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

## Conventions

- **Benchmarks:** run FOREGROUND with `timeout` (never `nohup`/`&`/background monitors);
  **force + assert** the measured op so it can't measure a no-op; report **p50/p99**,
  environment-stamped; **adversarially review** surprising numbers before trusting them.
- **Commit + push regularly** — the working container is ephemeral.
