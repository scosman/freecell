# SP1 — Non-blocking recompute & the engine↔render interop seam

The Phase-2 crux (functional_spec §6 SP1, architecture §4). IronCalc has **no incremental
recalc**: every edit needs a full-workbook `Model::evaluate()` (O(all cells), ~1.2 s at
10⁶ on this box). This experiment discovers what IronCalc 0.7.1's API permits and **locks
the engine↔render interop-seam design** so recompute never blocks the render loop.

Independent Cargo project; depends **read-only** by relative path on the frozen
`../harness` (IronCalc adapter + `SpreadsheetEngine` trait + `Viewport`) and
`../../shared/*` (datagen, bench_util). `target/` is gitignored repo-wide.

## Layout

| Path | What it is |
|------|------------|
| `src/shapes.rs` | The five DAG-shape builders for the latency matrix (sparse ~1%, wide fan-out 1000×1000, deep-serial `=PREV+1` chain, cross-sheet, volatile `=RAND()`) + force+assert tail metadata. |
| `src/matrix.rs` | `time_evaluate` — times one full `evaluate()`, force+asserting the tail changed (p50/p99). |
| `src/seam.rs` | The **locked** seam: `EvalWorker` (owns the `Send` `Model` on a worker thread; coalesces edits into one eval; publishes the visible viewport after each eval) + the compile-time `assert_model_send` proof. |
| `src/probes.rs` | Runtime API findings: the `UserModel` diff-list is edit-sites-only (no evaluated-cell stream); `to_bytes` snapshot round-trips. |
| `src/bin/latency_matrix.rs` | The `evaluate()` latency-matrix runner (sizes × shapes → `results/latency_*.json`). |
| `src/bin/nonblocking.rs` | The non-blocking render-loop harness + GATES (render-tick budget, coalescing, staleness, snapshot cost → `results/gate_*`, `staleness_*`). |
| `tests/seam.rs` | Integration tests for the seam + findings at fast sizes. |
| `results/` | Committed, env-stamped JSON. |
| `findings.md` | The write-up + the **locked interop-seam design** (the key output). |

## Run (foreground, with `timeout`)

```sh
cargo test
cargo run --release --bin latency_matrix -- --max-size 1000000        # 10^4..10^6, all shapes
cargo run --release --bin latency_matrix -- --shape volatile --size 10000000   # heavy: run alone
cargo run --release --bin nonblocking -- --shape deep_serial --size 1000000    # GATEs @10^6
cargo run --release --bin nonblocking -- --shape volatile    --size 10000000   # GATEs @10^7
```

Heavy scales (10⁶/10⁷) run **one at a time**; `deep_serial` 10⁷ is capped (Excel's
1,048,576-row limit + 10M-deep recursion) and records its ceiling. See `findings.md`.
