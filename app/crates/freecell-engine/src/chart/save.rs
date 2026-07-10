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
use std::ops::Range;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use roxmltree::{Document, Node};

use freecell_chart_model::{Category, Chart, SeriesData};

use super::load::{self, parse_chart_xml, SheetDrawing};
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
    /// Chart parts whose retained source was **patched** (edited-loaded reflow), rather than
    /// carried byte-for-byte — the chart part names present as keys in the reinject patch map.
    pub patched_charts: Vec<String>,
}

/// Loads `original` with IronCalc, runs IronCalc's real writer, re-injects the chart parts, and
/// writes the result to `out`. Returns what was preserved. Errors only on a genuinely broken
/// input (IronCalc can't load it) or an I/O failure.
///
/// This is the **byte-preserve** path (no live edits): it reloads `original`, so the regenerated
/// model has the *same* worksheet names — each chart-bearing worksheet maps to its output part by
/// name. The app's editable save (in-session renames possible) rides [`reinject_live_charts`].
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

    // model == original, so each chart-bearing worksheet keeps its name → its output part maps by
    // name (a name absent from the output is genuine corruption → fail loud). No edits → no patches.
    let orig_name_by_part = part_to_name_map_from_file(original)?;
    let out_part_by_name = name_to_part_map(&ironcalc_bytes)?;
    let targets = sheets
        .iter()
        .map(|sheet| {
            let name = orig_name_by_part.get(&sheet.sheet_part).ok_or_else(|| {
                anyhow!(
                    "original workbook has no name for chart-bearing part {}",
                    sheet.sheet_part
                )
            })?;
            let part = out_part_by_name.get(name).ok_or_else(|| {
                anyhow!(
                    "IronCalc output has no worksheet named {name:?} to re-inject a <drawing> into"
                )
            })?;
            Ok(Some(part.clone()))
        })
        .collect::<Result<Vec<_>>>()?;

    let (final_bytes, report) = reinject(
        original,
        &ironcalc_bytes,
        &sheets,
        &targets,
        &BTreeMap::new(),
    )?;
    std::fs::write(out, &final_bytes).with_context(|| format!("writing {}", out.display()))?;
    Ok(report)
}

/// One live chart the worker's `Command::Save` is persisting (charts/architecture §4.1/§5). The
/// worker builds these from its bound charts; each **self-describes** its part and host worksheet,
/// so the save never has to guess an association from XML bytes or list position.
#[derive(Clone, Debug)]
pub struct LiveChart {
    /// The CURRENT name of the worksheet this chart is anchored on, resolved live from its anchor
    /// `SheetId` — so it **tracks an in-session rename**. `None` when that sheet was deleted → the
    /// chart's `<drawing>` is dropped from the save (logged), never failing the save.
    pub sheet_name: Option<String>,
    /// The chart's `xl/charts/chartN.xml` part (stable across renames).
    pub chart_part: String,
    /// The chart's current (live-resolved) values — reflowed into its cache iff they differ from
    /// the file cache. `None` for an **Unsupported** chart (a retained surface/radar/… with no typed
    /// model, P14): it has no values to reflow, so it is always **byte-preserved** (never patched),
    /// while still tracking its host sheet for the drawing re-injection.
    pub chart: Option<Chart>,
}

/// Re-injects the workbook's **live** charts into `model_bytes` (IronCalc's chart-less zip of the
/// *current* model) and returns the final `.xlsx` bytes — the app-save entry (`worker::run`) the
/// UI's `Command::Save` / Save-As drives. Source-first save (charts/architecture §4.1/§5) on the
/// running model:
///
/// - `original` supplies the drawing parts, sheet→drawing structure, and content-type overrides
///   (never in the model); `model_bytes` is the edited workbook body (so cell edits land).
/// - `live` are the worker's bound charts, each carrying its own part + current host-sheet name +
///   current values. A chart whose values changed is **patched** (edited-loaded reflow,
///   [`patch_chart_source`]); an unchanged one is byte-preserved. Its drawing is re-injected into
///   the output worksheet its **current** name resolves to (rename-safe); a **deleted** host sheet
///   drops the drawing gracefully (logged). A worksheet that still exists in the model but has no
///   output part is genuine corruption → **hard error** (fail loudly, surfaced as a save failure —
///   charts/architecture §6).
pub fn reinject_live_charts(
    original: &Path,
    model_bytes: &[u8],
    live: &[LiveChart],
) -> Result<(Vec<u8>, SaveReport)> {
    let sheets = load::discover(original)?;
    let patches = build_live_patches(original, live)?;
    let targets = live_sheet_targets(original, model_bytes, &sheets, live)?;
    reinject(original, model_bytes, &sheets, &targets, &patches)
}

/// The edited-loaded patch map for [`reinject_live_charts`], keyed by each live chart's **own**
/// part (`chart_part`) — never by matching source XML across charts. Matching by byte-identical XML
/// is wrong: an unqualified `c:f` resolves against each chart's anchor sheet (`binding.rs`), so two
/// byte-identical parts bound to different sheets can carry different live values; keying by the
/// chart's own part writes each part's own values. A chart whose live values differ from its file
/// cache is patched; an unchanged one is left for `reinject` to carry byte-for-byte.
fn build_live_patches(original: &Path, live: &[LiveChart]) -> Result<BTreeMap<String, String>> {
    // P11: this re-opens `original` as a zip once PER chart (and `reinject` opens it again for the
    // carry parts). Fine at the line-slice's chart counts; if either grows, thread ONE open
    // `ZipArchive` (or the already-read carry map) through the save so a save opens the file once.
    let mut patches = BTreeMap::new();
    for lc in live {
        // An Unsupported chart has no typed model to reflow → never patched (byte-preserved by the
        // reinject carry path). Don't even parse its part (a surface/radar/… part won't parse).
        let Some(chart) = &lc.chart else {
            continue;
        };
        let part_xml = xlsx::read_entry(original, &lc.chart_part)
            .with_context(|| format!("reading chart part {}", lc.chart_part))?;
        let cached = parse_chart_xml(&part_xml)
            .with_context(|| format!("parsing chart part {}", lc.chart_part))?;
        if *chart != cached {
            patches.insert(lc.chart_part.clone(), patch_chart_source(&part_xml, chart)?);
        }
    }
    Ok(patches)
}

/// The per-`SheetDrawing` output-worksheet target for a live save (aligned with `sheets`) — where
/// each drawing's `<drawing>` re-injects, or `None` to drop it. Architecture §6: a chart is **never
/// silently dropped** while its host sheet survives.
///
/// - A drawing with a **bound** live chart follows that chart's CURRENT host name (rename-safe);
///   `sheet_name: None` (host deleted) drops it; a name that exists live but has no output part is
///   genuine corruption → hard error.
/// - A drawing with **no** bound chart (all its charts were unparseable at load — surface/radar/…)
///   is **best-effort byte-preserved** onto its host worksheet if that worksheet **still exists**
///   (matched by the drawing's original sheet **name** → the model's current worksheet). Only a
///   host that is gone from the model (deleted, or renamed with no bound chart to follow it) drops —
///   a logged best-effort drop, the narrow acceptable case.
fn live_sheet_targets(
    original: &Path,
    model_bytes: &[u8],
    sheets: &[SheetDrawing],
    live: &[LiveChart],
) -> Result<Vec<Option<String>>> {
    let out_part_by_name = name_to_part_map(model_bytes)?;
    let orig_name_by_part = part_to_name_map_from_file(original)?;
    let mut targets = Vec::with_capacity(sheets.len());
    for sheet in sheets {
        let bound = live
            .iter()
            .find(|lc| sheet.charts.iter().any(|dc| dc.part == lc.chart_part));
        let target = match bound {
            // Host sheet deleted in-session → drop gracefully (log); never fail the save.
            Some(lc) if lc.sheet_name.is_none() => {
                tracing::warn!(
                    drawing = %sheet.drawing_part,
                    "dropping a chart whose host worksheet was deleted"
                );
                None
            }
            Some(lc) => {
                let name = lc.sheet_name.as_ref().expect("Some case guarded above");
                Some(out_part_by_name.get(name).cloned().ok_or_else(|| {
                    anyhow!(
                        "worksheet {name:?} exists in the model but IronCalc emitted no worksheet \
                         part for it — refusing to silently corrupt the chart save"
                    )
                })?)
            }
            // No bound chart: best-effort byte-preserve the drawing onto its host worksheet if it
            // still exists (name-based); else a logged drop (deleted / renamed-away unsupported).
            None => match orig_name_by_part
                .get(&sheet.sheet_part)
                .and_then(|name| out_part_by_name.get(name))
            {
                Some(part) => Some(part.clone()),
                None => {
                    tracing::warn!(
                        drawing = %sheet.drawing_part,
                        "dropping an unparsed chart whose host worksheet is gone from the model"
                    );
                    None
                }
            },
        };
        targets.push(target);
    }
    Ok(targets)
}

/// A `sheet name → output worksheet part` map read from an IronCalc-serialized workbook body.
/// Duplicate sheet names are invalid `.xlsx` (Excel forbids them) and unreachable through a loaded
/// workbook; if two ever collided the `collect` keeps the last (defensive last-write-wins).
fn name_to_part_map(bytes: &[u8]) -> Result<HashMap<String, String>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("reading model zip")?;
    Ok(xlsx::workbook_sheet_parts(&mut zip)?.into_iter().collect())
}

/// A `worksheet part → sheet name` map read from an `.xlsx` file on disk (the original package).
fn part_to_name_map_from_file(path: &Path) -> Result<HashMap<String, String>> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut zip = zip::ZipArchive::new(file)
        .with_context(|| format!("reading {} as a zip", path.display()))?;
    Ok(xlsx::workbook_sheet_parts(&mut zip)?
        .into_iter()
        .map(|(name, part)| (part, name))
        .collect())
}

/// Re-injects the original chart machinery into IronCalc's regenerated zip and returns the final
/// bytes. Pure (no disk writes) so it is unit-testable; `save_with_charts` handles I/O.
///
/// `targets[i]` is the output worksheet part that `sheets[i]`'s `<drawing>` re-injects into
/// (`Some(part)`), or `None` to **drop** that drawing (its host sheet was deleted). Computing the
/// target — mapping each original chart-bearing worksheet to the current model's worksheet — is the
/// **caller's** job, because the right key differs: the byte-preserve path ([`save_with_charts`],
/// model == original) maps by name; the live edit path ([`reinject_live_charts`], renames possible)
/// maps each chart's anchor `SheetId` → current name. A `Some(part)` that isn't an actual worksheet
/// entry in `ironcalc_bytes` is genuine corruption → **hard error** (fail loudly — charts/architecture §6).
///
/// A **dropped** (`None`) drawing's whole part chain (drawing + its charts + their `_rels`/aux) is
/// excluded from both the carry and the `[Content_Types]` overrides, so the output has no orphaned
/// parts (charts/architecture §6 — the ONLY thing lost is a chart whose host worksheet is gone).
///
/// `patches` carries the **edited-loaded** charts (charts/architecture §5, mode 2): a
/// `chart part name → patched chart XML` entry (built with [`patch_chart_source`]) is written in
/// place of that chart part's original bytes, so its reflowed `numCache`/`strCache` values land
/// while `c:f` + all unmodeled styling stay intact. A chart part **absent** from `patches` is
/// byte-preserved (mode 1) — so an untouched chart stays bit-stable.
pub fn reinject(
    original: &Path,
    ironcalc_bytes: &[u8],
    sheets: &[SheetDrawing],
    targets: &[Option<String>],
    patches: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, SaveReport)> {
    debug_assert_eq!(
        sheets.len(),
        targets.len(),
        "one output-worksheet target per discovered sheet-drawing"
    );

    // --- 1. Read the carry parts + content types out of the ORIGINAL package. -----------------
    let orig_file =
        std::fs::File::open(original).with_context(|| format!("opening {}", original.display()))?;
    let mut orig = zip::ZipArchive::new(orig_file)
        .with_context(|| format!("reading {} as a zip", original.display()))?;

    // The whole part chain of every DROPPED drawing — excluded from carry + content-types so no
    // orphaned chart/drawing parts leak into the output.
    let mut dropped_parts: HashSet<String> = HashSet::new();
    for (sheet, target) in sheets.iter().zip(targets) {
        if target.is_none() {
            for part in drawing_chain_parts(&mut orig, sheet)? {
                dropped_parts.insert(part);
            }
        }
    }

    let carry_names: Vec<String> = (0..orig.len())
        .filter_map(|i| orig.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| is_carry_part(n) && !dropped_parts.contains(n))
        .collect();
    let mut carry: Vec<(String, Vec<u8>)> = Vec::new();
    for name in &carry_names {
        carry.push((name.clone(), read_named_bytes(&mut orig, name)?));
    }
    let orig_ct = xlsx::read_entry_from(&mut orig, "[Content_Types].xml")
        .context("original [Content_Types].xml")?;

    // --- 2. Read IronCalc's output part names (for the copy loop + target validation). --------
    let mut ic = zip::ZipArchive::new(Cursor::new(ironcalc_bytes))
        .context("reading IronCalc output as a zip")?;
    let ic_names: Vec<String> = (0..ic.len())
        .filter_map(|i| ic.by_index(i).ok().map(|f| f.name().to_string()))
        .collect();

    // --- 3. Plan the per-sheet worksheet patch from the caller-resolved targets. A `None` target
    // drops that drawing (deleted host sheet / unparseable charts). A distinctive relationship Id
    // per patched sheet, chosen to never collide with the rId1/rId2… IronCalc emits.
    let mut plan: Vec<SheetPatch> = Vec::new();
    for (k, (sheet, target)) in sheets.iter().zip(targets).enumerate() {
        let Some(out_part) = target else {
            continue; // dropped drawing (not re-injected into any worksheet)
        };
        if !ic_names.iter().any(|n| n == out_part) {
            return Err(anyhow!(
                "IronCalc output has no worksheet part {out_part} to re-inject a <drawing> into \
                 (would corrupt the chart save)"
            ));
        }
        plan.push(SheetPatch {
            sheet_part: out_part.clone(),
            rels_part: xlsx::rels_part_for(out_part),
            rel_id: format!("rIdChartPoc{}", k + 1),
            drawing_target: relative_part(out_part, &sheet.drawing_part),
            drawing_rel_type: sheet.drawing_rel_type.clone(),
        });
    }
    let patched_sheets: HashSet<&str> = plan.iter().map(|p| p.sheet_part.as_str()).collect();
    let patched_rels: HashSet<&str> = plan.iter().map(|p| p.rels_part.as_str()).collect();
    let rel_id_by_sheet: HashMap<&str, &str> = plan
        .iter()
        .map(|p| (p.sheet_part.as_str(), p.rel_id.as_str()))
        .collect();

    // --- 4. Rewrite IronCalc's zip, patching CT + worksheets, carrying the chart parts. -------
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
            let merged = merge_content_types(&ic_ct, &orig_ct, &dropped_parts)?;
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

    // Carry the original chart + drawing parts. A chart part with an edited-loaded patch is
    // written patched (its reflowed caches); every other part goes byte-for-byte (bit-stable).
    let mut patched_charts: Vec<String> = Vec::new();
    for (name, bytes) in &carry {
        match patches.get(name) {
            Some(patched_xml) => {
                write_part(&mut zw, opts, name, patched_xml.as_bytes())?;
                patched_charts.push(name.clone());
            }
            None => write_part(&mut zw, opts, name, bytes)?,
        }
    }

    let cursor = zw.finish().context("finishing re-injected zip")?;
    let report = SaveReport {
        // Charts on re-injected (non-dropped) sheets — a dropped drawing's charts aren't preserved.
        charts_preserved: sheets
            .iter()
            .zip(targets)
            .filter(|(_, t)| t.is_some())
            .map(|(s, _)| s.charts.len())
            .sum(),
        patched_sheets: plan.iter().map(|p| p.sheet_part.clone()).collect(),
        carried_parts: carry_names,
        patched_charts,
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

/// Every package part belonging to one drawing's chain (in `original`): the drawing part + its
/// `_rels`, and each chart it references + that chart's `_rels` and non-external aux targets
/// (`colorsN`/`styleN`/embeddings). Used to exclude a **dropped** drawing's whole chain from carry +
/// content-types so the output has no orphaned parts. Mirrors [`load::read_related_parts`]'s walk.
fn drawing_chain_parts<R: Read + std::io::Seek>(
    orig: &mut zip::ZipArchive<R>,
    sheet: &SheetDrawing,
) -> Result<Vec<String>> {
    let mut parts = vec![sheet.drawing_part.clone()];
    let drawing_rels = xlsx::rels_part_for(&sheet.drawing_part);
    if xlsx::has_entry(orig, &drawing_rels) {
        parts.push(drawing_rels);
    }
    for dc in &sheet.charts {
        parts.push(dc.part.clone());
        let chart_rels = xlsx::rels_part_for(&dc.part);
        if xlsx::has_entry(orig, &chart_rels) {
            let rels_xml = xlsx::read_entry_from(orig, &chart_rels)?;
            parts.push(chart_rels);
            for rel in xlsx::parse_rels(&rels_xml)?.values() {
                let target = xlsx::resolve_target(&dc.part, &rel.target);
                if xlsx::has_entry(orig, &target) {
                    parts.push(target);
                }
            }
        }
    }
    Ok(parts)
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
/// IronCalc's, skipping any PartName IronCalc already declares — and any part in `dropped_parts`
/// (a dropped drawing's chain), so a dropped chart leaves no dangling content-type override.
fn merge_content_types(
    ic_ct: &str,
    orig_ct: &str,
    dropped_parts: &HashSet<String>,
) -> Result<String> {
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
        // Content-type PartNames are package-absolute (leading `/`); dropped parts are stored
        // without it. Skip a dropped drawing's overrides so no orphaned override survives.
        if dropped_parts.contains(part.trim_start_matches('/')) {
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

// ---------------------------------------------------------------------------------------------
// Edited-loaded patcher: reflow numCache/strCache to a chart's current values (write mode 2)
// ---------------------------------------------------------------------------------------------

/// Patches a retained chart part's cached values to `chart`'s **current** series values — the
/// **edited-loaded** write mode (charts/architecture §5, mode 2). Only the `numCache`/`strCache`
/// elements are spliced; the `c:f` formulas, `c:spPr` styling, axes, legend, layout, namespace
/// prefixes, and the XML declaration are preserved **byte-for-byte** (the fidelity win over a
/// lossy `parse → model → regenerate` round-trip). This is the same targeted-XML second pass as
/// `open_fixups.rs` / [`reinject`], keyed off `roxmltree`'s byte ranges.
///
/// `<c:ser>` elements are aligned 1:1 with `chart.series` in document order — the same
/// first-chart-group rule [`parse_chart_xml`] and
/// [`parse_chart_binding`](super::binding::parse_chart_binding) use, so a series' value/category/
/// name caches reflow from `chart.series[i]`. A part with fewer `<c:ser>` than `chart.series`
/// patches the overlap and leaves extra series verbatim. `NaN` values are omitted (Excel's
/// sparse-blank shape); `ptCount` keeps the full length.
pub fn patch_chart_source(chart_xml: &str, chart: &Chart) -> Result<String> {
    let doc = Document::parse(chart_xml).context("parsing chart XML to patch")?;
    let root = doc.root_element();
    let group = child(&root, "chart")
        .and_then(|c| child(&c, "plotArea"))
        .and_then(|plot| {
            plot.children()
                .find(|n| n.is_element() && load::is_chart_group(n.tag_name().name()))
        })
        .ok_or_else(|| anyhow!("no recognized chart-group element to patch"))?;

    // (byte_range, replacement) per reflowed cache; applied in descending start order so earlier
    // offsets stay valid.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let sers = group.children().filter(|n| n.tag_name().name() == "ser");
    for (ser, series) in sers.zip(chart.series.iter()) {
        match &series.data {
            SeriesData::CategoryValue { categories, values } => {
                push_num_cache(chart_xml, &ser, "val", values, &mut edits);
                push_category_cache(chart_xml, &ser, "cat", categories, &mut edits);
            }
            SeriesData::Xy { x, y } => {
                push_num_cache(chart_xml, &ser, "yVal", y, &mut edits);
                push_num_cache(chart_xml, &ser, "xVal", x, &mut edits);
            }
        }
        if let Some(name) = &series.name {
            push_name_cache(chart_xml, &ser, name, &mut edits);
        }
    }

    // Descending start order → each splice leaves the unprocessed prefix (and its offsets) intact.
    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut patched = chart_xml.to_string();
    let mut prev_start = patched.len();
    for (range, replacement) in edits {
        // Distinct cache elements never overlap; assert it so a future bug is caught in tests.
        debug_assert!(
            range.end <= prev_start,
            "cache edit ranges must be disjoint"
        );
        prev_start = range.start;
        patched.replace_range(range, &replacement);
    }
    Ok(patched)
}

/// Reflow a numeric value cache (`c:val` / `c:yVal` / `c:xVal` → `numCache`) to `values`.
fn push_num_cache(
    src: &str,
    ser: &Node,
    holder_tag: &str,
    values: &[f64],
    edits: &mut Vec<(Range<usize>, String)>,
) {
    let Some(cache) = child(ser, holder_tag).and_then(|h| descendant(&h, "numCache")) else {
        return;
    };
    let prefix = element_prefix(src, &cache);
    let format_code = cache_format_code(&cache);
    edits.push((
        cache.range(),
        rebuild_num_cache(prefix, format_code.as_deref(), values),
    ));
}

/// Reflow a `c:cat` category cache to `categories`, preserving the existing cache tag: a
/// `strCache` takes each category's [label](Category::label); a `numCache` takes numeric
/// categories (a text category in a numeric cache blanks that point).
fn push_category_cache(
    src: &str,
    ser: &Node,
    holder_tag: &str,
    categories: &[Category],
    edits: &mut Vec<(Range<usize>, String)>,
) {
    let Some(holder) = child(ser, holder_tag) else {
        return;
    };
    if let Some(cache) = descendant(&holder, "strCache") {
        let prefix = element_prefix(src, &cache);
        let labels: Vec<String> = categories.iter().map(Category::label).collect();
        edits.push((cache.range(), rebuild_str_cache(prefix, &labels)));
    } else if let Some(cache) = descendant(&holder, "numCache") {
        let prefix = element_prefix(src, &cache);
        let format_code = cache_format_code(&cache);
        let nums: Vec<f64> = categories
            .iter()
            .map(|c| match c {
                Category::Number(n) => *n,
                Category::Text(_) => f64::NAN,
            })
            .collect();
        edits.push((
            cache.range(),
            rebuild_num_cache(prefix, format_code.as_deref(), &nums),
        ));
    }
}

/// Reflow a series' `c:tx` name cache to `name` (only a text-name `strCache`; a numeric `c:tx`
/// cache is left verbatim).
fn push_name_cache(src: &str, ser: &Node, name: &str, edits: &mut Vec<(Range<usize>, String)>) {
    let Some(cache) = child(ser, "tx").and_then(|h| descendant(&h, "strCache")) else {
        return;
    };
    let prefix = element_prefix(src, &cache);
    edits.push((
        cache.range(),
        rebuild_str_cache(prefix, &[name.to_string()]),
    ));
}

/// Rebuild a `numCache` element string with the same namespace `prefix`, preserving `format_code`
/// if the original carried one. Non-finite values are omitted (sparse blanks); `ptCount` keeps the
/// full length so blanked points still hold their axis slot.
fn rebuild_num_cache(prefix: &str, format_code: Option<&str>, values: &[f64]) -> String {
    let mut s = format!("<{prefix}numCache>");
    if let Some(fc) = format_code {
        s.push_str(&format!(
            "<{prefix}formatCode>{}</{prefix}formatCode>",
            escape_xml(fc)
        ));
    }
    s.push_str(&format!("<{prefix}ptCount val=\"{}\"/>", values.len()));
    for (idx, v) in values.iter().enumerate() {
        if v.is_finite() {
            s.push_str(&format!(
                "<{prefix}pt idx=\"{idx}\"><{prefix}v>{}</{prefix}v></{prefix}pt>",
                fmt_cache_num(*v)
            ));
        }
    }
    s.push_str(&format!("</{prefix}numCache>"));
    s
}

/// Rebuild a `strCache` element string with the same namespace `prefix`.
fn rebuild_str_cache(prefix: &str, values: &[String]) -> String {
    let mut s = format!("<{prefix}strCache>");
    s.push_str(&format!("<{prefix}ptCount val=\"{}\"/>", values.len()));
    for (idx, v) in values.iter().enumerate() {
        s.push_str(&format!(
            "<{prefix}pt idx=\"{idx}\"><{prefix}v>{}</{prefix}v></{prefix}pt>",
            escape_xml(v)
        ));
    }
    s.push_str(&format!("</{prefix}strCache>"));
    s
}

/// The namespace prefix (including the trailing `:`, or `""` when unprefixed) of `node`'s opening
/// tag, read from the source text at the node's byte range — so a rebuilt element uses the exact
/// prefix the file wrote (`c:numCache` vs `numCache`).
fn element_prefix<'a>(src: &'a str, node: &Node) -> &'a str {
    let after = &src[node.range().start + 1..]; // skip the '<'
    let qname_end = after
        .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
        .unwrap_or(after.len());
    let qname = &after[..qname_end];
    match qname.find(':') {
        Some(colon) => &qname[..colon + 1],
        None => "",
    }
}

/// The `<c:formatCode>` text under a cache element, if present (preserved across a reflow).
fn cache_format_code(cache: &Node) -> Option<String> {
    child(cache, "formatCode")
        .and_then(|n| n.text())
        .map(str::to_string)
}

/// Format a value for a cache `<c:v>`: whole numbers without a decimal point (matching Excel's
/// `numCache`), other values with Rust's default `f64` formatting.
fn fmt_cache_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Minimal XML text escaping for reflowed cached strings.
fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The first child *element* of `node` with this local (namespace-agnostic) tag name.
fn child<'a>(node: &Node<'a, '_>, name: &str) -> Option<Node<'a, 'a>> {
    node.children()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

/// The first descendant *element* of `node` (including `node` itself) with this local tag name.
fn descendant<'a>(node: &Node<'a, '_>, name: &str) -> Option<Node<'a, 'a>> {
    node.descendants()
        .find(|n| n.is_element() && n.tag_name().name() == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chart::authoring;
    use crate::chart::binding::{parse_chart_binding, resolve_chart, CellData};
    use crate::chart::load::{
        discover, discover_and_parse, discover_and_parse_by_sheet, load_charts_from_xlsx,
        parse_chart_xml,
    };
    use crate::document::WorkbookDocument;
    use freecell_core::{CellRef, SheetId};

    /// The first series' first value of a parsed `CategoryValue` chart (a common patch assertion).
    fn first_value(chart: &Chart) -> f64 {
        match &chart.series[0].data {
            SeriesData::CategoryValue { values, .. } => values[0],
            other => panic!("expected CategoryValue, got {other:?}"),
        }
    }

    /// Serialize an `.xlsx` at `path` through IronCalc's writer into a chart-less in-memory zip —
    /// the reinject base a save produces from the current model.
    fn ironcalc_bytes(path: &Path) -> Vec<u8> {
        let model =
            ironcalc::import::load_from_xlsx(path.to_str().unwrap(), "en", "UTC", "en").unwrap();
        let cursor =
            ironcalc::export::save_xlsx_to_writer(&model, Cursor::new(Vec::new())).unwrap();
        cursor.into_inner()
    }

    /// Name-based sheet targets (model == original, no rename) — what `save_with_charts` computes;
    /// used by the direct-`reinject` tests.
    fn name_targets(
        original: &Path,
        model_bytes: &[u8],
        sheets: &[SheetDrawing],
    ) -> Vec<Option<String>> {
        let orig = part_to_name_map_from_file(original).unwrap();
        let out = name_to_part_map(model_bytes).unwrap();
        sheets
            .iter()
            .map(|s| Some(out[&orig[&s.sheet_part]].clone()))
            .collect()
    }

    /// A [`LiveChart`] for a still-present host sheet (the common save-descriptor case in tests).
    fn live_on(sheet: &str, chart_part: &str, chart: Chart) -> LiveChart {
        LiveChart {
            sheet_name: Some(sheet.to_string()),
            chart_part: chart_part.to_string(),
            chart: Some(chart),
        }
    }

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
        let orig = r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/xl/charts/chart1.xml" ContentType="chart"/><Override PartName="/xl/charts/chart2.xml" ContentType="chart"/><Override PartName="/xl/drawings/drawing1.xml" ContentType="drawing"/><Override PartName="/xl/workbook.xml" ContentType="wb"/></Types>"#;
        // chart2 belongs to a DROPPED drawing → its override must be skipped.
        let dropped = HashSet::from(["xl/charts/chart2.xml".to_string()]);
        let merged = merge_content_types(ic, orig, &dropped).unwrap();
        assert!(merged.contains(r#"PartName="/xl/charts/chart1.xml""#));
        assert!(merged.contains(r#"PartName="/xl/drawings/drawing1.xml""#));
        assert!(
            !merged.contains(r#"PartName="/xl/charts/chart2.xml""#),
            "a dropped drawing's override must not be added"
        );
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
        assert_eq!(sheets[0].charts.len(), 3);
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

        // No charts were patched — every one is byte-preserved (mode 1).
        assert!(report.patched_charts.is_empty());
    }

    // --- P10: edited-loaded patcher (patch_chart_source) -------------------------------------

    #[test]
    fn patch_reflows_value_cache_keeping_cf_and_styling() {
        let xml = authoring::line_chart_xml_for_test();
        let mut chart = parse_chart_xml(&xml).unwrap();
        // Simulate a source-cell edit: Widgets Q1 120 → 999.
        if let SeriesData::CategoryValue { values, .. } = &mut chart.series[0].data {
            values[0] = 999.0;
        }
        let patched = patch_chart_source(&xml, &chart).unwrap();

        // The reflowed value landed; the `c:f` and the series' solid-fill styling are untouched.
        assert!(patched.contains("<c:v>999</c:v>"));
        assert!(patched.contains("<c:f>Data!$B$2:$B$5</c:f>"), "c:f kept");
        assert!(
            patched.contains(r#"<a:srgbClr val="4472C4"/>"#),
            "unmodeled spPr styling kept"
        );
        // Re-parses to the new value; the untouched second point stays 150.
        let reparsed = parse_chart_xml(&patched).unwrap();
        assert_eq!(first_value(&reparsed), 999.0);
        match &reparsed.series[0].data {
            SeriesData::CategoryValue { values, .. } => assert_eq!(values[1], 150.0),
            other => panic!("expected CategoryValue, got {other:?}"),
        }
        // Still well-formed XML.
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    #[test]
    fn patch_is_byte_identical_when_values_unchanged() {
        // A no-op reflow (patch with the file's own parsed values) reproduces the canonical Excel
        // cache shape byte-for-byte — proving the patch touches ONLY changed cache values.
        //
        // NOTE: this identity holds only because our fixture's caches are already in that canonical
        // shape (the patcher canonicalizes `<c:pt>`/`ptCount`/`formatCode`). A real-world cache with
        // different whitespace/attribute order would NOT round-trip byte-identically through the
        // patcher — and it never has to: an UNCHANGED chart is byte-preserved via `reinject`'s carry
        // path (the original bytes), never re-emitted by the patcher. Do not write a real-file test
        // expecting `patch_chart_source` to be a byte no-op.
        let xml = authoring::line_chart_xml_for_test();
        let chart = parse_chart_xml(&xml).unwrap();
        let patched = patch_chart_source(&xml, &chart).unwrap();
        assert_eq!(patched, xml, "no-op reflow is byte-identical");
    }

    #[test]
    fn patch_omits_nan_points_as_sparse_but_keeps_ptcount() {
        let xml = authoring::line_chart_xml_for_test();
        let mut chart = parse_chart_xml(&xml).unwrap();
        // Blank the second Widgets point (a value edited to empty/non-numeric resolves to NaN).
        if let SeriesData::CategoryValue { values, .. } = &mut chart.series[0].data {
            values[1] = f64::NAN;
        }
        let patched = patch_chart_source(&xml, &chart).unwrap();
        // idx 1 is dropped (sparse), but the count still spans all four categories.
        assert!(patched.contains(r#"<c:ptCount val="4"/>"#));
        assert!(!patched.contains(r#"<c:pt idx="1"><c:v>150</c:v></c:pt>"#));
        // The blank is transparent to re-parse: the remaining points still read back.
        let reparsed = parse_chart_xml(&patched).unwrap();
        match &reparsed.series[0].data {
            // The dropped point collapses the dense vector; idx 0/2/3 survive (120,90,170).
            SeriesData::CategoryValue { values, .. } => {
                assert_eq!(values, &vec![120.0, 90.0, 170.0])
            }
            other => panic!("expected CategoryValue, got {other:?}"),
        }
    }

    #[test]
    fn patch_reflows_category_str_cache_and_series_name() {
        let xml = authoring::line_chart_xml_for_test();
        let mut chart = parse_chart_xml(&xml).unwrap();
        if let SeriesData::CategoryValue { categories, .. } = &mut chart.series[0].data {
            categories[0] = Category::Text("Spring".into());
        }
        chart.series[0].name = Some("Doohickeys".into());
        let patched = patch_chart_source(&xml, &chart).unwrap();
        assert!(patched.contains("<c:v>Spring</c:v>"), "category reflowed");
        assert!(
            patched.contains("<c:v>Doohickeys</c:v>"),
            "series name reflowed"
        );
        // The value cache's formatCode survived the reflow.
        assert!(patched.contains("<c:formatCode>General</c:formatCode>"));
    }

    #[test]
    fn element_prefix_reads_namespace_prefix() {
        let prefixed = r#"<c:numCache xmlns:c="urn:x"/>"#;
        let doc = roxmltree::Document::parse(prefixed).unwrap();
        assert_eq!(element_prefix(prefixed, &doc.root_element()), "c:");
        let doc = roxmltree::Document::parse("<numCache/>").unwrap();
        assert_eq!(element_prefix("<numCache/>", &doc.root_element()), "");
    }

    // --- P10: edited round-trip (open → edit → reflow → save → reopen) -----------------------

    #[test]
    fn edited_line_chart_roundtrips_reflected_values() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("line_chart.xlsx");
        authoring::write_line_fixture(&original).unwrap();
        let sheets = discover(&original).unwrap();
        let spec = discover_and_parse(&original).unwrap().remove(0);

        // Open the model and edit a source cell (Data!B2 = 120 → 999).
        let mut doc = WorkbookDocument::open(&original).unwrap();
        doc.set_cell_input(0, CellRef::new(1, 1), "999").unwrap();
        doc.evaluate();

        // Reflow the chart from the CURRENT model values (the real live-binding path). Sheet
        // references resolve by name to a stable `SheetId`, then to a model index for the read —
        // exactly as the worker's `reresolve_charts` closures do.
        let binding = parse_chart_binding(&spec.source().unwrap().chart_xml);
        let props = doc.sheet_properties(); // (stable id, name), in workbook order
        let resolve_sheet = |name: &str| -> Option<SheetId> {
            props
                .iter()
                .find(|(_, n)| n == name)
                .map(|(id, _)| SheetId(*id))
        };
        let read_cell = |sheet: SheetId, cell: CellRef| -> CellData {
            match props.iter().position(|(id, _)| SheetId(*id) == sheet) {
                Some(idx) => doc.cell_value(idx as u32, cell),
                None => CellData::Empty,
            }
        };
        let data_sheet = resolve_sheet("Data").unwrap();
        let reflowed = resolve_chart(
            spec.chart().unwrap(),
            &binding,
            data_sheet,
            &resolve_sheet,
            &read_cell,
        );
        assert_eq!(first_value(&reflowed), 999.0, "reflow read the live cell");

        // Save: the edited model body + the patched chart source.
        let patched = patch_chart_source(&spec.source().unwrap().chart_xml, &reflowed).unwrap();
        let nochart = dir.path().join("edited_nochart.xlsx");
        doc.save(&nochart).unwrap();
        let mut patches = BTreeMap::new();
        patches.insert("xl/charts/chart1.xml".to_string(), patched);
        let model_bytes = std::fs::read(&nochart).unwrap();
        let targets = name_targets(&original, &model_bytes, &sheets);
        let (bytes, report) =
            reinject(&original, &model_bytes, &sheets, &targets, &patches).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        assert_eq!(report.charts_preserved, 1);
        assert_eq!(report.patched_charts, vec!["xl/charts/chart1.xml"]);

        // (a) Reopening via our loader shows the edited value; the untouched second series is kept.
        let reopened = discover_and_parse(&out).unwrap();
        assert_eq!(reopened.len(), 1);
        assert_eq!(first_value(reopened[0].chart().unwrap()), 999.0);
        match &reopened[0].chart().unwrap().series[1].data {
            SeriesData::CategoryValue { values, .. } => assert_eq!(values[0], 80.0),
            other => panic!("expected CategoryValue, got {other:?}"),
        }

        // (b) The edited cell landed in the saved MODEL body too (not just the chart cache).
        let reopened_doc = WorkbookDocument::open(&out).unwrap();
        assert_eq!(
            reopened_doc.cell_value(0, CellRef::new(1, 1)),
            CellData::Number(999.0)
        );

        // (c) A valid OPC package IronCalc reopens, with the chart CT + worksheet <drawing> intact.
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();
        assert!(xlsx::read_entry(&out, "[Content_Types].xml")
            .unwrap()
            .contains("/xl/charts/chart1.xml"));
        assert!(xlsx::read_entry(&out, "xl/worksheets/sheet1.xml")
            .unwrap()
            .contains("<drawing "));
    }

    #[test]
    fn untouched_chart_is_byte_identical_after_edited_save() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("charts_basic.xlsx");
        authoring::write_fixture(&original).unwrap();
        let sheets = discover(&original).unwrap();

        // Patch ONLY the line chart (chart2); the column + pie charts must byte-preserve.
        let chart2_xml = xlsx::read_entry(&original, "xl/charts/chart2.xml").unwrap();
        let mut chart2 = parse_chart_xml(&chart2_xml).unwrap();
        if let SeriesData::CategoryValue { values, .. } = &mut chart2.series[0].data {
            values[0] = 777.0;
        }
        let mut patches = BTreeMap::new();
        patches.insert(
            "xl/charts/chart2.xml".to_string(),
            patch_chart_source(&chart2_xml, &chart2).unwrap(),
        );

        let model_bytes = ironcalc_bytes(&original);
        let targets = name_targets(&original, &model_bytes, &sheets);
        let (bytes, report) =
            reinject(&original, &model_bytes, &sheets, &targets, &patches).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();
        assert_eq!(report.patched_charts, vec!["xl/charts/chart2.xml"]);

        // The two untouched charts are bit-identical to the original parts (byte-stable).
        for part in ["xl/charts/chart1.xml", "xl/charts/chart3.xml"] {
            assert_eq!(
                xlsx::read_entry(&out, part).unwrap(),
                xlsx::read_entry(&original, part).unwrap(),
                "{part} must be byte-stable"
            );
        }
        // The patched chart is not, and reflects the edit.
        let patched_out = xlsx::read_entry(&out, "xl/charts/chart2.xml").unwrap();
        assert_ne!(patched_out, chart2_xml);
        assert_eq!(first_value(&parse_chart_xml(&patched_out).unwrap()), 777.0);
    }

    // --- P10: multi-sheet part map + fail-loud ----------------------------------------------

    #[test]
    fn multi_sheet_save_maps_by_name_and_preserves_both_charts() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("two_sheet.xlsx");
        authoring::write_two_sheet_fixture(&original).unwrap();

        let out = dir.path().join("out.xlsx");
        let report = save_with_charts(&original, &out).unwrap();
        assert_eq!(report.charts_preserved, 2);
        // Both regenerated worksheets got a re-injected <drawing>.
        let mut patched = report.patched_sheets.clone();
        patched.sort();
        assert_eq!(
            patched,
            vec![
                "xl/worksheets/sheet1.xml".to_string(),
                "xl/worksheets/sheet2.xml".to_string()
            ]
        );

        // Reopen: both charts survive, IronCalc accepts the package, and each chart is grouped
        // under its OWN worksheet (column on Data, line on Summary).
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();
        assert_eq!(load_charts_from_xlsx(&out).unwrap().len(), 2);
        let groups = discover_and_parse_by_sheet(&out).unwrap();
        let names: Vec<&str> = groups.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"Data") && names.contains(&"Summary"),
            "{names:?}"
        );
        for (name, specs) in &groups {
            assert_eq!(specs.len(), 1, "{name} has one chart");
            match name.as_str() {
                "Data" => assert!(matches!(
                    specs[0].1.chart().unwrap().kind,
                    freecell_chart_model::ChartKind::Bar { .. }
                )),
                "Summary" => assert!(matches!(
                    specs[0].1.chart().unwrap().kind,
                    freecell_chart_model::ChartKind::Line { .. }
                )),
                other => panic!("unexpected sheet {other}"),
            }
        }
    }

    #[test]
    fn reinject_live_charts_fails_loudly_on_a_model_sheet_with_no_output_part() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("line_chart.xlsx");
        authoring::write_line_fixture(&original).unwrap();

        // GENUINE CORRUPTION (not a user rename/delete): the live chart asserts its host sheet is
        // "Ghost" (Some, so the worker believes the sheet exists), but the serialized model emits no
        // worksheet part for "Ghost". Re-injecting would corrupt the file → fail loudly.
        let chart =
            parse_chart_xml(&xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap()).unwrap();
        let live = vec![live_on("Ghost", "xl/charts/chart1.xml", chart)];
        let err = reinject_live_charts(&original, &ironcalc_bytes(&original), &live).unwrap_err();
        let msg = format!("{err:#}");
        assert!(
            msg.contains("Ghost"),
            "error must name the missing worksheet, got: {msg}"
        );
    }

    #[test]
    fn reinject_live_charts_drops_a_deleted_host_sheet_without_failing() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("two_sheet.xlsx");
        authoring::write_two_sheet_fixture(&original).unwrap();

        // The model body still has BOTH sheets, but the worker reports the Summary chart's host as
        // deleted (sheet_name None). Its drawing is dropped gracefully; Data's chart survives.
        let data_chart =
            parse_chart_xml(&xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap()).unwrap();
        let live = vec![
            live_on("Data", "xl/charts/chart1.xml", data_chart),
            LiveChart {
                sheet_name: None, // Summary deleted in-session
                chart_part: "xl/charts/chart2.xml".into(),
                chart: Some(
                    parse_chart_xml(&xlsx::read_entry(&original, "xl/charts/chart2.xml").unwrap())
                        .unwrap(),
                ),
            },
        ];
        let (bytes, report) =
            reinject_live_charts(&original, &ironcalc_bytes(&original), &live).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();

        // Save succeeded; only the surviving sheet's chart is re-injected + discoverable.
        assert_eq!(report.charts_preserved, 1);
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();
        let reopened = discover_and_parse(&out).unwrap();
        assert_eq!(reopened.len(), 1);
        assert!(matches!(
            reopened[0].chart().unwrap().kind,
            freecell_chart_model::ChartKind::Bar { .. }
        ));
        // The dropped drawing leaves NO orphaned parts or content-type overrides behind.
        assert!(xlsx::read_entry(&out, "xl/charts/chart2.xml").is_err());
        assert!(xlsx::read_entry(&out, "xl/drawings/drawing2.xml").is_err());
        assert!(!xlsx::read_entry(&out, "[Content_Types].xml")
            .unwrap()
            .contains("/xl/charts/chart2.xml"));
    }

    /// Architecture §6 (no silent chart drop): a drawing whose charts were ALL unparseable at load
    /// (surface/radar/…) is **byte-preserved** onto its host worksheet when that sheet survives —
    /// even though it was never bound. A supported chart on Data is edited + patched; the unsupported
    /// chart on Summary must still be present, byte-identical.
    #[test]
    fn reinject_live_charts_carries_an_unbound_drawing_when_its_sheet_survives() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("mixed.xlsx");
        authoring::write_two_sheet_supported_plus_unsupported_fixture(&original).unwrap();

        // Only the SUPPORTED column chart (chart1 on Data) is bound; the surface chart (chart2 on
        // Summary) parses to nothing → no LiveChart. Edit the supported chart.
        let mut supported =
            parse_chart_xml(&xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap()).unwrap();
        if let SeriesData::CategoryValue { values, .. } = &mut supported.series[0].data {
            values[0] = 888.0;
        }
        let live = vec![live_on("Data", "xl/charts/chart1.xml", supported)];

        let (bytes, report) =
            reinject_live_charts(&original, &ironcalc_bytes(&original), &live).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();

        // Both drawings re-injected (Data patched, Summary best-effort carried); IronCalc reopens.
        assert_eq!(report.charts_preserved, 2);
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();
        // The supported chart carries the edit.
        assert_eq!(
            first_value(
                &parse_chart_xml(&xlsx::read_entry(&out, "xl/charts/chart1.xml").unwrap()).unwrap()
            ),
            888.0
        );
        // The UNSUPPORTED chart is STILL PRESENT, byte-identical (not silently dropped).
        assert_eq!(
            xlsx::read_entry(&out, "xl/charts/chart2.xml").unwrap(),
            xlsx::read_entry(&original, "xl/charts/chart2.xml").unwrap(),
        );
        // Summary's worksheet kept its <drawing> (proving carry, not drop).
        assert!(xlsx::read_entry(&out, "xl/worksheets/sheet2.xml")
            .unwrap()
            .contains("<drawing "));
    }

    /// P14: an Unsupported chart bound with `chart: None` (as `discover_and_parse` now retains a
    /// surface/radar/… chart) is **byte-preserved** — never parsed or patched — while its drawing
    /// still follows its surviving host sheet. The "byte-preserve as a **bound** spec" outcome.
    #[test]
    fn reinject_live_charts_byte_preserves_a_bound_unsupported_chart() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("mixed.xlsx");
        authoring::write_two_sheet_supported_plus_unsupported_fixture(&original).unwrap();

        // Data's supported column chart is edited (Some); Summary's surface chart is a bound
        // Unsupported spec (None) — the shape `ChartBindings::live_charts` now produces for it.
        let mut supported =
            parse_chart_xml(&xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap()).unwrap();
        if let SeriesData::CategoryValue { values, .. } = &mut supported.series[0].data {
            values[0] = 424.0;
        }
        let live = vec![
            live_on("Data", "xl/charts/chart1.xml", supported),
            LiveChart {
                sheet_name: Some("Summary".into()),
                chart_part: "xl/charts/chart2.xml".into(),
                chart: None, // unsupported → byte-preserve, never patch
            },
        ];

        let (bytes, report) =
            reinject_live_charts(&original, &ironcalc_bytes(&original), &live).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();

        // Only the supported chart is patched; the unsupported one is byte-identical to the original.
        assert_eq!(report.patched_charts, vec!["xl/charts/chart1.xml"]);
        assert_eq!(
            xlsx::read_entry(&out, "xl/charts/chart2.xml").unwrap(),
            xlsx::read_entry(&original, "xl/charts/chart2.xml").unwrap(),
        );
        // Both drawings survived (Data patched, Summary's unsupported carried); IronCalc reopens.
        assert_eq!(report.charts_preserved, 2);
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();
        assert!(xlsx::read_entry(&out, "xl/worksheets/sheet2.xml")
            .unwrap()
            .contains("<drawing "));
    }

    /// Two byte-identical chart parts bound to DIFFERENT live values → each part is patched with ITS
    /// OWN values (not the first XML match), the wrong-patch the source-XML matcher would have hit.
    #[test]
    fn reinject_live_charts_patches_each_part_with_its_own_values() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("twins.xlsx");
        authoring::write_two_sheet_twin_charts_fixture(&original).unwrap();
        // The two chart parts are byte-identical (same authored XML).
        assert_eq!(
            xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap(),
            xlsx::read_entry(&original, "xl/charts/chart2.xml").unwrap(),
        );

        let base =
            parse_chart_xml(&xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap()).unwrap();
        let with_value = |v: f64| {
            let mut c = base.clone();
            if let SeriesData::CategoryValue { values, .. } = &mut c.series[0].data {
                values[0] = v;
            }
            c
        };
        let live = vec![
            live_on("Data", "xl/charts/chart1.xml", with_value(111.0)),
            live_on("Summary", "xl/charts/chart2.xml", with_value(222.0)),
        ];
        let (bytes, _report) =
            reinject_live_charts(&original, &ironcalc_bytes(&original), &live).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();

        // Each part carries its OWN first value — not both 111 (first-match wrong-patch bug).
        let v1 = first_value(
            &parse_chart_xml(&xlsx::read_entry(&out, "xl/charts/chart1.xml").unwrap()).unwrap(),
        );
        let v2 = first_value(
            &parse_chart_xml(&xlsx::read_entry(&out, "xl/charts/chart2.xml").unwrap()).unwrap(),
        );
        assert_eq!((v1, v2), (111.0, 222.0));
    }

    // --- P10: app-save orchestration (reinject_live_charts) ----------------------------------

    /// The engine seam the worker's `Command::Save` drives: given the live charts (with current
    /// values), an edited chart is patched and the rest byte-preserved — all in one call.
    #[test]
    fn reinject_live_charts_patches_edited_and_byte_preserves_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("charts_basic.xlsx");
        authoring::write_fixture(&original).unwrap();

        // The worker's live charts (all on "Data"), with the line chart's (chart2) value edited.
        let parts = [
            "xl/charts/chart1.xml",
            "xl/charts/chart2.xml",
            "xl/charts/chart3.xml",
        ];
        let mut live: Vec<LiveChart> = parts
            .iter()
            .map(|part| {
                let c = parse_chart_xml(&xlsx::read_entry(&original, part).unwrap()).unwrap();
                live_on("Data", part, c)
            })
            .collect();
        if let SeriesData::CategoryValue { values, .. } =
            &mut live[1].chart.as_mut().unwrap().series[0].data
        {
            values[0] = 555.0;
        }

        let (bytes, report) =
            reinject_live_charts(&original, &ironcalc_bytes(&original), &live).unwrap();
        let out = dir.path().join("out.xlsx");
        std::fs::write(&out, &bytes).unwrap();

        // Only the edited chart is patched; the other two are byte-identical to the originals.
        assert_eq!(report.patched_charts, vec!["xl/charts/chart2.xml"]);
        for part in ["xl/charts/chart1.xml", "xl/charts/chart3.xml"] {
            assert_eq!(
                xlsx::read_entry(&out, part).unwrap(),
                xlsx::read_entry(&original, part).unwrap(),
                "{part} byte-stable"
            );
        }
        // Reopen: the edit survived and IronCalc accepts the package.
        let reopened = discover_and_parse(&out).unwrap();
        assert_eq!(first_value(reopened[1].chart().unwrap()), 555.0);
        ironcalc::import::load_from_xlsx(out.to_str().unwrap(), "en", "UTC", "en").unwrap();
    }
}
