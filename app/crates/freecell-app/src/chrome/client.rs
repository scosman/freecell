//! The chrome ↔ engine seam ([`ChromeClient`]) and its test/demo double
//! ([`RecordingClient`]).
//!
//! The chrome sends [`Command`]s and reads a cell's resolved [`RenderStyle`] (for the
//! action-row toggle pressed states) through this trait rather than holding a concrete
//! `DocumentClient`, so Phase 9 exercises the chrome fully headless. Phase 11 drops the real
//! `DocumentClient` in — it implements this trait below.

use std::cell::RefCell;
use std::collections::HashMap;

use freecell_core::{CellKind, CellRef, RenderStyle, SheetId};
use freecell_engine::{Command, DocumentClient};

/// What the chrome needs from the engine: send commands, and read a cell's resolved style.
///
/// Kept deliberately narrow — the chrome never touches the publication or generation (those
/// are the grid's), only the style cache (for toggle states) and the command channel.
pub trait ChromeClient {
    /// Send a command to the worker (fire-and-forget, mirrors [`DocumentClient::send`]).
    fn send(&self, cmd: Command);

    /// The resolved style of a single cell, read at selection-change time to light the
    /// bold/italic/underline toggles + the fill indicator. `None` = no resident cache entry
    /// (the default, plain style).
    fn render_style(&self, sheet: SheetId, cell: CellRef) -> Option<RenderStyle>;

    /// The resolved number-format code string of a single cell (`components/action_bar.md`), for
    /// the number-format dropdown's category label + decimals ±. `None` = no resident cache for the
    /// sheet; a cell with no stored style resolves to `Some("general")` (the default format).
    fn num_fmt_code(&self, sheet: SheetId, cell: CellRef) -> Option<String>;

    /// The resolved font-family name of a single cell (`components/action_bar.md`), for the font
    /// family dropdown's active label. `None` = no resident cache for the sheet; a cell with no
    /// stored family resolves to `Some("")` (the workbook default = "Default (Inter)").
    fn font_family_name(&self, sheet: SheetId, cell: CellRef) -> Option<String>;

    /// The workbook's default font size in **points** (`components/action_bar.md`), so the size box
    /// can label a default cell (`font_size_q == 0`) with the real workbook default rather than a
    /// hardcoded value. `None` = no resident cache / the default size is unknown.
    fn default_font_size_pt(&self, sheet: SheetId) -> Option<f64>;

    /// The active cell's evaluated [`CellKind`] and formatted display text from the latest
    /// published viewport snapshot (`components/action_bar.md §decimals ±`), so the decimals ±
    /// buttons can tell a *numeric* General cell (adjustable, `200000`) from a text/date General
    /// cell (not adjustable). `None` = the cell is empty, or outside the published viewport (both
    /// map to "not a numeric cell here" → ± disabled). Read only at selection-change /
    /// style-refresh time, when the active cell is on screen.
    fn published_cell(&self, sheet: SheetId, cell: CellRef) -> Option<(CellKind, String)>;
}

impl ChromeClient for DocumentClient {
    fn send(&self, cmd: Command) {
        DocumentClient::send(self, cmd);
    }

    fn render_style(&self, sheet: SheetId, cell: CellRef) -> Option<RenderStyle> {
        // Brief read lock on the resident cache (the same the grid reads per frame); the
        // action row refreshes this only on SelectionChanged / StyleCacheUpdated, not per
        // frame, so the lock is uncontended here.
        let caches = self.caches();
        let guard = caches.read();
        guard
            .get(sheet)
            .and_then(|cache| cache.render_style(cell.row, cell.col))
            .copied()
    }

    fn num_fmt_code(&self, sheet: SheetId, cell: CellRef) -> Option<String> {
        let caches = self.caches();
        let guard = caches.read();
        let cache = guard.get(sheet)?;
        // A cell with no stored style resolves to the default "general" (index 0).
        let id = cache
            .render_style(cell.row, cell.col)
            .map(|s| s.num_fmt)
            .unwrap_or(0);
        Some(cache.num_fmt_code(id).to_string())
    }

    fn font_family_name(&self, sheet: SheetId, cell: CellRef) -> Option<String> {
        let caches = self.caches();
        let guard = caches.read();
        let cache = guard.get(sheet)?;
        // A cell with no stored family resolves to the workbook default (index 0 → "").
        let id = cache
            .render_style(cell.row, cell.col)
            .map(|s| s.font_family)
            .unwrap_or(0);
        Some(cache.font_family_name(id).to_string())
    }

    fn default_font_size_pt(&self, sheet: SheetId) -> Option<f64> {
        let caches = self.caches();
        let guard = caches.read();
        let q = guard.get(sheet)?.default_font_size_q();
        // `0` = unknown (a cache that never recorded it) → no label override.
        (q != 0).then(|| q as f64 / 4.0)
    }

    fn published_cell(&self, sheet: SheetId, cell: CellRef) -> Option<(CellKind, String)> {
        // A wait-free load of the latest published viewport snapshot (the same the grid reads).
        // The snapshot is for the active sheet; a mismatched sheet or an empty/off-viewport cell
        // has no entry → `None`.
        let publication = self.publication();
        if publication.sheet != sheet {
            return None;
        }
        publication
            .cells
            .iter()
            .find(|c| c.row == cell.row && c.col == cell.col)
            .map(|c| (c.kind, c.display_text.clone()))
    }
}

/// A test/demo double for [`ChromeClient`]: records every sent [`Command`] and answers
/// `render_style` from an injected map. Interior mutability (`RefCell`) so the chrome can
/// hold it behind a shared `Rc<dyn ChromeClient>` and still record from `&self`.
#[derive(Default)]
pub struct RecordingClient {
    commands: RefCell<Vec<Command>>,
    styles: RefCell<HashMap<(SheetId, CellRef), RenderStyle>>,
    num_fmts: RefCell<HashMap<(SheetId, CellRef), String>>,
    font_families: RefCell<HashMap<(SheetId, CellRef), String>>,
    default_font_size_pt: RefCell<Option<f64>>,
    published: RefCell<HashMap<(SheetId, CellRef), (CellKind, String)>>,
}

impl RecordingClient {
    /// A fresh double with no recorded commands and no styles.
    pub fn new() -> Self {
        Self::default()
    }

    /// Injects the resolved style `render_style` will return for `(sheet, cell)`.
    pub fn set_style(&self, sheet: SheetId, cell: CellRef, style: RenderStyle) {
        self.styles.borrow_mut().insert((sheet, cell), style);
    }

    /// Injects the number-format code `num_fmt_code` will return for `(sheet, cell)`.
    pub fn set_num_fmt(&self, sheet: SheetId, cell: CellRef, code: &str) {
        self.num_fmts
            .borrow_mut()
            .insert((sheet, cell), code.to_string());
    }

    /// Injects the font-family name `font_family_name` will return for `(sheet, cell)`.
    pub fn set_font_family(&self, sheet: SheetId, cell: CellRef, name: &str) {
        self.font_families
            .borrow_mut()
            .insert((sheet, cell), name.to_string());
    }

    /// Injects the workbook default font size (points) `default_font_size_pt` will return.
    pub fn set_default_font_size_pt(&self, pt: f64) {
        *self.default_font_size_pt.borrow_mut() = Some(pt);
    }

    /// Injects the published `(kind, display_text)` `published_cell` will return for `(sheet, cell)`.
    pub fn set_published_cell(&self, sheet: SheetId, cell: CellRef, kind: CellKind, display: &str) {
        self.published
            .borrow_mut()
            .insert((sheet, cell), (kind, display.to_string()));
    }

    /// Drains and returns every command recorded so far (clearing the log).
    pub fn take_commands(&self) -> Vec<Command> {
        std::mem::take(&mut self.commands.borrow_mut())
    }
}

impl ChromeClient for RecordingClient {
    fn send(&self, cmd: Command) {
        self.commands.borrow_mut().push(cmd);
    }

    fn render_style(&self, sheet: SheetId, cell: CellRef) -> Option<RenderStyle> {
        self.styles.borrow().get(&(sheet, cell)).copied()
    }

    fn num_fmt_code(&self, sheet: SheetId, cell: CellRef) -> Option<String> {
        // Mirrors the real client: a cell with an injected code returns it, else the default.
        Some(
            self.num_fmts
                .borrow()
                .get(&(sheet, cell))
                .cloned()
                .unwrap_or_else(|| "general".to_string()),
        )
    }

    fn font_family_name(&self, sheet: SheetId, cell: CellRef) -> Option<String> {
        // Mirrors the real client: an injected family, else the workbook default ("").
        Some(
            self.font_families
                .borrow()
                .get(&(sheet, cell))
                .cloned()
                .unwrap_or_default(),
        )
    }

    fn default_font_size_pt(&self, _sheet: SheetId) -> Option<f64> {
        *self.default_font_size_pt.borrow()
    }

    fn published_cell(&self, sheet: SheetId, cell: CellRef) -> Option<(CellKind, String)> {
        self.published.borrow().get(&(sheet, cell)).cloned()
    }
}
