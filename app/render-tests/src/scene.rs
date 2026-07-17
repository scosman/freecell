//! The scene builder — the harness's way of describing a render fixture, and the code that
//! **drives the real engine** to realize it (`components/render_test_harness.md §Test
//! definition model`).
//!
//! A [`Scene`] is a declarative recipe (cell inputs + styles + geometry). [`build_sources`]
//! runs it through the **real** stack: it spawns a [`DocumentClient`] over a new in-memory
//! workbook, applies the inputs/styles as real worker commands, sets the viewport, waits for
//! the worker's publish, and reads back the real `Publication` + [`SheetCaches`]. So a pixel
//! test failing means the product is wrong, not a stub (`components/render_test_harness.md`).
//!
//! ## Command-less render features
//!
//! The MVP worker protocol (`architecture.md §2`) only edits values (`SetCellInput`) and the
//! four action-row style toggles (`SetStyleAttr`: bold / italic / underline / fill). Alignment,
//! explicit font colour, and column/row geometry have **no edit command** — in the product they
//! arrive from an opened file, not a user edit. The scene applies those to the real
//! `SheetCache` the grid consumes (via its public mutators `set_col_width` / `set_cell_style`,
//! the same ones the worker uses) after the worker builds it. This mirrors how Phase 6 itself
//! exercised alignment/geometry, and keeps the render path (the thing under test) fully real.

use std::ops::Range;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, Result};
use arc_swap::ArcSwap;

use freecell_app::grid::GridDataSources;
use freecell_core::cache::SheetCaches;
use freecell_core::{
    Align, BorderSpec, CellRange, CellRef, CfRuleSpec, RenderStyle, Rgb, SheetId, VAlign,
};
use freecell_engine::{Command, DocumentClient, DocumentSource, StyleAttr, WorkerEvent};

/// One command-less style injection applied to the real `SheetCache` after the worker builds it.
enum Inject {
    Align(u32, u32, Align),
    /// Explicit vertical alignment (injected — the real `SetStylePath` path is exercised by the
    /// engine integration tests; the grid renders identically from either source).
    VAlign(u32, u32, VAlign),
    FontColor(u32, u32, Rgb),
    ColWidth(u32, f32),
    RowHeight(u32, f32),
    /// A per-cell font family (`None` = default) + size in quarter-points (`0` = default).
    Font(u32, u32, Option<String>, u16),
    /// A per-cell resolved border (interned into the real cache's `border_specs` side table). Files
    /// carry borders that arrive at the cache the same way (`components/style_render.md`); the real
    /// `SetBorders` write path is exercised by the engine integration tests.
    Border(u32, u32, BorderSpec),
}

/// A declarative render fixture. Build it fluently, then [`build_sources`] realizes it through
/// the real engine + real style cache.
pub struct Scene {
    inputs: Vec<(u32, u32, String)>,
    styles: Vec<(CellRange, StyleAttr)>,
    injects: Vec<Inject>,
    /// Inclusive 0-based row runs to hide via a real `Command::SetRowsHidden` (`gaps_closing_7_15
    /// §4`) — the same worker path the header-menu Hide drives, so the published cache carries the
    /// zero-size hidden geometry.
    hidden_rows: Vec<(u32, u32)>,
    /// Inclusive 0-based column runs to hide via a real `Command::SetColumnsHidden`.
    hidden_cols: Vec<(u32, u32)>,
    /// Conditional-formatting rules `(A1 range, spec)` applied via a real `Command::AddCondFmt`
    /// (`components/engine_cf.md §5`). The worker folds each winning rule into the published style
    /// cache (P3), so the captured `SheetCache` carries the value-dependent CF fills / font colour.
    cond_fmt: Vec<(String, CfRuleSpec)>,
    publish_rows: Range<u32>,
    publish_cols: Range<u32>,
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

impl Scene {
    /// An empty scene on a fresh Excel-max workbook. The published viewport defaults to a
    /// window (80 rows × 48 cols) that comfortably covers every small case's visible cells;
    /// override with [`Scene::publish`] for cases whose values sit deeper.
    pub fn new() -> Self {
        Self {
            inputs: Vec::new(),
            styles: Vec::new(),
            injects: Vec::new(),
            hidden_rows: Vec::new(),
            hidden_cols: Vec::new(),
            cond_fmt: Vec::new(),
            publish_rows: 0..80,
            publish_cols: 0..48,
        }
    }

    /// Sets a cell's raw input (a literal or an `=formula`). Number formats (currency /
    /// percent / thousands / date) are **inferred by the engine** from the input string, e.g.
    /// `"$1,234.50"`, `"50%"`, `"2021-01-01"` (probed against the pinned IronCalc).
    pub fn input(mut self, row: u32, col: u32, text: &str) -> Self {
        self.inputs.push((row, col, text.to_string()));
        self
    }

    fn style(mut self, cell: CellRef, attr: StyleAttr) -> Self {
        self.styles.push((CellRange::single(cell), attr));
        self
    }

    /// Toggles bold on a cell (a fresh cell → set) — a real `SetStyleAttr` worker edit.
    pub fn bold(self, row: u32, col: u32) -> Self {
        self.style(CellRef::new(row, col), StyleAttr::Bold)
    }

    /// Toggles italic on a cell — a real `SetStyleAttr` worker edit.
    pub fn italic(self, row: u32, col: u32) -> Self {
        self.style(CellRef::new(row, col), StyleAttr::Italic)
    }

    /// Toggles underline on a cell — a real `SetStyleAttr` worker edit.
    pub fn underline(self, row: u32, col: u32) -> Self {
        self.style(CellRef::new(row, col), StyleAttr::Underline)
    }

    /// Toggles strikethrough on a cell — a real `SetStyleAttr` worker edit.
    pub fn strikethrough(self, row: u32, col: u32) -> Self {
        self.style(CellRef::new(row, col), StyleAttr::Strikethrough)
    }

    /// Toggles wrap-text on a cell — a real `SetStyleAttr` worker edit.
    pub fn wrap(self, row: u32, col: u32) -> Self {
        self.style(CellRef::new(row, col), StyleAttr::WrapText)
    }

    /// Sets a solid fill on a cell — a real `SetStyleAttr` worker edit.
    pub fn fill(self, row: u32, col: u32, rgb: u32) -> Self {
        self.style(
            CellRef::new(row, col),
            StyleAttr::Fill(Some(Rgb::from_hex(rgb))),
        )
    }

    /// Sets a solid fill across a range — a real `SetStyleAttr` worker edit.
    pub fn fill_range(mut self, range: CellRange, rgb: u32) -> Self {
        self.styles
            .push((range, StyleAttr::Fill(Some(Rgb::from_hex(rgb)))));
        self
    }

    /// Explicitly clears a cell's fill ("No Fill") — a real `SetStyleAttr` worker edit. Used by
    /// `cell_fill_none_explicit` after a fill to show that clearing renders as the default cell.
    pub fn fill_none(self, row: u32, col: u32) -> Self {
        self.style(CellRef::new(row, col), StyleAttr::Fill(None))
    }

    /// Sets explicit horizontal alignment (injected into the real cache — no worker command).
    pub fn align(mut self, row: u32, col: u32, align: Align) -> Self {
        self.injects.push(Inject::Align(row, col, align));
        self
    }

    /// Sets explicit vertical alignment (injected into the real cache — no worker command).
    pub fn v_align(mut self, row: u32, col: u32, valign: VAlign) -> Self {
        self.injects.push(Inject::VAlign(row, col, valign));
        self
    }

    /// Sets an explicit font colour (injected into the real cache — no worker command).
    pub fn font_color(mut self, row: u32, col: u32, rgb: u32) -> Self {
        self.injects
            .push(Inject::FontColor(row, col, Rgb::from_hex(rgb)));
        self
    }

    /// Sets a column width override in px (injected into the real cache — no worker command).
    pub fn col_width(mut self, col: u32, px: f32) -> Self {
        self.injects.push(Inject::ColWidth(col, px));
        self
    }

    /// Sets a row height override in px (injected into the real cache — no worker command).
    pub fn row_height(mut self, row: u32, px: f32) -> Self {
        self.injects.push(Inject::RowHeight(row, px));
        self
    }

    /// Sets a per-cell font family (`Some(name)` renders that family; `None` keeps the default) and
    /// size in points (`Some(pt)` → `pt*4` quarter-points; `None` = default). Injected into the real
    /// cache — no worker command (mirrors how the worker's `SetFont` materialises `RenderStyle`).
    pub fn font(mut self, row: u32, col: u32, family: Option<&str>, pt: Option<f32>) -> Self {
        let size_q = pt.map(|p| (p * 4.0).round() as u16).unwrap_or(0);
        self.injects
            .push(Inject::Font(row, col, family.map(str::to_string), size_q));
        self
    }

    /// Sets a resolved [`BorderSpec`] on a cell (injected into the real cache — no worker command).
    pub fn border(mut self, row: u32, col: u32, spec: BorderSpec) -> Self {
        self.injects.push(Inject::Border(row, col, spec));
        self
    }

    /// Hides the inclusive 0-based row run `[start, end]` via a **real** `Command::SetRowsHidden`
    /// (`gaps_closing_7_15 §4`) — the same worker path the header-menu Hide drives — so the
    /// published cache renders those rows at zero size (neighbours abut, no header/gridline).
    pub fn hide_row(mut self, start: u32, end: u32) -> Self {
        self.hidden_rows.push((start, end));
        self
    }

    /// Hides the inclusive 0-based column run `[start, end]` via a real `Command::SetColumnsHidden`
    /// (the column analogue of [`Scene::hide_row`]).
    pub fn hide_col(mut self, start: u32, end: u32) -> Self {
        self.hidden_cols.push((start, end));
        self
    }

    /// Adds a conditional-formatting rule over the A1 `range` — a real `Command::AddCondFmt` worker
    /// edit (`components/engine_cf.md §5`). The worker folds the winning rule's differential (a
    /// highlight fill/font, or a color-scale's interpolated fill) into the published style cache via
    /// the value-dependent extended-style path (P3), and [`build_sources`] drains to idle after
    /// sending it, so the captured `SheetCache` carries the CF result the grid then paints — no cache
    /// injection needed (unlike alignment/geometry, CF *does* have a real worker command).
    pub fn cond_fmt(mut self, range: &str, spec: CfRuleSpec) -> Self {
        self.cond_fmt.push((range.to_string(), spec));
        self
    }

    /// Overrides the published viewport window (for cases whose values sit deeper than the
    /// default 80×48 window).
    pub fn publish(mut self, rows: Range<u32>, cols: Range<u32>) -> Self {
        self.publish_rows = rows;
        self.publish_cols = cols;
        self
    }
}

/// How long to wait for the worker's initial `Loaded` event.
const LOAD_TIMEOUT: Duration = Duration::from_secs(5);
/// The idle gap that signals the worker has finished processing the scene's commands: once no
/// event arrives for this long, the drain is complete.
const IDLE_GAP: Duration = Duration::from_millis(200);
/// A hard cap on the total drain, so a misbehaving worker fails the scene instead of hanging.
const DRAIN_CAP: Duration = Duration::from_secs(10);

/// Realizes a [`Scene`] through the real engine and returns the `GridDataSources` the grid
/// renders from — the real `Publication` (values) + `SheetCaches` (geometry + resolved styles).
pub fn build_sources(scene: &Scene) -> Result<GridDataSources> {
    let (client, events) = DocumentClient::spawn(DocumentSource::NewWorkbook);

    // The worker's first event is `Loaded` (or `LoadFailed`). It carries the sheet list, whose
    // first entry is the active sheet the publication + cache cover.
    let sheet = loop {
        match events.recv_timeout(LOAD_TIMEOUT) {
            Some(WorkerEvent::Loaded { sheets }) => {
                break sheets.first().map(|m| m.id).unwrap_or(SheetId(0));
            }
            Some(WorkerEvent::LoadFailed { error }) => bail!("worker load failed: {error}"),
            Some(_) => continue, // e.g. an early StyleCacheUpdated — keep waiting for Loaded
            None => bail!("worker never emitted Loaded within {LOAD_TIMEOUT:?}"),
        }
    };

    // Apply edits (values then styles), then set the viewport LAST so the final publish covers
    // every value. (The worker publishes the current viewport after each batch; if it coalesces
    // the whole scene into one batch it still sets the viewport before applying the edits.)
    for (row, col, text) in &scene.inputs {
        client.send(Command::SetCellInput {
            sheet,
            cell: CellRef::new(*row, *col),
            input: text.clone(),
        });
    }
    for (range, attr) in &scene.styles {
        client.send(Command::SetStyleAttr {
            sheet,
            range: *range,
            attr: *attr,
        });
    }
    // Conditional-formatting rules through the real worker path (`Command::AddCondFmt`). Sent after
    // the value inputs so the CF fold already sees the values (a value publish would re-fold anyway);
    // the worker folds each winning rule into the published style cache (P3), and the viewport-time
    // `build_and_store_cache(cf = has_cond_fmt)` + the final `drain_to_idle` guarantee the resident
    // cache the grid renders carries the value-dependent CF fills.
    for (range, spec) in &scene.cond_fmt {
        client.send(Command::AddCondFmt {
            sheet,
            range: range.clone(),
            spec: spec.clone(),
        });
    }
    // Hide row/column runs through the real worker path (`gaps_closing_7_15 §4`), so the rebuilt
    // cache carries the zero-size hidden geometry the grid renders.
    for (start, end) in &scene.hidden_rows {
        client.send(Command::SetRowsHidden {
            sheet,
            start: *start,
            end: *end,
            hidden: true,
        });
    }
    for (start, end) in &scene.hidden_cols {
        client.send(Command::SetColumnsHidden {
            sheet,
            start: *start,
            end: *end,
            hidden: true,
        });
    }
    client.send(Command::SetViewport {
        sheet,
        rows: scene.publish_rows.clone(),
        cols: scene.publish_cols.clone(),
    });

    drain_to_idle(&events)?;

    // Snapshot the real published values and take the shared resident cache.
    let publication = client.publication();
    let caches = client.caches();

    // Apply the command-less render features to the real read model the grid consumes.
    apply_injections(&caches, sheet, &scene.injects);

    // The worker is idle; tell it to shut down. The shared `caches`/`publication` Arcs we hold
    // keep the data alive after the worker thread exits.
    client.send(Command::Shutdown);

    Ok(GridDataSources {
        publication: Arc::new(ArcSwap::from(publication)),
        caches,
    })
}

/// Drain worker events until the queue stays empty for [`IDLE_GAP`] (the scene is fully applied
/// and published). Hitting the [`DRAIN_CAP`] is a **hard error**: the worker never went idle, so
/// the data may be incomplete and we must not silently render a half-applied scene. (No
/// well-behaved scene churns events for 10 s, so this fires only on a genuine worker fault.)
fn drain_to_idle(events: &freecell_engine::WorkerEventReceiver) -> Result<()> {
    let deadline = Instant::now() + DRAIN_CAP;
    loop {
        match events.recv_timeout(IDLE_GAP) {
            Some(_) => {
                if Instant::now() >= deadline {
                    bail!(
                        "worker still emitting events after {DRAIN_CAP:?} — the scene never went \
                         idle; refusing to render possibly-incomplete data"
                    );
                }
            }
            None => return Ok(()), // idle for IDLE_GAP → the worker finished the scene
        }
    }
}

/// Apply the alignment / font-colour / geometry injections to the real `SheetCache`. Alignment
/// and font colour merge onto whatever the worker resolved for the cell (so e.g. a bold cell can
/// also be right-aligned); geometry uses the axis-rebuilding setters.
fn apply_injections(caches: &parking_lot::RwLock<SheetCaches>, sheet: SheetId, injects: &[Inject]) {
    if injects.is_empty() {
        return;
    }
    let mut guard = caches.write();
    let Some(cache) = guard.get_mut(sheet) else {
        return;
    };
    for inject in injects {
        match inject {
            Inject::Align(row, col, align) => {
                let base = cache.render_style(*row, *col).copied().unwrap_or_default();
                cache.set_cell_style(
                    *row,
                    *col,
                    RenderStyle {
                        h_align: Some(*align),
                        ..base
                    },
                );
            }
            Inject::VAlign(row, col, valign) => {
                let base = cache.render_style(*row, *col).copied().unwrap_or_default();
                cache.set_cell_style(
                    *row,
                    *col,
                    RenderStyle {
                        v_align: Some(*valign),
                        ..base
                    },
                );
            }
            Inject::FontColor(row, col, rgb) => {
                let base = cache.render_style(*row, *col).copied().unwrap_or_default();
                cache.set_cell_style(
                    *row,
                    *col,
                    RenderStyle {
                        font_color: Some(*rgb),
                        ..base
                    },
                );
            }
            Inject::ColWidth(col, px) => cache.set_col_width(*col, *px),
            Inject::RowHeight(row, px) => cache.set_row_height(*row, *px),
            Inject::Font(row, col, family, size_q) => {
                let base = cache.render_style(*row, *col).copied().unwrap_or_default();
                let font_family = family
                    .as_deref()
                    .map(|name| cache.intern_font_family(name))
                    .unwrap_or(0);
                cache.set_cell_style(
                    *row,
                    *col,
                    RenderStyle {
                        font_family,
                        font_size_q: *size_q,
                        ..base
                    },
                );
            }
            Inject::Border(row, col, spec) => {
                let base = cache.render_style(*row, *col).copied().unwrap_or_default();
                let border = cache.intern_border_spec(*spec);
                cache.set_cell_style(*row, *col, RenderStyle { border, ..base });
            }
        }
    }
}

/// Test-only: expose the formatted display strings the engine produces for a scene's cells, so a
/// unit test can guard the number-format-inference assumption without rendering.
#[cfg(test)]
pub(crate) fn engine_display(scene: &Scene) -> Result<Vec<((u32, u32), String)>> {
    let sources = build_sources(scene)?;
    let publication = sources.publication.load();
    Ok(publication
        .cells
        .iter()
        .map(|c| ((c.row, c.col), c.display_text.clone()))
        .collect())
}
