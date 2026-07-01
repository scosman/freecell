# SP1 — Non-blocking recompute & the engine↔render interop seam

> Phase-2 crux (functional_spec §6 SP1, architecture §4). The real deliverable is a
> **locked engine↔render interop-seam design**, discovered from IronCalc 0.7.1's actual
> API, where recompute never blocks the render loop. This document answers the SP1
> questions, records the `evaluate()` latency matrix, and states the locked seam.

## Questions

1. **Non-blocking:** how do we run `evaluate()` so the render loop never stalls a frame,
   even during a multi-second recompute? Can the model be read while an eval runs, moved
   to run elsewhere, or only snapshot/cloned (at what cost)?
2. **Eval lifecycle / serialization:** is `evaluate()` reentrant, or must evals be
   serialized (one-at-a-time)? Any start/progress/completion signals? Chunkable?
3. **Change awareness:** does IronCalc expose a stream / pub-sub / diff of the cells
   *changed by an eval* — live (best), post-eval (acceptable), or nothing (fallback)?

## What was done

Three deliverables, all in this independent Cargo project (`sp1_async_interop`), depending
**read-only** on the frozen `../harness` (IronCalc adapter, `Viewport`) and
`../../shared/*` (datagen, bench_util):

- **API investigation** — by reading the IronCalc 0.7.1 source (`ironcalc_base-0.7.1`) and
  turning each answer into an **asserted probe**: a compile-time `Send` proof
  (`seam::assert_model_send`), a runtime diff-list probe (`probes::diff_list_is_edit_sites_only`),
  and a snapshot round-trip (`probes::snapshot_roundtrip`).
- **`evaluate()` latency matrix** — `src/bin/latency_matrix.rs`: sizes
  {10⁴,10⁵,10⁶,10⁷} × shapes {sparse ~1%, wide fan-out 1000×1000, deep-serial `=PREV+1`
  chain, cross-sheet, volatile `=RAND()`}. Build time is separated from the measured op;
  each timed sample **re-arms** a seed and **force+asserts the tail changed** (never the
  latency of a no-op re-eval). p50/p99, env-stamped, one JSON per cell + a summary.
- **Non-blocking harness + GATES** — `src/bin/nonblocking.rs`: a headless render loop
  (driver ticking at 60 fps; **NO GPUI**) driving an `EvalWorker` that owns the model on
  a worker thread. It fires a burst of rapid edits mid-run so an eval is in flight, and
  measures the render tick's synchronous work, the coalesced eval count, and the staleness
  window. **GATE 1 uses two controls**: a *positive* run (the seam — eval on the worker)
  that must pass, and a *negative control* (`run_negative_control`) that runs the same tick
  loop with `evaluate()` **inline on the render thread** and must **fail** the frame budget
  — so the gate discriminates a blocking design from the seam. The seam itself lives in
  `src/seam.rs`.

### Reproduce

```sh
# from experiments/round-2/01-async-interop/  (foreground, with timeout)
cargo test                                              # findings + seam logic (fast)
cargo run --release --bin latency_matrix -- --max-size 1000000   # 10^4..10^6 all shapes
cargo run --release --bin latency_matrix -- --shape sparse      --size 10000000
cargo run --release --bin latency_matrix -- --shape cross_sheet --size 10000000
cargo run --release --bin latency_matrix -- --shape volatile    --size 10000000   # run alone
cargo run --release --bin latency_matrix -- --shape deep_serial --size 10000000   # records the cap
cargo run --release --bin nonblocking -- --shape deep_serial --size 1000000       # GATEs @10^6
cargo run --release --bin nonblocking -- --shape volatile    --size 10000000      # GATEs @10^7
# → results/*.json
```

**Resource discipline:** heavy scales (10⁶/10⁷) run **one at a time, foreground**; each
matrix cell builds and drops its own model so only one big model is resident. `deep_serial`
10⁷ is **capped** (below) and records its ceiling, not run.

**Environment (stamped on every result):** Intel(R) Xeon(R) @ 2.80 GHz, 4 cores, x86_64,
linux (the shared 4c/15 GB container — a floor; real hardware is faster). Date 2026-07-01.

## Results / evidence

### 1. API investigation (the three questions, answered from the real API + asserted)

**Q2 — eval lifecycle / serialization.** `Model::evaluate(&mut self)` (`model.rs:1886`)
**clears all computed cells** (`self.cells.clear()`) then loops every populated cell and
evaluates it top-down. It is:
- **full-workbook, O(all cells), non-incremental** — no dirty tracking; every edit needs a
  whole re-eval (the central IronCalc contrast, confirmed);
- **non-interruptible, non-chunkable, no lifecycle signals** — a grep of `model.rs` for
  `callback`/`Sender`/`channel`/`progress`/`interrupt`/`cancel`/`Arc`/`Mutex` finds
  **none**. There is no "step" or "eval N cells then yield" API;
- **inherently serialized** — it takes `&mut self`, so two evals cannot overlap and an
  eval **cannot overlap a read of the same model** (Rust aliasing: `&mut` excludes `&`).

→ **We must serialize to one eval at a time**, and reads and evals of the *same* model
cannot interleave. There is nothing to make reentrant.

**Q1 — where can eval run without blocking?** `Model<'static>` is **`Send`** — proven at
**compile time** by `seam::assert_model_send` (`fn assert_send<T: Send>()` instantiated on
`Model<'static>`; test `model_is_send`). Its fields are all `Send` (Workbook/Vec/HashMap
plain data, `&'static Locale`/`&'static Language`, `chrono_tz::Tz` Copy enum, an owned
`Parser`); the only interior-mutability in the crate is a **test-only** `thread_local!` in
`mock_time.rs`, not a `Model` field. So the vehicle for non-blocking is: **move the model
onto a worker thread and evaluate there**; the render loop touches nothing eval touches.
`Model` is **not `Clone`**; the only snapshot route is `to_bytes()` (`bitcode::encode`
of the whole workbook) → `from_bytes()` (cost measured below).

**Q3 — change awareness (the key unknown): IronCalc exposes NO evaluated-cell change
stream.** IronCalc's only diff surface is the `UserModel` **send-queue** (`flush_send_queue()`
→ a `bitcode`-encoded `Vec<u8>`). Its `Diff` enum (`user_model/history.rs:20`) is
`pub(crate)`, and `Diff::SetCellValue` records only the **edited cell** `(sheet,row,column)`
+ old/new — **never the cascaded downstream cells** the eval recomputed. Probe
`diff_list_is_edit_sites_only` proves this empirically (test `no_evaluated_cell_diff`):

| chain length | cascaded cells (values that changed) | one-edit diff size |
|---|---|---|
| 10 | 9 | **28 bytes** |
| 1000 | 999 | **28 bytes** |

The cascade grows 111×; the single-edit diff **stays 28 bytes**. IronCalc tells you *where
the user typed*, never *what recompute changed*. There is **no live stream and no post-eval
evaluated-cell diff** — the renderer must re-pull the visible cells itself.

**Snapshot fidelity + cost.** `to_bytes()`→`from_bytes()` reproduces evaluated values
(probe `snapshot_roundtrip`, test `snapshot_roundtrips`). Cost on the big models (from the
harness run): **13.0 MB in ~200 ms** at 10⁶ cells; **96.8 MB in ~0.9 s** at 10⁷ cells. So
a per-publish full snapshot is affordable but far pricier than a viewport read — it stays
**off the render loop** either way (it only widens the staleness window, not the frame
budget).

### 2. `evaluate()` latency matrix (p50 / p99, force+asserted tail change)

Full workbook recompute, one `evaluate()` call. `results/latency_summary.json` +
per-cell JSON. (`wide_fanout` is fixed at 1000×1000 = 2000 cells by construction, so its
`size` column is the requested label, not the populated count; its cost is dominated by
1000 dependents each SUM-ing a 1000-cell range = ~10⁶ range reads/eval.)

| shape | 10⁴ (p50 / p99) | 10⁵ | 10⁶ | 10⁷ |
|---|---|---|---|---|
| **sparse** (~1% formulas) | 609 µs / 765 µs | 7.10 ms / 7.61 ms | 155.6 ms / 159.4 ms | 1.59 s / 1.60 s |
| **wide_fanout** (1000×1000, 2000 cells) | 63.8 ms / 91.1 ms | 64.9 ms / 65.3 ms | 64.5 ms / 64.7 ms | 66.1 ms / 66.4 ms |
| **deep_serial** (`=PREV+1` chain) | 4.50 ms / 4.95 ms | 80.6 ms / 93.2 ms | **1.20 s / 1.24 s** | **capped** |
| **cross_sheet** | 3.01 ms / 3.25 ms | 48.6 ms / 61.3 ms | 1.03 s / 1.12 s | 6.97 s / 7.05 s |
| **volatile** (`=RAND()`) | 3.44 ms / 3.69 ms | 60.6 ms / 72.0 ms | 1.10 s / 1.11 s | 7.30 s / 7.32 s |

Readings:
- **The 1M `=PREV+1` chain recompute is ~1.2 s** — the **expected known-FAIL** vs the
  <100 ms target (functional_spec §5.4; the spec anticipated ~2 s, this box is ~1.2 s).
  This is recorded, not gated — SP1's point is the non-blocking UX, not the raw number.
- Cost scales with **populated cells**, not just formula count (`evaluate()` clears and
  re-walks *every* cell). Sparse (mostly literals) is ~7–8× cheaper than an all-formula
  shape at the same populated count, but still ~1.6 s at 10⁷.
- Even the cheapest shape at 10⁷ (sparse, 1.6 s) and every all-formula shape at 10⁶ (~1 s)
  is **multiple frames** long — recompute categorically **cannot** run on the render path.

### Ceiling that had to be capped (honest step-down)

- **deep_serial 10⁷ — capped, not run.** A 10-million-deep `=PREV+1` chain is (a) a single
  column of 10⁷ cells, which **exceeds Excel's 1,048,576-row limit that IronCalc enforces**
  (`set_user_input` returns `"Incorrect row or column"`), and (b) a 10M-deep evaluation
  recursion risking stack overflow / OOM / multi-minute runs on the shared 4c/15 GB box.
  Recorded as `status:"capped"` in `results/latency_deep_serial_10000000.json`. The 10⁶
  chain (~1.2 s) is the recorded chain ceiling; the spec's deep-serial data point is met
  at 10⁶.
- **Single-column shapes wrap into columns at the Excel row limit** (volatile, cross_sheet)
  so they reach a true 10⁷ populated cells without exceeding 1,048,576 rows — the row limit
  is itself an SP1 finding (a real IronCalc/Excel constraint the app must respect).

### 3. Non-blocking harness — GATES (results/gate_*, staleness_*)

The render loop drives an `EvalWorker` (model on a worker thread) and fires 30 rapid edits
mid-run so a full eval is in flight. Two runs: deep_serial @10⁶ (~1.2 s eval) and volatile
@10⁷ (~7 s eval).

**GATE 1 is discriminating — it uses two controls on the *same* tick-measurement loop.**
The point is *not* "the slot-read is cheap" (it always is, ~1 µs, blocking or not); the
point is that non-blocking is a property of **ownership discipline** — the worker owns
`&mut Model`, and the render side holds nothing `evaluate()` touches. The two controls make
that testable:

| metric | deep_serial 10⁶ | volatile 10⁷ | GATE |
|---|---|---|---|
| **POSITIVE (seam):** render tick p99 **while eval in flight** | **2.51 µs** | **3.93 µs** | **< 8.3 ms ✓ (hard-fail > 16.6 ms)** |
| — during-eval ticks actually sampled | 76 | 361 | (proves the read overlapped a real in-flight eval) |
| render tick max (all 600 ticks) | 82 µs | 36 µs | — |
| **NEGATIVE (inline-eval blocking design):** during-eval tick p99 | **1.24 s** | **1.12 s** | **MUST fail; > 16.6 ms ✓** |
| 30 rapid edits ⇒ full `evaluate()` runs | **1** | **1** | **≤ 2 ✓ (coalesce)** |
| staleness window (edit → visible fresh) | 1.29 s | 6.11 s | DISCOVERY (≈ one eval) |
| snapshot `to_bytes` cost | 13.0 MB / 193 ms | 96.8 MB / 2.04 s | DISCOVERY (clone cost) |

- **GATE 1 (render non-blocking): PASS — and discriminating.**
  - *Positive control (the seam):* with eval on the worker thread, the render tick only
    reads the small published viewport (O(viewport), a short-held lock) and never calls
    `evaluate()`. Its during-eval p99 is ~2.5–3.9 µs — well inside the 8.3 ms budget — and
    the fallback that would use all-ticks p99 when no during-eval tick is sampled **did not
    fire**: 76 (10⁶) / 361 (10⁷) ticks were genuinely sampled *while the eval was in
    flight*, so the number is measured under the exact condition the gate tests.
  - *Negative control (the deliberately-blocking design):* the **same** tick loop but with
    `evaluate()` invoked **inline on the render thread** (a 10⁶-cell model) pays the full
    ~1.1–1.2 s recompute per tick — its during-eval p99 **blows past the 16.6 ms hard-fail
    by ~70×**. The harness asserts this control *fails*; if it didn't, GATE 1 would be
    meaningless. So the gate confirms a blocking design is **detected and rejected**, and
    the pass is a property of the worker-thread ownership discipline, not of the cheap
    slot-read alone.
- **GATE 2 (coalescing): PASS.** The worker drains the *whole* command channel before it
  runs a single `evaluate()`, so 30 rapid edits collapse to **1** eval in the measured
  runs. The **≤2 bound holds by construction, not by a mid-eval-injection test**: while an
  eval is running, further edits queue on the channel; the worker cannot start a second
  eval until the current one returns, and it then drains everything queued into one
  coalesced batch. So at any moment there is **at most one in-flight eval + one pending
  coalesced batch** — ≤2 — regardless of edit timing. Both the binary and the
  `rapid_edits_coalesce_to_few_evals` unit test (`seam.rs`) exercise the **bound-of-1**
  path (edits arrive while the worker is parked between evals); neither forces edits to
  land *mid-eval*, because the ≤2 ceiling is a structural property of the drain-then-eval
  loop rather than something a race-timed test would need to catch.
- **Staleness = one eval duration** (~1.3 s @10⁶, ~6 s @10⁷): the time from an edit to the
  edited/visible cells showing fresh values, because the only fidelity route is re-pull on
  eval completion. This is a discovery number, not a frame gate; during it the UI shows
  last-known cached values + a "recalculating…" flag (both provided by the seam). It is
  slightly fuzzy at burst boundaries (a burst coalesces into one eval and only the last
  edit's timestamp is tracked), which is immaterial at its ~1 s scale.

## Conclusion (direct answers)

- **Non-blocking is achievable with a clean seam, and the guarantee is real (not a
  tautology).** Non-blocking is a property of the **ownership discipline**: the worker owns
  `&mut Model`; the render side holds nothing `evaluate()` touches. GATE 1 proves this
  *discriminatingly* — the seam's during-eval tick p99 (~2.5–3.9 µs, genuinely sampled
  while the eval ran) passes, and the **negative control** (inline `evaluate()` on the
  render thread) **fails** the hard-fail budget by ~70× (~1.1–1.2 s/tick). So the pass is
  attributable to the design, not to a cheap slot-read. **GATE 1 (positive PASS + negative
  FAIL) and GATE 2 both hold at 10⁶ and 10⁷.**
- **Evals must be serialized** (one at a time; `&mut self`, non-reentrant, non-interruptible,
  no lifecycle signals) and **reads cannot overlap an eval of the same model**.
- **There is no live change-stream and no post-eval evaluated-cell diff.** The `UserModel`
  diff-list is edit-sites-only (28 bytes regardless of cascade). So progressive
  during-eval repaint is **not possible**; the renderer must **re-pull the visible cells**.
- **The 1M cascade recompute stays ~1.2 s** (known-FAIL vs <100 ms) — expected, recorded,
  and the reason non-blocking matters.

## Scroll-during-eval (follow-on probe — is a newly-scrolled cell readable mid-eval?)

A design claim was made that *"while the worker is inside `evaluate()` (1–7 s on a huge
edit), it cannot service a scroll (`SetViewport`), so newly-scrolled-in cells can't be read
until the eval finishes."* This was **empirically checked** (not assumed) by
`src/bin/scroll_during_eval.rs` → `results/scroll_during_eval_*.json`, at 10⁵ (fast) and
10⁶ (the ~1.2–1.5 s motivating eval). Every read is force+asserted real; env-stamped;
foreground.

**Reproduce** (from `experiments/round-2/01-async-interop/`, foreground, with `timeout`):
```sh
cargo run --release --bin scroll_during_eval -- --size 100000     # fast smoke
cargo run --release --bin scroll_during_eval -- --size 1000000    # ~1.4 s eval scale
```

| metric | deep_serial 10⁵ | deep_serial 10⁶ |
|---|---|---|
| warm `evaluate()` | 78.6 ms | 1.45 s |
| **Q1 live scroll issued mid-eval → new region published after** | **99.4 ms** | **1.39 s** (≈ one eval) |
| Q2 snapshot read during eval — p50 / p99 / max | 787 ns / 2.01 µs / 30.7 µs | 1.63 µs / 2.92 µs / 53.9 µs |
| — reads sampled while eval genuinely in flight | 2000 / 2000 | 2000 / 2000 |
| Q2 snapshot BUILD (`to_bytes`+`from_bytes`) | 49.9 ms (1.2 MB payload) | 3.23 s (12.4 MB payload) |
| Q2 second-model peak-RSS delta (high-water; overstated¹) | +36.8 MB | +400 MB |

1. **Is the live model readable during an in-flight eval? NO — blocked ≈ one eval.**
   A `SetViewport` issued *while the worker is mid-`evaluate()`* (asserted: `mid-eval=true`)
   published the newly-scrolled region only after **≈ the whole eval duration** (99 ms @10⁵
   ≈ its 79 ms eval; **1.39 s @10⁶ ≈ its 1.45 s eval**). The newly-published tail was the
   **fresh post-edit value** (e.g. 1 012 340, not the stale 1 000 000), proving the scroll
   was serviced by a *real re-pull only after the eval returned*, not by a stale echo. **The
   claim is confirmed.** *Mechanism (Rust/IronCalc):* `Model::evaluate(&mut self)` holds an
   **exclusive borrow**, which by Rust aliasing **excludes** any concurrent
   `get_cell_value_by_index(&self)` of the *same* model — so the live model is unreadable for
   the eval's whole duration. On top of that, in the seam `SetViewport` rides the *same*
   command channel as edits, and the worker only drains that channel **between** evals (it is
   inside `evaluate()`, not calling `rx.recv()`). Both effects point the same way: a mid-eval
   scroll of the live model waits one eval.

2. **Can a separate SNAPSHOT serve arbitrary scrolled reads concurrently? YES — not
   blocked.** A second `Model` built at the last settle (`to_bytes()` → `from_bytes()`) was
   read from another thread **while the real model was mid-`evaluate()`** (asserted: all 2000
   reads sampled with the eval in flight). Arbitrary (Knuth-hash-spread) scrolled-cell reads
   returned in **p50 ≈ 0.8–1.6 µs, p99 ≲ 3 µs, max ≲ 54 µs** — no aliasing with the worker's
   `&mut Model`, so nothing blocks. The reads return **stale, pre-eval** values (asserted:
   the snapshot tail stayed at its settled value while the real model recomputed to a
   different one) — which is exactly fine for scrolling. **Cost:** the snapshot BUILD is the
   expensive part — **~50 ms @10⁵ but ~3.2 s @10⁶** (dominated by `from_bytes`: 3.04 s;
   `to_bytes` is only 185 ms), plus a **second resident model** (~the size of the first;
   payload 12.4 MB @10⁶, and the measured peak-RSS high-water rose ~400 MB during the build¹).
   ¹ The RSS delta is a high-water mark that also captures the transient encode buffer and
   `from_bytes` scratch, so it **overstates** the steady-state second-model footprint (the
   ~162 B/cell storage ⇒ order ~160 MB for a 10⁶ live model; the 12.4 MB `to_bytes` payload is
   the compact figure). Either way a snapshot is a **whole-workbook** operation with no partial
   route.

3. **Overscan headroom — the cheap alternative.** If the published viewport is a symmetric
   `k×` overscan window around a visible `V_rows × V_cols` viewport, the user can scroll
   **±(k−1)/2 · V** in each dimension while staying inside already-published cells (**no
   worker read at all**). For a screenful of `40×12` visible cells: **k=2 → ±20 rows / ±6
   cols**, **k=3 → ±40 rows / ±12 cols**, **k=5 → ±80 rows / ±24 cols**. The published buffer
   is only `k²·V` cells (e.g. k=3 → 4 320 cells), so overscan is essentially free to build and
   costs no extra model. A force-asserted demo confirms an in-window scroll is served from the
   published buffer alone (`in_window_read_served_from_buffer=true`).

**Recommendation: overscan as the default; snapshot only if the app must scroll *beyond* the
overscan window mid-eval — not both by default.** Overscan (a 3× window ≈ ±40 rows / ±12
cols of free headroom) absorbs virtually all *interactive* scrolling during an eval at zero
extra memory and a trivial per-publish read, and it is already compatible with the locked
seam (just widen the published viewport). The snapshot fully solves *arbitrary* mid-eval
scrolling — its concurrent reads are ~1–3 µs and correctly stale — **but its build is the
catch: ~3.2 s at 10⁶ (worse than the ~1.4 s eval it is meant to paint over) plus a second
resident model**, so building it *per eval* would widen staleness more than the eval itself.
Practical policy: **ship overscan; reach for a snapshot only on demand** — i.e. if/when the
user scrolls past the overscan margin *during* a long eval, build a one-off snapshot (bounded,
off the render loop) to serve stale values until the eval settles. For the common case (short
evals, or scrolling within a screen or two) overscan alone is sufficient and the snapshot is
never paid.

## Recommended (locked) engine↔render interop-seam design

The seam implemented in `src/seam.rs` and validated by the GATES. It is the
**"snapshot/publish + wait-then-repull"** branch of architecture §4.2 — locked because
IronCalc's API leaves no higher-fidelity option.

1. **Ownership / non-blocking (chosen because `Model` is `Send`, `&mut`, non-reentrant).**
   A single **`EvalWorker` thread owns the authoritative `Model`** and runs **all**
   `evaluate()` calls. The render loop owns none of it. This satisfies serialization for
   free (one worker = one eval at a time) and keeps eval entirely off the render path.
2. **Edits in (decoupling): a channel of edit commands.** The render loop sends edits over
   an `mpsc` channel (non-blocking send) and returns immediately; edited cells
   optimistically show their new literal input.
3. **Coalescing (GATE 2): drain-then-eval.** Before each eval the worker **drains every
   queued command**, applies them, and runs **one** `evaluate()`. A burst of N edits ⇒ 1
   eval (measured). This is latest-wins debounce with zero configured delay — the eval's
   own duration is the natural coalescing window.
4. **Change propagation (locked fallback, because no evaluated-cell diff exists):
   publish-on-completion, re-pull the visible viewport.** Immediately after each eval, the
   worker reads the *current visible viewport* (the ~10³ on-screen cells) into a **small
   value snapshot** and publishes it to a shared slot. The render loop, each tick, does the
   **cheap O(viewport) read** of that slot (a short-held `Mutex` clone) — never the big
   model. Publish is ordered **before** the generation counter bumps, so a reader that sees
   generation N is guaranteed to see its values (a race we found and fixed).
   - *Why viewport-read and not the `to_bytes` snapshot:* both are implemented/measured; the
     worker-side viewport read needs **no clone** and is orders cheaper than a 13–97 MB
     `to_bytes` (~0.2–0.9 s). The full snapshot is kept as the documented alternative (e.g.
     if the render side ever needs the whole model), with its cost recorded.
5. **Staleness UX (§4.3).** Between an edit and the next publish (~one eval), the loop paints
   **last-known cached values** (IronCalc persists cached results, so even a freshly-opened
   file paints immediately) plus a light **"recalculating…"** indicator (the worker exposes
   an `is_evaluating()` flag). Staleness window = the eval duration (discovery: ~1.3 s @10⁶,
   ~7 s @10⁷ on this floor box).

**How the two halves stay decoupled:** the only shared surface is a channel (edits in) and a
small published-viewport slot (values out) + two atomics (`eval_count`, `evaluating`). The
renderer never sees `Model`; the engine never sees the frame clock. Each half can be its
best — GPU renderer at frame rate, IronCalc the authoritative model.

## Next-best alternative

If future profiling shows the post-eval viewport read is itself too slow for a huge overscan
window, publish a **compact value+styleId projection of the populated cells** once per eval
(still off the render loop) and let the render loop index into it. Cost = projection build
per eval (bounded by populated cells), widening staleness, not the frame budget. The
`to_bytes` snapshot (measured) is the coarsest such route.

## Risks / open questions (carried forward)

- **Live "settling" feel is capped by the lack of an evaluated-cell stream.** On a huge edit
  the on-screen cells go fresh only when the whole eval finishes (~1.2 s @10⁶, ~7 s @10⁷) —
  not progressively. Acceptable (last-known + "recalculating…"), but it is the ceiling on
  liveness; only an IronCalc change (expose cascaded cells, or a chunked/interruptible eval)
  would lift it. **This is the exact constraint functional_spec §7 flagged; SP1 confirms it
  is "wait-then-repull," and that this is acceptable.**
- **Excel's 1,048,576-row limit is enforced by IronCalc** — the app's data model / datagen
  must respect it (single-column shapes past ~10⁶ rows error). A finding for the real build.
- **Snapshot is whole-workbook** (no partial serialize) — if a snapshot-publish variant is
  ever chosen, its cost grows with total populated cells (~10 MB/10⁶ cells here).
- **This box is a 4-core floor.** Real hardware evaluates faster (shorter staleness), which
  only improves the UX; the non-blocking guarantee is independent of eval speed.
