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

use freecell_chart_model::{
    Anchor, AnchorCell, Category, Chart, ChartColor, ChartKind, Color, SeriesData, ThemePalette,
};

use super::load::{self, parse_chart_xml, SheetDrawing};
use super::{chrome, xlsx};

/// The drawingml-main namespace URI — the `a:` prefix the chrome patcher resolves against the file's
/// own namespace declarations (so an inserted title / fill keeps the file's exact `a:` spelling).
const NS_DRAWINGML_MAIN: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";

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
        &HashMap::new(),
        &HashSet::new(),
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
    anchor_edits: &HashMap<String, Anchor>,
    deletes: &HashSet<String>,
) -> Result<(Vec<u8>, SaveReport)> {
    let sheets = load::discover(original)?;
    let patches = build_live_patches(original, live)?;
    let targets = live_sheet_targets(original, model_bytes, &sheets, live)?;
    reinject(
        original,
        model_bytes,
        &sheets,
        &targets,
        &patches,
        anchor_edits,
        deletes,
    )
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
pub(super) fn name_to_part_map(bytes: &[u8]) -> Result<HashMap<String, String>> {
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
    anchor_edits: &HashMap<String, Anchor>,
    deletes: &HashSet<String>,
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

    // P18: a drawing whose charts are ALL deleted **and which holds nothing else** is dropped whole
    // (reuses the deleted-host path); one with only SOME charts deleted, a moved/resized chart, or
    // **co-located non-chart anchors** (shapes / textboxes live in the SAME `drawingN.xml` as chart
    // anchors) survives with a **patched** drawing XML (+ `_rels`) so those shapes are never silently
    // dropped (functional_spec §7 / architecture §6 — "never silently drop"). `SheetDrawing.charts`
    // tracks only the chart anchors, so we compare it against the drawing's TOTAL anchor count.
    // `eff_targets` folds the wholly-dropped override into the caller's targets so the plan,
    // content-types, and report all agree.
    let mut eff_targets: Vec<Option<String>> = targets.to_vec();
    for (k, sheet) in sheets.iter().enumerate() {
        let all_charts_deleted =
            !sheet.charts.is_empty() && sheet.charts.iter().all(|dc| deletes.contains(&dc.part));
        if all_charts_deleted
            && drawing_anchor_count(&mut orig, &sheet.drawing_part)? <= sheet.charts.len()
        {
            eff_targets[k] = None;
        }
    }

    // The whole part chain of every DROPPED drawing — excluded from carry + content-types so no
    // orphaned chart/drawing parts leak into the output.
    let mut dropped_parts: HashSet<String> = HashSet::new();
    for (sheet, target) in sheets.iter().zip(&eff_targets) {
        if target.is_none() {
            for part in drawing_chain_parts(&mut orig, sheet)? {
                dropped_parts.insert(part);
            }
        }
    }

    // Patched drawing parts (moved/resized anchors, individually-removed deleted anchors) — a
    // `drawing_part → patched XML` / `drawing_rels_part → patched rels` substitution applied in the
    // carry loop. A surviving drawing's individually-deleted charts have their part chains dropped.
    let mut drawing_subs: HashMap<String, String> = HashMap::new();
    let mut drawing_rels_subs: HashMap<String, String> = HashMap::new();
    for (sheet, target) in sheets.iter().zip(&eff_targets) {
        if target.is_none() {
            continue; // wholly dropped — no per-anchor patch
        }
        let touched = sheet
            .charts
            .iter()
            .any(|dc| deletes.contains(&dc.part) || anchor_edits.contains_key(&dc.part));
        if !touched {
            continue; // this drawing's charts are all untouched → byte-preserve verbatim
        }
        let part_by_rel = drawing_part_by_rel(&mut orig, sheet)?;
        let drawing_xml = xlsx::read_entry_from(&mut orig, &sheet.drawing_part)
            .with_context(|| format!("reading drawing part {}", sheet.drawing_part))?;
        let (patched_xml, _remaining) =
            patch_drawing_xml(&drawing_xml, &part_by_rel, anchor_edits, deletes)?;
        drawing_subs.insert(sheet.drawing_part.clone(), patched_xml);
        // Remove each individually-deleted chart's rel from the drawing `_rels` + drop its part
        // chain, so no dangling relationship or orphaned chart part survives.
        let deleted_rel_ids: Vec<String> = part_by_rel
            .iter()
            .filter(|(_, part)| deletes.contains(*part))
            .map(|(rel_id, _)| rel_id.clone())
            .collect();
        if !deleted_rel_ids.is_empty() {
            let rels_part = xlsx::rels_part_for(&sheet.drawing_part);
            if xlsx::has_entry(&mut orig, &rels_part) {
                let rels_xml = xlsx::read_entry_from(&mut orig, &rels_part)?;
                drawing_rels_subs
                    .insert(rels_part, patch_drawing_rels(&rels_xml, &deleted_rel_ids)?);
            }
        }
        for dc in &sheet.charts {
            if deletes.contains(&dc.part) {
                for part in chart_chain_parts(&mut orig, &dc.part)? {
                    dropped_parts.insert(part);
                }
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
    for (k, (sheet, target)) in sheets.iter().zip(&eff_targets).enumerate() {
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

    // Carry the original chart + drawing parts. A chart part with an edited-loaded patch is written
    // patched (its reflowed caches); a moved/resized or partially-deleted drawing (+ its `_rels`) is
    // written with its P18-patched XML; every other part goes byte-for-byte (bit-stable).
    let mut patched_charts: Vec<String> = Vec::new();
    for (name, bytes) in &carry {
        if let Some(patched_xml) = patches.get(name) {
            write_part(&mut zw, opts, name, patched_xml.as_bytes())?;
            patched_charts.push(name.clone());
        } else if let Some(patched_xml) = drawing_subs.get(name) {
            write_part(&mut zw, opts, name, patched_xml.as_bytes())?;
        } else if let Some(patched_rels) = drawing_rels_subs.get(name) {
            write_part(&mut zw, opts, name, patched_rels.as_bytes())?;
        } else {
            write_part(&mut zw, opts, name, bytes)?;
        }
    }

    let cursor = zw.finish().context("finishing re-injected zip")?;
    let report = SaveReport {
        // Charts on re-injected (non-dropped) sheets, minus any individually deleted (P18).
        charts_preserved: sheets
            .iter()
            .zip(&eff_targets)
            .filter(|(_, t)| t.is_some())
            .map(|(s, _)| {
                s.charts
                    .iter()
                    .filter(|dc| !deletes.contains(&dc.part))
                    .count()
            })
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
// Drawing-anchor patching (P18: move/resize/delete a loaded chart in the retained drawing part)
// ---------------------------------------------------------------------------------------------

/// The `relationship id → target chart part` map of one drawing's `_rels` (resolved against the
/// drawing part), so a `<c:chart r:id=…>` in the drawing XML resolves to its `xl/charts/chartN.xml`.
/// Empty when the drawing has no `_rels` (a chartless drawing never reaches the P18 patch path).
fn drawing_part_by_rel<R: Read + std::io::Seek>(
    orig: &mut zip::ZipArchive<R>,
    sheet: &SheetDrawing,
) -> Result<HashMap<String, String>> {
    let rels_part = xlsx::rels_part_for(&sheet.drawing_part);
    if !xlsx::has_entry(orig, &rels_part) {
        return Ok(HashMap::new());
    }
    let rels_xml = xlsx::read_entry_from(orig, &rels_part)?;
    Ok(xlsx::parse_rels(&rels_xml)?
        .into_iter()
        .map(|(rel_id, rel)| {
            (
                rel_id,
                xlsx::resolve_target(&sheet.drawing_part, &rel.target),
            )
        })
        .collect())
}

/// The chart part + its `_rels` + non-external aux targets (`colorsN`/`styleN`/embeddings) — one
/// deleted chart's whole package chain, dropped from carry + content-types so no orphan survives
/// when a chart is individually removed from a *surviving* drawing (P18). Mirrors the chart portion
/// of [`drawing_chain_parts`].
fn chart_chain_parts<R: Read + std::io::Seek>(
    orig: &mut zip::ZipArchive<R>,
    chart_part: &str,
) -> Result<Vec<String>> {
    let mut parts = vec![chart_part.to_string()];
    let chart_rels = xlsx::rels_part_for(chart_part);
    if xlsx::has_entry(orig, &chart_rels) {
        let rels_xml = xlsx::read_entry_from(orig, &chart_rels)?;
        parts.push(chart_rels);
        for rel in xlsx::parse_rels(&rels_xml)?.values() {
            let target = xlsx::resolve_target(chart_part, &rel.target);
            if xlsx::has_entry(orig, &target) {
                parts.push(target);
            }
        }
    }
    Ok(parts)
}

/// Whether a local element name is one of the three spreadsheet-drawing anchor kinds.
fn is_anchor_element_name(name: &str) -> bool {
    matches!(name, "twoCellAnchor" | "oneCellAnchor" | "absoluteAnchor")
}

/// The total number of `<xdr:*Anchor>` elements in a drawing part — chart frames **and** any
/// co-located shapes/textboxes/images. Compared against the chart-anchor count (`SheetDrawing.charts`)
/// to decide whether a fully-chart-deleted drawing can be dropped whole (only when it holds nothing
/// but those charts) or must be patched to preserve its non-chart anchors (P18).
fn drawing_anchor_count<R: Read + std::io::Seek>(
    orig: &mut zip::ZipArchive<R>,
    drawing_part: &str,
) -> Result<usize> {
    let xml = xlsx::read_entry_from(orig, drawing_part)
        .with_context(|| format!("reading drawing part {drawing_part}"))?;
    let doc = Document::parse(&xml)
        .with_context(|| format!("parsing drawing part {drawing_part} to count anchors"))?;
    Ok(doc
        .descendants()
        .filter(|n| n.is_element() && is_anchor_element_name(n.tag_name().name()))
        .count())
}

/// Patch a worksheet drawing part's `twoCellAnchor`s for P18 move/resize/delete: for each anchor
/// element, resolve the chart it frames (via its `<c:chart r:id>` → `part_by_rel`) and either
/// **remove** the whole anchor (the chart is in `deletes`) or **rewrite** its `<xdr:from>`/`<xdr:to>`
/// (the chart has an entry in `anchor_edits`). Untouched anchors — and all non-anchor bytes,
/// namespaces, and shape ids — are preserved **byte-for-byte** (the same targeted-splice pattern as
/// [`patch_chart_source`]). Returns the patched XML + the count of anchors that remain.
fn patch_drawing_xml(
    drawing_xml: &str,
    part_by_rel: &HashMap<String, String>,
    anchor_edits: &HashMap<String, Anchor>,
    deletes: &HashSet<String>,
) -> Result<(String, usize)> {
    let doc = Document::parse(drawing_xml).context("parsing drawing XML to patch")?;
    // (byte_range, replacement) edits, applied descending so earlier offsets stay valid.
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let mut remaining = 0usize;
    for anchor in doc
        .descendants()
        .filter(|n| n.is_element() && is_anchor_element_name(n.tag_name().name()))
    {
        // The chart part this anchor frames, via its `<c:chart r:id>` (if any).
        let chart_part = anchor
            .descendants()
            .find(|n| n.is_element() && n.tag_name().name() == "chart")
            .and_then(|c| xlsx::attr(&c, "id"))
            .and_then(|rel_id| part_by_rel.get(rel_id));
        match chart_part {
            Some(part) if deletes.contains(part) => {
                edits.push((anchor.range(), String::new())); // remove the whole anchor
            }
            Some(part) if anchor_edits.contains_key(part) => {
                remaining += 1;
                let new = &anchor_edits[part];
                if let Some(from) = child(&anchor, "from") {
                    let prefix = element_prefix(drawing_xml, &from);
                    edits.push((
                        from.range(),
                        serialize_anchor_corner(prefix, "from", &new.from),
                    ));
                }
                if let Some(to) = child(&anchor, "to") {
                    let prefix = element_prefix(drawing_xml, &to);
                    edits.push((to.range(), serialize_anchor_corner(prefix, "to", &new.to)));
                }
            }
            _ => remaining += 1, // untouched (or a non-chart anchor — image/shape) → keep verbatim
        }
    }

    edits.sort_by_key(|(range, _)| std::cmp::Reverse(range.start));
    let mut patched = drawing_xml.to_string();
    let mut prev_start = patched.len();
    for (range, replacement) in edits {
        debug_assert!(
            range.end <= prev_start,
            "drawing edit ranges must be disjoint"
        );
        prev_start = range.start;
        patched.replace_range(range, &replacement);
    }
    Ok((patched, remaining))
}

/// One `<xdr:from>`/`<xdr:to>` corner element, rebuilt with the source's namespace `prefix` (e.g.
/// `xdr:`) so the patched drawing keeps the file's exact prefixes.
fn serialize_anchor_corner(prefix: &str, tag: &str, cell: &AnchorCell) -> String {
    format!(
        "<{p}{tag}><{p}col>{c}</{p}col><{p}colOff>{co}</{p}colOff>\
         <{p}row>{r}</{p}row><{p}rowOff>{ro}</{p}rowOff></{p}{tag}>",
        p = prefix,
        tag = tag,
        c = cell.col,
        co = cell.col_off_emu,
        r = cell.row,
        ro = cell.row_off_emu,
    )
}

/// Remove the `<Relationship>` entries whose `Id` is in `deleted_rel_ids` from a drawing `_rels`
/// part (P18 delete) — so a deleted chart leaves no dangling relationship to a dropped chart part.
fn patch_drawing_rels(rels_xml: &str, deleted_rel_ids: &[String]) -> Result<String> {
    let doc = Document::parse(rels_xml).context("parsing drawing _rels to patch")?;
    let mut ranges: Vec<Range<usize>> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Relationship")
        .filter(|n| xlsx::attr(n, "Id").is_some_and(|id| deleted_rel_ids.iter().any(|d| d == id)))
        .map(|n| n.range())
        .collect();
    ranges.sort_by_key(|r| std::cmp::Reverse(r.start));
    let mut patched = rels_xml.to_string();
    for range in ranges {
        patched.replace_range(range, "");
    }
    Ok(patched)
}

// ---------------------------------------------------------------------------------------------
// Worksheet + content-types + rels patching
// ---------------------------------------------------------------------------------------------

/// Injects `<drawing r:id="{rel_id}"/>` before `</worksheet>` (idempotent) and ensures the
/// worksheet root binds the `r:` prefix the injected element needs.
pub(super) fn patch_worksheet(ws: &str, rel_id: &str) -> Result<String> {
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
pub(super) fn ensure_r_namespace(ws: &str) -> Result<String> {
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
pub(super) fn build_sheet_rels(
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
pub(super) fn relative_part(from_part: &str, to_part: &str) -> String {
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

pub(super) fn read_named_bytes<R: Read + std::io::Seek>(
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

pub(super) fn read_named_string<R: Read + std::io::Seek>(
    archive: &mut zip::ZipArchive<R>,
    name: &str,
) -> Result<String> {
    String::from_utf8(read_named_bytes(archive, name)?)
        .with_context(|| format!("zip entry {name} is not UTF-8"))
}

pub(super) fn write_part<W: Write + std::io::Seek>(
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

    // --- Chrome edits (P20 edit contract, functional_spec §6) ------------------------------
    // Splice ONLY the chrome fields that DIFFER from the file XML — title, legend, axis titles,
    // series colors, data labels — so an unmodeled DrawingML element (a gradient, a theme effect,
    // rounded corners) stays byte-for-byte. The file XML is re-parsed to diff against the target
    // model; a chart whose chrome is untouched adds no chrome edit at all.
    if let (Ok(cached), Some(chart_node)) = (parse_chart_xml(chart_xml), child(&root, "chart")) {
        collect_chrome_edits(chart_xml, &root, &chart_node, chart, &cached, &mut edits);
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

// ---------------------------------------------------------------------------------------------
// Chrome patcher (P20): splice only the changed title / legend / axis-title / series-color /
// data-label sub-elements into a loaded chart's retained source, preserving everything else.
// ---------------------------------------------------------------------------------------------

// Per-parent schema orders: the local names that come strictly AFTER an upserted child. A fresh
// insert lands before the first present following sibling (else before the parent's close tag), so
// it is always schema-valid. Deliberately broad (naming later siblings we don't emit) so an
// unmodeled later element still anchors the insert correctly.

/// After `c:chart/c:title`.
const TITLE_FOLLOWING: &[&str] = &[
    "autoTitleDeleted",
    "pivotFmts",
    "view3D",
    "floor",
    "sideWall",
    "backWall",
    "plotArea",
    "legend",
    "plotVisOnly",
    "dispBlanksAs",
    "showDLblsOverMax",
    "extLst",
];
/// After `c:chart/c:autoTitleDeleted`.
const AUTO_TITLE_FOLLOWING: &[&str] = &[
    "pivotFmts",
    "view3D",
    "floor",
    "sideWall",
    "backWall",
    "plotArea",
    "legend",
    "plotVisOnly",
    "dispBlanksAs",
    "showDLblsOverMax",
    "extLst",
];
/// After `c:chart/c:legend`.
const LEGEND_FOLLOWING: &[&str] = &["plotVisOnly", "dispBlanksAs", "showDLblsOverMax", "extLst"];
/// After a `c:catAx`/`c:valAx` `c:title`.
const AXIS_TITLE_FOLLOWING: &[&str] = &[
    "numFmt",
    "majorTickMark",
    "minorTickMark",
    "tickLblPos",
    "spPr",
    "txPr",
    "crossAx",
    "crosses",
    "crossesAt",
    "auto",
    "lblAlgn",
    "lblOffset",
    "tickLblSkip",
    "tickMarkSkip",
    "noMultiLvlLbl",
    "dispUnits",
    "crossBetween",
    "majorUnit",
    "minorUnit",
    "extLst",
];
/// After a `c:ser` `c:spPr` (whole-element insert when the series has none).
const SER_SPPR_FOLLOWING: &[&str] = &[
    "marker",
    "invertIfNegative",
    "pictureOptions",
    "explosion",
    "dPt",
    "dLbls",
    "trendline",
    "errBars",
    "cat",
    "val",
    "xVal",
    "yVal",
    "smooth",
    "bubble3D",
    "bubbleSize",
    "shape",
    "extLst",
];
/// After a `c:ser` `c:dLbls`.
const SER_DLBLS_FOLLOWING: &[&str] = &[
    "trendline",
    "errBars",
    "cat",
    "val",
    "xVal",
    "yVal",
    "smooth",
    "bubble3D",
    "bubbleSize",
    "shape",
    "extLst",
];
/// After a `c:spPr` fill (a solid/gradient/… fill), i.e. inside `spPr` before the line stroke.
const SPPR_FILL_FOLLOWING: &[&str] = &["ln", "effectLst", "effectDag", "scene3d", "sp3d", "extLst"];
/// The DrawingML fill variants a series `spPr` fill upsert replaces (only one may be present).
const FILL_VARIANTS: &[&str] = &[
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
];

/// The `a:` (drawingml-main) prefix (incl. the trailing `:`, or `""` for a default namespace) the
/// chart part declares — so an inserted title / fill keeps the file's exact spelling. Real chart
/// parts always declare this namespace; the `"a:"` fallback only fires for a (non-conformant) part
/// that uses no drawingml at all.
fn drawingml_prefix(root: &Node) -> String {
    if let Some(p) = root.lookup_prefix(NS_DRAWINGML_MAIN) {
        return format!("{p}:");
    }
    if root.default_namespace() == Some(NS_DRAWINGML_MAIN) {
        return String::new();
    }
    "a:".to_string()
}

/// The `RRGGBB` [`Color`] for a [`ChartColor`] (a theme reference resolves to its office-default RGB,
/// as the loaded-edit path only ever sets a concrete sRGB).
fn chart_color_srgb(c: &ChartColor) -> Color {
    match c {
        ChartColor::Rgb(color) => *color,
        ChartColor::Theme { slot, .. } => ThemePalette::office_default().color(*slot),
    }
}

/// Whether `node` is a **self-closing** element (`<x/>`) — which has no content region to splice a
/// child into, so a would-be child insert must instead replace the whole element (the caller rebuilds
/// it in open/close form). `<x></x>` (empty with a close tag) is **not** self-closing (its close tag
/// is a valid insert anchor).
fn is_self_closing(src: &str, node: &Node) -> bool {
    let range = node.range();
    src[range.start..range.end].trim_end().ends_with("/>")
}

/// The byte offset at which to insert a fresh child of `parent`: before the first present sibling in
/// `following` (schema order), else just before the parent's closing tag.
///
/// Precondition: `parent` is **not** self-closing — a `<x/>` element has no content region, so its
/// `rfind('<')` would return the element's own opening `<` and splice the child *before* the parent.
/// The only realistic self-closing chrome parent is a series `<c:spPr/>`, which
/// [`patch_series_color`] special-cases (whole-element replace) before ever reaching here.
fn insertion_offset(src: &str, parent: &Node, following: &[&str]) -> usize {
    debug_assert!(
        !is_self_closing(src, parent),
        "insertion_offset called on a self-closing parent — cannot insert a child into it",
    );
    if let Some(n) = parent
        .children()
        .find(|n| n.is_element() && following.contains(&n.tag_name().name()))
    {
        return n.range().start;
    }
    let range = parent.range();
    range.start
        + src[range.start..range.end]
            .rfind('<')
            .expect("an element node has a closing tag")
}

/// Upsert `parent`'s child into the replace / insert buckets. `replace_names` are the local names
/// that count as the existing element (usually `[local]`; a series fill counts every fill variant).
/// `new_xml` = `Some(_)` to set (replace-or-insert), `None` to remove. A fresh insert lands before
/// the first `following` sibling; `seq` orders inserts that share an offset (a series' new
/// `spPr` + `dLbls`).
#[allow(clippy::too_many_arguments)]
fn upsert_child(
    src: &str,
    parent: &Node,
    replace_names: &[&str],
    new_xml: Option<String>,
    following: &[&str],
    seq: usize,
    replaces: &mut Vec<(Range<usize>, String)>,
    inserts: &mut Vec<(usize, usize, String)>,
) {
    let existing = parent
        .children()
        .find(|n| n.is_element() && replace_names.contains(&n.tag_name().name()));
    match (existing, new_xml) {
        (Some(node), Some(xml)) => replaces.push((node.range(), xml)),
        (Some(node), None) => replaces.push((node.range(), String::new())),
        (None, Some(xml)) => inserts.push((insertion_offset(src, parent, following), seq, xml)),
        (None, None) => {}
    }
}

/// Collect the targeted splices for every chrome field that differs between the target `chart` and
/// the file-parsed `cached`, appending them to `edits` (charts/functional_spec §6 edit contract).
/// Only changed fields are touched.
fn collect_chrome_edits(
    src: &str,
    root: &Node,
    chart_node: &Node,
    chart: &Chart,
    cached: &Chart,
    edits: &mut Vec<(Range<usize>, String)>,
) {
    let c = element_prefix(src, chart_node);
    let a = drawingml_prefix(root);
    let mut replaces: Vec<(Range<usize>, String)> = Vec::new();
    let mut inserts: Vec<(usize, usize, String)> = Vec::new();

    // --- Title -------------------------------------------------------------------------------
    if chart.title != cached.title {
        patch_title(
            src,
            chart_node,
            c,
            &a,
            chart.title.as_deref(),
            &mut replaces,
            &mut inserts,
        );
    }

    // --- Legend ------------------------------------------------------------------------------
    if chart.legend != cached.legend {
        let new = chart.legend.map(|l| chrome::legend_element(c, l.position));
        upsert_child(
            src,
            chart_node,
            &["legend"],
            new,
            LEGEND_FOLLOWING,
            0,
            &mut replaces,
            &mut inserts,
        );
    }

    // --- Axis titles -------------------------------------------------------------------------
    if let Some(plot_area) = child(chart_node, "plotArea") {
        let is_scatter = matches!(chart.kind, ChartKind::Scatter { .. });
        let axis_nodes = axis_nodes(&plot_area, is_scatter);
        if chart.cat_axis.title != cached.cat_axis.title {
            if let Some(ax) = axis_nodes.0 {
                let new = chart
                    .cat_axis
                    .title
                    .as_deref()
                    .map(|t| chrome::title_element(c, &a, t));
                upsert_child(
                    src,
                    &ax,
                    &["title"],
                    new,
                    AXIS_TITLE_FOLLOWING,
                    0,
                    &mut replaces,
                    &mut inserts,
                );
            }
        }
        if chart.val_axis.title != cached.val_axis.title {
            if let Some(ax) = axis_nodes.1 {
                let new = chart
                    .val_axis
                    .title
                    .as_deref()
                    .map(|t| chrome::title_element(c, &a, t));
                upsert_child(
                    src,
                    &ax,
                    &["title"],
                    new,
                    AXIS_TITLE_FOLLOWING,
                    0,
                    &mut replaces,
                    &mut inserts,
                );
            }
        }
    }

    // --- Series colors + data labels ---------------------------------------------------------
    let group = child(chart_node, "plotArea").and_then(|plot| {
        plot.children()
            .find(|n| n.is_element() && load::is_chart_group(n.tag_name().name()))
    });
    if let Some(group) = group {
        let sers: Vec<Node> = group
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "ser")
            .collect();
        for (i, ser) in sers.iter().enumerate() {
            let (Some(series), Some(cached_series)) = (chart.series.get(i), cached.series.get(i))
            else {
                continue;
            };
            // Series color → the `solidFill` inside `spPr` (a co-located `a:ln` stroke survives).
            if series.color != cached_series.color {
                patch_series_color(
                    src,
                    ser,
                    c,
                    &a,
                    series.color.as_ref(),
                    &mut replaces,
                    &mut inserts,
                );
            }
            // Data labels → the series' `c:dLbls` (schema-ordered before the data roles).
            if series.data_labels != cached_series.data_labels {
                let new = series
                    .data_labels
                    .as_ref()
                    .map(|l| chrome::dlbls_element(c, l));
                upsert_child(
                    src,
                    ser,
                    &["dLbls"],
                    new,
                    SER_DLBLS_FOLLOWING,
                    1, // after a same-anchor new spPr (seq 0)
                    &mut replaces,
                    &mut inserts,
                );
            }
        }
    }

    // Merge inserts sharing an offset (a series' new spPr + dLbls) in schema (seq) order, so an
    // inserted spPr always precedes an inserted dLbls at the same anchor.
    inserts.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.cmp(&y.1)));
    let mut merged: Vec<(Range<usize>, String)> = Vec::new();
    for (off, _seq, xml) in inserts {
        match merged.last_mut() {
            Some((r, s)) if r.start == off => s.push_str(&xml),
            _ => merged.push((off..off, xml)),
        }
    }
    edits.extend(replaces);
    edits.extend(merged);
}

/// The `(category, value)` axis nodes to patch an axis title into: for scatter the two `c:valAx`
/// (first = X = category, second = Y = value); otherwise `c:catAx` + `c:valAx` — mirroring
/// [`load::parse_axes`].
fn axis_nodes<'a>(
    plot_area: &Node<'a, '_>,
    is_scatter: bool,
) -> (Option<Node<'a, 'a>>, Option<Node<'a, 'a>>) {
    if is_scatter {
        let mut vals = plot_area
            .children()
            .filter(|n| n.is_element() && n.tag_name().name() == "valAx");
        (vals.next(), vals.next())
    } else {
        (child(plot_area, "catAx"), child(plot_area, "valAx"))
    }
}

/// Splice a chart title change: replace an existing title's text holder (`c:tx`) to keep its own
/// styling, insert a fresh `c:title` when there was none (clearing `autoTitleDeleted`), or remove it
/// (setting `autoTitleDeleted=1`) when cleared.
fn patch_title(
    src: &str,
    chart_node: &Node,
    c: &str,
    a: &str,
    new_title: Option<&str>,
    replaces: &mut Vec<(Range<usize>, String)>,
    inserts: &mut Vec<(usize, usize, String)>,
) {
    let existing_title = child(chart_node, "title");
    match (existing_title, new_title) {
        (Some(title), Some(text)) => match child(&title, "tx") {
            // Replace only the text holder → preserve the title's own spPr/txPr/overlay/layout.
            Some(tx) => replaces.push((tx.range(), chrome::title_tx(c, a, text))),
            None => replaces.push((title.range(), chrome::title_element(c, a, text))),
        },
        (None, Some(text)) => {
            inserts.push((
                insertion_offset(src, chart_node, TITLE_FOLLOWING),
                0,
                chrome::title_element(c, a, text),
            ));
            // A title that was auto-deleted must be un-deleted so the new title shows.
            set_auto_title_deleted(src, chart_node, c, false, replaces, inserts);
        }
        (Some(title), None) => {
            replaces.push((title.range(), String::new()));
            set_auto_title_deleted(src, chart_node, c, true, replaces, inserts);
        }
        (None, None) => {}
    }
}

/// Set `c:autoTitleDeleted` to `deleted` (`val="1"`/`"0"`): replace an existing one, or (only when
/// `deleted`, i.e. removing a title) insert one where the title was.
fn set_auto_title_deleted(
    src: &str,
    chart_node: &Node,
    c: &str,
    deleted: bool,
    replaces: &mut Vec<(Range<usize>, String)>,
    inserts: &mut Vec<(usize, usize, String)>,
) {
    let val = if deleted { "1" } else { "0" };
    let element = format!("<{c}autoTitleDeleted val=\"{val}\"/>");
    match child(chart_node, "autoTitleDeleted") {
        Some(node) => replaces.push((node.range(), element)),
        None if deleted => inserts.push((
            insertion_offset(src, chart_node, AUTO_TITLE_FOLLOWING),
            0,
            element,
        )),
        None => {} // adding a title with no autoTitleDeleted present: absence already means "show"
    }
}

/// Splice a series color change into its `c:spPr` (upsert the `solidFill` inside an existing `spPr`
/// so a co-located `a:ln` stroke survives; insert a whole `spPr` when the series has none; remove the
/// `solidFill` when cleared).
fn patch_series_color(
    src: &str,
    ser: &Node,
    c: &str,
    a: &str,
    new_color: Option<&ChartColor>,
    replaces: &mut Vec<(Range<usize>, String)>,
    inserts: &mut Vec<(usize, usize, String)>,
) {
    let new_srgb = new_color.map(chart_color_srgb);
    match child(ser, "spPr") {
        // A self-closing `<c:spPr/>` has no content region to splice a fill into (inserting inside it
        // would land the fill *outside* the spPr, as an invalid direct child of `<c:ser>` — silent
        // color loss on reopen). Setting a color replaces the whole (empty) element with a full spPr
        // carrying the fill (lossless: a self-closing spPr held nothing else). Clearing is a no-op.
        Some(sp_pr) if is_self_closing(src, &sp_pr) => {
            if let Some(color) = new_srgb {
                replaces.push((sp_pr.range(), chrome::series_sppr_element(c, a, color)));
            }
        }
        Some(sp_pr) => {
            let new_fill = new_srgb.map(|color| chrome::sppr_solid_fill(a, color));
            // Set → replace/insert the fill inside spPr; clear → remove only a `solidFill`.
            let replace_names: &[&str] = if new_fill.is_some() {
                FILL_VARIANTS
            } else {
                &["solidFill"]
            };
            upsert_child(
                src,
                &sp_pr,
                replace_names,
                new_fill,
                SPPR_FILL_FOLLOWING,
                0,
                replaces,
                inserts,
            );
        }
        None => {
            if let Some(color) = new_srgb {
                inserts.push((
                    insertion_offset(src, ser, SER_SPPR_FOLLOWING),
                    0, // a new spPr precedes a same-anchor new dLbls (seq 1)
                    chrome::series_sppr_element(c, a, color),
                ));
            }
        }
    }
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
///
/// Shared with the write-from-model serializer ([`super::write`]) so an **authored** value cache is
/// byte-identical to a **reflowed** one (charts/components/write-path §4 — the reconciliation
/// invariant).
pub(super) fn rebuild_num_cache(prefix: &str, format_code: Option<&str>, values: &[f64]) -> String {
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

/// Rebuild a `strCache` element string with the same namespace `prefix`. Shared with the
/// write-from-model serializer ([`super::write`]) — see [`rebuild_num_cache`].
pub(super) fn rebuild_str_cache(prefix: &str, values: &[String]) -> String {
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
pub(super) fn fmt_cache_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

/// Minimal XML text escaping for reflowed cached strings.
pub(super) fn escape_xml(s: &str) -> String {
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

    // ---- P18 drawing-anchor patching (move/resize/delete a loaded chart) --------------------

    /// A minimal two-anchor drawing (two charts) with declared namespaces, in the `xdr:`-prefixed
    /// shape real files (and our authored writer) use.
    fn two_chart_drawing() -> &'static str {
        r#"<xdr:wsDr xmlns:xdr="http://schemas.openxmlformats.org/drawingml/2006/spreadsheetDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" xmlns:c="http://schemas.openxmlformats.org/drawingml/2006/chart"><xdr:twoCellAnchor><xdr:from><xdr:col>1</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>1</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>6</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>14</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><a:graphic><a:graphicData><c:chart r:id="rId1"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor><xdr:twoCellAnchor><xdr:from><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>14</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>18</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:graphicFrame><a:graphic><a:graphicData><c:chart r:id="rId2"/></a:graphicData></a:graphic></xdr:graphicFrame><xdr:clientData/></xdr:twoCellAnchor></xdr:wsDr>"#
    }

    fn part_by_rel() -> HashMap<String, String> {
        HashMap::from([
            ("rId1".to_string(), "xl/charts/chart1.xml".to_string()),
            ("rId2".to_string(), "xl/charts/chart2.xml".to_string()),
        ])
    }

    #[test]
    fn patch_drawing_xml_rewrites_a_moved_charts_from_to() {
        let new = Anchor::new(
            AnchorCell::with_offsets(3, 9525, 5, 19050),
            AnchorCell::with_offsets(9, 4762, 17, 4762),
        );
        let edits = HashMap::from([("xl/charts/chart1.xml".to_string(), new)]);
        let (patched, remaining) =
            patch_drawing_xml(two_chart_drawing(), &part_by_rel(), &edits, &HashSet::new())
                .unwrap();
        assert_eq!(
            remaining, 2,
            "both anchors remain (one moved, one untouched)"
        );
        // chart1's from/to were rewritten to the new cells + EMU offsets (prefix preserved).
        assert!(patched.contains("<xdr:from><xdr:col>3</xdr:col><xdr:colOff>9525</xdr:colOff><xdr:row>5</xdr:row><xdr:rowOff>19050</xdr:rowOff></xdr:from>"));
        assert!(patched.contains("<xdr:to><xdr:col>9</xdr:col><xdr:colOff>4762</xdr:colOff><xdr:row>17</xdr:row><xdr:rowOff>4762</xdr:rowOff></xdr:to>"));
        // The graphic frame (chart ref) is untouched, and chart2's anchor is byte-identical.
        assert!(patched.contains(r#"<c:chart r:id="rId1"/>"#));
        assert!(patched.contains("<xdr:from><xdr:col>8</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>2</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from>"));
        // The patched drawing still parses.
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    #[test]
    fn patch_drawing_xml_removes_a_deleted_charts_anchor() {
        let deletes = HashSet::from(["xl/charts/chart1.xml".to_string()]);
        let (patched, remaining) = patch_drawing_xml(
            two_chart_drawing(),
            &part_by_rel(),
            &HashMap::new(),
            &deletes,
        )
        .unwrap();
        assert_eq!(remaining, 1, "only chart2's anchor remains");
        assert!(
            !patched.contains(r#"r:id="rId1""#),
            "chart1's frame is gone"
        );
        assert!(
            patched.contains(r#"r:id="rId2""#),
            "chart2's frame survives"
        );
        // Exactly one twoCellAnchor left.
        assert_eq!(patched.matches("<xdr:twoCellAnchor>").count(), 1);
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    #[test]
    fn patch_drawing_rels_drops_deleted_relationships() {
        let rels = r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/chart" Target="../charts/chart2.xml"/></Relationships>"#;
        let patched = patch_drawing_rels(rels, &["rId1".to_string()]).unwrap();
        assert!(!patched.contains(r#"Id="rId1""#), "the deleted rel is gone");
        assert!(
            patched.contains(r#"Id="rId2""#),
            "the surviving rel is kept"
        );
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    /// Overwrite one entry of a zip on disk (test helper for crafting a fixture).
    fn rewrite_zip_entry(path: &Path, entry: &str, new_content: &str) {
        let bytes = std::fs::read(path).unwrap();
        let mut zin = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut zw = zip::ZipWriter::new(Cursor::new(Vec::<u8>::new()));
        let opts =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for i in 0..zin.len() {
            let mut f = zin.by_index(i).unwrap();
            let name = f.name().to_string();
            zw.start_file(&name, opts).unwrap();
            if name == entry {
                zw.write_all(new_content.as_bytes()).unwrap();
            } else {
                let mut buf = Vec::new();
                f.read_to_end(&mut buf).unwrap();
                zw.write_all(&buf).unwrap();
            }
        }
        std::fs::write(path, zw.finish().unwrap().into_inner()).unwrap();
    }

    /// Read a package part out of in-memory `.xlsx` bytes.
    fn entry_from_bytes(bytes: &[u8], name: &str) -> String {
        let mut z = zip::ZipArchive::new(Cursor::new(bytes.to_vec())).unwrap();
        let mut f = z.by_name(name).unwrap();
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        s
    }

    /// Moderate CR: deleting the only chart in a drawing that ALSO holds a non-chart anchor (a
    /// textbox) must preserve that co-located anchor — the whole-drawing-drop shortcut only applies
    /// when the drawing holds nothing but the deleted charts (never silently drop shapes/textboxes).
    #[test]
    fn deleting_a_chart_preserves_a_co_located_textbox_anchor() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("book.xlsx");
        authoring::write_line_fixture(&original).unwrap();

        // Splice a textbox `<xdr:sp>` anchor into the SAME drawing as the chart.
        let sheets = discover(&original).unwrap();
        let drawing_part = sheets[0].drawing_part.clone();
        let chart_part = sheets[0].charts[0].part.clone();
        let drawing_xml = xlsx::read_entry(&original, &drawing_part).unwrap();
        let textbox = r#"<xdr:twoCellAnchor><xdr:from><xdr:col>0</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>20</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:from><xdr:to><xdr:col>3</xdr:col><xdr:colOff>0</xdr:colOff><xdr:row>24</xdr:row><xdr:rowOff>0</xdr:rowOff></xdr:to><xdr:sp macro="" textlink=""><xdr:nvSpPr><xdr:cNvPr id="99" name="TextBox 99"/><xdr:cNvSpPr txBox="1"/></xdr:nvSpPr><xdr:spPr/><xdr:txBody><a:bodyPr/><a:p><a:r><a:t>ANNOTATION_MARKER</a:t></a:r></a:p></xdr:txBody></xdr:sp><xdr:clientData/></xdr:twoCellAnchor>"#;
        let with_textbox = drawing_xml.replace("</xdr:wsDr>", &format!("{textbox}</xdr:wsDr>"));
        assert!(with_textbox.contains("ANNOTATION_MARKER"));
        rewrite_zip_entry(&original, &drawing_part, &with_textbox);

        // The chart is deleted (removed from bindings → `live` empty), recorded in `deletes`.
        let model_bytes = ironcalc_bytes(&original);
        let deletes = HashSet::from([chart_part.clone()]);
        let (out, _report) =
            reinject_live_charts(&original, &model_bytes, &[], &HashMap::new(), &deletes).unwrap();

        // The drawing survives with its textbox anchor; the chart frame is gone.
        let out_drawing = entry_from_bytes(&out, &drawing_part);
        assert!(
            out_drawing.contains("ANNOTATION_MARKER"),
            "the co-located textbox anchor must survive the chart delete"
        );
        assert!(
            !out_drawing.contains("<c:chart") && !out_drawing.contains("r:id="),
            "the deleted chart's anchor is removed from the drawing"
        );
        // The chart part is dropped, and the package still opens with zero charts.
        let out_path = dir.path().join("out.xlsx");
        std::fs::write(&out_path, &out).unwrap();
        crate::document::WorkbookDocument::open(&out_path).expect("reopens");
        assert_eq!(discover_and_parse(&out_path).unwrap().len(), 0);
    }

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

    // --- P20: chrome patcher (title / legend / axis title / series color / data labels) ------

    /// The line fixture with an **unmodeled** `<c:roundedCorners>` (our parser ignores it) spliced
    /// into the chartSpace — the sentinel the edit contract must preserve byte-for-byte.
    fn line_fixture_with_unmodeled() -> String {
        let xml = authoring::line_chart_xml_for_test()
            .replace("<c:chart>", "<c:roundedCorners val=\"1\"/><c:chart>");
        assert!(
            xml.contains("<c:roundedCorners val=\"1\"/>"),
            "sentinel spliced"
        );
        xml
    }

    /// **The headline edit-contract test.** Editing the title of a loaded chart patches ONLY the
    /// title's text holder: the edited title re-parses to the new text, while an unmodeled element
    /// AND every unchanged chrome field (legend, axis titles, both series' colors) stay byte-stable.
    #[test]
    fn patch_edits_title_and_preserves_unmodeled_and_unchanged_chrome() {
        let xml = line_fixture_with_unmodeled();
        let mut chart = parse_chart_xml(&xml).unwrap();
        chart.title = Some("Edited & Renamed".into());
        let patched = patch_chart_source(&xml, &chart).unwrap();

        // (a) The title changed on re-parse.
        let reparsed = parse_chart_xml(&patched).unwrap();
        assert_eq!(reparsed.title.as_deref(), Some("Edited & Renamed"));

        // (b) The unmodeled element survives byte-identically (the preserve-unmodeled guarantee).
        assert!(
            patched.contains("<c:roundedCorners val=\"1\"/>"),
            "an unmodeled OOXML element must survive a title edit byte-for-byte",
        );

        // (c) Every OTHER chrome field is untouched — legend, both series colors, axis titles.
        assert!(
            patched.contains(r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/></c:legend>"#),
            "the unchanged legend is byte-stable",
        );
        assert!(
            patched.contains(r#"<a:srgbClr val="4472C4"/>"#),
            "Widgets color kept"
        );
        assert!(
            patched.contains(r#"<a:srgbClr val="ED7D31"/>"#),
            "Gadgets color kept"
        );
        assert_eq!(reparsed.cat_axis.title.as_deref(), Some("Quarter"));
        assert_eq!(
            reparsed.val_axis.title.as_deref(),
            Some("Units (thousands)")
        );
        // The title's own `<c:overlay>` wrapper survived (only the tx text holder was spliced).
        assert!(patched.contains(r#"<c:overlay val="0"/></c:title>"#));
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    #[test]
    fn patch_toggles_legend_off_then_a_new_one_on() {
        let xml = line_fixture_with_unmodeled();

        // Off: clear the legend → the element is gone, everything else byte-stable.
        let mut off = parse_chart_xml(&xml).unwrap();
        off.legend = None;
        let patched_off = patch_chart_source(&xml, &off).unwrap();
        assert!(!patched_off.contains("<c:legend>"), "the legend is removed");
        assert!(patched_off.contains("<c:roundedCorners val=\"1\"/>"));
        assert_eq!(parse_chart_xml(&patched_off).unwrap().legend, None);

        // On (position change): right → bottom.
        let mut moved = parse_chart_xml(&xml).unwrap();
        moved.legend = Some(freecell_chart_model::Legend {
            position: freecell_chart_model::LegendPosition::Bottom,
        });
        let patched_on = patch_chart_source(&xml, &moved).unwrap();
        assert_eq!(
            parse_chart_xml(&patched_on).unwrap().legend,
            Some(freecell_chart_model::Legend {
                position: freecell_chart_model::LegendPosition::Bottom
            }),
        );
        assert!(roxmltree::Document::parse(&patched_on).is_ok());
    }

    #[test]
    fn patch_sets_a_legend_on_a_chart_that_had_none() {
        // Strip the legend from the fixture, then add one back via a patch (insert path).
        let xml = authoring::line_chart_xml_for_test().replace(
            r#"<c:legend><c:legendPos val="r"/><c:overlay val="0"/></c:legend>"#,
            "",
        );
        let mut chart = parse_chart_xml(&xml).unwrap();
        assert_eq!(chart.legend, None, "the stripped fixture has no legend");
        chart.legend = Some(freecell_chart_model::Legend {
            position: freecell_chart_model::LegendPosition::Top,
        });
        let patched = patch_chart_source(&xml, &chart).unwrap();
        // The inserted legend lands before plotVisOnly (schema order) and re-parses.
        assert!(patched.contains(r#"<c:legendPos val="t"/>"#));
        assert!(roxmltree::Document::parse(&patched).is_ok());
        assert_eq!(
            parse_chart_xml(&patched).unwrap().legend,
            Some(freecell_chart_model::Legend {
                position: freecell_chart_model::LegendPosition::Top
            }),
        );
    }

    #[test]
    fn patch_sets_axis_titles() {
        let xml = line_fixture_with_unmodeled();
        let mut chart = parse_chart_xml(&xml).unwrap();
        chart.cat_axis.title = Some("Period".into());
        chart.val_axis.title = None; // clear the value-axis title
        let patched = patch_chart_source(&xml, &chart).unwrap();
        let reparsed = parse_chart_xml(&patched).unwrap();
        assert_eq!(reparsed.cat_axis.title.as_deref(), Some("Period"));
        assert_eq!(reparsed.val_axis.title, None);
        assert!(patched.contains("<c:roundedCorners val=\"1\"/>"));
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    #[test]
    fn patch_series_color_preserves_a_co_located_line_stroke() {
        // Give the Widgets series a line stroke alongside its shape fill — a color edit must splice
        // only the shape solidFill and leave the `a:ln` (the visible line styling) byte-identical.
        let ln = r#"<a:ln w="19050"><a:solidFill><a:srgbClr val="112233"/></a:solidFill></a:ln>"#;
        let xml = authoring::line_chart_xml_for_test().replacen(
            r#"<a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>"#,
            &format!(r#"<a:solidFill><a:srgbClr val="4472C4"/></a:solidFill>{ln}</c:spPr>"#),
            1,
        );
        assert!(xml.contains(ln), "the a:ln was injected");

        let mut chart = parse_chart_xml(&xml).unwrap();
        chart.series[0].color = Some(freecell_chart_model::ChartColor::Rgb(
            freecell_chart_model::Color::from_hex(0x70AD47),
        ));
        let patched = patch_chart_source(&xml, &chart).unwrap();

        // The shape fill is the new color; the co-located line stroke is untouched.
        assert!(
            patched.contains(r#"<a:srgbClr val="70AD47"/>"#),
            "new shape fill"
        );
        assert!(
            patched.contains(ln),
            "the a:ln stroke survives the color edit byte-for-byte"
        );
        assert!(
            !patched.contains(r#"<a:solidFill><a:srgbClr val="4472C4"/></a:solidFill>"#)
                || patched.matches(r#"<a:srgbClr val="4472C4"/>"#).count() == 0,
            "the old shape fill color is replaced"
        );
        assert_eq!(
            parse_chart_xml(&patched).unwrap().series[0].color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x70AD47)
            )),
        );
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    /// CR regression: a series whose `spPr` is a **self-closing** `<c:spPr/>` (no fill) must get its
    /// color spliced INSIDE the spPr, not as an invalid direct child of `<c:ser>`. The pre-fix bug
    /// inserted the fill before the spPr → schema-invalid XML that reads the color back as `None`
    /// (silent color loss on reopen). The color must survive on re-parse.
    #[test]
    fn patch_series_color_into_a_self_closing_sppr() {
        // Replace the Widgets series' full spPr with a self-closing one (a color-less series).
        let xml = authoring::line_chart_xml_for_test().replacen(
            r#"<c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>"#,
            "<c:spPr/>",
            1,
        );
        assert!(
            xml.contains("<c:spPr/>"),
            "the self-closing spPr was injected"
        );

        let mut chart = parse_chart_xml(&xml).unwrap();
        assert_eq!(
            chart.series[0].color, None,
            "the self-closing spPr carries no color"
        );
        chart.series[0].color = Some(freecell_chart_model::ChartColor::Rgb(
            freecell_chart_model::Color::from_hex(0x5B9BD5),
        ));
        let patched = patch_chart_source(&xml, &chart).unwrap();

        // The fill is INSIDE the spPr (a well-formed, schema-valid element), and re-parses to the color.
        assert!(
            patched.contains(
                r#"<c:spPr><a:solidFill><a:srgbClr val="5B9BD5"/></a:solidFill></c:spPr>"#
            ),
            "the fill is spliced inside a rebuilt spPr, not before it:\n{patched}",
        );
        assert!(
            !patched.contains("<c:spPr/>"),
            "the self-closing spPr was replaced"
        );
        assert!(roxmltree::Document::parse(&patched).is_ok());
        assert_eq!(
            parse_chart_xml(&patched).unwrap().series[0].color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x5B9BD5)
            )),
            "the color reads back on reopen (no silent loss)",
        );
    }

    /// CR: a color edit on an **empty-but-closed** `<c:spPr></c:spPr>` inserts the fill before the
    /// close tag (the non-self-closing path) and round-trips — locking that only the `/>` form takes
    /// the whole-element-replace branch.
    #[test]
    fn patch_series_color_into_an_empty_closed_sppr() {
        let xml = authoring::line_chart_xml_for_test().replacen(
            r#"<c:spPr><a:solidFill><a:srgbClr val="4472C4"/></a:solidFill></c:spPr>"#,
            "<c:spPr></c:spPr>",
            1,
        );
        let mut chart = parse_chart_xml(&xml).unwrap();
        chart.series[0].color = Some(freecell_chart_model::ChartColor::Rgb(
            freecell_chart_model::Color::from_hex(0x5B9BD5),
        ));
        let patched = patch_chart_source(&xml, &chart).unwrap();
        assert!(roxmltree::Document::parse(&patched).is_ok());
        assert_eq!(
            parse_chart_xml(&patched).unwrap().series[0].color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x5B9BD5)
            )),
        );
    }

    /// CR (phase-20 Tests list): a loaded chart with an unmodeled element and **no** chrome (or value)
    /// change re-patches **byte-for-byte** — collect_chrome_edits adds nothing when every field
    /// matches, so the whole patch is a no-op that preserves the unmodeled element exactly.
    #[test]
    fn patch_with_no_chrome_change_is_byte_identical() {
        let xml = line_fixture_with_unmodeled();
        let chart = parse_chart_xml(&xml).unwrap();
        let patched = patch_chart_source(&xml, &chart).unwrap();
        assert_eq!(
            patched, xml,
            "an unchanged loaded chart (chrome + values) re-patches byte-for-byte",
        );
        assert!(patched.contains("<c:roundedCorners val=\"1\"/>"));
    }

    #[test]
    fn patch_adds_data_labels_to_a_series() {
        let xml = line_fixture_with_unmodeled();
        let mut chart = parse_chart_xml(&xml).unwrap();
        assert!(
            chart.series[0].data_labels.is_none(),
            "no labels in the fixture"
        );
        chart.series[0].data_labels =
            Some(freecell_chart_model::DataLabels::new().value().percent());
        let patched = patch_chart_source(&xml, &chart).unwrap();
        assert!(patched.contains("<c:dLbls>"), "a dLbls was inserted");
        assert!(patched.contains("<c:roundedCorners val=\"1\"/>"));
        let reparsed = parse_chart_xml(&patched).unwrap();
        let dl = reparsed.series[0]
            .data_labels
            .clone()
            .expect("labels present on reopen");
        assert!(dl.show_value && dl.show_percent && !dl.show_category_name);
        assert!(roxmltree::Document::parse(&patched).is_ok());
    }

    /// A combined chrome edit (title + legend + a series color + labels) on a chart with an
    /// unmodeled element: all edits land and the sentinel is byte-stable — the whole panel's worth
    /// of edits patched in one save.
    #[test]
    fn patch_applies_multiple_chrome_edits_together() {
        let xml = line_fixture_with_unmodeled();
        let mut chart = parse_chart_xml(&xml).unwrap();
        chart.title = Some("Q Report".into());
        chart.legend = Some(freecell_chart_model::Legend {
            position: freecell_chart_model::LegendPosition::Bottom,
        });
        chart.series[1].color = Some(freecell_chart_model::ChartColor::Rgb(
            freecell_chart_model::Color::from_hex(0x5B9BD5),
        ));
        chart.series[0].data_labels = Some(freecell_chart_model::DataLabels::new().value());
        let patched = patch_chart_source(&xml, &chart).unwrap();

        let reparsed = parse_chart_xml(&patched).unwrap();
        assert_eq!(reparsed.title.as_deref(), Some("Q Report"));
        assert_eq!(
            reparsed.legend.map(|l| l.position),
            Some(freecell_chart_model::LegendPosition::Bottom)
        );
        assert_eq!(
            reparsed.series[1].color,
            Some(freecell_chart_model::ChartColor::Rgb(
                freecell_chart_model::Color::from_hex(0x5B9BD5)
            )),
        );
        assert!(reparsed.series[0].data_labels.as_ref().unwrap().show_value);
        assert!(patched.contains("<c:roundedCorners val=\"1\"/>"));
        assert!(roxmltree::Document::parse(&patched).is_ok());
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
        let (bytes, report) = reinject(
            &original,
            &model_bytes,
            &sheets,
            &targets,
            &patches,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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
        let (bytes, report) = reinject(
            &original,
            &model_bytes,
            &sheets,
            &targets,
            &patches,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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
        let err = reinject_live_charts(
            &original,
            &ironcalc_bytes(&original),
            &live,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap_err();
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
        let (bytes, report) = reinject_live_charts(
            &original,
            &ironcalc_bytes(&original),
            &live,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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

        let (bytes, report) = reinject_live_charts(
            &original,
            &ironcalc_bytes(&original),
            &live,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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

        let (bytes, report) = reinject_live_charts(
            &original,
            &ironcalc_bytes(&original),
            &live,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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
        let (bytes, _report) = reinject_live_charts(
            &original,
            &ironcalc_bytes(&original),
            &live,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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

        let (bytes, report) = reinject_live_charts(
            &original,
            &ironcalc_bytes(&original),
            &live,
            &std::collections::HashMap::new(),
            &std::collections::HashSet::new(),
        )
        .unwrap();
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
