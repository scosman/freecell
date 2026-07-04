# FreeCell

A GPU-rendered (GPUI, à la Zed/Ghostty), Rust, **Excel-compatible spreadsheet** built to
be **stupid-fast on huge sheets** (Excel-max = 1,048,576 rows × 16,384 cols). Engine =
[IronCalc](https://github.com/ironcalc/ironcalc); UI = [GPUI](https://github.com/zed-industries/zed)
(a custom raw-GPUI virtualized grid + [gpui-component](https://github.com/longbridge/gpui-component)
for chrome).

The **MVP app lives in [`app/`](app/)** — a self-contained Cargo workspace that opens /
edits / saves real `.xlsx` files, stays smooth on a million-row sheet, applies basic
formatting, and manages multiple sheets. It is a **workable functional proof of concept**,
not a design-polished or feature-complete product (see `specs/projects/mvp/functional_spec.md`).

**Platforms:** macOS (primary, Metal) + Linux (blade/Vulkan). Windows is out of scope.

## Repository layout

| Path | What |
|---|---|
| [`app/`](app/) | **The FreeCell application** — Cargo workspace (`freecell-core` / `freecell-engine` / `freecell-app` + `render-tests`). See [`app/README.md`](app/README.md) for build/run/test. |
| [`specs/projects/mvp/`](specs/projects/mvp/) | The spec-driven build artifacts for the MVP: overview → functional spec → architecture → UI design → components → implementation plan → phase plans, plus the Phase-13 **coverage matrix** + **smoke checklist** and the `DECISIONS_TO_REVIEW.md` log. Managed via the `spec` skill. |
| [`experiments/`](experiments/) | De-risking experiments (Phase-1 `00`–`06` + `round-2/` + `round-3/`), each an independent Cargo project with `findings.md` + committed `results/`. Frozen; the app **ports** (never path-depends on) POC code. |
| [`PROJECTS.md`](PROJECTS.md) + [`projects/`](projects/) | The "save for later" backlog — optimizations/features/goals off the MVP critical path (e.g. `.xlsx` preservation, bundled Inter, type-aware alignment, pre-distribution security audit). Distinct from `specs/projects/`. |
| [`CLAUDE.md`](CLAUDE.md) | Project conventions (benchmark discipline, commit cadence, the projects backlog rule). |
| [`.github/workflows/`](.github/workflows/) | CI: `checks.yml` (Linux, required), `perf-gates.yml` (Linux, required), `macos-verify.yml` (manual/weekly). |

## Quick start

```sh
cd app
cargo build --workspace          # builds freecell-app on Linux + macOS
cargo run -p freecell-app        # launches FreeCell (Welcome window)
cargo run -p freecell-app -- path/to/Book.xlsx   # opens a workbook directly (CLI argv)
cargo test --workspace           # core + engine + app logic tests
```

Full build prerequisites, the render-test + perf harness usage, and the human baseline
process are in [`app/README.md`](app/README.md) and
[`app/render-tests/README.md`](app/render-tests/README.md). The system-level design is in
[`specs/projects/mvp/architecture.md`](specs/projects/mvp/architecture.md).

## Status

The MVP is built across Phases 1–13 (each a committed, tested phase — see
`specs/projects/mvp/implementation_plan.md`). Phase 13 is the hardening & completion sweep:
a coverage matrix proving every spec behavior is tested or documented-manual, an eyeballed
render-baseline suite, a recorded smoke checklist, and a finalized decisions log. The
`checks` + `perf-gates` CI jobs are the required-green gates.
