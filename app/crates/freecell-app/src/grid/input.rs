//! Pure keyboard mapping for the grid — keystroke → [`GridKeyCommand`] with **no gpui**, so
//! the MVP keyboard map (`ui_design.md §6`) is unit-tested headless on Linux
//! (`components/grid.md §Input`, §Test plan). The [`GridView`](super::GridView) event handler
//! is a thin layer that reads the platform keystroke, calls [`command_for_key`], and dispatches
//! the result (`apply_motion` for a motion, an event for a clear).
//!
//! Per the Phase-2 model (`DECISIONS_TO_REVIEW`, Phase 2): Tab/Enter and their Shift variants
//! are **not** distinct motions — they map to `Move(Right/Left/Down/Up)` here at the keymap
//! layer (their only extra behaviour, committing a pending data-row edit, is the window's job).

use freecell_core::selection::{Direction, Motion};

/// A resolved grid keyboard command: a selection motion, or a request to clear the selected
/// cells' contents (Delete/Backspace). The clear is forwarded to the worker by the window
/// (`components/grid.md §Input`: "Delete emits a ClearCells request via the event sink").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GridKeyCommand {
    /// Apply this motion to the selection via `freecell_core::apply_motion`.
    Motion(Motion),
    /// Clear the current selection's cell contents (keep styles).
    ClearCells,
}

/// Maps a platform keystroke — decomposed into plain data so this stays gpui-free — to a
/// [`GridKeyCommand`], or `None` when the grid ignores the key (the caller then propagates it).
///
/// - `key` is the gpui key name (`"up"`, `"pagedown"`, `"home"`, `"delete"`, …).
/// - `shift` is the Shift modifier (extend vs collapse).
/// - `secondary` is the semantically-secondary modifier — **Cmd on macOS, Ctrl on Linux** — the
///   caller resolves it from `Modifiers::secondary()` so this function is platform-agnostic.
/// - `page_rows` is the current viewport height in rows, passed through to the Page motions.
///
/// The map is `ui_design.md §6` (MVP-complete): arrows move/extend/jump-edge, Tab/Enter move,
/// Page Up/Down page, Home to column A (Cmd+Home to A1), Delete/Backspace clear.
pub fn command_for_key(
    key: &str,
    shift: bool,
    secondary: bool,
    page_rows: u32,
) -> Option<GridKeyCommand> {
    use Direction::*;
    use GridKeyCommand::Motion as M;

    // Arrows: the (secondary, shift) quadrant picks move / extend / jump-edge / extend-edge.
    if let Some(dir) = arrow_direction(key) {
        let motion = match (secondary, shift) {
            (true, true) => Motion::ExtendEdge(dir),
            (true, false) => Motion::JumpEdge(dir),
            (false, true) => Motion::Extend(dir),
            (false, false) => Motion::Move(dir),
        };
        return Some(M(motion));
    }

    match key {
        // Tab/Enter (and Shift variants) are plain moves at the keymap layer.
        "tab" => Some(M(Motion::Move(if shift { Left } else { Right }))),
        "enter" => Some(M(Motion::Move(if shift { Up } else { Down }))),

        // Page Up/Down by the current viewport height in rows.
        "pageup" => Some(M(page_motion(Up, shift, page_rows))),
        "pagedown" => Some(M(page_motion(Down, shift, page_rows))),

        // Home → column A of the active row; Cmd/Ctrl+Home → cell A1.
        "home" => Some(M(match (secondary, shift) {
            (true, true) => Motion::ExtendDocumentStart,
            (true, false) => Motion::DocumentStart,
            (false, true) => Motion::ExtendRowStart,
            (false, false) => Motion::RowStart,
        })),

        // Delete/Backspace clear the selection's contents.
        "delete" | "backspace" => Some(GridKeyCommand::ClearCells),

        // Everything else (printable characters, Escape, Cmd+B/I/U bound at window level, …)
        // is not the grid's — propagate.
        _ => None,
    }
}

/// The cardinal direction for an arrow key name, if any.
fn arrow_direction(key: &str) -> Option<Direction> {
    match key {
        "up" => Some(Direction::Up),
        "down" => Some(Direction::Down),
        "left" => Some(Direction::Left),
        "right" => Some(Direction::Right),
        _ => None,
    }
}

/// A Page motion (extend variant when `shift`) in `direction` by `rows`.
fn page_motion(direction: Direction, shift: bool, rows: u32) -> Motion {
    if shift {
        Motion::ExtendPage { direction, rows }
    } else {
        Motion::Page { direction, rows }
    }
}

#[cfg(test)]
mod tests {
    use super::GridKeyCommand::ClearCells;
    use super::*;
    use freecell_core::selection::Direction::*;

    /// Page height passed through in the page tests.
    const PAGE: u32 = 20;

    /// `Some(GridKeyCommand::Motion(_))` — a tiny helper so the assertions stay readable
    /// (the `Motion` enum and the `GridKeyCommand::Motion` variant share a name).
    fn m(motion: Motion) -> Option<GridKeyCommand> {
        Some(GridKeyCommand::Motion(motion))
    }

    #[test]
    fn arrows_map_by_shift_and_secondary() {
        // Plain arrow → Move; Shift → Extend; secondary → JumpEdge; both → ExtendEdge.
        assert_eq!(
            command_for_key("right", false, false, PAGE),
            m(Motion::Move(Right))
        );
        assert_eq!(
            command_for_key("up", true, false, PAGE),
            m(Motion::Extend(Up))
        );
        assert_eq!(
            command_for_key("left", false, true, PAGE),
            m(Motion::JumpEdge(Left))
        );
        assert_eq!(
            command_for_key("down", true, true, PAGE),
            m(Motion::ExtendEdge(Down))
        );
    }

    #[test]
    fn tab_enter_map_to_moves() {
        assert_eq!(
            command_for_key("tab", false, false, PAGE),
            m(Motion::Move(Right))
        );
        assert_eq!(
            command_for_key("tab", true, false, PAGE),
            m(Motion::Move(Left))
        );
        assert_eq!(
            command_for_key("enter", false, false, PAGE),
            m(Motion::Move(Down))
        );
        assert_eq!(
            command_for_key("enter", true, false, PAGE),
            m(Motion::Move(Up))
        );
    }

    #[test]
    fn page_keys_map() {
        assert_eq!(
            command_for_key("pagedown", false, false, PAGE),
            m(Motion::Page {
                direction: Down,
                rows: PAGE
            })
        );
        assert_eq!(
            command_for_key("pageup", true, false, PAGE),
            m(Motion::ExtendPage {
                direction: Up,
                rows: PAGE
            })
        );
    }

    #[test]
    fn home_and_cmd_home() {
        // Home → column A of the row; Shift+Home extends there.
        assert_eq!(
            command_for_key("home", false, false, PAGE),
            m(Motion::RowStart)
        );
        assert_eq!(
            command_for_key("home", true, false, PAGE),
            m(Motion::ExtendRowStart)
        );
        // Cmd/Ctrl+Home → A1; Cmd/Ctrl+Shift+Home extends to A1.
        assert_eq!(
            command_for_key("home", false, true, PAGE),
            m(Motion::DocumentStart)
        );
        assert_eq!(
            command_for_key("home", true, true, PAGE),
            m(Motion::ExtendDocumentStart)
        );
    }

    #[test]
    fn delete_backspace_clear() {
        assert_eq!(
            command_for_key("delete", false, false, PAGE),
            Some(ClearCells)
        );
        assert_eq!(
            command_for_key("backspace", false, false, PAGE),
            Some(ClearCells)
        );
    }

    #[test]
    fn unknown_key_is_none() {
        // Printable characters and unhandled keys propagate (no grid command).
        assert_eq!(command_for_key("a", false, false, PAGE), None);
        assert_eq!(command_for_key("escape", false, false, PAGE), None);
        assert_eq!(command_for_key("f5", true, true, PAGE), None);
    }
}
