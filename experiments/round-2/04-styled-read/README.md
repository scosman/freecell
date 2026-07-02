# SP4 — Styled viewport read at scale + style-API coverage

Part of FreeCell **Phase 2 (Round-2)**, build-out cohort. Extends the frozen
`round-2/harness` viewport-read benchmark to read **value AND style**
(`get_style_for_cell`) per visible cell at **Excel-max positions**, and probes IronCalc
0.7.1's public **style API** (per-cell / row+column band / empty-cell) with assertions.
See [`findings.md`](findings.md) for the full write-up.

## Reproduce (all foreground, in-container)

```
cargo test                                    # 10 unit tests (read core + style semantics)
cargo run --release --bin probe               # results/style_api_coverage.{json,md}
timeout 400 cargo run --release --bin bench   # results/styled_read.json + summary.md + env.txt
```

## Headline results

- **Styled read GATE (p99 < 2 ms at Excel-max): PARTIAL PASS.** The Phase-1-comparable
  **1,800-cell viewport passes** (value+style p99 ≈ 0.85–1.2 ms), but `get_style_for_cell`
  costs ~700 ns/cell (~10× a value read) and the read is linear, so the budget is crossed
  at **≈ 3,500–4,800 cells** and the **10,000-cell overscan FAILS** (p99 ≈ 4.4–7.2 ms).
  Not an engine pivot — a binding constraint: cap the synchronous styled window or move
  large reads off the render path (findings §5).
- **Style-API coverage: ALL SUPPORTED.** Per-cell, row-band, column-band, and empty-cell
  styling all resolve through IronCalc's public API (cell > row > column > default). The
  **overview §2 formatting decision STANDS** — SP4 forces no style side-store. (The
  pre-existing merges / conditional-formatting gaps are unchanged and carried, not an SP4
  finding.)

## Layout

| Path | What it is |
|------|------------|
| `src/lib.rs` | `read_styled_viewport` (value+style read) + style-resolution helpers + 8 tests. |
| `src/bin/bench.rs` | Styled viewport-read benchmark at Excel-max: two windows + crossover sweep + value-only control. |
| `src/bin/probe.rs` | Assertion-backed style-API coverage probe (per-cell / band / empty-cell / precedence). |
| `results/` | Committed, env-stamped results (`styled_read.json`, `summary.md`, `env.txt`, `style_api_coverage.{json,md}`). |

Independent Cargo project (not a workspace member). Depends **read-only** by relative
path on the frozen `../harness` (IronCalc 0.7.1 adapter + `Viewport`/`Profile`/`pan_path`)
and `../../shared/bench_util`. Builds the raw IronCalc `Model` locally to set styles
(setters need `&mut Model`), then wraps it read-only via `IronCalcEngine::from_model` for
the timed read — the frozen harness is never modified.
