# Investigation A — Style/geometry cache sync + structural editing (the crux)

> Phase-3 crux (functional_spec §6-A, architecture §4). The real deliverable is a
> **locked cache-sync design** — the always-resident style/geometry cache
> (`projects/style-cache.md`) stays provably in lockstep with IronCalc across
> insert/delete row/column and undo/redo — plus the **`Model`-vs-`UserModel`**
> recommendation and whether the **SP1 worker seam** carries over. Every claim is backed
> by a runnable probe/test (`cargo test`, 17 runtime GATE assertions + 1 compile-time
> `Send` proof) or a cited IronCalc 0.7.1 source location.

## Questions (from the spec)

1. `Model` vs `UserModel` — which does the app build on? Is `UserModel` `Send`? Does the
   SP1 seam still hold? Capture its API.
2. Do insert/delete row & column correctly shift (a) formula references, (b) band styles,
   (c) sizes, (d) merges? Survive an `.xlsx` round-trip?
3. Undo/redo — present? coverage? does structural undo fully un-shift?
4. Copy/paste of a range — relative-reference translation?
5. The cache-sync design — how does the resident cache shift on insert/delete, at what
   cost, reversibly for undo, and does it **agree with IronCalc**?

## What was done

An independent Cargo project `cache_sync` (`experiments/round-3/A-cache-sync/`), depending
**read-only** on the frozen `../../round-2/harness` (`cpu_model`, env stamping) and
`../../shared/bench_util` (p50/p99 + env-stamped JSON), and directly on `ironcalc` /
`ironcalc_base` 0.7.1 for the interactive `UserModel` API:

- **`src/probe.rs`** — a compile-time `assert_send::<UserModel<'static>>()`, a runtime API
  smoke, and a **worker-seam probe** that moves a `UserModel<'static>` onto a spawned
  thread, runs `insert_rows` + `evaluate` there, hands it back, and reads the shifted
  result (the SP1 seam shape applied to `UserModel`).
- **`src/cache.rs`** — the resident cache prototype: per-axis default + sparse override
  maps (`BTreeMap`) for sizes and band styles, a per-cell style override map, a
  `StyleInterner` (deduping on the serialized `Style`, since `Style: Eq` but not `Hash`),
  and a **dense prefix-sum** cumulative-size structure (architecture §4.3 candidate **(a)**)
  with `offset`/`index_at` scroll math + a `shift`/`restore_removed` primitive.
- **`src/harness.rs`** — builds a reference sheet (cross-referencing formulas + row/column
  band styles + custom heights/widths), hydrates the cache from IronCalc's authoritative
  getters, and runs the **agreement contract** `assert_cache_agrees` (architecture §4.4):
  after each edit/undo/redo, for a sample of indices spanning the edit point (including the
  banded rows and their shifted positions), assert cache size/band-style == IronCalc
  re-read, and the cumulative offset is self-consistent.
- **`tests/correctness.rs`** — 17 GATE tests (structural shift, `.xlsx` round-trip,
  undo/redo of value/style/structural, cache agreement after every op, **a negative
  control** proving the agreement check detects an un-shifted / wrong-direction cache, the
  interner, the cumulative-offset math, and copy/paste reference translation).
- **`src/main.rs`** — the cost harness (DISCOVERY): IronCalc `insert_row`/`delete_row` cost
  and the cache-shift cost, **measured separately**, foreground, **force+asserted** (a band
  style past the edit point must have moved), p50/p99, env-stamped to `results/`.

## Findings

### 1. `Model` vs `UserModel`, `Send`-ness, and the SP1 seam — RECOMMENDATION: build on `UserModel`

- **`UserModel<'a>` wraps `Model<'a>`** (`user_model/common.rs:222`: `pub struct
  UserModel<'a> { model: Model<'a>, history: History, send_queue: Vec<QueueDiffs>, .. }`).
  It adds the interactive layer the app needs: `undo`/`redo`/`can_undo`/`can_redo`
  (common.rs:306-340), a diff/history stack, and **automatic evaluation** after each edit.
- **`UserModel<'static>` is `Send`** — proven at compile time
  (`probe::assert_usermodel_send`, mirroring SP1's `assert_send::<Model<'static>>()`) and at
  runtime (`probe::probe_worker_seam` moves it onto a worker thread and reads back A3=20).
  Because `UserModel` is just `Model` + `History` (`Vec<DiffList>`) + `Vec<QueueDiffs>` + a
  `bool`, and all are `Send`, the `Send`-ness carries over for free.
- **The SP1 worker seam holds unchanged.** Structural edits and `set_user_input` are
  `&mut self` and **auto-run a full `evaluate()`** via `evaluate_if_not_paused`
  (common.rs:430, 891, 963, 2106). This is the *same* blocking shape SP1 characterized for
  `Model::evaluate` — a full-workbook, non-incremental, `&mut self` recompute. So the
  worker-owns-the-model + coalesce-then-eval + publish-viewport seam applies verbatim, now
  with `UserModel` on the worker. `pause_evaluation()`/`resume_evaluation()`
  (common.rs:347-357) additionally let the worker **batch** a burst of edits and evaluate
  once — a natural fit for the seam's drain-then-one-eval coalescing.
- **Recommendation: the app builds on `UserModel`.** It is the only public path to
  undo/redo + structural edits + the collaborative diff-list, it is `Send`, and it does not
  change the seam. `Model` remains reachable read-only via `get_model()` for the style/size
  getters and the `.xlsx` export path.

### 2. Structural edits — GATE PASS (references + band styles + sizes shift correctly)

`UserModel` exposes `insert_rows` / `insert_columns` / `delete_rows` / `delete_columns`
(common.rs:882-1022), each delegating to the `Model` primitive (actions.rs:136-460) then
auto-evaluating. Proven by `tests/correctness.rs`:

- **(a) formula references shift** (`insert_row_shifts_refs_styles_sizes`,
  `delete_row_shifts_refs_styles_sizes`): after an insert at row 4, the formula that was
  `B10 = A10 + A9` is now at `B11` and reads `A11 + A10`; the `=SUM(A1:A30)` total
  re-targets across the edit (unchanged 465 after inserting a blank row; 461 after deleting
  row 4 which held `A4=4`). IronCalc does this via `displace_cells` (actions.rs:379).
- **(b) row/column band styles shift** — band styles live in `worksheet.rows[r].s` /
  `worksheet.cols[..].style` and are re-keyed by the primitive (actions.rs:362-376 rows,
  symmetric for cols). The green row band on row 10 moves to row 11 on insert / row 9 on
  delete; the blue column band on col 5 moves to col 6 on an insert at col 3.
- **(c) sizes shift** — custom row heights (in `worksheet.rows[r].height`) and column
  widths (`worksheet.cols`) move with their band; asserted equal to `CUSTOM_ROW_HEIGHT` /
  `CUSTOM_COL_WIDTH` at the shifted index.
- **`.xlsx` round-trip** (`xlsx_roundtrip_preserves_structural_edit`): after an insert + a
  column delete, `save_to_xlsx(get_model())` -> `load_from_xlsx` preserves the shifted
  band, height, and formula reference.
- **(d) merges — GAP (recorded, not an A blocker).** `merge_cells: Vec<String>` exists on
  `Worksheet` (types.rs:113) but there is **no public `Model`/`UserModel` setter or getter**
  in 0.7.1 (only touched internally by the `DeleteSheet` undo). Merges are unreachable
  through the public API — confirming overview §2's known-open item. The correctness harness
  omits merges by necessity; this is a **B-audit / product-scope** item (owning `.xlsx`
  writing), not an A off-ramp.

### 3. Undo/redo — GATE PASS (value + style + structural, fully un-shifts)

`UserModel` keeps an undo/redo stack of `DiffList`s (`history.rs:191`) and replays them
(`apply_undo_diff_list` / `apply_diff_list`, common.rs:2112-2666). Proven:

- **value** (`undo_redo_value_edit`): `=6*7` -> undo -> empty -> redo -> 42.
- **style** (`undo_redo_style_edit`): bold A1 -> undo (un-bold) -> redo (re-bold).
- **structural, fully un-shifts** (`undo_redo_structural_edit_fully_unshifts`): insert row
  -> undo restores the row-10 formula, band, and height to their pre-edit indices -> redo
  re-shifts. IronCalc implements undo-of-insert as a delete and undo-of-delete as an insert
  that **restores saved row/column data** (`DeleteRows { old_data }`, common.rs:2198-2245),
  so a deleted banded/sized row comes back intact.
- **Granularity:** one `DiffList` per user action (a whole `insert_rows(count)` is a single
  history entry). The diff-list carries **edit-sites only** (matching SP1) — no
  downstream-dirty set.

### 4. Copy/paste — reference translation PRESENT; the clipboard is NOT externally chainable

- **Relative-reference translation works** (`copy_paste_translates_relative_refs`):
  `Model::extend_copied_value("=A1+B1", C1, C2)` -> `"=A2+B2"` (public, model.rs:1179). This
  is the reference-displacement logic paste uses.
- **But the high-level clipboard cannot be chained through the public API from an external
  crate.** `copy_to_clipboard()` returns a `Clipboard` whose `data` field is `pub(crate)`
  (common.rs:42) and `ClipboardCell` is not externally constructible, so you cannot feed a
  copy result into `paste_from_clipboard(.., &ClipboardData, ..)` from outside the crate
  (IronCalc's own tests do — they are in-crate, `test_paste_csv.rs:106`). **Implication:**
  the app either (i) drives copy->paste as a within-engine op it can't inspect, (ii) uses
  `paste_csv_string` (public, literal paste, no ref translation) + `extend_copied_value`
  for formulas itself, or (iii) upstreams a public accessor. Not an A blocker; a **B-audit**
  scope note.

### 5. Cache-sync design — GATE PASS (agrees with IronCalc, reversible for undo)

**Locked design (chosen + validated against IronCalc):**

- **Cumulative-size structure: dense prefix-sum array — candidate (a), CHOSEN.**
  Architecture §4.3 said "measure the simple dense-array option before rejecting." It is
  correct AND cheap at scale (see §6: the `Vec::splice` is **0.44 ms even at 1M**), so the
  more complex derived default+sparse-delta+Fenwick (b) and chunked (c) options are **not
  needed** for v1. `offset(index)` is an O(1) prefix lookup; `index_at(pixel)` is an
  O(log n) binary search; a structural edit is an O(extent) `Vec::splice` of the dense
  array + O(overrides) map re-key. `cumulative_offset_matches_sizes` asserts the math is
  exact through a shift.
- **Sparse override maps: `BTreeMap<index, _>`, re-keyed on shift.** `Axis::shift(at, delta)`
  re-keys every override with key `>= at` by `±delta` — **O(overrides shifted)** (measured
  invariant at 2000 overrides regardless of 100k vs 1M sheet size). On a delete it captures
  + returns the removed overrides (`RemovedOverrides` / `RemovedCellStyles`) so undo
  restores them exactly.
- **Undo strategy: mirror-the-primitive, CHOSEN.** The cache applies the inverse structural
  primitive on undo (undo-insert = delete at `at`; undo-delete = insert at `at` +
  `restore_removed`). Fully local (no engine re-read) and matches how IronCalc's own undo
  works, so the two stay in lockstep. `cache_agrees_after_undo_and_redo` and
  `cache_agrees_after_delete_undo_restores_removed_overrides` prove it — the second deletes
  a *banded* row (so the band override is genuinely dropped) and asserts undo restores it.
  (The alternative, re-sync-from-engine, is a viable fallback but costs a bounded re-read
  and buys nothing here since mirror-the-primitive already agrees.)
- **The agreement contract holds** (architecture §4.4, the load-bearing test): after each of
  {insert, delete} x {row, col} and each undo/redo, the mirrored cache's sizes + band
  styles == IronCalc's re-read `get_row_height`/`get_column_width` /
  `get_row_style`/`get_column_style`, sampled over indices spanning the edit **including the
  banded rows and their shifted positions**.
- **Negative control** (`negative_control_wrong_shift_is_detected`): if the engine is
  shifted but the cache is *not* (or shifted the wrong way), `assert_cache_agrees` **fails**
  — the contract has real discriminating power, not a rubber stamp. *(This control caught a
  real test-coverage bug: an earlier sample set happened to miss every moved band, so it
  "agreed" even un-shifted. Fixed by always sampling the banded indices — logged here per
  the adversarial-review convention.)*

**A key constraint the design must respect (finding):** the IronCalc diff-list is
`pub(crate)` (`history.rs:20` `enum Diff`) and only extractable as **opaque bitcode** via
`flush_send_queue()` (common.rs:376) — it is **not publicly inspectable field-by-field**.
So the cache **cannot** consume structured diffs to learn what shifted; it must **mirror
the structural primitive it issued** (exactly the locked design). This is fine for FreeCell
(it originates the edit, so it knows `(kind, at, count)`), but it means the diff-list is
*not* a usable surgical-update channel for structural edits — a note for Investigation B.

## 6. Cost at scale (DISCOVERY) — 4-core Xeon @ 2.80GHz, IronCalc 0.7.1 (round-2 pin)

Single-op `insert_row` / `delete_row` on a populated sheet, and the resident-cache
`shift_rows`, **measured separately, foreground, force+asserted** (a band style past the
edit point must have moved), build/eval separated from the measured op. p50/p99 in
`results/*.json`.

| Scale | IronCalc `insert_row` | IronCalc `delete_row` | Cache shift (2000 overrides) |
|---|---|---|---|
| **100k rows** | 327 ms p50 / 392 ms p99 | 318 ms p50 / 357 ms p99 | **0.136 ms** p50 / 0.190 ms p99 |
| **1M rows**   | 4.61 s p50 / 4.98 s p99 | 4.60 s p50 / 5.97 s p99 | **0.443 ms** p50 / 0.520 ms p99 |

(n=200 at 100k, n=8 at 1M — capped so the foreground run stays bounded; build+initial eval
was 0.31 s @100k / 4.05 s @1M and is excluded from the op timing.)

**Interpretation (adversarially reviewed):**
- **The IronCalc-side structural edit is the expensive part**, and it is **~O(populated
  cells moved)**: 10x the rows -> ~14x the time (327 ms -> 4.6 s). An insert/delete near the
  top of a 1M-row sheet moves ~1M cells (`actions.rs` iterates `sheet_data.keys()` and
  `move_cell`s each downstream cell) and then runs a **full non-incremental `evaluate()`**.
  This is multi-second at Excel scale — but it is exactly the kind of op the **SP1 seam was
  built for**: it runs on the worker thread, coalesced, while the render loop keeps drawing
  from the resident cache. It is *not* a new blocker; it is the already-accepted "recompute
  staleness ~= one eval" (overview §2) applied to structural edits.
- **The cache shift is trivially cheap** — sub-millisecond at both scales, **~2,400x
  faster** than the engine op at 100k and **~10,000x** at 1M. The 100k->1M growth
  (0.136 -> 0.443 ms) is the dense-array `Vec::splice` (O(extent)), NOT the map re-key
  (O(overrides), flat at 2000). **This validates architecture §4.3(a): the simple dense
  array is good enough — its splice stays well under a 16 ms frame even at 1M — so
  candidates (b) Fenwick and (c) chunked are unnecessary for v1.**
- Net: the cache-shift adds **negligible** cost on the render side; the structural-edit
  latency is engine-side and absorbed by the existing worker seam. No architecture change
  required.

## Grade against pass criteria

| Criterion | Type | Result |
|---|---|---|
| insert/delete row/col shift references + band styles + sizes (probe-backed) | GATE | **PASS** |
| undo/redo covers value + style + structural edits (structural fully un-shifts) | GATE | **PASS** |
| validated cache-sync design: shifts correctly, **agrees with IronCalc**, reversible for undo | GATE | **PASS** (17 tests + negative control) |
| `UserModel` `Send`-ness + SP1 seam holds | DISCOVERY | **`Send` = TRUE; seam holds unchanged** |
| structural-edit cost at 10^5-10^6 (IronCalc + cache, separately) | DISCOVERY | **recorded (§6): engine O(cells moved), multi-second @1M; cache sub-ms** |
| merges | — | **GAP: no public API (recorded; product-scope, not an A blocker)** |
| copy/paste clipboard externally chainable | — | **GAP: `Clipboard.data` is `pub(crate)`; ref-translation via `extend_copied_value` works** |

**No A off-ramp fired.** Structural edits are correct, undo/redo is complete, and the
resident-cache shift provably agrees with IronCalc at a negligible measured cost,
reversibly. The dense-array design is validated as good-enough by measurement. The two gaps
(merges, external clipboard chaining) are pre-known / B-audit scope, not
architecture-forcing for the cache. **Verdict for A: clear to build the resident cache as
designed; build on `UserModel`; the SP1 seam carries over.**

Carry-forward for the build / other investigations:
- **B (api-audit):** the diff-list is opaque (bitcode only) and cannot drive structured
  surgical updates for structural edits; the external clipboard is not chainable; merges +
  (per overview) conditional formatting have no public API.
- **Build:** structural edits are multi-second at 1M and must run on the SP1 worker (never
  the render thread); the cache-shift is a render-side op mirrored from the issued
  primitive.

## Reproduce

```
cd experiments/round-3/A-cache-sync
cargo test                        # 17 GATE correctness/undo/agreement tests (+ negative control)
timeout 480 cargo run --release   # UserModel probe + cost harness -> results/*.json
```

Environment stamped into every `results/*.json`.
