//! The chrome ↔ engine seam ([`ChromeClient`]) and its test/demo double
//! ([`RecordingClient`]).
//!
//! The chrome sends [`Command`]s and reads a cell's resolved [`RenderStyle`] (for the
//! action-row toggle pressed states) through this trait rather than holding a concrete
//! `DocumentClient`, so Phase 9 exercises the chrome fully headless. Phase 11 drops the real
//! `DocumentClient` in — it implements this trait below.

use std::cell::RefCell;
use std::collections::HashMap;

use freecell_core::{CellRef, RenderStyle, SheetId};
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
}

/// A test/demo double for [`ChromeClient`]: records every sent [`Command`] and answers
/// `render_style` from an injected map. Interior mutability (`RefCell`) so the chrome can
/// hold it behind a shared `Rc<dyn ChromeClient>` and still record from `&self`.
#[derive(Default)]
pub struct RecordingClient {
    commands: RefCell<Vec<Command>>,
    styles: RefCell<HashMap<(SheetId, CellRef), RenderStyle>>,
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
}
