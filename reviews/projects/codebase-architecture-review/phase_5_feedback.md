# Phase 5: Persistence, data fidelity & IronCalc coupling

Scope: the file-in/file-out path — `freecell-engine::{document, document::cond_fmt,
cond_fmt_convert, formula_refs, fixtures, chart::save}`, the `freecell-core` model types,
the app's open/save/backup flow, the round-trip test suite, and the `[patch.crates-io]` fork
dependency. I read the pinned fork's own importer/exporter (`~/.cargo/git/checkouts/
ironcalc-9be9638f021aeb24/cee2859/xlsx/src/{import,export}`) to answer the preservation
question from code rather than from the repo's notes, since that is the only place the real
answer lives.

---

## What's Good

- **[app/crates/freecell-engine/src/document.rs:2088] Save durability is genuinely correct.**
  `persist_atomically` writes to a `NamedTempFile` created *in the destination directory*
  (`new_temp_beside`, :2078 — guaranteeing same-filesystem rename), `sync_all()`s data **and**
  metadata, then `persist()`s over the target. Both save entry points (`save` and
  `write_xlsx_bytes_atomic`, the chart-reinject path) share it, so they cannot diverge. This
  is better than most shipping document editors and the failure-mode tests
  (`tests/roundtrip.rs:157`, `:186`) inject failure root-proof (EISDIR / ENOTDIR) rather than
  relying on permission bits. Table stakes met.

- **[app/crates/freecell-app/src/shell/lifecycle.rs:39] The one-time `.back` backup is the
  right instinct.** `backup_target` is precise: only when opened-from-disk, only when saving
  to that same path, write-once across sessions. And `window.rs:979` makes a backup *failure*
  abort the save with a dialog rather than proceeding — data safety over convenience. Given
  how lossy the save actually is (Critical #1 below), this is the single thing standing
  between a user and permanent loss of their pivot tables.

- **[app/crates/freecell-engine/Cargo.toml:19] IronCalc is contained to exactly one crate.**
  Measured: 139 `ironcalc` references across 12 files, all in `freecell-engine/src`. The
  references in `freecell-core`, `freecell-chart-model` and `freecell-app` are **comments and
  an About-box URL** — zero type leakage. The `const _: fn()` `Send` assertion at
  `document.rs:193` and the "no `ironcalc` type crosses this boundary" discipline are actually
  held. This is unusually good containment and it is the thing that makes the fork position
  survivable at all.

- **[app/crates/freecell-engine/src/cond_fmt_convert.rs:245] The CF converter's read-only
  degradation is a well-designed pattern.** `color_scale_stops` returns `None` for a `Formula`
  cfvo or a `Color::Theme` stop, and `cf_rule_to_view` turns that into a non-editable
  `badge_view` — *specifically* so a later edit can't overwrite the file's theme colours. The
  comment states the invariant explicitly. Likewise `merge_cf_format_into_dxf` (:73) folds
  onto the *existing* `Dxf` so unmodelled `border`/`num_fmt`/`alignment`/`strike`/`u`/`sz`
  survive an edit. Both are the correct shape for a partial-model editor: model a subset,
  preserve the rest, refuse to edit what you'd damage.

- **[app/crates/freecell-core/src/style.rs:38] `RenderStyle` is a projection, not a parallel
  model.** Its doc comment is explicit that fields the grid doesn't paint are "preserved in
  the engine and on save, never in this render form". This is the correct answer to the
  duplication question for styles — it is a render cache keyed to what the painter needs, not
  a second source of truth. The same is true of `format_ui.rs` (classifies format *code
  strings*, formats nothing) and `format_color.rs`.

- **[app/crates/freecell-engine/src/document.rs:2114] Open-failure classification is honest
  work.** `classify_magic` reads magic bytes *before* handing to the engine, so
  not-xlsx / OLE(password-or-legacy-`.xls`) / damaged-zip become three distinct typed
  `LoadError`s with accurate user-facing sentences, instead of one generic zip error. The
  short-read loop is correct. The typed matrix is tested (`tests/roundtrip.rs:207`–`:257`).

- **Zero `unwrap`/`expect`/`panic!` in FreeCell's own non-test load/save code.** I counted:
  `document.rs`, `document/cond_fmt.rs`, `cond_fmt_convert.rs`, `chart/load.rs`,
  `chart/write.rs`, `formula_refs.rs`, `open_files.rs`, `recents.rs`, `recent.rs` — all zero
  outside `#[cfg(test)]`. The panic risk on this path is entirely inherited from the engine,
  not authored here. Credit where due.

---

## Critical (must fix)

- **[app/crates/freecell-engine/src/chart/save.rs:518] Save is lossy-by-default: everything
  except cells, styles, CF, defined names and charts is silently destroyed.** This is the most
  important fact about this codebase and it is not surfaced anywhere the user can see it.

  The mechanism is structural, not a bug: IronCalc's exporter
  (`xlsx/src/export/mod.rs::save_xlsx_to_writer`) does not rewrite the input package — it
  **synthesizes a new zip from the model**, containing exactly eleven parts
  (`[Content_Types].xml`, `docProps/{app,core}.xml`, `_rels/.rels`,
  `xl/{sharedStrings,styles,workbook,metadata}.xml`, `xl/theme/theme1.xml`,
  `xl/_rels/workbook.xml.rels`, `xl/worksheets/sheetN.xml`). FreeCell's only preservation
  layer is `is_carry_part`, which carries back exactly two prefixes:

  ```rust
  fn is_carry_part(name: &str) -> bool {
      name.starts_with("xl/charts/") || name.starts_with("xl/drawings/")
  }
  ```

  Concretely, opening a real `.xlsx`, editing one cell, and saving **permanently deletes**:
  pivot tables and their caches (`xl/pivotCache/*`, `xl/pivotTables/*`); VBA macros
  (`xl/vbaProject.bin`); data validation (never imported — no `dataValidation` reader exists
  in the fork); hyperlinks (never imported); cell comments/notes (imported into
  `Worksheet.comments` but **never written back** — no exporter touches them); Excel Tables
  (imported by `xlsx/src/import/tables.rs`, no `xl/tables/*` part in the exporter's list);
  autofilters; sheet protection; page setup, print areas, headers/footers; external links;
  form controls, slicers, sparklines; custom XML parts and `docProps/custom.xml`; and every
  embedded image that isn't part of a chart drawing. Rich-text runs inside a cell are also
  flattened — `export/shared_strings.rs` emits `<si><t>…</t></si>` unconditionally, so mixed
  formatting within one cell's text is lost.

  The preservation model is therefore **allowlist-by-prefix over a regenerated package**, which
  is the weakest possible posture for a product whose pitch is Excel compatibility. Nothing in
  the app warns the user. `roundtrip.rs` never notices, because it only round-trips workbooks
  FreeCell itself authored (see Critical #4). The `.back` backup is the only safety net, and it
  is silent, one-shot, and leaves clutter next to the user's files.

  *Direction:* the preservation seam already exists — `chart/save.rs::reinject` reads the
  original package, filters parts, merges `[Content_Types]` overrides, and patches worksheet
  XML. Generalise it from "two chart prefixes" to a **default-carry / explicit-drop** policy:
  carry every original part IronCalc does not itself regenerate, drop only the parts whose
  content the model authoritatively owns. That will not be perfect (a pivot cache referencing
  a deleted row is stale), but stale beats deleted, and it converts an unbounded loss into a
  bounded, enumerable one. Whatever the outcome, the app must tell the user at save time which
  original features it could not preserve — silence here is the actual defect.

- **[app/crates/freecell-engine/src/worker/run.rs:369] and [:757] The load and save paths are
  the *only* engine calls not wrapped in `catch_unwind`, and a panic there strands the window
  permanently.** Edits are carefully guarded (`run.rs:862`, `:1101`, `:1166`, `:1264`, `:1521`,
  `:1634` — six `catch_unwind` sites, plus a `degraded` mode and a `panic_count`). Load and
  save have none:

  ```rust
  // run.rs:369  — thread entry, no guard
  let doc = match WorkbookDocument::from_source(&source) { … };
  // run.rs:757  — no guard
  for (path, req_id) in saves { match self.save_workbook(&path) { … } }
  ```

  A panic in either unwinds out of `Worker::load_and_run`, killing the `eval-worker` thread.
  `DocumentClient::send` is documented as "infallible to the caller: if the worker is gone the
  send is dropped" (`worker/client.rs:118`), and the window's event loop
  (`shell/window.rs:351`) simply falls out of `while let Some(event) = receiver.recv().await`
  and **exits with no notification**. Result:

  - *Panic during load* → `self.loading` is never cleared (`window.rs:463`/`:477` are the only
    clear sites), so the window shows "Opening Budget.xlsx…" forever. No error, no recovery.
  - *Panic during save* → no `Saved`, no `SaveFailed`. The document stays dirty, the title keeps
    its edited dot, every subsequent command is silently dropped, and there is **no way to save
    the user's work**. That is a data-loss path, not just a UX bug.

  These are not hypothetical panics. The pinned engine contains reachable ones on both paths
  — see the two findings below.

  *Direction:* wrap both in `catch_unwind` like every other engine call, and map a caught panic
  to `LoadFailed`/`SaveFailed` so the existing dialogs fire. Separately, the window's event
  task must treat *channel closure while a save/load is pending* as a hard error and say so.

- **[`~/.cargo/git/checkouts/ironcalc-9be9638f021aeb24/cee2859/xlsx/src/export/worksheets.rs:177`]
  The pinned exporter contains an explicit `panic!` on the save path, and FreeCell's
  no-eval-on-open design is what can reach it.**

  ```rust
  Cell::CellFormula { v: FormulaValue::Unevaluated, .. } | Cell::ArrayFormula { … } => {
      // TODO: We should NOT panic here.
      panic!("Model needs to be evaluated before saving!");
  }
  ```

  `document.rs:212` states the design: "Opens an `.xlsx` file into a workbook. Does **not**
  evaluate — first paint uses the file's cached values (SP2)." The worker only calls
  `doc.evaluate()` inside an edit batch (`run.rs:915`, `:1106`, `:1171`, `:1526`). So a
  document that is opened and saved — or whose committed edits leave any cell in the transient
  `Unevaluated` state — reaches an unguarded `panic!` inside an unguarded `save_workbook`,
  producing exactly the stranded-window failure above. Beyond that one line, the whole
  exporter is `#![allow(clippy::unwrap_used)]` (`export/{mod,workbook,worksheets}.rs:1`) with
  `.unwrap()`s on worksheet lookup and column-name conversion.

  *Direction:* short term, guard the save (previous finding) so this degrades to a
  "couldn't save" dialog rather than a zombie window. Medium term this belongs in the fork as
  a `Result`, not a `panic!` — it is upstream's own TODO.

- **[app/crates/freecell-engine/tests/roundtrip.rs:16] The round-trip suite proves the writer
  and reader agree with each other, not that Excel content survives.** Every `save_and_reopen`
  call takes a `fixtures::*` document — a workbook FreeCell built in memory through
  `set_user_input` (`src/fixtures.rs:19`). Nothing in the suite opens a real third-party
  `.xlsx`, saves it, and reopens it.

  The repo *has* real fixtures — `personal_monthly_budget.xlsx` (an unmodified real Excel
  template), `numbers_table.xlsx`, `FONTS.xlsx`, `dates.xlsx`,
  `libreoffice_custom_height_wrap.xlsx` — and the tests that use them
  (`personal_monthly_budget_fixture.rs`, `open_numbers_fixture.rs`, `dates_fixture.rs`) assert
  **open** behaviour only. Not one of them saves. The `roundtrip` CI workflow
  (`.github/workflows/roundtrip.yml`) is charts-only (`charts_roundtrip_libreoffice`), so the
  one place where FreeCell's output is validated by a *different* spreadsheet application
  covers a single line chart.

  This is why Critical #1 could exist for the whole life of the project without a test going
  red: the suite is a closed loop over IronCalc's own serialization. It proves the app doesn't
  crash and that FreeCell-authored values/formulas/formats survive. It proves nothing about
  fidelity.

  *Direction:* add an open→save→reopen fidelity test over the existing real fixtures that
  asserts a **part-level inventory** of the original vs. the saved zip (what survived, what
  didn't) and fails on regression. That single test would have turned Critical #1 into a
  visible, tracked, bounded list on day one. Extend the LibreOffice gate beyond charts.

---

## Moderate (should fix)

- **[app/crates/freecell-engine/src/cond_fmt_convert.rs:73] A CF rule imported with a *theme*
  fill or font colour is marked editable, and editing it silently destroys the colour.** The
  converter guards this correctly for colour scales and not at all for the highlight family.

  `dxf_to_cf_format` (:116) maps `Color::Theme(..)` → `None` (via `color_to_rgb`, :41). But
  `highlight_view` (:712) hardcodes `editable: true` for every `CellIs`/`Text`/`Top10`/
  `Bottom10`/`AboveAverage`/`DuplicateValues`/`Blanks`/`Errors`/`Formula` rule regardless of
  what its `Dxf` actually held. When the user edits such a rule — even just changing the
  operator — `update_cond_fmt` (`document/cond_fmt.rs:46`) calls:

  ```rust
  existing.fill = fmt.fill.map(|rgb| Fill { color: rgb_to_color(rgb) });   // :74
  … color: fmt.text_color.map(rgb_to_color).unwrap_or(Color::None),        // :109
  ```

  `fmt.fill` is `None` (it was a theme colour), so `existing.fill` becomes `None` and the
  theme font colour becomes `Color::None`. The file's original colour is gone.

  The author already reasoned through exactly this hazard 170 lines earlier, at :245: *"a
  non-RGB stop colour … Returning `None` for a theme colour is load-bearing: coercing it to a
  concrete `Rgb` here would let a later edit+save overwrite the file's original theme
  colours."* The protection was applied to `ColorScale` and not carried to the highlight
  family, whose merge path is the one that actually writes back.

  *Direction:* mirror the colour-scale guard — a highlight rule whose `Dxf` carries a non-RGB
  fill or font colour becomes a `badge_view` (read-only), or `merge_cf_format_into_dxf` learns
  to leave `fill`/`color` untouched when the incoming `CfFormat` field is `None` *and* the
  existing value is `Color::Theme`.

- **[app/Cargo.toml:105] The `[patch.crates-io]` position is stated as `=0.7.1` but is
  actually an unreleased, unreviewed, single-maintainer branch — and the tracking doc
  understates the dependency by ~5×.** The workspace pins `ironcalc = "=0.7.1"` and then
  patches both crates to `git+https://github.com/scosman/ironcalc?branch=freecell-fixes`.
  Positives first, honestly: `Cargo.lock` **does** pin a SHA
  (`cee2859dceda65ff64e52192be4ec47a259870e1`), so a `cargo build` today is reproducible, and
  the fork's version strings still say `0.7.1`.

  That version string is fiction. `specs/projects/ironcalc-upstreaming/fork_audit.md` records
  that upstream `main` replaced the whole style-colour API (`Fill { pattern_type, fg_color,
  bg_color }` → `Fill { color: Color }`) and that FreeCell **migrated to the new API** — so
  FreeCell cannot compile against the crates.io `0.7.1` it nominally pins. Remove the
  `[patch]` and the build does not degrade; it fails.

  The depth is larger than the project tracks. `implementation_plan.md`'s status table lists
  two items (E2 num-fmt, E5 indexed colours). The branch actually carries **ten** `fix/*`
  merges, several of which are load-bearing product features, not bug fixes:
  `merged-cells core/model implementation` (a four-phase feature — FreeCell's
  `merge_cells`/`unmerge_cells`/`merged_regions`/`merge_would_lose_data` are built on it),
  `UserModel::set_user_inputs` (the batched single-undo write the whole paste/replace path
  depends on), `set_worksheet_index` (sheet reorder), font-`<name>` import, xlsx boolean-attr
  import, frozen-pane structural edits, plus `TRIM`/`DOLLAR`/`ADDRESS`/`XMATCH` correctness
  fixes with committed regression tests in `document.rs`. **Zero upstream PRs are merged** —
  the status table shows both tracked items "⏳ awaiting sign-off".

  Risk assessment as an owner: build reproducibility is fine *today* but rests on one personal
  GitHub account; `freecell-fixes` is an integration branch that is rebased when upstream
  `main` moves, and a rebase or force-push makes the locked SHA unreachable (git GC) — at
  which point the project cannot build from a clean checkout. `cargo update` silently moves
  the pin. There is no vendored copy and no `patches/` coverage for eight of the ten fixes
  (`specs/projects/ironcalc-upstreaming/patches/` holds two). The stated exit — "upgrade to a
  release containing all the fixes" — currently has no path, because merged cells and the
  batched-input API are features upstream has not agreed to take.

  *Direction:* three cheap, high-value moves, independent of the upstreaming question.
  (1) Pin the **SHA** in `Cargo.toml`, not the branch, so the dependency cannot move under
  you. (2) Mirror the fork (a second remote, or `cargo vendor` into the repo) so a lost
  upstream account is an inconvenience, not an extinction event. (3) Make
  `implementation_plan.md`'s status table reflect all ten fixes, ranked by *how hard FreeCell
  breaks without each* — that is the document a future owner needs, and right now it is
  ~20% complete.

- **[app/crates/freecell-engine/src/document.rs:216] A formula-bearing file with no cached
  values silently renders — and re-saves — every formula as `0`.** By design FreeCell does not
  evaluate on open. When IronCalc imports a `<c>` with an `<f>` but no `<v>`, it takes the
  `"n"` branch and does `cell_value.unwrap_or("0").parse::<f64>().unwrap_or(0.0)`
  (`xlsx/src/import/worksheets.rs:487`) — the formula becomes a `Number(0.0)`. Files written by
  most non-Excel tooling (openpyxl and friends), and any file saved with recalc-on-load, have
  no cached values.

  So `=SUM(A1:A100)` displays `0`, the user has no signal that anything is wrong, and if they
  save, `0` is written out as the cached value into a package whose `<calcPr/>`
  (`export/workbook.rs:93`) carries no `fullCalcOnLoad`. The wrong number propagates into a
  file other applications will read. Wrong-and-silent is worse than a slow open.

  *Direction:* detect the no-cached-values case at load (cheap — the importer already knows)
  and force one `evaluate()` before first publish, exactly as `import_csv` already does
  (`document.rs:338`).

- **[app/crates/freecell-engine/src/document.rs:216] The formula input cap protects typed
  input but not opened files, so a hostile or merely pathological `.xlsx` can `SIGABRT` the
  process.** `freecell-core/src/input_cap.rs:1` documents the threat model precisely: IronCalc's
  parser and evaluator are recursive with no depth cap, a stack overflow is an **abort that
  `catch_unwind` cannot catch**, and "the only real fix is to reject over-budget formulas
  before they reach the engine". The cap is applied in the UI and re-checked worker-side
  (`run.rs:790`) — but `open()` hands the path straight to `load_from_xlsx`, which parses every
  formula with the same recursive parser (`import/worksheets.rs:305`,
  `import/mod.rs:86` for defined names). No cap, no depth budget, on the most obviously
  untrusted input the app accepts. The 64 MiB worker stack raises the ceiling; it does not
  eliminate the class, which is the standard the module itself sets.

  *Direction:* apply the same length/depth budget to formulas arriving from a file — reject or
  neutralise the offending cell with a typed `LoadError`/warning rather than letting the parser
  decide.

- **[app/crates/freecell-app/src/shell/window.rs:979] No external-modification detection: a
  save silently clobbers changes made outside FreeCell.** `send_save` writes to
  `self.path`/`opened_from` with no record of the file's mtime, size, or hash at open time, and
  nothing re-checks at save. Open `Budget.xlsx` in FreeCell, edit it in Excel or a script, save
  from FreeCell → the other application's work is gone, with no prompt. Every mature document
  editor checks this. The path is already canonicalised at open (`app.rs:291`), so the hook
  point exists.

  *Direction:* stamp mtime+len at open and after each successful save; on a mismatch at save
  time, prompt (Overwrite / Save As / Reload).

- **[`~/.cargo/git/checkouts/…/xlsx/src/import/worksheets.rs:230`] A cell comment containing an
  empty text run panics the importer.**

  ```rust
  .map(|n| n.text().unwrap().to_string())
  ```

  A `<t/>` element inside a comment part yields `None` from `roxmltree`'s `text()` and this
  aborts the load. Excel does emit empty runs. Combined with the unguarded load path (Critical
  #2) this is a real-file → permanently-stuck-window path. Note the irony: comments are parsed
  (and can crash the open) but are never written back on save (Critical #1).

  Two neighbouring `.unwrap()`s on attribute access sit in `import/util.rs:55`/`:64`/`:88`,
  guarded by `has_attribute` checks — those are fine.

- **[app/crates/freecell-engine/src/document.rs:2078] Save-in-place silently changes the file's
  permissions.** `NamedTempFile::new_in` creates the staging file `0600` on Unix; the rename
  carries that mode onto the destination. A workbook that was `0644` (or group-writable in a
  shared directory, or carrying macOS extended attributes / Finder tags) comes back `0600` with
  its xattrs dropped. This is a standard atomic-replace pitfall and the fix is mechanical: stat
  the destination before the rename and restore mode/ownership onto the temp file. Also worth
  fsyncing the destination *directory* after `persist()` — without it the rename itself isn't
  durable across power loss on several filesystems, which partially undercuts the otherwise
  careful `sync_all()`.

---

## Mild (consider fixing)

- **[app/crates/freecell-engine/src/document.rs:90] "Damaged" is the wrong word for
  "unsupported".** `LoadError::Corrupt` reads *"This workbook appears to be damaged and can't be
  opened"*, and `open()` (:245) funnels every non-IO engine error into it — including
  `XlsxError::NotImplemented`, which the fork raises for Excel What-If Data Tables
  (`import/worksheets.rs:1042`). A user with a perfectly healthy workbook is told their file is
  damaged. Split the variant: `Unsupported(feature)` with its own sentence.

- **[app/crates/freecell-app/src/shell/lifecycle.rs:29] The `.back` backup has no lifecycle.**
  It is created once, next to the user's file, in the same directory they browse in Finder, and
  is never cleaned up, never surfaced in the UI, and never offered as a restore. As the sole
  mitigation for Critical #1 it deserves a "Revert to original" menu entry, or at minimum a
  mention in the save dialog — otherwise most users will never learn it exists, and the ones
  who do will find `Budget.xlsx.back` and delete it as junk.

- **[app/crates/freecell-core/src/recent.rs:102] The recents store is written non-atomically.**
  `fs::write` truncates then writes; a crash mid-write leaves a truncated JSON file. Harmless
  in effect (`from_json` degrades to an empty list, :78) but it is the same
  temp-plus-rename primitive already available two crates away, and using it costs nothing.

- **[app/crates/freecell-core/src/sheet_name.rs:33] and [format_color.rs:23] Two small,
  deliberate reimplementations of engine rules carry quiet drift risk.**
  `validate_sheet_name` re-encodes Excel's naming constraints UI-side, and `is_date_format` is a
  hand-rolled heuristic over format codes used to reclassify a cell as a date. Both are
  reasonable calls (the first is re-checked worker-side; the second only affects default
  alignment). But both will silently diverge if the engine's rules move, and neither has a test
  that pins it *against the engine*. A single cross-check test each would convert a silent
  divergence into a red build.

- **[app/crates/freecell-engine/src/document.rs:285] CSV import type-coerces every field with
  no opt-out.** Each non-empty field goes through `set_user_input`, so `01234` becomes `1234`,
  `1/2` becomes a date, and a leading `=` becomes a formula. Excel behaves the same way, so this
  is defensible as a default — but with no "keep as text" affordance anywhere, a CSV of ZIP
  codes or part numbers is corrupted on import with no way to recover but re-typing.

---

## Phase Summary

The persistence layer is a study in contrasts. The *mechanics* of writing a file are excellent:
atomic temp+fsync+rename shared by both save paths, root-proof failure tests, a well-judged
one-time `.back` backup that aborts the save if it can't be taken, typed open errors driven by
magic-byte classification, and zero panic sites in FreeCell's own I/O code. The IronCalc
containment is likewise better than I expected — 139 engine references, all inside
`freecell-engine`, with `freecell-core` and `freecell-app` genuinely engine-free. If IronCalc had
to be replaced, the blast radius is one crate (~15k production lines), not the product.

What is written, however, is a fraction of what was read. IronCalc's exporter regenerates the
`.xlsx` package from its model — eleven parts, nothing else — and FreeCell's only preservation
layer is a two-prefix allowlist for charts and drawings. Opening a real workbook, editing one
cell, and saving permanently deletes pivot tables, macros, data validation, hyperlinks, comments,
Excel Tables, autofilters, sheet protection, print setup, images, and in-cell rich text, with no
warning at any point. The round-trip suite cannot see this because it only round-trips workbooks
FreeCell itself authored — a closed loop over IronCalc's own serializer — while the five real
Excel fixtures already in the repo are used for *open* assertions only. Layered on top, the load
and save paths are the only two engine calls without a `catch_unwind` guard, and the pinned
exporter contains a literal `panic!("Model needs to be evaluated before saving!")` on a state
FreeCell's deliberate no-eval-on-open design can produce; when that fires, the worker thread dies,
the window's event loop exits silently, and the user is left with a dirty document that can never
be saved.

The fork position is survivable but under-managed. `Cargo.lock` does pin a SHA, so builds are
reproducible today — but the `=0.7.1` pin in `Cargo.toml` is fiction (FreeCell has migrated to a
post-0.7.1 API and cannot compile against the released crate), the branch carries ten `fix/*`
merges rather than the two the project's own status table tracks — including an entire merged-cells
feature and the batched-input API the paste path depends on — and zero of them are upstream. A
rebase of `freecell-fixes` can orphan the locked SHA; there is no mirror and no vendored copy.
Pinning the SHA in `Cargo.toml`, mirroring the fork, and making the status table honest are three
hours of work that materially de-risk the whole project.

**Counts:** Critical 4 · Moderate 7 · Mild 5.
