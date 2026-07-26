---
status: complete
---

# Phase 1: Declaration (packaging config)

## Overview

Seam A of the project (architecture §2): declare FreeCell as a candidate OS handler for
`.xlsx` and `.csv` by adding cargo-packager `file-associations` metadata to
`crates/freecell-app/Cargo.toml`. This is pure packaging config — no runtime code. From these
two entries cargo-packager 0.11.8 emits the per-format association metadata (macOS
`CFBundleDocumentTypes`, Linux `.desktop` `MimeType=`/`Exec=%F`, Windows NSIS ProgId). Because
argv delivery is already wired (`main.rs::open_arg` → `open_path`), this phase alone enables the
Windows/Linux double-click / Open-With / shell-arg flows end-to-end; only macOS (Apple Event)
remains for Phase 2.

## Steps

1. **`crates/freecell-app/Cargo.toml` — add two `[[package.metadata.packager.file-associations]]`
   array-of-tables**, placed after the `icons` key and **before** the
   `[package.metadata.packager.deb]` header (a new table header ends the packager table):
   - `xlsx`: `mime-type = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"`,
     `name`/`description = "Excel Workbook"`, `role = "editor"`.
   - `csv`: `mime-type = "text/csv"`, `name`/`description = "CSV Document"`, `role = "editor"`.
   - **Correction vs. spec text:** the spec drafts wrote `role = "Editor"`, but cargo-packager's
     `BundleTypeRole` is `#[serde(rename_all = "camelCase", deny_unknown_fields)]` (confirmed
     against the 0.11.8 source + `schema.json`), so the only valid spelling is lowercase
     `"editor"` — `"Editor"` fails to deserialize and would break packaging. `"editor"` is also
     the default. Used the valid lowercase form; documented the trap in a manifest comment.
2. **`crates/freecell-app/Cargo.toml` — add `toml = "0.8"` dev-dependency** (already resolved at
   `0.8.23` in `Cargo.lock`, so it adds no new crate) to back the guard test below.
3. **`src/lib.rs` — add a `#[cfg(test)] mod packaging_metadata`** that parses this crate's own
   `Cargo.toml` (via `CARGO_MANIFEST_DIR`) and asserts the two entries are present and
   well-formed. This is the only automated coverage for a config block that no runtime code
   exercises, and it specifically pins the `role = "editor"` casing so a future edit back to
   `"Editor"` is caught.

## Tests

- `packaging_metadata::declares_exactly_xlsx_and_csv` — exactly two entries; one registers
  `.xlsx`, one registers `.csv`.
- `packaging_metadata::xlsx_entry_is_well_formed` — xlsx entry carries the OpenXML spreadsheet
  mime-type and the `Excel Workbook` name/description.
- `packaging_metadata::csv_entry_is_well_formed` — csv entry carries `text/csv` and the
  `CSV Document` name/description.
- `packaging_metadata::roles_use_cargo_packager_lowercase_enum` — every entry's `role` is the
  lowercase `"editor"` cargo-packager accepts (guards the deny-unknown-fields/camelCase trap).

## Verification notes

- cargo-packager 0.11.8 `.desktop`/plist emission was confirmed by reading the pinned crate
  source (`src/package/deb/mod.rs`: `exec_arg = Some("%F")` when file-associations non-empty +
  `mime_type.join(";")`; `src/package/app/mod.rs`: `CFBundleDocumentTypes` with
  `CFBundleTypeExtensions`/`Name`/`Role`) rather than a cold full-release `.deb` build, per the
  repo build-efficiency rule — the source read is definitive on the emission format and free,
  whereas a cold release+packager build is tens of minutes and not CI-reproducible. The actual
  `.deb`/`.app` production is exercised by the release smoke (Phase 4).
- Whole-workspace `cargo fmt --all --check`; crate-scoped `cargo build -p freecell-app` +
  `cargo test -p freecell-app --lib`. (Linux gpui link needs the documented system deps —
  `libxkbcommon-dev`, etc. from `app/README.md` — installed in-env.) No pixel render suite (no
  baseline pixels move).
