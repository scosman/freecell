# Research: IronCalc links API + fork sync cost

Point-in-time research (2026-08-07) against upstream `ironcalc/IronCalc` main
`91d343c3b3a4c579621fda559f60d81784a5b266`, our fork `main` `60d9db19`, and
`origin/freecell-fixes` `23443cd5`. Line numbers are upstream-main-relative.

> Provenance note: the links work is **not** a single merge commit. `eac0034` (cited in the
> PR discussion) is a plain webapp commit; the feature landed as ~30 linear commits from
> `4762b486` ("Adds Links and links API in the engine") through `91d343c3`.

## 1. Types

### `Link` — `base/src/types.rs:182`

```rust
#[derive(Serialize, Deserialize, Encode, Decode, Debug, PartialEq, Eq, Clone)]
#[serde(tag = "type")]
pub enum Link {
    External { target: String, tooltip: Option<String> },
    Internal { location: String, tooltip: Option<String> },
}
```

- `External` covers URLs, `mailto:`, and files. A target pointing inside another document is
  stored as one string, `target#location` (e.g. `file.xlsx#Sheet1!A1`).
- `Internal` is a location in this workbook: `"Sheet1!A30"` or a defined name.
- **Display text is not part of the link** — it is the cell's content. Links are pure cell
  metadata.
- **No scheme validation anywhere in the engine.** `set_cell_link`, XLSX import, and
  `HYPERLINK()` all accept arbitrary strings. The allowlist
  (`http, https, ftp, ftps, file, mailto`) exists **only** in the webapp, at
  `webapp/IronCalc/src/components/LinkDialog/util.ts:1`. See §7 — FreeCell must reimplement it.

### Storage — `base/src/types.rs:223`

```rust
pub struct Worksheet {
    ...
    pub links: HashMap<(i32, i32), Link>,   // keyed by (row, column)
}
```

One link per cell. An XLSX `<hyperlink ref="B2:C3">` range is expanded to one entry per cell
on import.

### Dynamic (formula) links — `base/src/model.rs:226`

```rust
pub(crate) links: HashMap<(u32, i32, i32), Link>,   // (sheet, row, column)
```

`pub(crate)`, **cleared and rebuilt on every `evaluate()`** (`model.rs:3049`), never persisted.
This is where `HYPERLINK()` results live.

### `CellLinkView` — `base/src/links.rs:56`

```rust
pub struct CellLinkView {
    pub row: i32,
    pub column: i32,
    pub dynamic: bool,
    #[serde(flatten)] pub link: Link,
}
```

## 2. Rust method surface

### `Model` — `base/src/links.rs` (raw, **not** undoable)

| Signature | Line |
|---|---|
| `get_cell_link(&self, sheet: u32, row: i32, column: i32) -> Result<Option<Link>, String>` | `:87` |
| `set_cell_link(&mut self, sheet, row, column, link: Link) -> Result<(), String>` | `:98` |
| `delete_cell_link(&mut self, sheet, row, column) -> Result<(), String>` | `:115` |
| `get_links(&self, sheet) -> Result<&HashMap<(i32,i32), Link>, String>` | `:125` |
| `get_links_list(&self, sheet) -> Result<Vec<CellLinkView>, String>` | `:173` |
| `pub(crate) auto_link_cell(&mut self, sheet, row, column, value: &str)` | `:136` |
| `pub(crate) fn detect_link_target(value: &str) -> Option<String>` (free fn) | `:20` |
| `pub(crate) const THEME_COLOR_HYPERLINK: i32 = 10;` | `:15` |

### `UserModel` — `base/src/user_model/links.rs` (**undoable**)

```rust
pub fn get_cell_link(&self, sheet, row, column) -> Result<Option<Link>, String>          // :10
pub fn get_links(&self, sheet) -> Result<&HashMap<(i32,i32), Link>, String>              // :15
pub fn get_links_list(&self, sheet) -> Result<Vec<CellLinkView>, String>                 // :20
pub fn set_cell_link(&mut self, sheet, row, column,
                     link: Link, label: Option<&str>) -> Result<(), String>              // :34
pub fn delete_cell_link(&mut self, sheet, row, column) -> Result<(), String>             // :114
```

`set_cell_link`'s `label: Option<&str>` also sets the cell's content in the **same undo step**
— exactly what an "insert link with display text" dialog wants.

Undo variant — `base/src/user_model/history.rs:286`:

```rust
SetCellLink { sheet: u32, row: i32, column: i32,
              old_value: Box<Option<Link>>, new_value: Box<Option<Link>> },
```

Undo `undo_redo.rs:625`, redo `:1067`. `set_cell_link` bundles up to three diffs
(`SetCellLink` + optional `SetCellValue` for the label + `SetCellStyle` for the link styling)
into **one** `push_diff_list` — a single undo step. No-ops are suppressed.

**Style side effect (important for rendering):** creating a *new* link applies
`style.font.u = true` and `style.font.color = Color::Theme(10, 0.0)` to the cell. It is
ordinary formatting — `delete_cell_link` does **not** remove it
(`user_model/links.rs:112` says so explicitly). Dynamic `HYPERLINK()` links get **no** style.

### Critical asymmetry

`get_cell_link` / `get_links` are **worksheet-links only** — they deliberately exclude dynamic
`HYPERLINK()` links. `get_links_list` is the union (worksheet wins on collision), sorted by
`(row, column)`.

There is **no per-cell call that covers both**. The webapp's pattern
(`webapp/IronCalc/src/components/WorksheetCanvas/cellLinks.ts:36`) is: call the union list
once per sheet, build a map, look up per cell, refresh after every evaluation.

> Naming trap: the WASM binding exports `get_links_list` under the JS name `getLinks`
> (`bindings/wasm/src/lib.rs:742`), so JS `getLinks` is the union while Rust `get_links` is
> worksheet-only.

## 3. Structural displacement — links DO move

`base/src/actions.rs:14` adds `fn displace_links<F>(worksheet, map)`, called from
insert_columns `:559`, delete_columns `:659`, insert_rows `:913`, delete_rows `:993`,
move_column_unchecked `:1029`, move_row_unchecked `:1184`. Deleted rows/cols drop their links;
everything after shifts.

Row/column **move** is subtler: links are lifted out before the cell rebuild and re-attached
after (`actions.rs:1152`, `:1302`), with a `retain` first — because the rebuild goes through
`set_user_input`, which would auto-link URL-shaped values and fabricate a link at the
destination.

`Model::delete_rows` destroys links irreversibly, so `UserModel::delete_rows` /
`delete_columns` snapshot them into `SetCellLink` diffs first (`user_model/common.rs:1161`,
`:1238`) via `range_link_diffs` (`common.rs:842`).

Other paths that touch links: clearing a cell removes its link (`model.rs:2384`);
`range_clear_contents` / `range_clear_all` drop links in the area (`model.rs:3147`, `:3261`,
undo captured at `common.rs:784`, `:824`); clipboard `ClipboardCell` gained
`#[serde(default)] link: Option<Link>` (`user_model/clipboard.rs:34`) with copy `:68`, paste
re-applying links *after* values so they override auto-linking `:231`, cut clearing the source
`:281`; autofill copies the source link to each filled cell (`user_model/autofill.rs:372`).

## 4. `HYPERLINK()`

New enum variant `Function::Hyperlink` at `base/src/functions/mod.rs:169`, lookup `:719`,
localized name `:1247`, `into_iter()` const generic bumped `IntoIter<Function, 494>` →
`495` (`:1611`), dispatch `:2373`; `Functions { pub hyperlink: String }` in
`base/src/language/mod.rs:88`; regenerated `language.bin`; static analysis at
`expressions/parser/static_analysis.rs:1054` (`args_signature_scalars(arg_count, 1, 1)`) and
`:1705` (`StaticResult::Scalar`).

Eval — `base/src/functions/lookup_and_reference/mod.rs:702`:

- `HYPERLINK(link_location, [friendly_name])`; 1–2 args else `#ERROR!`.
- **Returns** `friendly_name` if given, else `link_location` — so the cell *displays* the
  friendly name.
- `#` prefix on `link_location` ⇒ `Link::Internal`, otherwise `Link::External`. `tooltip` is
  always `None`.
- If the display value is an error, no link is attached and the error is returned.
- Side effect: inserts into `Model::links`, i.e. it takes `&mut self` during evaluation.

**Dynamic** ⇒ `get_cell_link` returns `None` for a HYPERLINK cell; the link is not editable or
deletable, only the formula is. Confirmed by `base/src/test/test_fn_hyperlink.rs:34`.

## 5. Auto-linking is in the ENGINE, not the webapp

`Model::set_user_input` calls `auto_link_cell` at `base/src/model.rs:2461`:

```rust
None => {
    self.set_cell_with_string(sheet, row, column, &value, new_style_index)?;
    // If the input looks like an URL or an email address a link is
    // attached to the cell, the same way other inputs change the
    // number format. Note that a quote prefix prevents this.
    self.auto_link_cell(sheet, row, column, &value)?;
}
```

**`set_user_input("http://x.com")` alone is enough** — FreeCell inherits auto-linking for free
and must NOT reimplement URL detection for typed input.

Detection (`base/src/links.rs:20`): value trimmed; rejected if it contains whitespace; accepted
for `http:// https:// ftp:// ftps:// mailto:` (case-insensitive prefix, original case kept);
`www.x.y` gets `https://` prepended; bare `local@domain.tld` becomes `mailto:...`. Rejects
`https://` alone, `www.`, `www.example`, `a@b@c.com`, `daniel@localhost`.

- Fires **only in the string branch**, after number/boolean/error/formula parsing all fail
  (`model.rs:2420`). Numbers, dates, and formulas are never auto-linked.
- A **quote prefix** (`'http://x.com`) short-circuits at `model.rs:2390` — the user's escape
  hatch.
- An existing link has only its *target* updated; the link styling is applied only for a
  brand-new link (`links.rs:145`).
- Undo correctness is `UserModel`'s job: `set_user_input_with_link_diffs`
  (`user_model/common.rs:490`) wraps `model.set_user_input` and emits the `SetCellLink` (+
  `SetCellStyle`) diffs. Called from `UserModel::set_user_input` (`common.rs:459`) and CSV
  paste (`clipboard.rs:530`).

## 6. XLSX round-trip

Files changed: `xlsx/src/import/worksheets.rs` (`load_sheet_rels` now returns
`(Vec<Comment>, HashMap<String,String>)` relId→target; new `load_hyperlinks`; `SheetSettings`
gained `hyperlink_rels`), `xlsx/src/export/worksheets.rs` (`get_hyperlinks_section`,
`get_worksheet_xml_rels`, `get_sorted_links`, `split_external_target`,
`get_tooltip_attribute`), `xlsx/src/export/mod.rs` (writes
`xl/worksheets/_rels/sheet{N}.xml.rels`), `xlsx/tests/test.rs`, fixture
`xlsx/tests/link_test.xlsx`.

Round-trip is verified upstream by `test_hyperlinks_export_roundtrip`; `test_hyperlinks_import`
covers 13 links (http/https/ftp/mailto incl. query+body, `file:///C:/...`, `file:///share/...`,
internal cell refs, a defined name, tooltips on both kinds). Rel-id assignment is deterministic
— both the `<hyperlinks>` section and the rels part iterate `get_sorted_links()` in
`(row, column)` order.

Known limits:

1. **`MAX_HYPERLINK_RANGE_CELLS = 10_000`** — a `<hyperlink ref="G1:XFD1048576">` gives only
   the top-left cell a link (memory guard). Explicitly tested upstream.
2. Malformed hyperlinks (no `r:id` and no `location`, empty `location`, dangling `r:id`) are
   silently skipped.
3. The `display=` attribute is ignored on import (text comes from the cell).
4. `tooltip` (Excel's ScreenTip) is preserved both ways.
5. External `target#fragment` is split on export — target into the rels part, fragment into the
   `location` attribute, matching Excel.
6. Export writes `xl/worksheets/_rels/sheetN.xml.rels` containing **only** hyperlink
   relationships. Latent conflict if IronCalc ever exports another sheet-level rel (comments,
   tables, drawings); harmless today. **Interacts with `projects/xlsx-preservation.md`** —
   check that FreeCell's preservation layer doesn't round-trip a sheet rels part that this now
   overwrites.
7. No `[Content_Types].xml` Override is added — correct; `.rels` is covered by the `Default`
   extension entry.
8. A range hyperlink imports as N per-cell links and exports as N `<hyperlink ref="B2"/>`
   elements + N relationships. File grows, semantics preserved.

## 7. Security: the engine will hand us `javascript:` URLs

No layer of the engine validates schemes. An imported `.xlsx` can carry
`Link::External { target: "javascript:..." }` or an arbitrary `file://` path, and
`get_links_list` will return it verbatim. **FreeCell must apply its own allowlist immediately
before navigating**, mirroring the webapp's `ALLOWED_SCHEMES`
(`http, https, ftp, ftps, file, mailto`), and should treat `file://` deliberately rather than
inheriting it by default.

This is FreeCell-side work with no upstream counterpart, and it is a prerequisite for
click-to-open — not a nice-to-have.

## 8. Sync cost

### 8a. `main` fast-forwards cleanly

`git merge-base --is-ancestor 60d9db19 upstream/main` → **true**. Fork `main` can fast-forward
to `91d343c3`.

### 8b. Volume

35 commits, `86 files changed, 5717 insertions(+), 431 deletions(-)`. Restricted to engine code
(`base/src/*.rs` + `xlsx/src/*.rs`): **25 files, +2261/-32**. The rest is webapp/TS (~1800
lines), locale JSON, bindings, and a large chunk of unrelated `docs/ironcalc.dev` restyling
(commits `6159ab88`, `3fd4aedf`, `aadc0429`, `b52855ed`).

### 8c. `freecell-fixes` merging the new `main` — 6 files, 11 hunks

From a trial `git merge-tree --write-tree upstream/main origin/freecell-fixes` (tree
`c9b9a0c1`; no branches were touched).

| File | Hunks | Difficulty |
|---|---|---|
| `base/src/lib.rs` | 1 | Trivial — `pub mod links;` vs `mod merge_cells;` in the same alphabetical slot. Keep both. |
| `base/src/user_model/mod.rs` | 1 | Trivial — `mod links;` vs `mod merge_cells;`. Keep both. |
| `base/src/test/user_model/mod.rs` | 1 | Trivial — `mod test_links;` vs `mod test_merge_cells;`. Keep both. |
| `base/src/actions.rs` | 1 | Easy — import line only. Union the `use crate::types::{...}` imports, keep `displace_links`. All six insert/delete/move bodies auto-merged (but see 8d.2). |
| `base/src/user_model/undo_redo.rs` | 2 | Easy — both sides appended `match` arms at the tail of the undo and redo matches. Keep both. |
| `base/src/user_model/common.rs` | 5 | **The only real work.** See below. |

`common.rs` breakdown:

- **1 hunk (~:513–619)** — both sides added a *new method* at the same spot: upstream's
  `set_user_input_with_link_diffs` vs our batch `set_user_inputs`. Purely positional; keep
  both. (`set_user_input` itself auto-merged and already picked up upstream's
  `set_user_input_with_link_diffs(...)` call alongside our merged-region edit guard.)
- **4 hunks (delete_rows ×2, delete_columns ×2)** — genuine three-way overlap. Upstream turned
  `let diff_list = vec![Diff::DeleteRows{..}]` into
  `let mut diff_list = self.range_link_diffs(&Area{..})?; ... diff_list.push(Diff::DeleteRows{..});`
  while our frozen-pane + merged-cells work added `old_merge_cells` and
  `old_frozen_rows`/`old_frozen_columns` fields to the same `Diff` variant. Take upstream's
  structure, keep our extra fields. Mechanical — but must not be blind-resolved.

Auto-merged despite both sides touching them (verify by compiling, no manual work expected):
`base/src/types.rs` (our `MergeCell` at ~:806, upstream's `Link` at ~:185; we reuse the
pre-existing `merge_cells: Vec<String>` field so there's no `Worksheet` struct collision),
`base/src/user_model/history.rs`, `base/src/user_model/clipboard.rs`, `base/src/test/mod.rs`,
all of `bindings/{nodejs,python,wasm}/*`, `xlsx/tests/test.rs`.

### 8d. Semantic gaps the merge will NOT flag

These compile fine and are wrong. Budget for them.

1. **`UserModel::set_user_inputs` (our batch API) misses link diffs.**
   `freecell-fixes:base/src/user_model/common.rs:565` calls `self.model.set_user_input(...)`
   directly and pushes only `Diff::SetCellValue`. Post-merge it must call
   `set_user_input_with_link_diffs` instead — mirroring what upstream did to `set_user_input`
   and to CSV paste — or a batched URL entry auto-creates a link *and* a style that undo cannot
   reverse. **The single most important follow-up in the sync.**
2. **`actions.rs` row/column move: link re-attach vs merged-region rebuild.** Upstream lifts
   links out at the top and re-attaches after the cell rebuild; our merged-cells displacement
   runs in the same functions. Git interleaved them without conflict. Read the ordering and add
   a test that moves a row containing both a merged region and a link.
3. **Merged-cells clear/unmerge vs `range_link_diffs`.** Our "clear-all unmerges" logic and
   upstream's "clearing a range drops links" logic both hook `range_clear_all` /
   `range_clear_contents`. Both auto-merged; worth one integration test.
4. **`Diff` enum discriminants shift** — upstream inserted `SetCellLink` mid-enum
   (`history.rs:288`) and we insert merge-cell variants. `Diff` derives `Encode/Decode`;
   a wire-compat break only if FreeCell persists the send queue.
5. **`Worksheet` bitcode layout changes** (new `links` field). `Worksheet` derives
   `Encode, Decode` but **not** serde — persistence is `Model::to_bytes` / `from_bytes`. Any
   `.ic` blob written by a pre-links build fails to decode against the new one. Confirm whether
   FreeCell persists `to_bytes()` output anywhere (autosave, crash recovery, the engine worker
   snapshot); if so this is a migration event independent of the rest of the sync.

### 8e. Per-`fix/*` branch status

Bases recomputed after `git fetch --unshallow`; `git cherry` used for patch-equivalence.

| Branch | Commits | Status vs `91d343c3` | Trial merge |
|---|---|---|---|
| `fix/trim-internal-runs` | 1 | **Already upstream** — delete | clean |
| `fix/address-empty-sheet` | 1 | **Already upstream** — delete | clean |
| `fix/xmatch-array-constant` | 2 | **Already upstream** — delete | clean |
| `fix/xlsx-bool-import` | 1 | **Already upstream** — delete | clean |
| `fix/e2-numfmt` | 1 | **Already upstream** — delete | clean |
| `fix/paste-fill-relative-refs` | 1 | Still ours (`cut_paste.rs`, `clipboard.rs`, tests) | **clean** despite upstream's heavy `clipboard.rs` edits |
| `fix/batch-set-inputs` | 3 (2 non-merge) | Still ours (`common.rs`, tests) | 1 positional conflict; **needs the real change from 8d.1** |
| `fix/structural-edits-adjust-frozen-pane` | 1 | Still ours (`actions.rs`, `user_model/{common,history,undo_redo}.rs`, tests) | 4 conflicts, all the `common.rs` `delete_rows`/`delete_columns` restructure |
| `claude/merged-cells-implementation-yv1pr7` | 5 | **Stale — ignore.** Based on `cedba4ea`, predates the Python-bindings restructure; conflicts in 11 files incl. `bindings/python/src/{lib,types}.rs` which no longer exist in that shape. The merged-cells work is already in `freecell-fixes` via `b922df5e`. |

Five of eight `fix/*` branches can be deleted. Real remaining fork surface:
**40 files, +3849/-95** (`git diff --stat 60d9db19 origin/freecell-fixes`), dominated by merged
cells.

## 9. `Function` enum collision check

**No collision.** Upstream added exactly one variant (`Function::Hyperlink`) and bumped the
`into_iter()` const generic 494 → 495. Our fork adds **zero** function variants —
`base/src/functions/mod.rs`, `base/src/language/mod.rs`, `base/src/language/language.bin`, and
`expressions/parser/static_analysis.rs` do not appear in
`git diff --name-only 60d9db19 origin/freecell-fixes`. Our function fixes (TRIM, ADDRESS,
XMATCH) changed existing implementations, not the enum, and are all upstream now anyway.
