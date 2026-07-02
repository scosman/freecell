# SP2 — End-to-end large styled `.xlsx` open (time + peak memory)

Measures how a **fresh process** opens a real **≥100 MB styled `.xlsx`** with IronCalc
0.7.1: open wall-clock, **peak RSS from a separately-spawned child**, a coarse-but-honest
stage breakdown, and **time-to-first-paint** (cached values queryable before full
recompute). Closes functional_spec §5.4's one un-run perf target. See **`findings.md`**
for the verdict and **`results/`** for the committed artifacts.

Independent Cargo project depending read-only by relative path on the frozen `../harness`
(canonical `peak_rss()` + the IronCalc 0.7.1 pin), `../../shared/datagen` (cell/format
CONTENT — the `.xlsx` writer lives HERE, since `datagen` is frozen and has none), and
`../../shared/bench_util` (env-stamped results).

## Reproduce (all foreground, `timeout`-wrapped)

```
# Canonical one-command path: generate a >=100 MB styled .xlsx from committed code, then
# open it 3x from fresh child processes and write results/.
SP2_COMMIT=$(git rev-parse --short HEAD) SP2_DATE=2026-07-01 \
  timeout 400 cargo run --release --bin measure -- 100 3 data/large.xlsx

# Split path (used in the timing-constrained container; identical open numbers):
timeout 200 cargo run --release --bin gen -- 100 data/large.xlsx           # ~100s generate
SP2_REUSE_FILE=1 SP2_COMMIT=$(git rev-parse --short HEAD) \
  timeout 200 cargo run --release --bin measure -- 100 3 data/large.xlsx   # reuse the file

cargo test --release   # 10 unit/integration tests
```

`measure -- [target_mb] [runs] [out_path]`. Peak RSS always comes from the spawned `open`
child (canonical `round2_harness::peak_rss()` VmHWM — never `sysinfo::peak_rss_bytes`).

## Binaries

- `gen` — builds the deterministic styled workbook (IronCalc's native `save_to_xlsx`) and
  grows the row count until the on-disk file crosses the target. Generation time is
  reported **separately** from open time.
- `open` — the fresh child: opens the file once, force+asserts a sentinel, and prints one
  JSON line with per-stage timings + its own peak RSS. Does nothing else, so its VmHWM is
  the honest open-only high-water mark.
- `measure` — the orchestrator: ensures the file, spawns `open` N times, aggregates
  (median/min/max), computes the RSS multiple, applies the judgment GATE + off-ramp flag,
  and writes `results/`.

## Outputs (committed)

- `results/open_summary.json` — gate verdicts, TTFP, RSS multiples, dominant stage, off-ramp.
- `results/open_stage_timings.json` — per-run + median stage timings, env-stamped.
- `results/env.txt` — CPU/OS/cores/commit + file/spec stamp.

The generated `data/large.xlsx` (~105 MB) is **gitignored** — reproduced from committed
code by `gen` (seed 20260701). Never commit the binary.
