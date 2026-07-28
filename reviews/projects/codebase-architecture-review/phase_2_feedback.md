# Phase 2: Engine core & concurrency architecture

Scope: `app/crates/freecell-engine/` (worker seam, protocol, document adapter, caches) plus
`freecell-core::{publication, cache, axis}`. Judged as code, not as the outcome of prior
experiments. Read-only review; nothing was built or run.

**Measured facts used below** (so the numbers aren't hand-waved):

| Thing | Number |
|---|---|
| `worker/run.rs` total | 9,288 lines |
| `worker/run.rs` **production** (test module starts line 3985) | **3,984 lines (43%)** |
| `worker/run.rs` tests | 5,304 lines (57%), plus `tests/worker_seam.rs` 2,913 |
| `Command` variants | **53** (52 + one `#[cfg(test)]`) |
| `WorkerEvent` variants | **23** |
| `unwrap`/`expect`/`panic!` in *production* `run.rs` | 7 (3 `unreachable!`, 3 provably-checked `expect`, 1 test-only `panic!`) |
| `unwrap`/`expect`/`panic!` in *production* `document.rs` / `cache.rs` | 1 / 0 |
| `catch_unwind` guards in the worker | 6 mutation regions + 6 recovery guards |
| Shared read surfaces between worker and UI | 4 (2 `ArcSwap`, 2 `RwLock`) |
| Version/generation counters | 4, plus one **unversioned** surface |

---

## What's Good

- **[`worker/client.rs:80-110`] The publication path is genuinely well built.** `ArcSwap<Publication>`
  with publish-then-bump, the swap container itself handed to the grid so the render loop does one
  wait-free atomic load per frame, and no engine call on the render path. There is no `block_on`, no
  blocking `recv`, and no lock in the UI's value-read path anywhere in `freecell-app`. That part of
  the seam is correct and I would not change it.

- **[`document.rs`] IronCalc containment is real, not aspirational.** Almost the entire
  `WorkbookDocument` surface is `pub(crate)`; only 10 methods are `pub`, and none of them names an
  IronCalc type. `Worksheet`, `Style`, `Theme`, `UserModel` appear exclusively on `pub(crate)`
  signatures. The protocol in `worker/protocol.rs` is genuinely engine-free — even IronCalc's
  `BorderType`/`BorderStyle` cross as `&'static str` tags built from FreeCell enums
  (`BorderPreset::border_type_tag`, `BorderLine::style_tag`). This is the single best-executed
  architectural decision in the crate, and it is what makes the whole engine testable headless.

- **[`instrument.rs:1-51`] The "zero engine calls on the render path" invariant is instrumented and
  falsifiable.** A process-global counter bumped at the entry of every model-touching method, read
  before/after a scroll sweep, *with a documented negative control* so the gate can't pass
  vacuously. Enforcing an architectural invariant with a discriminating runtime assertion instead of
  a doc comment is exactly right.

- **[`freecell-core/src/axis.rs:31-73`] `Axis` is the right data structure for the stated goal.**
  Two-level segment sums: O(n/BLOCK) memory (~2k `f64` for 1M rows), O(log(n/B) + B) queries, sizes
  from a closure so nothing per-track is materialized. This is a real Excel-max primitive, and the
  `Send + Sync` compile-time guard at line 175 is a nice touch.

- **[`document.rs:1248-1382`] The read queries are algorithmically correct for huge sheets.**
  `find_matches`, `selection_stats`, and `resolve_edge` all walk `sheet_data` (populated cells)
  rather than the selected rectangle, and `resolve_edge` reduces to binary searches over a sorted
  occupancy line. A full-column selection on a sparse Excel-max sheet is O(populated), and the
  results are correct *past the published viewport*. Many spreadsheet UIs get this wrong; this one
  doesn't.

- **[`worker/run.rs:518-614`] Dispatch is exhaustive and typed, with no catch-all.** Every `Command`
  must be explicitly classified as control/edit/chart/clipboard/etc.; a new variant fails to compile
  rather than silently falling into the apply path. There is no stringly-typed dispatch anywhere in
  the seam.

- **Production panic discipline is excellent.** One `expect` in `document.rs` (on a just-inserted
  JSON object), three in `run.rs` all immediately preceded by the check they assert, three
  `unreachable!` on bucketing invariants the compiler nearly proves. For a 6k-line production engine
  that is unusually clean, and it makes the `catch_unwind` policy meaningful rather than a fig leaf.

- **[`worker/run.rs:459-467`] Drain-coalesce is the right loop shape.** Block on `recv()`, drain
  `try_iter()`, collapse to one paused apply + one `evaluate()`. Simple, and the `eval_count` metric
  makes the coalescing testable.

---

## Critical (must fix)

- **[`worker/run.rs:3245-3278`] The frozen-pane band in `build_publication` is unbounded, and it runs
  on every publish. This wedges the worker permanently and is reachable in two clicks.**

  `MAX_PUBLISH_ROWS`/`MAX_PUBLISH_COLS` (512×256) clamp only the *body* window
  (`clamp_viewport`, line 3969). The frozen band is read straight off the cache and iterated raw:

  ```rust
  let (m, k) = self.shared.caches.read().get(sheet).map(|c| (c.frozen_rows(), c.frozen_cols()))…;
  for row in (0..m).chain(body_rows.clone()) {
      for col in (0..k).chain(body_cols.clone()) { … self.doc.formatted_value(idx, cell) … }
  ```

  `m` and `k` have no ceiling anywhere on the path: `Command::SetFrozen` is unvalidated in
  `pre_validate`, `apply_one` (run.rs:3578) passes it through, and IronCalc's
  `Model::set_frozen_rows` accepts anything `< LAST_ROW` (1,048,575). The UI computes the count as
  `menu.run.1 + 1` — the last row of the header run the user right-clicked
  (`freecell-app/src/grid/view.rs:4695`) — so right-clicking row 500,000 and choosing "Freeze rows"
  sets `m = 500_000`. Every subsequent publish (i.e. every scroll frame, every keystroke) then
  performs ~500,000 × up-to-256 `formatted_value` calls and pushes millions of `PublishedCell`s into
  a `Vec`. The worker never returns; the app is a zombie with no error and no way out. The same
  state also arrives from a crafted `.xlsx` — `build_sheet_cache` copies `ws.frozen_rows` verbatim
  (`cache.rs`, `builder.set_frozen_rows(ws.frozen_rows.max(0) as u32)`), so opening such a file
  wedges on the first publish, and once saved the document is permanently unopenable.

  *Direction:* clamp `m`/`k` in `clamp_viewport`'s sibling position — the band is a UI affordance, so
  a hard cap (Excel uses the visible pane; a cap of, say, `MAX_PUBLISH_ROWS`) belongs in
  `build_publication`, and `SetFrozen` should be range-validated in `pre_validate` like every other
  boundary input. More generally: **every loop in `build_publication` must be bounded by a constant,
  not by a value that came from a file or a click.** That is the invariant the clamp was introduced
  for, and the band silently escaped it.

- **[`worker/client.rs:80-92`, `worker/run.rs:369-375`, `freecell-app/src/shell/window.rs:351-357`]
  Worker-thread death is a silent hang with no error, no watchdog, and no recovery.**

  Only the *mutation* regions are inside `catch_unwind`. Everything else on the worker thread is
  unguarded: `WorkbookDocument::from_source` (the xlsx import), `build_publication`
  (`formatted_value` per cell), `cache::build_sheet_cache`, `save_workbook` (zip + XML), the chart
  discovery/parse path (`zip` + `roxmltree` over an untrusted file), `find_matches`,
  `selection_stats`, `export_csv`. A panic in any of them unwinds out of `Worker::run` and kills the
  thread.

  What the user sees: `DocumentClient::spawn` discards the `JoinHandle` (line 88), so nothing
  observes the death. `DocumentClient::send` swallows the resulting `SendError` by design
  (`let _ = self.tx.send(cmd)`). The event `Sender` drops, so
  `while let Some(event) = receiver.recv().await` in `window.rs:351` simply exits. The window keeps
  rendering the last publication, edits vanish, Save does nothing, and **no dialog, no degraded bar,
  and no log entry is produced**. If the panic happens during load, the window sits on "Opening
  <name>…" forever (`window.rs:463` is the only thing that clears `loading`). There is no
  `panic::set_hook` in production and no liveness check anywhere in the crate.

  This is the more serious of the two Criticals because it is the *default* outcome for any engine
  panic outside the six guarded regions — including the file-parsing paths, which are exactly where
  untrusted input lands.

  *Direction:* make the seam fail loudly instead of silently. Either wrap the whole
  `Worker::load_and_run` body in a `catch_unwind` that emits a terminal `WorkerDegraded`/`LoadFailed`
  before the thread exits, or have the UI treat "event stream ended without a `Shutdown` we asked
  for" as a fatal worker error and surface it. Keeping the `JoinHandle` so the client can report
  `is_finished()` costs nothing. Silent zombie state is the worst possible failure mode for an app
  whose whole value proposition is not losing the user's data.

---

## Moderate (should fix)

- **[`worker/run.rs:1913-1957`] The cache write lock is held across up to 100,000 IronCalc reads,
  blocking the render thread for the duration.**

  `refresh_cache_cells` takes `caches.write()` at line 1913 and *then* loops
  `for row in range.rows() { for col in range.cols() { cache::refresh_cell(cache, &self.doc, …) } }`
  — each `refresh_cell` is a real engine style read. `MAX_REFRESH_CELLS` is 100,000, so a large
  (non-band) style edit holds the exclusive lock across 100k engine calls. Meanwhile
  `freecell-app/src/grid/view.rs` acquires `caches.read()` **23 distinct times**, several of them per
  frame. `parking_lot::RwLock` will park the render thread for the whole loop.

  Contrast with `build_and_store_cache` (line 1978), which correctly builds off-lock and takes the
  write lock only to `insert`. The mirror path should do the same: compute the updated entries into a
  scratch structure, then take the lock to apply them.

- **[`worker/run.rs:1913`, `freecell-app/src/grid/view.rs:1255 & 3932`] The style/geometry cache has
  no snapshot semantics, so a single render frame can read a torn cache.**

  `resolve_frame` takes one read lock for the axes; `build_grid_layers` takes separate read locks for
  styles. A worker `build_and_store_cache` landing between them produces a frame that mixes
  pre-edit geometry with post-edit styles. The publication solved exactly this problem with
  `ArcSwap`; the cache — which carries geometry, borders, fills, merges, hidden/frozen state — did
  not. Given that `SheetCache` is already rebuilt-and-replaced wholesale on most mutations,
  `ArcSwap<SheetCache>` per sheet (or `ArcSwap<SheetCaches>`) would give the render path the same
  wait-free, internally-consistent read it already has for values, and would delete the
  write-lock-stall above as a side effect.

- **[`worker/run.rs:966-971`] There is no single commit point: four shared surfaces, three different
  versioning disciplines, and one with none at all.**

  The worker publishes across `ArcSwap<Publication>` (versioned by `Shared::generation` *and* a
  duplicate `Publication::generation`), `ArcSwap<ChartSnapshot>` (its own independent
  `chart_version`), `RwLock<SheetCaches>` (**no version — only a `StyleCacheUpdated { sheet }`
  event**), and `RwLock<HashMap<SheetId, Vec<CfRuleView>>>` (no version). The commit order in
  `apply_edit_batch` is: chart snapshot (`reresolve_charts`, 964) → publication + generation
  (`publish`, 966) → **`Published` event emitted (967)** → style cache (`apply_cache_refresh`, 971)
  → CF caches (980) → CF map (988). The UI repaints on `Published`, i.e. *between* the value commit
  and the style commit — so a cell edit that also changes a style renders new text with the old style
  for a frame, then repaints again on `StyleCacheUpdated`. Separately, `commit_chart_op` (2926) emits
  `Published` **without** calling `publish()`, so `Published` does not even imply a new generation.

  Individually each of these is a one-frame cosmetic tear. Collectively they mean there is no answer
  to "what does the UI see at generation N" — which is the question you must be able to answer to
  reason about this seam at all. *Direction:* one version stamp covering all four surfaces, and emit
  the repaint notification once, after everything is committed.

- **[`worker/run.rs:3578-3589` + IronCalc `user_model/common.rs:1596-1622`] The parallel undo stack's
  1:1 invariant with IronCalc's history is maintained by convention, not by construction — and there
  is already one latent violation.**

  `Worker::undo_stack` holds one `UndoEntry::Cell(Touch)` per IronCalc history entry, and *every*
  cache-invalidation decision on undo/redo depends on that alignment holding. Nothing checks it —
  there is no assertion, no depth comparison, no test that the two stacks agree after an arbitrary
  command sequence. Alignment is preserved by per-command reasoning recorded in comments, and the
  reasoning has to be redone (correctly) for each of the 30-odd undoable commands.

  Two pieces of evidence that this is not merely theoretical:
  1. `RaiseCondFmtPriority`/`LowerCondFmtPriority` needed a bespoke before/after rule-list comparison
     (run.rs:3670-3689) purely to avoid pushing a phantom worker entry when the engine records no
     diff. That is the failure mode, caught once.
  2. `Command::SetFrozen` with both `rows: Some` and `cols: Some` calls both
     `set_frozen_rows_count` and `set_frozen_columns_count`, each of which does its own
     `push_diff_list` — **two** engine history entries against **one** worker `Touch::Rebuild`. The
     comment at run.rs:3580 asserts "handling both defensively keeps the command total either way";
     that is wrong about the undo stack. It is latent only because the UI happens to send one axis
     (`grid/view.rs:4703`).

  When the invariant does break, the symptom is a silently stale style cache after Undo — the
  hardest class of bug to attribute. *Direction:* if the fork can expose history depth, assert it
  (debug-only is enough); otherwise make `apply_one` return the number of engine entries it created
  and push that many touches, so the arithmetic is explicit rather than assumed.

- **[`worker/run.rs:459-467`] There is no cancellation, no prioritisation, and effectively no
  backpressure — one blocking loop serialises everything.**

  Both channels are unbounded (`mpsc::channel`, `async_channel::unbounded`), so nothing ever blocks —
  but nothing can ever be interrupted either. A `Find`/`ReplaceAll` over a large sheet, a
  `Save` (zip + XML + chart re-inject), an `ExportCsv`, a big `evaluate()`, or the lazy chart-XML
  walk on first paint all run to completion on the one thread, during which **no publish happens** —
  the grid shows stale/blank cells while the user scrolls, with no feedback. `EvalStarted`/
  `EvalFinished` bracket only the eval paths; Find, Save, and ExportCsv emit nothing, so the UI
  cannot even show a spinner for them.

  Related: `Command::Shutdown` exists in the protocol but is **never sent by the application** (zero
  occurrences in `freecell-app`) — teardown relies on the command channel closing, which is only
  observed after the current batch finishes, and the thread is never joined. Closing a window during
  a long save leaves a detached thread writing a file into a process that may exit.

  *Direction:* at minimum, a shared `AtomicBool` cancel flag the long scans poll (`find_matches`,
  `selection_stats`, `replace_all_matches`, the publication probe) so a superseded request or a
  window close can abandon work; and bracket every long-running command with a start/finish event so
  the UI can show progress.

- **[`worker/protocol.rs:200-618`] 53 `Command` + 23 `WorkerEvent` variants is an RPC surface, not a
  document-mutation algebra — and it makes every feature a five-place change.**

  The vocabulary tracks UI affordances one-for-one: `FillDown`/`FillRight`/`FillDrag`,
  `PasteInternal`/`PasteValues`/`PasteTsv`, `RaiseCondFmtPriority`/`LowerCondFmtPriority`,
  `SetChartAnchor`/`SetChartType`/`SetChartRange`/`SetChartChrome`, `AutoGrowRowHeights`. Several
  carry UI concerns into the engine outright — `SetColumnWidths`/`SetRowHeights` are in **device
  pixels**, and `AutoGrowRowHeights` ships *render-thread text measurements* into the engine.

  Adding one feature requires touching: the `Command` enum, the `process_batch` bucketing match, the
  `apply_one` match (or a bespoke handler), the `op_of` match, the emitting UI, and the window's
  event fold — six sites, four of them in this crate. The exhaustive matches mean the compiler
  catches omissions, which is why this is Moderate rather than Critical, but the coupling is real:
  the engine's public contract cannot be understood without knowing the UI's menu structure.

  *Direction:* this needs an intermediate layer. Most of these are the same three shapes —
  "write values into a rectangle", "apply a style delta to a region", "structural op on an axis".
  Collapsing the fill/paste family and the style family onto a small algebra, with the
  UI-specific interpretation done UI-side, would cut the surface substantially and stop the enum
  growing linearly with the feature list.

- **[`worker/run.rs`] `run.rs` is a god-module. 3,984 production lines, ~15 distinct
  responsibilities, and roughly a third of it is chart machinery living in the eval-loop file.**

  The 43/57 production/test split is the good news — this is not 9k lines of logic. But `Worker` has
  **28 fields** and the module owns, in one file: command routing + coalescing; the paused-apply/eval
  cycle; `catch_unwind` + degraded policy; publication construction; the resident style-cache mirror
  and rebuild; a three-way row-height model (`manual_rows`, `wrap_heights`, IronCalc heights); CF
  publication and value-dependent CF invalidation; a unified undo/redo timeline spanning two
  different inversion mechanisms; a clipboard slot and four paste flavours; font application with
  row auto-grow; find/replace; lazy chart discovery (zip walking); chart binding/re-resolve; chart
  authoring (insert/anchor/type/chrome/range) with its own snapshot-based undo; chart-preserving save
  orchestration; xlsx save and CSV export.

  Roughly lines 2242-3210 and 3397-3448 (~1,000 lines) are chart logic that has nothing to do with
  the eval loop and would sit naturally behind a `ChartHost` type. Same for the clipboard slot and
  the row-height model. The dispatch loop itself (471-787) is clear and readable; it is drowning in
  its neighbours.

- **[`freecell-core/src/cache.rs:479-497`, `freecell-engine/src/cache.rs`] O(rows) and
  O(populated-cells) work sits behind ordinary edits, which undercuts the Excel-max claim.**

  - Every axis rebuild is `Axis::new(1_048_576, …)` — 1M invocations of a boxed closure, each doing a
    `BTreeMap::get` (`axis.rs:44-62` + `axis_from`). `set_row_height` rebuilds unconditionally;
    `set_row_heights` is batched and change-guarded (good), but any real height change pays the full
    1M-iteration walk. `build_and_store_cache` pays it twice (rows + cols) on *every* full rebuild.
  - Full rebuilds are common, not rare: sheet activation, every geometry op, every structural op,
    every merge/unmerge, every CF op, any band-creating style range, any range over
    `MAX_REFRESH_CELLS` — **and `refresh_cf_caches_after_recompute` (run.rs:1856) rebuilds the entire
    style cache of every resident CF sheet after every recompute.** On a large sheet with one
    conditional-formatting rule, every committed cell edit re-scans all populated cells, re-interns
    every style, and rebuilds both axes.

  The design is defensible for a few thousand populated cells. It is not obviously defensible at the
  scale the project claims, and nothing in the code bounds it. *Direction:* make the axis rebuild
  proportional to the override count (the block sums are recomputable from `overrides` +
  `default_px` arithmetic without visiting untouched blocks), and give the CF path an incremental
  invalidation instead of a wholesale rebuild.

- **[`worker/run.rs:342-349, 2022-2131, 1937-1957`] Row height has three sources of truth reconciled
  by hand-written `max()` in three separate places.**

  IronCalc's own row heights (font/newline auto-fit + user resize), the worker's `wrap_heights`
  (render-thread wrap measurements), and `manual_rows` (a session-only exemption set) combine as
  `manual ? base : max(base, wrap)` — implemented independently in `project_wrap_heights` (2027),
  `refresh_cache_cells` (1940-1956), and `apply_auto_grow` (2087-2106). The comment at 1929-1936
  documents a bug already caused by one of the three getting it wrong. On top of that,
  `AutoGrowRowHeights` is a **render-thread → worker → shared-cache → render-thread feedback loop**
  whose termination depends on an epsilon comparison (`AUTO_GROW_EPS_PX`) and a UI-side signature
  cache. Neither `manual_rows` nor `wrap_heights` is pruned when a sheet is deleted.

  This is the part of the engine I would expect to produce the next hard-to-reproduce bug.
  *Direction:* one function that computes a row's effective height from the three inputs, called from
  all three sites; prune the maps in the same place the caches map is reconciled (run.rs:1817-1824).

---

## Mild (consider fixing)

- **[`freecell-app/src/grid/view.rs:4462-4493`] A second, UI-side mutation path into the shared cache
  exists in the shipped binary.** `GridView::autogrow_measure_now` is `pub fn` (not `#[cfg(test)]`),
  takes `caches.write()`, and mutates row heights directly, bypassing the worker entirely. It is
  documented as a render-test hook and is not called by the app — but "the worker is the only writer"
  is the load-bearing invariant of this whole design, and it should be enforced by the type system or
  a feature gate, not by a doc comment.

- **[`worker/run.rs:2163-2168`] `probe_model` hardcodes sheet index 0.** The post-panic liveness probe
  reads `formatted_value(0, A1)` regardless of which sheet is active or which one panicked. It
  answers "is *some* sheet readable", not "is the model usable", which is the question the degraded
  policy actually needs.

- **[`worker/run.rs:2334-2352, 2436-2455`] Chart fidelity depends on external filesystem state.** The
  save path re-reads the *original file on disk* (`chart_source_path`) to re-inject chart parts, and
  lazy discovery re-opens it during a paint. If the file was moved, deleted, or modified externally,
  the save silently proceeds with a chart-less writer or with stale chart XML — a documented
  `tracing::warn!` and nothing user-visible. The in-memory model is not self-sufficient for
  round-trip fidelity, which is a surprising property for a document engine.

- **[`worker/client.rs:113-116`, `worker/run.rs:3387-3389`] Both channel endpoints swallow their
  errors.** `send` does `let _ = self.tx.send(cmd)` and `emit` does `let _ = try_send(event)`. Both
  are defensible individually, but together they are why the Critical worker-death case is
  invisible. Even a `tracing::error!` on the send failure would have surfaced it.

- **[`freecell-app/src/chrome/client.rs:113-127`] `published_cell` linear-scans `Publication::cells`.**
  O(published cells) per single-cell lookup. The grid builds a `HashMap` index per frame
  (`view.rs:3932`); the chrome does not. Small today (≤131k-cell cap), but it is the kind of thing
  that becomes a profile hit once the action bar reads more cells.

- **[`worker/run.rs:342-349`] Per-sheet worker maps are never pruned.** `manual_rows`,
  `wrap_heights`, `discovered_chart_sheets` accumulate entries for deleted sheets;
  `apply_cache_refresh` prunes only `Shared::caches` and `Shared::cond_fmt`. Small leak, and a
  restored (undo-of-delete) sheet inherits stale manual-row marks.

- **[`worker/protocol.rs:612-617`] `Command::Shutdown` is dead in production** (see the cancellation
  finding). Either wire it into window teardown with a join, or delete it so the protocol doesn't
  imply a graceful-shutdown story that doesn't exist.

---

## Phase Summary

The seam has a good spine and two serious holes. The parts that were designed as a *seam* — the
`ArcSwap` publication with publish-then-bump, the engine-free protocol types, the `pub(crate)`
IronCalc containment, the instrumented "zero engine calls on the render path" gate, the `Axis`
primitive, and the O(populated) read queries — are correct, well-reasoned, and worth keeping as-is.
Production panic discipline is genuinely excellent (effectively zero unguarded `unwrap`/`expect`
across ~6k lines).

The two Critical findings are both cases of the same failure: **an unbounded or unguarded path that
escaped an invariant the code otherwise takes seriously.** The frozen-pane band escaped the
publication clamp and can wedge the worker forever from a two-click UI action or a crafted file; and
`catch_unwind` covers only the mutation regions, so a panic in load, save, publish, chart parsing, or
cache build kills the thread into a completely silent zombie UI — no dialog, no log, no watchdog, no
join handle. Neither is exotic; both sit on paths that touch untrusted file input.

Structurally, the recurring theme is **one well-designed surface (the publication) and three
after-thoughts around it.** The style cache, chart snapshot, and CF map each got their own
synchronisation primitive, their own version discipline (or none), and their own commit point, so
there is no coherent answer to "what does the UI see at generation N", and the cache's `RwLock`
gives the render thread neither wait-freedom nor frame consistency while the worker holds the write
lock across up to 100k engine reads. Meanwhile the protocol has grown to 53+23 variants that mirror
the UI's menus one-for-one, and `run.rs` has absorbed ~15 responsibilities — a third of its
production code is chart machinery — because there is no layer between "the eval loop" and "every
feature". The scalability claim is supported by the primitives (`Axis`, the `sheet_data` walks) but
undercut by the cache: O(1M) axis rebuilds on any geometry change, and a full populated-cell rescan
after *every recompute* on any sheet carrying a conditional-formatting rule.

None of this is unrecoverable, and the exhaustive typed dispatch means refactoring is safe. But I
would fix the two Criticals before anything else ships, and I would not add another feature to
`run.rs` before extracting the chart host and giving the four shared surfaces one commit point.
