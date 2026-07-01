---
status: draft
---

# Phase SP1: Non-blocking recompute & the engine↔render interop seam

## Overview

SP1 is the Phase-2 crux (functional_spec §6 SP1, architecture §4). IronCalc has **no
incremental recalc**: every edit needs a full-workbook `Model::evaluate()` that clears
all computed cells and re-evaluates the whole graph (O(all cells), ~2 s at 10⁶–10⁷).
Run on the render path, every edit on a big sheet freezes the app.

The real deliverable is a **locked engine↔render interop-seam design** where recompute
is **non-blocking** — the "render loop" (a headless driver ticking at frame cadence,
NO GPUI) never stalls a frame, and IronCalc stays the authoritative model. The exact
concurrency mechanism is an **output** discovered from IronCalc's real 0.7.1 API, not
pre-chosen.

This phase produces three things:
1. **API investigation** (written into `findings.md`), backed by compile-time and
   runtime probes: `evaluate()` lifecycle; `Model` `Send`/`Sync`; read-during-eval
   safety; reentrancy/serialization; the changed-cells question (does the `UserModel`
   diff-list carry cascaded cells, or only edit-sites?); snapshot cost (`to_bytes`).
2. **evaluate() latency matrix**: sizes {10⁴,10⁵,10⁶,10⁷} × DAG shapes {sparse ~1%,
   wide fan-out 1000×1000, deep-serial 1M `=PREV+1` chain, cross-sheet, volatile},
   p50/p99, env-stamped, force+assert the tail cell actually changed.
3. **Minimal non-blocking harness**: a driver render loop that stays < one frame while
   a 10⁶–10⁷ eval runs on a worker thread; N rapid edits coalesce to ≤ a small bounded
   number of `evaluate()` runs; measures per-tick work + staleness window.

## API findings gathered so far (from the IronCalc 0.7.1 source; confirmed in code)

These drive the design; the phase code turns them into runnable, asserted probes.

- **`Model::evaluate(&mut self)`** (`model.rs:1886`) clears `self.cells` then loops
  `get_all_cells()` evaluating each — **full workbook, O(all cells), not incremental,
  not chunked, not interruptible, no callbacks/channels/progress/cancel** (grep of
  `model.rs` for callback/Sender/channel/interrupt/cancel/Arc/Mutex = none). Takes
  `&mut self`, so **it is inherently one-at-a-time (serialized) and cannot overlap a
  read of the same model** (Rust aliasing: `&mut` excludes `&`).
- **`Model` is not `Clone`.** The only snapshot route is `Model::to_bytes()`
  (`model.rs:2016` = `bitcode::encode(&self.workbook)`) → `from_bytes()` — a full
  serialize of the whole workbook. Cost measured in the phase (the "clone cost" §4.2).
- **`Send`/`Sync`:** `Model`'s fields are `Workbook` (plain `derive(Clone)` data),
  `Vec`, `HashMap`, `&'a Locale`, `&'a Language`, `Tz` (chrono_tz `Copy` enum),
  `Parser` (owned data + `&'a` refs). No `Rc`/`RefCell`/`Cell`/raw pointers in the
  library path (only `mock_time.rs`, a `thread_local!`, which is not a field). So
  `Model<'static>` is **expected `Send`** — the phase **asserts this at compile time**
  (`fn assert_send<T: Send>()`), which is the authoritative answer.
- **Change awareness — the key unknown, resolved:** IronCalc's diff-list lives on
  `UserModel` (`send_queue: Vec<QueueDiffs>`, `flush_send_queue()` →
  bitcode `Vec<u8>`). Each `Diff::SetCellValue` (`user_model/history.rs:20`) carries
  only the **edited** `(sheet,row,column)` + old/new — **NOT the cascaded/evaluated
  downstream cells**. The `Diff` enum is `pub(crate)`; only the opaque encoded blob is
  public. **Conclusion: IronCalc exposes NO changed-cells stream and NO post-eval
  evaluated-cell diff.** It tells you *where the user typed*, never *what recompute
  changed*. This forces the fallback (§4.2): re-pull the visible cells. The phase
  proves this with a probe that edits one head cell of a chain, flushes the send
  queue, and asserts the queue does not enumerate the cascaded cells.
- **Volatile:** `RAND`/`RANDBETWEEN` (`rand::random()`) and `NOW` are registered
  (`functions/mod.rs`); `RAND` gives a genuinely volatile shape. `NOW`/`TODAY` read a
  `thread_local!` mock clock frozen at 2022-11 unless `set_mock_time` is called, so the
  volatile scenario uses `RAND` (values actually change across evals — assertable).
- **Cross-sheet:** `Model::new_sheet() -> (String, u32)` adds a sheet; a formula on
  sheet 0 can reference `Sheet2!A1` for the cross-sheet shape.

**Design implication (the locked seam, justified above):** eval must run on a **worker
thread** owning the `Model` (it's `Send`, `&mut`, non-reentrant); the render loop must
**never** touch that model while an eval is in flight (no read-during-eval). Because
there is no evaluated-cell diff, the renderer learns "what changed" by **re-pulling the
visible viewport** from a fresh readable model **after** each eval completes
(publish-on-completion), backed by the SP4-class per-cell read. Rapid edits **coalesce**
into a single pending eval (debounce/latest-wins). This is the "snapshot/publish"
branch of architecture §4.2 with the "wait-then-repull" change-propagation fallback —
locked because IronCalc's API leaves no faster-fidelity option.

## Steps

### 1. Scaffold the independent Cargo project `experiments/round-2/01-async-interop/`

- `Cargo.toml` — package `sp1_async_interop`, `publish = false`, edition 2021.
  - `[dependencies]`: `round2_harness = { path = "../harness" }`,
    `datagen = { path = "../../shared/datagen" }`,
    `bench_util = { path = "../../shared/bench_util" }`,
    `ironcalc_base = "0.7"` (for direct `Model` API probes not on the trait),
    `serde_json = "1"`, `anyhow = "1"`.
  - `[[bin]]` `latency_matrix` (the evaluate() matrix runner),
    `[[bin]]` `nonblocking` (the non-blocking harness / gate runner).
  - No `criterion` needed — SP1's numbers are end-to-end wall-clock (bench_util timers),
    matching the foreground-timeout discipline; the matrix and gates are custom drivers.
- `README.md` — one-paragraph index (what SP1 is, how to run each binary, that it
  depends read-only on `../harness` + `../../shared/*`).
- Rely on repo-root `.gitignore` (`target/`) — do not add a local one.

### 2. `src/lib.rs` — shared building blocks (library, unit-tested)

- `pub mod shapes` — DAG-shape builders returning `Vec<(u32,u32,CellInput)>` (or a
  sheet-populating closure) + the address of a **tail cell** whose value must change,
  for each shape at a given target populated-cell count `n`:
  - `sparse(n)` — ~1% formula density over a square-ish region: mostly literals, ~1%
    cells `=<neighbor>+1`. Tail = last formula cell.
  - `wide_fanout()` — reuse `datagen::wide_fanout(1000, 1000)` (1000 sources, 1000
    dependents summing them). Tail = last dependent.
  - `deep_serial(n)` — reuse `datagen::linear_chain(n, col)` (the `=PREV+1` chain).
    Tail = last cell (== n at value level). The 1M variant is the known ~2 s FAIL.
  - `cross_sheet(n)` — literals on sheet 2, formulas on sheet 0 referencing them.
  - `volatile(n)` — n cells `=RAND()`; tail changes value every eval (assertable).
- `pub mod matrix` — `EvalCase { size, shape, tail, expected_change }`, a
  `build_model(case) -> Model<'static>` that populates inputs **without** evaluating
  (deferred, like the adapter), and `time_evaluate(&mut Model, samples) -> LatencyStats`
  that (a) records the tail value, (b) times a single `evaluate()`, (c) re-reads the
  tail and **asserts it changed / matches expected** (force+assert), repeating for
  p50/p99. Re-seeding the head between samples where needed so each eval does real work.
- `pub mod seam` — the locked interop seam, engine-agnostic over the worker model:
  - `EvalWorker` — owns a `Model<'static>` on a spawned thread; receives `EditBatch`
    commands over a channel; **coalesces** all queued edits then runs **one**
    `evaluate()`; on completion publishes a fresh readable snapshot (see below) to a
    shared slot the render loop reads. Exposes an `eval_count` counter (for the
    coalescing gate) and the current publish generation.
  - Publish mechanism: after eval, the worker serves visible-cell reads. Two candidate
    routes are both implemented and the cheaper-latency one is locked in findings:
    (a) **worker-side read**: render loop sends a viewport request; worker (idle
    between evals) replies with the values — no clone; or
    (b) **snapshot publish**: worker `to_bytes()` → publishes an
    `Arc<Vec<u8>>`/rebuilt read-only `Model` the render loop reads directly.
    The phase measures both; (a) avoids the `to_bytes` cost and is the expected lock,
    with (b)'s cost recorded as the "snapshot/clone cost" discovery.
  - `RenderLoop` — a driver that ticks at frame cadence; each tick does only
    **cheap** work: read the latest published visible viewport (or last-known if an
    eval is mid-flight), advance a synthetic scroll, enqueue any scripted edits. Its
    per-tick synchronous cost is what the GATE measures — it must never call
    `evaluate()` and never block on the worker.
- Static assertions module: `assert_send::<Model<'static>>()` (compile-time Send proof),
  plus a runtime probe function returning the diff-list finding (edits head, flushes
  `UserModel` send queue, inspects that it does not carry cascaded cells).

### 3. `src/bin/latency_matrix.rs` — the evaluate() latency matrix

- For each (size ∈ {10⁴,10⁵,10⁶,10⁷}) × (shape ∈ the five), **foreground**:
  - Build the model (build time separated from the measured op — not timed into eval).
  - Warm one eval, then time N evals (N scaled down for the heavy 10⁶/10⁷ so total
    runtime stays bounded), force+assert the tail changed each time.
  - Record a `BenchResult` (p50/p99, `input_size`, env-stamped via `Environment::detect`
    + `cpu_model()`, date `2026-07-01`, `extra` = {shape, populated_cells, tail_addr,
    tail_before, tail_after}) to `results/latency_<shape>_<size>.json`.
- **Resource discipline:** 10⁷ runs **one at a time** (the binary takes a
  `--size`/`--shape` filter or runs sequentially with drops between cases so only one
  big model is resident). Deep-serial 10⁷ (10M chain) may exceed memory/time on the
  shared 4c/15 GB box → if a case is too heavy, **cap it and record the ceiling** in
  `results/` + findings, never push into swap/OOM. Deep-serial 10⁶ (~2 s) is the
  expected known-FAIL vs the <100 ms target — recorded, not gated.
- Writes a `results/latency_summary.json` aggregating the matrix for findings.

### 4. `src/bin/nonblocking.rs` — the non-blocking harness + GATES

- Build a 10⁶ (and, resources permitting, 10⁷) wide-fanout/sparse model on an
  `EvalWorker`.
- Run the `RenderLoop` for a fixed number of frames (e.g. 600 frames ≈ 10 s at 60 fps
  simulated) while:
  - a scripted **edit burst** fires (e.g. 20 rapid edits within ~2 frames) that must
    trigger recompute, and
  - the loop keeps ticking, reading the latest visible viewport.
- **GATE 1 (render-loop non-blocking):** record every tick's synchronous work; assert
  p99 (and max) tick cost **< 8.3 ms** (hard-fail > 16.6 ms) **even while a 10⁶–10⁷
  eval is in flight**. Emit `GateResult` + `results/gate_render_loop.json`.
- **GATE 2 (coalescing):** after the burst of N rapid edits, assert the worker ran
  **≤ a small bounded number** of `evaluate()` calls (target ≤ 2: at most one in-flight
  + one coalesced). Emit `GateResult` + `results/gate_coalesce.json`.
- **DISCOVERY — staleness window:** measure edit-timestamp → first tick that reads the
  post-eval fresh value for an edited/visible cell; record p50/p99 to
  `results/staleness.json`. Also record snapshot (`to_bytes`) cost for the 10⁶/10⁷
  model as the clone-cost discovery.
- All timing foreground; process exits non-zero if a hard GATE fails (so the run is
  self-checking), but the ~2 s deep-serial recompute is a DISCOVERY not a gate.

### 5. `findings.md` (functional_spec §5.2 headings) + committed `results/`

Write `experiments/round-2/01-async-interop/findings.md` with the standard headings
(Question / Method / Results / Analysis / Threats to validity / Conclusion) and, as the
**key output**, the **locked engine↔render interop-seam design**:
- how recompute stays non-blocking (worker thread owns the `Send` `&mut` model; render
  loop never touches it mid-eval),
- how the renderer learns what changed (no IronCalc evaluated-cell diff exists →
  **locked fallback**: re-pull the visible viewport post-eval; worker-side read vs
  `to_bytes` snapshot, with the measured costs justifying the choice),
- how the two halves stay decoupled (channel of edit-batches in, published viewport out;
  coalescing; last-known values + "recalculating…" during the staleness window),
- the full evaluate() latency matrix (p50/p99), the serialization requirement, the
  no-live-stream finding, and the snapshot cost.
Commit the `results/*.json`. Env-stamp everything.

## Tests

Unit/integration tests (fast, deterministic — the heavy matrix is the binaries, run by
hand foreground; tests use small sizes):

- **`shapes_*`**: each shape builder produces the expected populated-cell count and a
  tail address; a small model built from each shape evaluates to the expected tail value
  (sparse tail increments, chain tail == len, fanout tail == source sum, cross-sheet
  tail == referenced value, volatile tail is a number in [0,1)).
- **`model_is_send`**: compiles `assert_send::<Model<'static>>()` — the authoritative
  Send proof. (Compile-time; presence of the test = the proof.)
- **`no_evaluated_cell_diff`**: build a small chain on a `UserModel`, set the head,
  `flush_send_queue()`, and assert the flushed diff count corresponds to edit-sites
  only (does not scale with the cascade length) — the "no changed-cells stream" finding,
  asserted in code.
- **`snapshot_roundtrips`**: `to_bytes()` → `from_bytes()` reproduces cell values —
  proving the snapshot publish route is correct (cost measured separately in the bin).
- **`worker_coalesces_rapid_edits`**: drive `EvalWorker` with N rapid edits in a tight
  loop; assert `eval_count` ≤ small bound after draining (the coalescing GATE logic,
  unit-level, small model so it's fast).
- **`render_tick_is_cheap_during_eval`**: with a mid-size model evaluating on the
  worker, assert the render loop's per-tick synchronous cost stays well under one frame
  (the GATE logic exercised at a test-friendly size; the headline number comes from the
  10⁶–10⁷ binary run).
- **`matrix_forces_and_asserts_change`**: `time_evaluate` on a tiny case returns stats
  and the tail value genuinely changed (force+assert plumbing works).
- **`staleness_is_measured`**: a short scripted run yields a finite, positive staleness
  window (measurement plumbing works).
