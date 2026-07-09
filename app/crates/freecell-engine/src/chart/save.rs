//! Save re-injection (functional_spec §5, §10 #2 — **byte-preservation re-injection is the
//! accepted bar**). IronCalc's writer regenerates the `.xlsx` from a model that has no charts,
//! so it drops every `xl/charts/*`, `xl/drawings/*`, the worksheet `<drawing>` reference, and
//! the chart `[Content_Types]` overrides. We run IronCalc's real writer into an in-memory zip,
//! then splice the original chart machinery back in **byte-for-byte**:
//!
//! - carry every original `xl/charts/*` + `xl/drawings/*` entry verbatim (charts, drawings,
//!   their `_rels`, plus any `colors*`/`style*`/embeddings a real file has — all by prefix);
//! - merge the chart/drawing `<Override>`s into IronCalc's `[Content_Types].xml`;
//! - re-inject a `<drawing r:id=…/>` into each affected worksheet + a matching `_rels`.
//!
//! Scope (PoC): single-sheet fixtures, so IronCalc's `xl/worksheets/sheet1.xml` maps 1:1 to the
//! original worksheet by identical part name. Multi-sheet index→part mapping (via
//! `xl/_rels/workbook.xml.rels`) is out of scope and documented as a follow-on concern.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read, Write};
use std::path::Path;

use anyhow::{anyhow, Context, Result};

use super::load::{self, SheetDrawing};
use super::xlsx;

/// A summary of what a save re-injection preserved — for the round-trip report + tests.
#[derive(Clone, Debug)]
pub struct SaveReport {
    /// Number of embedded charts carried through into the output.
    pub charts_preserved: usize,
    /// Worksheet parts that got a re-injected `<drawing>` reference.
    pub patched_sheets: Vec<String>,
    /// Original package parts carried verbatim (charts + drawings + their rels).
    pub carried_parts: Vec<String>,
}

/// Loads `original` with IronCalc, runs IronCalc's real writer, re-injects the chart parts, and
/// writes the result to `out`. Returns what was preserved. Errors only on a genuinely broken
/// input (IronCalc can't load it) or an I/O failure.
pub fn save_with_charts(original: &Path, out: &Path) -> Result<SaveReport> {
    let sheets = load::discover(original)?;

    let path_str = original
        .to_str()
        .ok_or_else(|| anyhow!("path is not valid UTF-8: {}", original.display()))?;
    let model = ironcalc::import::load_from_xlsx(path_str, "en", "UTC", "en")
        .map_err(|e| anyhow!("IronCalc failed to load {}: {e:?}", original.display()))?;

    let cursor = Cursor::new(Vec::<u8>::new());
    let cursor = ironcalc::export::save_xlsx_to_writer(&model, cursor)
        .map_err(|e| anyhow!("IronCalc writer failed: {e:?}"))?;
    let ironcalc_bytes = cursor.into_inner();

    let (final_bytes, report) = reinject(original, &ironcalc_bytes, &sheets)?;
    std::fs::write(out, &final_bytes).with_context(|| format!("writing {}", out.display()))?;
    Ok(report)
}

/// Re-injects the original chart machinery into IronCalc's regenerated zip and returns the final
/// bytes. Pure (no disk writes) so it is unit-testable; `save_with_charts` handles I/O.
pub fn reinject(
    original: &Path,
    ironcalc_bytes: &[u8],
    sheets: &[SheetDrawing],
) -> Result<(Vec<u8>, SaveReport)> {
    // --- 1. Read the carry parts + content types out of the ORIGINAL package. -----------------
    let orig_file =
        std::fs::File::open(original).with_context(|| format!("opening {}", original.display()))?;
    let mut orig = zip::ZipArchive::new(orig_file)
        .with_context(|| format!("reading {} as a zip", original.display()))?;

    let carry_names: Vec<String> = (0..orig.len())
        .filter_map(|i| orig.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| is_carry_part(n))
        .collect();
    let mut carry: Vec<(String, Vec<u8>)> = Vec::new();
    for name in &carry_names {
        carry.push((name.clone(), read_named_bytes(&mut orig, name)?));
    }
    let orig_ct = xlsx::read_entry_from(&mut orig, "[Content_Types].xml")
        .context("original [Content_Types].xml")?;

    // --- 2. Plan the per-sheet worksheet patch. -----------------------------------------------
    // A distinctive relationship Id per patched sheet, chosen to never collide with the
    // rId1/rId2… IronCalc emits.
    let mut plan: Vec<SheetPatch> = Vec::new();
    for (k, sheet) in sheets.iter().enumerate() {
        plan.push(SheetPatch {
            sheet_part: sheet.sheet_part.clone(),
            rels_part: xlsx::rels_part_for(&sheet.sheet_part),
            rel_id: format!("rIdChartPoc{}", k + 1),
            drawing_target: relative_part(&sheet.sheet_part, &sheet.drawing_part),
            drawing_rel_type: sheet.drawing_rel_type.clone(),
        });
    }
    let patched_sheets: HashSet<&str> = plan.iter().map(|p| p.sheet_part.as_str()).collect();
    let patched_rels: HashSet<&str> = plan.iter().map(|p| p.rels_part.as_str()).collect();
    let rel_id_by_sheet: HashMap<&str, &str> = plan
        .iter()
        .map(|p| (p.sheet_part.as_str(), p.rel_id.as_str()))
        .collect();

    // --- 3. Rewrite IronCalc's zip, patching CT + worksheets, carrying the chart parts. -------
    let mut ic = zip::ZipArchive::new(Cursor::new(ironcalc_bytes))
        .context("reading IronCalc output as a zip")?;
    let ic_names: Vec<String> = (0..ic.len())
        .filter_map(|i| ic.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    // Every patched worksheet must exist in IronCalc's output (else our part-name mapping is
    // wrong — fail loudly rather than silently drop the chart).
    for p in &plan {
        if !ic_names.iter().any(|n| n == &p.sheet_part) {
            return Err(anyhow!(
                "IronCalc output has no {} to re-inject a <drawing> into (multi-sheet mapping is out of PoC scope)",
                p.sheet_part
            ));
        }
    }

    let out = Cursor::new(Vec::<u8>::new());
    let mut zw = zip::ZipWriter::new(out);
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // IronCalc's existing sheet-rels (if any) are held aside for merge; we write merged ones
    // after the main copy so we don't emit a part twice.
    let mut existing_sheet_rels: HashMap<String, String> = HashMap::new();

    for name in &ic_names {
        if name == "[Content_Types].xml" {
            let ic_ct = read_named_string(&mut ic, name)?;
            let merged = merge_content_types(&ic_ct, &orig_ct)?;
            write_part(&mut zw, opts, name, merged.as_bytes())?;
        } else if patched_sheets.contains(name.as_str()) {
            let ws = read_named_string(&mut ic, name)?;
            let rel_id = rel_id_by_sheet[name.as_str()];
            let patched = patch_worksheet(&ws, rel_id)?;
            write_part(&mut zw, opts, name, patched.as_bytes())?;
        } else if patched_rels.contains(name.as_str()) {
            // Defer: merge with our drawing relationship after the copy loop.
            existing_sheet_rels.insert(name.clone(), read_named_string(&mut ic, name)?);
        } else {
            let bytes = read_named_bytes(&mut ic, name)?;
            write_part(&mut zw, opts, name, &bytes)?;
        }
    }

    // Merged worksheet _rels (IronCalc's own, if present, plus our drawing relationship).
    for p in &plan {
        let existing = existing_sheet_rels.get(&p.rels_part).map(String::as_str);
        let rels = build_sheet_rels(existing, &p.rel_id, &p.drawing_target, &p.drawing_rel_type)?;
        write_part(&mut zw, opts, &p.rels_part, rels.as_bytes())?;
    }

    // Carry the original chart + drawing parts byte-for-byte.
    for (name, bytes) in &carry {
        write_part(&mut zw, opts, name, bytes)?;
    }

    let cursor = zw.finish().context("finishing re-injected zip")?;
    let report = SaveReport {
        charts_preserved: sheets.iter().map(|s| s.chart_parts.len()).sum(),
        patched_sheets: plan.iter().map(|p| p.sheet_part.clone()).collect(),
        carried_parts: carry_names,
    };
    Ok((cursor.into_inner(), report))
}

/// A per-worksheet re-injection plan.
struct SheetPatch {
    sheet_part: String,
    rels_part: String,
    rel_id: String,
    /// The drawing part path relative to the worksheet (`../drawings/drawing1.xml`).
    drawing_target: String,
    drawing_rel_type: String,
}

/// Whether a package part is carried verbatim: the chart + drawing machinery.
fn is_carry_part(name: &str) -> bool {
    name.starts_with("xl/charts/") || name.starts_with("xl/drawings/")
}

// ---------------------------------------------------------------------------------------------
// Worksheet + content-types + rels patching
// ---------------------------------------------------------------------------------------------

/// Injects `<drawing r:id="{rel_id}"/>` before `</worksheet>` (idempotent) and ensures the
/// worksheet root binds the `r:` prefix the injected element needs.
fn patch_worksheet(ws: &str, rel_id: &str) -> Result<String> {
    let ws = ensure_r_namespace(ws)?;
    if ws.contains("<drawing ") || ws.contains("<drawing/>") {
        return Ok(ws); // already anchored (nothing to do)
    }
    let close = ws
        .rfind("</worksheet>")
        .ok_or_else(|| anyhow!("IronCalc worksheet XML has no </worksheet>"))?;
    let drawing = format!(r#"<drawing r:id="{rel_id}"/>"#);
    Ok(format!("{}{}{}", &ws[..close], drawing, &ws[close..]))
}

/// Ensures the `<worksheet …>` root declares `xmlns:r` (IronCalc may omit it when the sheet
/// has no relationship-bearing elements — but our injected `<drawing r:id>` needs the prefix).
fn ensure_r_namespace(ws: &str) -> Result<String> {
    let start = ws
        .find("<worksheet")
        .ok_or_else(|| anyhow!("no <worksheet root element"))?;
    let open_end = ws[start..]
        .find('>')
        .map(|i| start + i)
        .ok_or_else(|| anyhow!("unterminated <worksheet tag"))?;
    if ws[start..open_end].contains("xmlns:r=") {
        return Ok(ws.to_string());
    }
    let decl = format!(
        r#" xmlns:r="{}""#,
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
    );
    Ok(format!(
        "{}{}{}",
        &ws[..start + "<worksheet".len()],
        decl,
        &ws[start + "<worksheet".len()..]
    ))
}

/// Merges the chart/drawing `<Override>`s from the original `[Content_Types].xml` into
/// IronCalc's, skipping any PartName IronCalc already declares.
fn merge_content_types(ic_ct: &str, orig_ct: &str) -> Result<String> {
    let ic_parts = declared_part_names(ic_ct)?;
    let doc = roxmltree::Document::parse(orig_ct).context("parsing original content types")?;

    let mut additions = String::new();
    for node in doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Override")
    {
        let (Some(part), Some(ct)) = (
            xlsx::attr(&node, "PartName"),
            xlsx::attr(&node, "ContentType"),
        ) else {
            continue;
        };
        if !(part.starts_with("/xl/charts/") || part.starts_with("/xl/drawings/")) {
            continue;
        }
        if ic_parts.contains(part) {
            continue;
        }
        additions.push_str(&format!(
            r#"<Override PartName="{part}" ContentType="{ct}"/>"#
        ));
    }

    if additions.is_empty() {
        return Ok(ic_ct.to_string());
    }
    let close = ic_ct
        .rfind("</Types>")
        .ok_or_else(|| anyhow!("IronCalc [Content_Types].xml has no </Types>"))?;
    Ok(format!(
        "{}{}{}",
        &ic_ct[..close],
        additions,
        &ic_ct[close..]
    ))
}

/// The set of `PartName`s an `[Content_Types].xml` already declares (via `<Override>`).
fn declared_part_names(ct: &str) -> Result<HashSet<String>> {
    let doc = roxmltree::Document::parse(ct).context("parsing content types")?;
    Ok(doc
        .descendants()
        .filter(|n| n.tag_name().name() == "Override")
        .filter_map(|n| xlsx::attr(&n, "PartName").map(str::to_string))
        .collect())
}

/// Builds a worksheet `_rels` part: IronCalc's existing relationships (if any) plus the drawing
/// relationship we re-inject.
fn build_sheet_rels(
    existing: Option<&str>,
    rel_id: &str,
    drawing_target: &str,
    drawing_rel_type: &str,
) -> Result<String> {
    // Reconstruct existing relationships (normalized), if present.
    let mut rels: BTreeMap<String, xlsx::Relationship> = BTreeMap::new();
    if let Some(xml) = existing {
        rels = xlsx::parse_rels(xml)?;
    }
    let mut body = String::new();
    for (id, rel) in &rels {
        body.push_str(&format!(
            r#"<Relationship Id="{id}" Type="{}" Target="{}"/>"#,
            rel.rel_type, rel.target
        ));
    }
    // Our drawing relationship (the id is chosen non-colliding upstream).
    let rel_type = if drawing_rel_type.is_empty() {
        "http://schemas.openxmlformats.org/officeDocument/2006/relationships/drawing"
    } else {
        drawing_rel_type
    };
    body.push_str(&format!(
        r#"<Relationship Id="{rel_id}" Type="{rel_type}" Target="{drawing_target}"/>"#
    ));

    // No inter-element whitespace: IronCalc's `load_sheet_rels` iterates raw children and reads
    // `Type` on each, so a whitespace text node between `<Relationship>`s would break re-opening.
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{body}</Relationships>"#
    ))
}

// ---------------------------------------------------------------------------------------------
// Small helpers
// ---------------------------------------------------------------------------------------------

/// The path of `to_part` relative to the directory of `from_part` (both package-absolute).
/// Example: `xl/worksheets/sheet1.xml`, `xl/drawings/drawing1.xml` → `../drawings/drawing1.xml`.
fn relative_part(from_part: &str, to_part: &str) -> String {
    let from_dir: Vec<&str> = from_part.split('/').collect();
    let from_dir = &from_dir[..from_dir.len().saturating_sub(1)]; // drop the file name
    let to: Vec<&str> = to_part.split('/').collect();

    let mut common = 0;
    while common < from_dir.len() && common < to.len() && from_dir[common] == to[common] {
        common += 1;
    }
    let ups = from_dir.len() - common;
    let mut segs: Vec<String> = std::iter::repeat_n("..".to_string(), ups).collect();
    segs.extend(to[common..].iter().map(|s| s.to_string()));
    segs.join("/")
}

fn read_named_bytes<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<Vec<u8>> {
    let mut f = archive
        .by_name(name)
        .with_context(|| format!("zip entry {name} not found"))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)
        .with_context(|| format!("reading zip entry {name}"))?;
    Ok(buf)
}

fn read_named_string<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String> {
    String::from_utf8(read_named_bytes(archive, name)?)
        .with_context(|| format!("zip entry {name} is not UTF-8"))
}

fn write_part<W: Write + std::io::Seek>(
    zw: &mut zip::ZipWriter<W>,
    opts: zip::write::FileOptions,
    name: &str,
    bytes: &[u8],
) -> Result<()> {
    zw.start_file(name, opts)
        .with_context(|| format!("starting {name}"))?;
    zw.write_all(bytes)
        .with_context(|| format!("writing {name}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::authoring;
    use crate::chart::load::{discover, load_charts_from_xlsx};

    #[test]
    fn relative_part_between_worksheet_and_drawing() {
        assert_eq!(
            relative_part("xl/worksheets/sheet1.xml", "xl/drawings/drawing1.xml"),
            "../drawings/drawing1.xml"
        );
    }

    #[test]
    fn patch_worksheet_injects_drawing_and_binds_r_namespace() {
        let ws =
            r#"<?xml version="1.0"?><worksheet xmlns="http://x/main"><sheetData/></worksheet>"#;
        let patched = patch_worksheet(ws, "rIdChartPoc1").unwrap();
        assert!(patched.contains("xmlns:r="), "must bind r: prefix");
        assert!(patched.contains(r#"<drawing r:id="rIdChartPoc1"/></worksheet>"#));
        // Idempotent-ish: a worksheet that already has a <drawing> is untouched by the inject.
        let again = patch_worksheet(&patched, "rIdChartPoc1").unwrap();
        assert_eq!(again.matches("<drawing ").count(), 1);
    }

    #[test]
    fn merge_content_types_adds_missing_chart_overrides() {
        let ic = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="wb"/></Types>"#;
        let orig = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/charts/chart1.xml" ContentType="chart"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="drawing"/><Override PartName="/xl/workbook.xml" ContentType="wb"/></Types>"#;
        let merged = merge_content_types(ic, orig).unwrap();
        assert!(merged.contains(r#"PartName="/xl/charts/chart1.xml""#));
        assert!(merged.contains(r#"PartName="/xl/drawings/drawing1.xml""#));
        // The workbook override is not duplicated.
        assert_eq!(merged.matches(r#"PartName="/xl/workbook.xml""#).count(), 1);
    }

    /// The Gate-4 save proof: author → IronCalc save → re-inject → reopen with our loader and
    /// with IronCalc, and confirm the charts survive with the same cached values.
    #[test]
    fn roundtrip_preserves_charts() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("charts_basic.xlsx");
        authoring::write_fixture(&original).unwrap();
        let before = load_charts_from_xlsx(&original).unwrap();

        let out = dir.path().join("roundtrip.xlsx");
        let report = save_with_charts(&original, &out).unwrap();
        assert_eq!(report.charts_preserved, 3);
        assert_eq!(report.patched_sheets, vec!["xl/worksheets/sheet1.xml"]);

        // (a) Our own loader re-finds all three charts, same cached values.
        let sheets = discover(&out).unwrap();
        assert_eq!(sheets.len(), 1);
        assert_eq!(sheets[0].chart_parts.len(), 3);
        let after = load_charts_from_xlsx(&out).unwrap();
        assert_eq!(after, before, "charts survive the round-trip unchanged");

        // (b) The re-injected output still opens in IronCalc (not corrupted).
        let out_str = out.to_str().unwrap();
        ironcalc::import::load_from_xlsx(out_str, "en", "UTC", "en")
            .expect("round-tripped file reopens in IronCalc");

        // (c) The worksheet carries a <drawing> ref and CT declares the chart parts.
        let ws = xlsx::read_entry(&out, "xl/worksheets/sheet1.xml").unwrap();
        assert!(ws.contains("<drawing "));
        let ct = xlsx::read_entry(&out, "[Content_Types].xml").unwrap();
        assert!(ct.contains("/xl/charts/chart1.xml"));
    }
}
