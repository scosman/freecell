# Research: CSV import + export seams (2026-07-16)

Codebase findings feeding the functional spec + architecture. Paths are `app/crates/…`.

## Open path (import hooks)

- All open entry points funnel into `FreeCellApp::open_path` → `do_open_path`
  (`shell/app.rs:266`): canonicalize → record recent → registry dedupe →
  `open_document(DocumentSource::OpenFile(canonical), …)`.
- **CLI argv filter blocks csv today:** `main.rs:33` `xlsx_arg()` only accepts `.xlsx`.
- **Open panel has no extension filter** (`shell/window.rs:1840`,
  `PathPromptOptions` limitation — U1 in GAPS); `.xlsx` is enforced *post-selection* by
  the magic-byte check: `freecell-engine/src/document.rs:1745` `classify_magic` → in
  `WorkbookDocument::open` (`document.rs:192`) `Other → LoadError::NotXlsx`. A CSV hits
  `NotXlsx` today — import must branch on extension *before* this.
- Untitled workbooks: `DocumentSource::NewWorkbook` → `new_empty()` (`document.rs:177`),
  window `path: None`; save routes to `SaveTarget::Prompt { suggested_name:
  "Untitled.xlsx" }` (`lifecycle.rs:93-103`), `.xlsx` enforced by
  `with_xlsx_extension` (`lifecycle.rs:108`). `opened_from: None` → no `.back` backup
  attempted (correct for an import).
- Dirty/title: op-accounting `lifecycle::is_dirty`; title via `lifecycle::window_title`.

## Save path (export hooks)

- `WorkbookWindow::save(save_as)` (`window.rs:844`) → `resolve_save_target` →
  `send_save` / `prompt_then_save` (native panel via `cx.prompt_for_new_path`).
- Atomic write helpers reusable for export: `new_temp_beside` (`document.rs:1707`) +
  `persist_atomically` (`document.rs:1721`); bytes variant `write_xlsx_bytes_atomic`
  (`document.rs:1735`).

## Existing TSV/CSV parsing (import parser)

- External TSV paste: `shell/clipboard.rs:101` → `Command::PasteTsv` → worker
  `run.rs:1347` `apply_paste_tsv` (overflow guard `tsv_dims`/`paste_fits`,
  `freecell-core/src/tsv.rs:35,56`) → `document.rs:1450` `paste_tsv` → IronCalc
  `paste_csv_string` (tab-delimited, fields applied as user input).
- `freecell-core/tsv.rs` already uses `csv::ReaderBuilder` (delimiter `\t`, flexible,
  RFC-4180 quoting) — the same crate/config family works for `,`.
- **`csv` crate v1.4.0 already a workspace dep** (`app/Cargo.toml:83`,
  used by `freecell-core`; `freecell-engine` would need `csv.workspace = true` added).
- Known TSV-paste behavior: empty tokens are *skipped*, not cleared (GAPS.md:194) —
  irrelevant for import into an empty new workbook.

## Export data access

- Used range: `worksheet(sheet_idx).dimension()` (`document.rs:375`; `pub(crate)` —
  IronCalc types don't leave `freecell-engine`, so the export walk lives in the engine
  crate). Populated-cell iteration pattern: `selection_stats` (`document.rs:1010`),
  `resolve_edge` (`document.rs:1058`).
- Cell value renderers: `formatted_value` (`document.rs:294`, engine display string) vs
  `value_token` (`document.rs:1368`, computed literal — what paste-values uses via
  `copied_value_tokens`, `document.rs:1316`).
- Reply-path precedent: `run.rs:1123` `apply_copy` → `WorkerEvent::CopyReady { tsv }`.

## Menu/command wiring

- Actions in `shell/mod.rs` (~:50); menus + keybindings `shell/menus.rs:29,60` (File menu
  items :67-76); app-level handlers `shell/app.rs:86`; window-scoped handlers
  `shell/window.rs:1148`. Welcome buttons `shell/welcome.rs:199-214`.
- Import = app-level action (like `OpenFile`); Export = window-scoped (like `SaveAs`).

## Implied lowest-risk design

Import: relax argv/panel branch on `.csv` extension → build `WorkbookDocument` from
`new_empty()` + comma-delimited parse applying cells as user input → open with
`path: None` (untitled ⇒ Save naturally becomes Save-As-to-.xlsx; no `.back`).
Export: window action → `.csv` save panel → new worker `Command` walking
`dimension()` + per-cell value rendering, serialized with `csv::WriterBuilder`,
written via the existing atomic helpers.
