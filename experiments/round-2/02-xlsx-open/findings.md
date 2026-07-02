# SP2 — End-to-end large styled `.xlsx` open: findings

_IronCalc 0.7.1 — open time, peak RSS, stage breakdown, time-to-first-paint. FreeCell
Phase 2 (Round-2), engine-risk cohort. Closes functional_spec §5.4's one un-run target._

Reproduce everything from this folder (all foreground, `timeout`-wrapped):

```
# One-command canonical path: generate a >=100 MB styled .xlsx from committed code,
# then open it 3x from fresh child processes and write results/.
SP2_COMMIT=$(git rev-parse --short HEAD) SP2_DATE=2026-07-01 \
  timeout 400 cargo run --release --bin measure -- 100 3 data/large.xlsx

# Split path (used in the timing-constrained container; identical numbers):
timeout 200 cargo run --release --bin gen -- 100 data/large.xlsx          # ~100s
SP2_REUSE_FILE=1 SP2_COMMIT=$(git rev-parse --short HEAD) \
  timeout 200 cargo run --release --bin measure -- 100 3 data/large.xlsx  # reuse the file

cargo test --release   # 10 unit/integration tests
```

The `.xlsx` itself is **gitignored** (a ~105 MB binary); it is reproduced deterministically
by `gen` from committed code (seed 20260701). Committed outputs live in `results/`.

---

## 1. Questions

1. **Open cost.** How long does a **fresh process** take to open a real ≥100 MB styled
   `.xlsx`, and what is **peak RSS**?
2. **Where does the time go?** unzip / XML parse / shared-strings / style ingest / graph
   build / first eval — as far as IronCalc's public API lets us split it.
3. **Time-to-first-paint.** How fast can cached values be shown *before* a full recompute
   (IronCalc persists cached results)?
4. **Verdict.** Does open complete in **seconds, not minutes**, with peak RSS a **sane
   multiple of file size**? Or does it trip the off-ramp (minutes-scale, or RSS ≫10×
   uncompressed)?

---

## 2. What was done (approach + code pointers)

### 2.1 The file generator (built HERE, not in frozen `datagen`) — `src/lib.rs::generate`

`shared/datagen` is frozen and has **no `.xlsx` writer**, so the writer lives in this
folder and uses **IronCalc's own native styled writer** (`ironcalc::export::save_to_xlsx`)
— the same code path a real FreeCell save would exercise, so the file is representative of
what IronCalc actually reads back. Content is a **realistic mix**, all deterministic
(reproducible from committed code):

- **Values** from `datagen::SyntheticSheet` — a number/text/empty mix per cell.
- **Shared strings:** text comes from `datagen`'s bounded word pool, so strings recur →
  a dense, deduplicated `sharedStrings.xml` (3.1 MB uncompressed in the shipped file).
- **A formula column per sheet:** `=SUM(A{r}:{lastcol}{r})` over the row's literals. `SUM`
  ignores text/empty operands (matching Excel), so it is well-defined on the mixed content
  and makes the formula graph non-trivial so the first `evaluate()` does real work.
- **Per-cell styles:** bold/italic/fills (`solid` `fg_color`)/alignment, plus a rotated
  number format (`#,##0.00`, `0.0%`, `$#,##0.00`, …). All deduped by IronCalc's style
  table (styles.xml is only 37 kB — the dedup works).
- **Band styling:** per-column widths (`set_column_width`) so the file carries column-band
  metadata, not just per-cell styles.
- **Multiple sheets:** 4 sheets (default), each seeded distinctly.

The model is `evaluate()`d **once** before saving, so the writer persists correct cached
formula values (`<f>…</f><v>…</v>`). That cached value is the premise behind
time-to-first-paint (§4.3).

**Shipped size:** `GenSpec::large()` = seed 20260701, 4 sheets × 245 000 rows × 12 cols
(+1 formula col) = **12.74 M cells**, landing **104.9 MB compressed on disk / 507.8 MB
uncompressed** in a single generate attempt. Generation cost (kept **separate** from open,
per benchmark discipline): **~72 s build + ~28 s write ≈ 101 s** foreground.

### 2.2 The open measurement — fresh child process, canonical peak RSS

`src/bin/open.rs` is a **separately-spawned child** whose only job is to open the file once
and stamp its own **peak RSS via `round2_harness::peak_rss()`** — the canonical VmHWM
helper (NOT `sysinfo::peak_rss_bytes`, which returns 0 on `/proc` failure). Because the
child does nothing else, its VmHWM high-water mark is the honest open-only peak (architecture
§3: peak RSS must come from a fresh child, not the allocator-polluted harness process).
`src/bin/measure.rs` (parent) generates the file, spawns `open` 3× via `std::process::Command`,
parses each child's one JSON line, and aggregates.

**Stage breakdown — the coarsest *honest* split (instrumentation opacity is a finding).**
IronCalc's public API exposes only two seams inside the open: `load_from_xlsx` (which fuses
unzip + XML parse + shared-strings + style ingest + workbook build + formula parse) and
`Model::evaluate()`. The finer sub-stages (unzip vs XML vs shared-strings vs style ingest vs
graph build) are **not separable without patching the engine** (`load_from_excel` is a single
opaque call — architecture §8). Rather than invent precision, we record four honest stages:

| Stage | What it is | How measured |
|-------|-----------|--------------|
| `read` | file bytes → memory (`std::fs::read`, warms page cache) | wall-clock |
| `parse+build` | `load_from_xlsx`: unzip + XML + shared-strings + styles + workbook + formula parse (**no eval**) | wall-clock |
| **first paint** | process start → cached values queryable = `read + parse+build` | derived |
| first eval | `Model::evaluate()` (full recompute) | wall-clock, **separate** |

**Force + assert.** `open_stages` reads a known **sentinel** (sheet 0's row-0 `SUM` cell,
whose value is computed independently from `datagen` content) both at the cached stage and
after `evaluate()`, `black_box`-es the reads, and asserts both equal the expected number —
so nothing is optimized away and the cached value is proven correct (not stale/zero).

---

## 3. Results

Container: 4-core Intel Xeon @ 2.80 GHz, ~15 GB, Linux, no GPU (a 4-core floor; real
hardware is faster). IronCalc 0.7.1 (pinned, same as the Round-2 harness). Commit `5e9c3a3`.
Median of 3 fresh-child opens; per-run spread is ±0.3 s (see `results/open_stage_timings.json`).

**File:** 104.9 MB compressed / **507.8 MB uncompressed** (4.84× compression), 12.74 M cells,
4 sheets. Dominant uncompressed content = the four worksheet XMLs at ~126 MB each.

| Metric | Value |
|--------|-------|
| **Open → recompute-ready (median)** | **21.98 s** |
| &nbsp;&nbsp;• file read | 0.062 s (0.3%) |
| &nbsp;&nbsp;• **parse+build (unzip/XML/shared-strings/styles/graph)** | **18.09 s (82%) — dominant** |
| &nbsp;&nbsp;• first eval (full recompute) | 3.82 s (17%) |
| **Time-to-first-paint (median)** | **18.16 s** (cached values queryable, no eval) |
| **Peak RSS** | **2 520 MB** |
| &nbsp;&nbsp;• as multiple of compressed file (104.9 MB) | **24.0×** |
| &nbsp;&nbsp;• as multiple of uncompressed payload (507.8 MB) | **4.96×** |

**Gates & off-ramp:**

| Judgment | Threshold | Result | Verdict |
|----------|-----------|--------|---------|
| Open in **seconds, not minutes** | < 60 s | 21.98 s | **PASS** |
| Peak RSS a **sane multiple of file size** | ≤ 8× uncompressed | 4.96× uncompressed | **PASS** |
| **Off-ramp trigger** | minutes-scale OR ≫10× uncompressed | 22 s / 4.96× | **CLEAR (not triggered)** |

Raw artifacts: `results/open_summary.json` (gate verdicts + all numbers),
`results/open_stage_timings.json` (per-run + median, env-stamped `BenchResult`),
`results/env.txt` (env + file/spec stamp).

---

## 4. Interpretation

### 4.1 Open is seconds, not minutes — the GATE passes
A 105 MB / 12.7 M-cell styled workbook opens to fully-recompute-ready in **~22 s** on a
4-core floor. That is comfortably "seconds, not minutes." It is not instant, but it is a
one-time open cost, it is dominated by a stage that parallelizes poorly today (single-threaded
XML parse — see §4.4), and real hardware is faster. **The "open huge files" promise holds.**

### 4.2 Peak RSS is a sane multiple of the *uncompressed* payload
Peak RSS is 24× the *compressed* file but only **~5× the uncompressed payload** — and the
uncompressed payload (508 MB) is the honest denominator, because that is the OOXML text
IronCalc must actually parse and hold. ~5× uncompressed for a fully-materialized cell graph
(nested-`HashMap` storage at ~162 B/cell, cached values, parsed formula ASTs, style table,
shared strings) is unsurprising and **well under the ≫10×-uncompressed off-ramp**. The
memory is a function of the *content*, not zip trickery, so it scales predictably. Note the
absolute ceiling: 12.7 M cells cost ~2.5 GB, so the ~15 GB box tops out around ~60–75 M
cells for a single open — recorded for the memory-ceiling picture, though not this
experiment's gate.

### 4.3 Time-to-first-paint ≈ load, and it precedes full recompute — the DISCOVERY
**Cached values are queryable the instant load finishes, before any `evaluate()`** — this is
a real, verified property of IronCalc, not an assumption:
- `Model::from_workbook` runs `parse_formulas()` (builds the AST) but **does not call
  `evaluate()`** (verified in the 0.7.1 source).
- Formula cells load as `CellFormulaNumber { f, v }` carrying the cached `<v>`;
  `get_cell_value_by_index` returns that cached `v` with **no eval**.
- The `save_to_xlsx` writer persists `<f>…</f><v>{v}</v>`, so any IronCalc-written file (and
  any Excel-written file, which does the same) opens paint-ready.

Measured, **time-to-first-paint = 18.16 s** — the full open minus the 3.82 s recompute.
The practical UX implication for FreeCell: **the last 3.8 s (17%) of the open is deferrable.**
The grid can paint the sheet from cached values at ~18 s and run the authoritative recompute
in the background (exactly the SP1 non-blocking seam), rather than making the user wait the
full ~22 s. First paint is dominated by the same parse+build stage, so it is *not* dramatically
cheaper than full open here — but it is a genuine, correct earlier milestone, and it removes
recompute from the critical path.

### 4.4 Where the time actually goes
**82% of open is the single opaque `load_from_xlsx` call** — unzip + XML parse + shared-strings
+ style ingest + workbook/graph build, fused and single-threaded. The pure disk read is
negligible (0.06 s; the file is ~105 MB and the page cache is warm). The first full recompute
of a 12.7 M-cell graph is only 3.8 s. So the open bottleneck is **parsing 508 MB of OOXML XML**,
not evaluation and not I/O. If open latency ever needs to drop, the lever is the XML/ingest
path (streaming or parallel parse), not the recompute — a useful pointer for a future round,
and squarely inside IronCalc's own code.

---

## 5. Threats to validity

- **Stage opacity.** IronCalc fuses unzip/XML/shared-strings/style-ingest/graph-build into one
  `load_from_xlsx` call; we cannot split them from a downstream crate without patching the
  engine. We report the coarsest **honest** seam and label it as such (architecture §8) rather
  than inventing sub-stage numbers. The `read`/`parse+build`/`first-eval` split *is* real.
- **Warm page cache.** `parse+build` runs after we read the bytes once, so the disk cost is
  paid inside `read` (0.06 s) and `parse+build` reflects CPU-bound parse, not cold I/O. On a
  truly cold cache the first read would add a few hundred ms of disk — negligible vs the 18 s
  parse, and it does not change the verdict.
- **Generator = IronCalc's own writer.** The file is produced by `save_to_xlsx`, so it is a
  faithful IronCalc round-trip, but it is not a byte-for-byte Excel export. The OOXML shapes
  (shared strings, styles, cached formula values, per-sheet worksheet XML) match what Excel
  emits, so the parse cost is representative; a hand-authored-in-Excel file of the same size
  would parse comparably. SP5 separately probes long-tail style fidelity.
- **4-core floor.** All numbers are a container floor (architecture §0); real hardware is
  faster. The parse being single-threaded means more cores would not help *this* stage today.
- **Reuse fast-path.** `SP2_REUSE_FILE=1` measures an existing file with `GenSpec::large()`'s
  exact spec (hence its exact sentinel); the sentinel assertion would fail loudly on any spec
  mismatch, so reuse cannot silently measure the wrong file. Without the flag, `measure`
  regenerates from committed code — the canonical, fully-reproducible path.

---

## 6. Verdict

**SP2 GATE: PASS. Off-ramp: NOT triggered.**

- Open completes in **~22 s (seconds, not minutes)** for a 105 MB / 12.7 M-cell styled `.xlsx`
  on a 4-core floor.
- Peak RSS is **~5× the uncompressed payload (24× the compressed file)** — a sane, predictable
  multiple, comfortably under the ≫10×-uncompressed off-ramp.
- **Dominant cost = OOXML parse+build (82%)**, single-threaded; recompute is only 17%.
- **Time-to-first-paint = 18.2 s** (cached values queryable before any recompute), removing the
  3.8 s recompute from the critical path and feeding directly into the SP1 non-blocking seam.

The `.xlsx`-open axis of the engine-risk cohort is **clean**: no minutes-scale open, no RSS
balloon. IronCalc's "open huge styled files" promise holds within the Phase-2 bar. The only
noted lever, should open latency ever need improving, is IronCalc's XML/ingest path — not a
FreeCell-side or engine-choice concern.
