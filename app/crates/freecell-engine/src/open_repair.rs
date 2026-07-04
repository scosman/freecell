//! `open_repair` — a **best-effort, reactive** pre-parse repair for one class of
//! over-strict rejection in IronCalc 0.7.1's xlsx importer.
//!
//! IronCalc's styles parser requires an `xfId` attribute on **every** `<cellXfs>`/`<xf>`
//! element (`ironcalc-0.7.1/src/import/styles.rs` — `get_attribute(&xfs, "xfId")?`). But per
//! OOXML/ECMA-376 §18.8.10, `xfId` on a `cellXfs/xf` is **optional** (it references a
//! `cellStyleXfs` entry; the default is "no style xf"). Real exporters routinely omit it —
//! Apple Numbers, LibreOffice, and various generators all write `<cellXfs>` `<xf>` elements
//! with no `xfId`. IronCalc then fails the whole open with
//! `XML Error: Missing "xfId" XML attribute`, even though the file is perfectly valid.
//!
//! Note the asymmetry that makes this safe to target: `cellStyleXfs` entries carry no `xfId`
//! that IronCalc reads (its `CellStyleXfs` struct has no such field, and the `cellStyleXfs`
//! loop never looks for one); only `cellXfs` entries require it. So the repair is scoped
//! strictly to the `<cellXfs>` block; `<cellStyleXfs>` is never touched.
//!
//! ## Contract (identical to [`open_fixups`](crate::open_fixups))
//!
//! * **Reactive only.** The repair runs *after* a normal `load_from_xlsx` has already failed
//!   with this specific error. A file that parses is never read, patched, or reloaded — the
//!   common path is byte-for-byte unchanged.
//! * **Best-effort, never worse.** Any failure to read / patch / reload returns `None`, and
//!   the caller then surfaces the *original* typed `LoadError` (the file's real problem), not
//!   a repair-path error. No panics/unwraps on malformed input.
//! * **Minimal surgery.** Only `<xf>` start tags inside `<cellXfs>` that *lack* `xfId` get
//!   `xfId="0"` injected; every other byte of the archive (theme, fonts, fills, worksheets,
//!   the `cellStyleXfs` block) is copied through unchanged.

use std::io::{Cursor, Read, Write};
use std::path::Path;

use ironcalc::error::XlsxError;
use ironcalc_base::Model;

use crate::document::{DEFAULT_LANGUAGE, DEFAULT_LOCALE, DEFAULT_TIMEZONE, NEW_WORKBOOK_NAME};

/// The single entry point the opener calls on a load failure. Returns `Some(model)` **iff**
/// `err` is the repairable missing-`xfId` class *and* the read → patch → reload all succeed;
/// otherwise `None`, so the caller surfaces the original typed error unchanged.
///
/// The returned `Model` has **not** had [`open_fixups`](crate::open_fixups) applied yet — the
/// caller applies those (theme/number-format corrections) uniformly to both the normal and the
/// repaired model.
pub(crate) fn try_repair_and_reload(path: &Path, err: &XlsxError) -> Option<Model<'static>> {
    if !is_repairable_xf_id_error(err) {
        return None;
    }
    let bytes = repair_xlsx_bytes(path)?;
    reload_from_bytes(path, &bytes)
}

/// Whether `err` is the specific "IronCalc rejected an optional-`xfId`-less `cellXfs`" failure.
///
/// IronCalc raises this as `XlsxError::Xml("Missing \"xfId\" XML attribute")`. We gate on the
/// `Xml` variant *and* the attribute name so we never send an unrelated XML/Zip/Workbook
/// failure down the repair path (which would waste work and could mask the real problem).
fn is_repairable_xf_id_error(err: &XlsxError) -> bool {
    matches!(err, XlsxError::Xml(msg) if msg.contains("xfId"))
}

/// Reads the `.xlsx` (a Zip), patches `xl/styles.xml` to inject the optional `xfId`, and
/// re-emits the whole archive to an in-memory buffer. Returns `None` on any I/O / Zip error,
/// if `xl/styles.xml` is absent, or if no `<cellXfs>` `<xf>` actually needed patching (the
/// error was something else that merely mentioned `xfId`) — in every such case the caller
/// keeps the original error.
fn repair_xlsx_bytes(path: &Path) -> Option<Vec<u8>> {
    let file = std::fs::File::open(path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;

    // Read styles.xml and decide whether a repair even applies before rewriting anything.
    let styles_xml = {
        let mut entry = archive.by_name("xl/styles.xml").ok()?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf).ok()?;
        buf
    };
    // `None` here → nothing to patch (all `<xf>` already carry `xfId`, or there is no
    // `<cellXfs>` block): don't rebuild, let the original error stand.
    let patched_styles = inject_cell_xfs_xf_id(&styles_xml)?;

    // Rewrite every entry to a fresh in-memory Zip, swapping in the patched styles.xml and
    // copying all other members (theme, worksheets, sharedStrings, …) through untouched.
    let mut out = Vec::new();
    {
        let mut writer = zip::ZipWriter::new(Cursor::new(&mut out));
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i).ok()?;
            let name = entry.name().to_string();
            // Preserve the member's original compression so sizes/behaviour don't drift.
            let options =
                zip::write::FileOptions::default().compression_method(entry.compression());

            if entry.is_dir() {
                writer.add_directory(name, options).ok()?;
                continue;
            }
            writer.start_file(&name, options).ok()?;
            if name == "xl/styles.xml" {
                writer.write_all(patched_styles.as_bytes()).ok()?;
            } else {
                let mut data = Vec::new();
                entry.read_to_end(&mut data).ok()?;
                writer.write_all(&data).ok()?;
            }
        }
        writer.finish().ok()?;
    }
    Some(out)
}

/// Re-parses the repaired bytes into a `Model`, matching the exact pipeline `load_from_xlsx`
/// uses internally (`load_from_xlsx_bytes` → `Model::from_workbook`) so the repaired model is
/// indistinguishable from a normally loaded one. The workbook name mirrors what a direct open
/// of `path` would derive (its file stem). Best-effort: `None` on any parse failure.
fn reload_from_bytes(path: &Path, bytes: &[u8]) -> Option<Model<'static>> {
    let name = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| NEW_WORKBOOK_NAME.to_string());
    let workbook =
        ironcalc::import::load_from_xlsx_bytes(bytes, &name, DEFAULT_LOCALE, DEFAULT_TIMEZONE)
            .ok()?;
    Model::from_workbook(workbook, DEFAULT_LANGUAGE).ok()
}

/// Injects `xfId="0"` into every `<xf>` start tag inside the `<cellXfs>` block that lacks an
/// `xfId` attribute, and returns the rewritten document. Returns `None` when there is no
/// `<cellXfs>` block or when no `<xf>` needed patching (so the caller can decline the repair).
///
/// The scope is the single `<cellXfs>…</cellXfs>` span: `"<cellXfs"` does not match
/// `"<cellStyleXfs"` (the char after `<cell` is `S`, not `X`) and `"</cellXfs>"` does not match
/// `"</cellStyleXfs>"`, so the sibling `cellStyleXfs` block — whose `<xf>` legitimately omit
/// `xfId` and which IronCalc parses fine — is provably untouched. Operates on a minified
/// single-line styles.xml (how Numbers/Excel write it) as well as pretty-printed variants.
fn inject_cell_xfs_xf_id(styles_xml: &str) -> Option<String> {
    let block_start = styles_xml.find("<cellXfs")?;
    // The `<cellXfs …>` opening tag ends at its first '>'; content begins right after.
    let content_start = block_start + styles_xml[block_start..].find('>')? + 1;
    let close_rel = styles_xml[content_start..].find("</cellXfs>")?;
    let content_end = content_start + close_rel;

    let block = &styles_xml[content_start..content_end];
    let mut patched = String::with_capacity(block.len() + 32);
    let mut patched_any = false;
    let mut rest = block;

    loop {
        let Some(pos) = rest.find("<xf") else {
            patched.push_str(rest);
            break;
        };
        // Confirm `<xf` is a tag-name boundary (start tag), not a prefix of some other token.
        // `</xf>` can't match (`<` is followed by `/`, not `x`); this guards any exotic name.
        let boundary = rest.as_bytes().get(pos + 3).copied();
        let is_xf_start = matches!(boundary, Some(b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'>'));

        if is_xf_start {
            if let Some(gt_rel) = rest[pos..].find('>') {
                let tag = &rest[pos..pos + gt_rel]; // the start tag, excluding '>'
                if !tag.contains("xfId") {
                    patched.push_str(&rest[..pos]);
                    patched.push_str("<xf xfId=\"0\"");
                    // Remainder of the start tag after `<xf`, including its closing '>'.
                    patched.push_str(&rest[pos + 3..pos + gt_rel + 1]);
                    rest = &rest[pos + gt_rel + 1..];
                    patched_any = true;
                    continue;
                }
            }
        }
        // Not a patchable `<xf>` (already has `xfId`, or a false match): emit up to and
        // including the `<xf` we found, then keep scanning past it (no infinite loop).
        patched.push_str(&rest[..pos + 3]);
        rest = &rest[pos + 3..];
    }

    if !patched_any {
        return None;
    }
    let mut out = String::with_capacity(styles_xml.len() + patched.len() - block.len());
    out.push_str(&styles_xml[..content_start]);
    out.push_str(&patched);
    out.push_str(&styles_xml[content_end..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A styles.xml with a `cellStyleXfs` xf (no `xfId` — legal, and IronCalc parses it) and a
    /// `cellXfs` xf with no `xfId` (the thing IronCalc chokes on). Only the `cellXfs` one is
    /// patched.
    const STYLES_MISSING_XF_ID: &str = concat!(
        r#"<?xml version="1.0"?><styleSheet xmlns="ns">"#,
        r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0"/></cellStyleXfs>"#,
        r#"<cellXfs count="2">"#,
        r#"<xf numFmtId="0" fontId="0" applyFont="1"><alignment vertical="top"/></xf>"#,
        r#"<xf numFmtId="49" fontId="2"/>"#,
        r#"</cellXfs>"#,
        r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#,
        r#"</styleSheet>"#,
    );

    #[test]
    fn injects_xf_id_only_in_cell_xfs() {
        let out = inject_cell_xfs_xf_id(STYLES_MISSING_XF_ID).expect("cellXfs xf needed patching");

        // Both cellXfs `<xf>` gained `xfId="0"` (start tag AND self-closing forms).
        assert!(out
            .contains(r#"<cellXfs count="2"><xf xfId="0" numFmtId="0" fontId="0" applyFont="1">"#));
        assert!(out.contains(r#"<xf xfId="0" numFmtId="49" fontId="2"/></cellXfs>"#));

        // The `cellStyleXfs` xf is left exactly as it was — no `xfId` injected there.
        assert!(
            out.contains(r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0"/></cellStyleXfs>"#)
        );

        // Exactly two injections happened (the two cellXfs entries), nowhere else.
        assert_eq!(out.matches(r#"xfId="0""#).count(), 3); // 2 injected + the pre-existing cellStyle one

        // The child `<alignment>` and the `</xf>` closers are untouched.
        assert!(out.contains(r#"<alignment vertical="top"/></xf>"#));
    }

    #[test]
    fn returns_none_when_all_xf_already_have_xf_id() {
        // A well-formed file whose cellXfs already carry xfId must not be rewritten at all.
        let ok = concat!(
            r#"<styleSheet><cellStyleXfs count="1"><xf numFmtId="0"/></cellStyleXfs>"#,
            r#"<cellXfs count="1"><xf xfId="0" numFmtId="0"/></cellXfs></styleSheet>"#,
        );
        assert_eq!(inject_cell_xfs_xf_id(ok), None);
    }

    #[test]
    fn returns_none_without_cell_xfs_block() {
        let no_block = r#"<styleSheet><fonts count="0"/></styleSheet>"#;
        assert_eq!(inject_cell_xfs_xf_id(no_block), None);
    }

    #[test]
    fn preserves_a_partially_annotated_block() {
        // First xf has xfId, second doesn't → only the second is injected; the first is intact.
        let mixed =
            r#"<cellXfs count="2"><xf xfId="7" numFmtId="0"/><xf numFmtId="49"/></cellXfs>"#;
        let out = inject_cell_xfs_xf_id(mixed).expect("second xf needed patching");
        assert!(
            out.contains(r#"<xf xfId="7" numFmtId="0"/>"#),
            "first xf untouched: {out}"
        );
        assert!(
            out.contains(r#"<xf xfId="0" numFmtId="49"/>"#),
            "second xf injected: {out}"
        );
    }

    #[test]
    fn only_repairs_the_xf_id_error_class() {
        // A non-xfId error must not trigger any file access / repair.
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does_not_exist.xlsx");
        assert!(
            try_repair_and_reload(&missing, &XlsxError::Workbook("unrelated".into())).is_none()
        );
        // Even the right error class returns None when the path can't be read (best-effort).
        assert!(
            try_repair_and_reload(&missing, &XlsxError::Xml("Missing \"xfId\" attr".into()))
                .is_none()
        );
    }

    /// End-to-end at the zip level: craft a minimal `.xlsx` whose `cellXfs` omit `xfId`,
    /// confirm IronCalc rejects it, then confirm the repaired bytes load into a `Model`.
    #[test]
    fn repairs_a_crafted_xlsx_that_ironcalc_rejects() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crafted.xlsx");
        write_minimal_xlsx(&path, /* with_xf_id: */ false);

        // Baseline: IronCalc's own loader fails with exactly the xfId error we target.
        // (`Model` isn't `Debug`, so match rather than `expect_err`.)
        let err = match ironcalc::import::load_from_xlsx(
            path.to_str().unwrap(),
            DEFAULT_LOCALE,
            DEFAULT_TIMEZONE,
            DEFAULT_LANGUAGE,
        ) {
            Ok(_) => panic!("IronCalc should reject the missing-xfId cellXfs"),
            Err(e) => e,
        };
        assert!(is_repairable_xf_id_error(&err), "unexpected error: {err}");

        // The repair reads → patches → reloads into a usable model.
        let model = try_repair_and_reload(&path, &err).expect("repair should reload the model");
        assert_eq!(model.workbook.worksheets.len(), 1);
    }

    /// Writes a tiny but structurally complete `.xlsx`. When `with_xf_id` is false the single
    /// `<cellXfs>` `<xf>` omits `xfId` (the shape IronCalc rejects).
    fn write_minimal_xlsx(path: &std::path::Path, with_xf_id: bool) {
        const CONTENT_TYPES: &str = concat!(
            r#"<?xml version="1.0"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
            r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>"#,
            r#"<Default Extension="xml" ContentType="application/xml"/>"#,
            r#"<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>"#,
            r#"<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>"#,
            r#"<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>"#,
            r#"</Types>"#,
        );
        const ROOT_RELS: &str = concat!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>"#,
            r#"</Relationships>"#,
        );
        const WORKBOOK: &str = concat!(
            r#"<?xml version="1.0"?><workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" "#,
            r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">"#,
            r#"<sheets><sheet name="Sheet 1" sheetId="1" r:id="rId1"/></sheets></workbook>"#,
        );
        const WORKBOOK_RELS: &str = concat!(
            r#"<?xml version="1.0"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">"#,
            r#"<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>"#,
            r#"<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>"#,
            r#"</Relationships>"#,
        );
        const SHEET: &str = concat!(
            r#"<?xml version="1.0"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
            r#"<dimension ref="A1"/><sheetData><row r="1"><c r="A1" s="0"><v>42</v></c></row></sheetData></worksheet>"#,
        );
        let xf = if with_xf_id {
            r#"<xf xfId="0" numFmtId="0" fontId="0" fillId="0" borderId="0"/>"#
        } else {
            r#"<xf numFmtId="0" fontId="0" fillId="0" borderId="0"/>"#
        };
        let styles = format!(
            concat!(
                r#"<?xml version="1.0"?><styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">"#,
                r#"<fonts count="1"><font><sz val="10"/><name val="Arial"/></font></fonts>"#,
                r#"<fills count="1"><fill><patternFill patternType="none"/></fill></fills>"#,
                r#"<borders count="1"><border><left/><right/><top/><bottom/><diagonal/></border></borders>"#,
                r#"<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>"#,
                r#"<cellXfs count="1">{}</cellXfs>"#,
                r#"<cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles>"#,
                r#"</styleSheet>"#,
            ),
            xf,
        );

        let file = std::fs::File::create(path).unwrap();
        let mut zw = zip::ZipWriter::new(file);
        let opts = zip::write::FileOptions::default();
        for (name, body) in [
            ("[Content_Types].xml", CONTENT_TYPES),
            ("_rels/.rels", ROOT_RELS),
            ("xl/workbook.xml", WORKBOOK),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELS),
            ("xl/styles.xml", &styles),
            ("xl/worksheets/sheet1.xml", SHEET),
        ] {
            zw.start_file(name, opts).unwrap();
            zw.write_all(body.as_bytes()).unwrap();
        }
        zw.finish().unwrap();
    }
}
