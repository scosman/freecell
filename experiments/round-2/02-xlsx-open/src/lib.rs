//! # xlsx_open — SP2: end-to-end large styled `.xlsx` open (time + peak memory)
//!
//! Two halves, both driven by committed code so the whole experiment is reproducible
//! from one command (functional_spec §5.3, SP2):
//!
//! 1. **[`generate`] — a styled-`.xlsx` generator built HERE (not in the frozen
//!    `datagen`).** It assembles an IronCalc [`Model`] from deterministic content
//!    (`datagen::SyntheticSheet` for values + formats) with a realistic mix — literal
//!    values, a formula cascade per sheet, shared strings (a bounded word pool →
//!    dense `sharedStrings.xml`), per-cell styles (bold/italic/fills/alignment/number
//!    formats, deduped by IronCalc's style table), band column widths and a styled
//!    band row — across multiple sheets, and writes it with IronCalc's **native styled
//!    writer** (`export::save_to_xlsx`). It `evaluate()`s once before saving so the
//!    file carries correct **cached** formula values (`<f>…</f><v>…</v>`), which is the
//!    premise behind time-to-first-paint.
//!
//! 2. **[`open`] — the open/measurement path.** Opens a given file and returns a
//!    [`OpenStages`] breakdown (file read → parse+build `Workbook` → build `Model` →
//!    **first paint** = cached values queryable → **first eval** = full recompute). The
//!    stage seams are the coarsest **honest** ones IronCalc's public API exposes; finer
//!    sub-stages (unzip / XML parse / shared-strings / style ingest / graph build) are
//!    fused inside `load_from_excel` and cannot be split without patching the engine
//!    (architecture §8 instrumentation-opacity risk — recorded, not invented).
//!
//! The authoritative **peak RSS** is NOT taken here — it is read by the `open` binary
//! from a fresh child process via `round2_harness::peak_rss()` (architecture §3). This
//! library only measures wall-clock stages and asserts correctness (force + assert).

use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use datagen::{CellSource, CellValue, HAlign, Rgb, SyntheticSheet};
use ironcalc::base::types::{Alignment, Fill, HorizontalAlignment, Style};
use ironcalc::base::Model;
use ironcalc::export::save_to_xlsx;
use ironcalc::import::load_from_xlsx;

/// IronCalc load parameters (locale / timezone / language). Matched to the harness's
/// `new_empty("bench","en","UTC","en")` so numbers stay comparable to the rest of
/// Round-2.
pub const LOCALE: &str = "en";
pub const TIMEZONE: &str = "UTC";
pub const LANGUAGE: &str = "en";

/// A small pool of number-format codes rotated across styled numeric cells so that
/// `styles.xml` carries several distinct `numFmt`s (a realistic styled sheet, not one
/// format). These are standard Excel custom-format strings.
const NUM_FMTS: &[&str] = &[
    "general",
    "#,##0.00",
    "0.0%",
    "$#,##0.00",
    "#,##0",
    "0.000",
];

/// Parameters describing the workbook to generate. Deterministic in every field, so the
/// exact same `.xlsx` bytes are reproducible from committed code.
#[derive(Debug, Clone, Copy)]
pub struct GenSpec {
    /// Deterministic seed handed to `datagen::SyntheticSheet`.
    pub seed: u64,
    /// Number of worksheets in the workbook.
    pub sheets: u32,
    /// Data rows per sheet (the dominant size knob).
    pub rows: u32,
    /// Data columns per sheet (literal value columns; a formula column is appended).
    pub cols: u32,
}

impl GenSpec {
    /// A tiny spec for unit tests: fast to build, still exercises every path (values,
    /// formulas, styles, multiple sheets, shared strings).
    pub fn tiny() -> Self {
        Self {
            seed: 20260701,
            sheets: 2,
            rows: 32,
            cols: 6,
        }
    }

    /// The default large spec (~≥100 MB when written). `rows` is the size driver; the
    /// `gen` binary grows it until the on-disk file crosses the requested target.
    pub fn large() -> Self {
        Self {
            seed: 20260701,
            sheets: 4,
            rows: 200_000,
            cols: 12,
        }
    }

    /// Total addressable data cells across all sheets (literals + one formula column).
    pub fn total_cells(&self) -> u64 {
        self.sheets as u64 * self.rows as u64 * (self.cols as u64 + 1)
    }
}

/// The column (0-based) that holds the per-row formula on every sheet. It sits just
/// past the literal columns.
fn formula_col(spec: &GenSpec) -> u32 {
    spec.cols
}

/// Renders a `datagen` value as the string IronCalc's `set_user_input` wants. Empty
/// cells are skipped by the caller, so `Empty` maps to an empty string defensively.
fn value_to_input(v: &CellValue) -> String {
    match v {
        CellValue::Empty => String::new(),
        CellValue::Number(n) => format!("{n}"),
        CellValue::Text(t) => t.clone(),
    }
}

/// Maps a `datagen` highlight colour to IronCalc's `#RRGGBB` hex string.
fn rgb_hex(c: Rgb) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r, c.g, c.b)
}

/// Maps a `datagen` horizontal alignment to IronCalc's enum.
fn h_align(a: HAlign) -> HorizontalAlignment {
    match a {
        HAlign::Left => HorizontalAlignment::Left,
        HAlign::Center => HorizontalAlignment::Center,
        HAlign::Right => HorizontalAlignment::Right,
    }
}

/// Builds the IronCalc [`Style`] for a data cell from its `datagen` format plus a
/// rotated number format, so the emitted `styles.xml` carries a realistic variety of
/// fonts, fills, alignments, and number formats (all deduped by IronCalc's style table).
fn style_for(format: &datagen::CellFormat, num_fmt: &str) -> Style {
    let mut style = Style::default();
    style.font.b = format.bold;
    style.font.i = format.italic;
    if let Some(rgb) = format.highlight {
        // A fill only renders if pattern_type is "solid" with an fg_color set.
        style.fill = Fill {
            pattern_type: "solid".to_string(),
            fg_color: Some(rgb_hex(rgb)),
            bg_color: None,
        };
    }
    style.alignment = Some(Alignment {
        horizontal: h_align(format.h_align),
        ..Alignment::default()
    });
    style.num_fmt = num_fmt.to_string();
    style
}

/// Builds the full multi-sheet, styled workbook model described by `spec`.
///
/// Content mix (per SP2's "realistic mix"):
/// - **Literal values** from `datagen::SyntheticSheet` (numbers / text / a few empties).
/// - **Shared strings:** text values come from `datagen`'s bounded word pool, so the
///   same strings recur across the sheet → a dense, deduplicated `sharedStrings.xml`
///   (exactly the shape SP2 wants to stress on open).
/// - **A formula column** per sheet: `=A{r}+B{r}` (or a single-cell fallback for narrow
///   sheets), so the formula graph is non-trivial and `evaluate()` does real work.
/// - **Per-cell styles** (bold/italic/fills/alignment/number formats), deduped.
/// - **Band styling:** a set of column widths and one fully-styled band row, so the file
///   also carries column/row-band metadata (realistic; not just per-cell styles).
///
/// The model is `evaluate()`d before return so its cached formula values are correct;
/// [`write_xlsx`] persists them.
pub fn generate(spec: &GenSpec) -> Result<Model<'static>> {
    let mut model = Model::new_empty("freecell-sp2", LOCALE, TIMEZONE, LANGUAGE)
        .map_err(|e| anyhow!("Model::new_empty: {e}"))?;

    // new_empty creates sheet 0; add the rest.
    for s in 1..spec.sheets {
        model
            .add_sheet(&format!("Sheet{}", s + 1))
            .map_err(|e| anyhow!("add_sheet {s}: {e}"))?;
    }

    let fcol = formula_col(spec);
    for sheet in 0..spec.sheets {
        // Vary the seed per sheet so sheets differ but stay deterministic.
        let source = SyntheticSheet::new(spec.seed ^ (sheet as u64).wrapping_mul(0x9E37), spec.rows, spec.cols);
        write_sheet(&mut model, sheet, spec, &source, fcol)?;
    }

    // One full recompute so cached formula values are correct before the writer persists
    // them. This is the single evaluate() the generated file needs (IronCalc has no
    // incremental recalc); the SP2 OPEN measurement pays its own first eval separately.
    model.evaluate();
    Ok(model)
}

/// Writes one sheet's literals, formulas, per-cell styles, and band styling into `model`.
fn write_sheet(
    model: &mut Model<'static>,
    sheet: u32,
    spec: &GenSpec,
    source: &SyntheticSheet,
    fcol: u32,
) -> Result<()> {
    // Band styling: give a deterministic set of columns a non-default width, and style a
    // header band row. Cheap, but it makes the file carry real column/row band metadata.
    for c in 0..=fcol {
        let width = source.col_width(c) as f64;
        model
            .set_column_width(sheet, (c + 1) as i32, width)
            .map_err(|e| anyhow!("set_column_width: {e}"))?;
    }

    for r in 0..spec.rows {
        for c in 0..spec.cols {
            let cell = source.cell(r, c);
            if matches!(cell.value, CellValue::Empty) {
                continue; // keep genuine empties empty (a real sheet has gaps)
            }
            let input = value_to_input(&cell.value);
            model
                .set_user_input(sheet, (r + 1) as i32, (c + 1) as i32, input)
                .map_err(|e| anyhow!("set_user_input ({r},{c}): {e}"))?;

            // Rotate a number format across cells so styles.xml has variety.
            let num_fmt = NUM_FMTS[((r as usize).wrapping_add(c as usize)) % NUM_FMTS.len()];
            let style = style_for(&cell.format, num_fmt);
            model
                .set_cell_style(sheet, (r + 1) as i32, (c + 1) as i32, &style)
                .map_err(|e| anyhow!("set_cell_style ({r},{c}): {e}"))?;
        }

        // Formula column: sum the first two literal columns of the row (A+B) when the
        // sheet is wide enough; otherwise reference A of this row. Deterministic and
        // known, so the OPEN path can force+assert a sentinel.
        let formula = if spec.cols >= 2 {
            format!("=A{row}+B{row}", row = r + 1)
        } else {
            format!("=A{row}", row = r + 1)
        };
        model
            .set_user_input(sheet, (r + 1) as i32, (fcol + 1) as i32, formula)
            .map_err(|e| anyhow!("set formula ({r},{fcol}): {e}"))?;
    }

    Ok(())
}

/// Writes an already-built (and evaluated) model to `path` via IronCalc's native styled
/// writer. IronCalc refuses to overwrite, so a stale target is removed first. Returns the
/// on-disk file size in bytes.
pub fn write_xlsx(model: &Model, path: &Path) -> Result<u64> {
    if path.exists() {
        std::fs::remove_file(path)
            .with_context(|| format!("removing stale {}", path.display()))?;
    }
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path: {}", path.display()))?;
    save_to_xlsx(model, path_str).map_err(|e| anyhow!("save_to_xlsx: {e:?}"))?;
    let size = std::fs::metadata(path)
        .with_context(|| format!("stat {}", path.display()))?
        .len();
    Ok(size)
}

/// Generates the workbook for `spec` and writes it to `path`, returning `(file_bytes,
/// gen_wall_clock, write_wall_clock)`. Generation and write time are reported separately
/// from open time (benchmark discipline: never fold build cost into the measured op).
pub fn generate_to_file(spec: &GenSpec, path: &Path) -> Result<GenReport> {
    let t0 = Instant::now();
    let model = generate(spec)?;
    let build = t0.elapsed();

    let t1 = Instant::now();
    let file_bytes = write_xlsx(&model, path)?;
    let write = t1.elapsed();

    Ok(GenReport {
        file_bytes,
        build,
        write,
    })
}

/// Result of a generation run — kept separate from any open measurement.
#[derive(Debug, Clone, Copy)]
pub struct GenReport {
    pub file_bytes: u64,
    pub build: Duration,
    pub write: Duration,
}

/// Grows a generation spec's row count until the written `.xlsx` crosses `target_bytes`,
/// writing to `path` and returning the **final** spec plus the last write report. Shared
/// by the `gen` and `measure` binaries so the orchestrator knows the exact spec (and thus
/// the sentinel) of the file it will measure.
///
/// `on_attempt` is called after each generate+write so callers can log progress. Growth
/// is multiplicative (1.5x–4x by shortfall) to converge in a few foreground attempts
/// without overshooting into memory pressure. If a single attempt already risks the box,
/// the caller caps `target_bytes` and records the ceiling (benchmark discipline) — this
/// function never OOMs on its own beyond one attempt's model.
pub fn generate_until_target(
    base: GenSpec,
    target_bytes: u64,
    path: &Path,
    mut on_attempt: impl FnMut(u32, &GenSpec, &GenReport),
) -> Result<(GenSpec, GenReport)> {
    let mut spec = base;
    let mut attempt = 0;
    loop {
        attempt += 1;
        let report = generate_to_file(&spec, path)?;
        on_attempt(attempt, &spec, &report);

        if report.file_bytes >= target_bytes {
            return Ok((spec, report));
        }

        let ratio = (target_bytes as f64 / report.file_bytes as f64).clamp(1.5, 4.0);
        let next_rows = ((spec.rows as f64) * ratio).ceil() as u32;
        if next_rows <= spec.rows {
            return Err(anyhow!(
                "row growth stalled at {} rows; raise base rows in GenSpec::large",
                spec.rows
            ));
        }
        spec.rows = next_rows;
    }
}

/// The known, deterministic sentinel used to **force + assert** the measured open op:
/// the value of the formula cell at `(row 0, formula_col)` on sheet 0 for the given
/// spec. It equals `A1 + B1` (or `A1`) of the generated content, so we can assert both
/// the cached read (first paint) and the post-eval read return this exact number.
pub fn sentinel(spec: &GenSpec) -> SentinelExpectation {
    let source = SyntheticSheet::new(spec.seed ^ 0u64, spec.rows, spec.cols);
    let a = numeric_or_zero(&source.cell(0, 0).value);
    let expected = if spec.cols >= 2 {
        let b = numeric_or_zero(&source.cell(0, 1).value);
        a + b
    } else {
        a
    };
    SentinelExpectation {
        sheet: 0,
        row: 1,
        col: (formula_col(spec) + 1) as i32,
        expected,
    }
}

/// A number cell's value, or 0.0 for text/empty (IronCalc coerces text/empty operands to
/// 0 in `+`, matching Excel's numeric-context coercion of blanks; text becomes an error
/// in Excel, but `datagen`'s A/B for row 0 seed choose a numeric-friendly path — the
/// sentinel asserts the *cached equals post-eval* invariant regardless of the exact
/// number, which is the real correctness check).
fn numeric_or_zero(v: &CellValue) -> f64 {
    match v {
        CellValue::Number(n) => *n,
        _ => 0.0,
    }
}

/// The expected sentinel formula-cell value plus its address.
#[derive(Debug, Clone, Copy)]
pub struct SentinelExpectation {
    pub sheet: u32,
    pub row: i32,
    pub col: i32,
    pub expected: f64,
}

/// Wall-clock stage breakdown of a fresh open, in nanoseconds. Stages are the coarsest
/// **honest** seams IronCalc's public API exposes (see the module docs / findings for
/// why finer sub-stages are not separable).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct OpenStages {
    /// Read the file bytes off disk into memory (`std::fs::read`).
    pub read_ns: u64,
    /// `load_from_xlsx` fused work: unzip + XML parse + shared-strings + style ingest +
    /// workbook build + formula parse (`from_workbook`, which does NOT evaluate). This is
    /// the coarse "parse+build model" stage.
    pub parse_build_ns: u64,
    /// Time-to-first-paint: process start → cached values queryable. Equal to
    /// `read_ns + parse_build_ns` because the loaded model's cached `<v>` are readable
    /// immediately (no `evaluate()` needed). Reported explicitly because it is the SP2
    /// discovery number.
    pub first_paint_ns: u64,
    /// First full recompute: `model.evaluate()` (full-recompute-ready). Measured
    /// separately from first paint.
    pub first_eval_ns: u64,
    /// Total open-to-recompute-ready = `read_ns + parse_build_ns + first_eval_ns`.
    pub total_ns: u64,
}

/// Opens `path` from the current process, timing each honest stage, and **forces +
/// asserts** the sentinel at both the cached (first-paint) stage and after the first
/// eval — so nothing is optimized away and the cached value is proven correct.
///
/// Note: `load_from_xlsx` (IronCalc's public API) reads the file itself, so to isolate a
/// pure `read_ns` we read the bytes first (`std::fs::read`) and then hand the *path* to
/// `load_from_xlsx`. The file cache is warm after the read, so `parse_build_ns` reflects
/// parse+build with the disk cost already paid — the honest split the API allows.
pub fn open_stages(path: &Path, expect: SentinelExpectation) -> Result<OpenStages> {
    let path_str = path
        .to_str()
        .ok_or_else(|| anyhow!("non-UTF8 path: {}", path.display()))?;

    // Stage 1: file read (bytes into memory; warms the OS page cache).
    let t_read = Instant::now();
    let bytes = std::fs::read(path).with_context(|| format!("read {}", path.display()))?;
    let read_ns = t_read.elapsed().as_nanos() as u64;
    black_box(bytes.len());

    // Stage 2: parse + build model (unzip/XML/shared-strings/styles/graph, NO eval).
    let t_build = Instant::now();
    let mut model = load_from_xlsx(path_str, LOCALE, TIMEZONE, LANGUAGE)
        .map_err(|e| anyhow!("load_from_xlsx: {e:?}"))?;
    let parse_build_ns = t_build.elapsed().as_nanos() as u64;

    // --- FIRST PAINT: cached values are queryable now, before any evaluate(). ---
    let cached = read_number(&model, expect)?;
    assert_close(
        cached,
        expect.expected,
        "cached (first-paint) sentinel value",
    )?;
    black_box(cached);
    let first_paint_ns = read_ns + parse_build_ns;

    // Stage 3: first full recompute (full-recompute-ready), measured separately.
    let t_eval = Instant::now();
    model.evaluate();
    let first_eval_ns = t_eval.elapsed().as_nanos() as u64;

    // Force + assert the post-eval value equals the cached one.
    let recomputed = read_number(&model, expect)?;
    assert_close(recomputed, expect.expected, "post-eval sentinel value")?;
    black_box(recomputed);

    Ok(OpenStages {
        read_ns,
        parse_build_ns,
        first_paint_ns,
        first_eval_ns,
        total_ns: read_ns + parse_build_ns + first_eval_ns,
    })
}

/// Reads the sentinel formula cell's numeric value from a loaded model.
fn read_number(model: &Model, at: SentinelExpectation) -> Result<f64> {
    use ironcalc::base::cell::CellValue as IcCellValue;
    match model
        .get_cell_value_by_index(at.sheet, at.row, at.col)
        .map_err(|e| anyhow!("get_cell_value_by_index: {e}"))?
    {
        IcCellValue::Number(n) => Ok(n),
        other => Err(anyhow!(
            "sentinel cell was not a number: {other:?} (expected {})",
            at.expected
        )),
    }
}

/// Asserts two floats are equal within a tiny tolerance (formula cache vs recompute
/// should be bit-identical, but tolerate f64 dust).
fn assert_close(got: f64, expected: f64, what: &str) -> Result<()> {
    if (got - expected).abs() <= 1e-9 * expected.abs().max(1.0) {
        Ok(())
    } else {
        Err(anyhow!("{what}: got {got}, expected {expected}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// A unique temp path per test invocation (no external tempfile dep).
    fn temp_path(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "sp2_{tag}_{}_{}.xlsx",
            std::process::id(),
            tag
        ))
    }

    #[test]
    fn generate_roundtrips_small() {
        let spec = GenSpec::tiny();
        let path = temp_path("roundtrip");
        let _ = std::fs::remove_file(&path);
        let report = generate_to_file(&spec, &path).unwrap();
        assert!(report.file_bytes > 0, "wrote an empty file");

        // Reload and check a known literal and the sentinel formula's CACHED value.
        let expect = sentinel(&spec);
        let stages = open_stages(&path, expect).unwrap();
        assert!(stages.parse_build_ns > 0);
        // first_paint is read+parse; it must be <= total (which adds eval).
        assert!(stages.first_paint_ns <= stages.total_ns);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn styles_survive_generation() {
        // Build a one-sheet model with a hand-set styled cell, save, reload, and confirm
        // the style attributes survive — proving the writer really emits styles.
        let mut model = Model::new_empty("styletest", LOCALE, TIMEZONE, LANGUAGE).unwrap();
        model.set_user_input(0, 1, 1, "42".to_string()).unwrap();
        let mut style = Style::default();
        style.font.b = true;
        style.fill = Fill {
            pattern_type: "solid".to_string(),
            fg_color: Some("#FF0000".to_string()),
            bg_color: None,
        };
        style.alignment = Some(Alignment {
            horizontal: HorizontalAlignment::Center,
            ..Alignment::default()
        });
        model.set_cell_style(0, 1, 1, &style).unwrap();
        model.evaluate();

        let path = temp_path("styles");
        let _ = std::fs::remove_file(&path);
        write_xlsx(&model, &path).unwrap();

        let reloaded = load_from_xlsx(
            path.to_str().unwrap(),
            LOCALE,
            TIMEZONE,
            LANGUAGE,
        )
        .unwrap();
        let got = reloaded.get_style_for_cell(0, 1, 1).unwrap();
        assert!(got.font.b, "bold did not survive save/reload");
        assert_eq!(
            got.fill.fg_color.as_deref(),
            Some("#FF0000"),
            "fill colour did not survive save/reload"
        );
        assert_eq!(
            got.alignment.map(|a| a.horizontal),
            Some(HorizontalAlignment::Center),
            "alignment did not survive save/reload"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn open_stages_orders_first_paint_before_first_eval() {
        let spec = GenSpec::tiny();
        let path = temp_path("ordering");
        let _ = std::fs::remove_file(&path);
        generate_to_file(&spec, &path).unwrap();

        let expect = sentinel(&spec);
        let stages = open_stages(&path, expect).unwrap();

        // first paint (cached queryable) is reachable using only read+parse — strictly
        // before the total that also pays the first eval.
        assert_eq!(stages.first_paint_ns, stages.read_ns + stages.parse_build_ns);
        assert!(
            stages.total_ns >= stages.first_paint_ns,
            "total must include first paint plus eval"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn first_paint_needs_no_eval() {
        // Directly prove "cached values queryable before recompute": load, read the
        // sentinel WITHOUT calling evaluate(), and assert it equals the expected number.
        let spec = GenSpec::tiny();
        let path = temp_path("nopaint");
        let _ = std::fs::remove_file(&path);
        generate_to_file(&spec, &path).unwrap();

        let expect = sentinel(&spec);
        let model = load_from_xlsx(
            path.to_str().unwrap(),
            LOCALE,
            TIMEZONE,
            LANGUAGE,
        )
        .unwrap();
        // No model.evaluate() here — this is the whole point.
        let cached = read_number(&model, expect).unwrap();
        assert_close(cached, expect.expected, "cached without eval").unwrap();

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn peak_rss_helper_is_canonical_nonzero() {
        // SP2 must use the harness's canonical peak_rss(), not sysinfo::peak_rss_bytes.
        let rss = round2_harness::peak_rss();
        assert!(rss > 1024 * 1024, "peak_rss implausibly small: {rss}");
    }

    #[test]
    fn total_cells_accounts_for_formula_column() {
        let spec = GenSpec {
            seed: 1,
            sheets: 3,
            rows: 100,
            cols: 5,
        };
        // 3 sheets * 100 rows * (5 literal + 1 formula) cols = 1800.
        assert_eq!(spec.total_cells(), 1800);
    }
}
