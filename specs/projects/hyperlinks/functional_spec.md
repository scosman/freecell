---
status: draft
---

# Functional Spec: Hyperlinks

What hyperlink support means in FreeCell, behaviour by behaviour. Technical design lives in
`architecture.md`; engine facts cited here are established in
`research/ironcalc-links-api.md` and `research/freecell-integration.md`.

## Vocabulary

- **Worksheet link** — link metadata attached to a cell, stored in `Worksheet::links`, created by
  the user (⌘K, auto-link) or imported from xlsx. Persisted, undoable, xlsx round-tripped.
- **Dynamic link** — the link produced by a `HYPERLINK()` formula. Lives only in
  `Model::links`, rebuilt on every evaluation, never persisted, not editable except by editing
  the formula.
- **Effective link** — whichever of the two applies to a cell. Worksheet wins on collision. This
  is what the UI acts on.
- **Target** — the URL/`mailto:`/file path (external) or workbook location (internal).
- **Display text** — the cell's own content. Never part of the link.

## F1 — Link appearance

**F1.1** A cell with an effective link renders in link blue (`0x2563EB`, the constant already
used at `shell/about.rs:33`) with an underline.

**F1.2** Worksheet links get this appearance from the **engine**: `UserModel::set_cell_link` and
engine auto-linking both apply `font.u = true` and `font.color = Color::Theme(10, 0.0)` as real
cell formatting, and FreeCell's `resolve_rgb` already resolves theme-indexed colors. No
FreeCell-side styling is applied to these cells.

> Verification gate (Phase 1): confirm theme index 10 actually resolves to a blue. If it does
> not, FreeCell substitutes its own `LINK` constant at cache-build time — see architecture §3.

**F1.3** Dynamic links carry **no** engine styling. FreeCell synthesizes the link appearance for
them at render time, so a `HYPERLINK()` cell is visually indistinguishable from a worksheet link.

**F1.4** Explicit user formatting wins. Because F1.2 is ordinary formatting, a user who sets a
link cell's font color to red gets red — matching Excel. For dynamic links (F1.3), the
synthesized appearance is applied **only** where the cell has no explicit font color of its own;
an explicit color or underline setting is left alone.

**F1.5** Link appearance is preserved everywhere cell text is drawn, specifically including
**spilled** text (a long URL is the common case) and the **anchor of a merged region**.

**F1.6** Removing a link does **not** remove the formatting. This is upstream's documented
behaviour and matches Excel: the blue underline is real formatting the user can clear
themselves. FreeCell does not attempt to unwind it.

**F1.7** The **in-cell editor** does not inherit link appearance. Editing a link cell shows
normal edit text.

**F1.8** Because the engine's link styling is real formatting, the action row's Underline toggle
shows as active on a worksheet-link cell. This is correct and matches Excel's Hyperlink cell
style; it is not a bug to be worked around.

## F2 — Opening a link

**F2.1** A plain single left click on a cell with an effective link opens it. Excel parity.

**F2.2** A click does **not** open the link when any of these hold; the click behaves as an
ordinary selection click:
- the mouse moved between press and release (a drag — the user is selecting a range)
- press and release landed on different cells
- the primary modifier (⌘/Ctrl) is held — the deliberate "select without opening" affordance
- `click_count > 1` (double click opens the in-cell editor, as today)
- a formula edit is reference-ready and point-mode is claiming the click
- the app is in degraded mode

**F2.3** Keyboard and menu routes exist because there is no Linux menu bar and every action must
be reachable without a mouse: **Edit ▸ Open link** and a cell context-menu **Open link** item,
both enabled only when the active cell has an effective link.

**F2.4** Hovering a link cell shows a tooltip with the target, plus the link's stored tooltip
(Excel's ScreenTip) when it has one.

**F2.5** The mouse cursor becomes a pointer over a cell with an effective link.

## F3 — Where links are allowed to go

The engine performs **no** scheme validation at any layer — not `set_cell_link`, not xlsx
import, not `HYPERLINK()`. An opened workbook is untrusted input and can carry
`javascript:`, `data:`, `smb:`, or an arbitrary local path. Upstream's allowlist exists only in
its TypeScript web UI. FreeCell therefore validates immediately before navigating.

**F3.1** These schemes open directly: `http`, `https`, `mailto`, `ftp`, `ftps`.

**F3.2** `file:` opens only after an explicit confirmation dialog naming the path, because
following it reaches the local filesystem on the say-so of a downloaded document. Cancel is the
default.

**F3.3** Every other scheme is refused with a non-blocking message naming the scheme. Nothing is
launched.

**F3.4** Validation happens at **open** time, not at store time. A link with a refused scheme
still imports, still renders as a link, still round-trips to xlsx, and is still visible in the
⌘K dialog — FreeCell does not silently rewrite a user's file. It just will not launch it.

**F3.5** Validation is a pure, gpui-free function with direct unit tests, per the repo's
pure-logic-first house style.

## F4 — Internal links

**F4.1** An internal link (`Link::Internal`) targets a location in this workbook: `Sheet1!A30`,
or a defined name. `HYPERLINK("#Sheet1!A1")` produces one, and xlsx files carry them.

**F4.2** Opening an internal link moves the selection to that location, switching the active
sheet if needed, and scrolls it into view.

**F4.3** A target that does not resolve (missing sheet, unknown defined name, malformed
reference) produces a non-blocking message and no navigation.

**F4.4** Internal links are not subject to F3 — no scheme is involved and nothing is launched.

## F5 — Creating and editing links (⌘K)

**F5.1** ⌘K on the active cell opens the link dialog. The Edit menu gains **Insert link… ⌘K**,
the cell context menu gains an equivalent item, and the action row gains a link button.

**F5.2** The dialog has two fields: **Target** and **Display text**. On a cell that already has
a worksheet link, both are pre-filled from the existing link and the cell's content, and the
dialog reads as an edit (title, plus a Remove button); otherwise Target is empty and Display
text is pre-filled from the cell's current content.

**F5.3** Confirming with a non-empty Target sets the link and, when Display text differs from
the cell's current content, sets that too — as **one undo step**. Upstream's
`set_cell_link(…, label: Option<&str>)` does exactly this, so FreeCell does not compose two
operations.

**F5.4** Enter confirms, Escape cancels, the Target field is focused with its text selected on
open, and clicking the backdrop cancels.

**F5.5** Confirming with an empty Target removes the link, if any.

**F5.6** The dialog operates on the **single active cell**. With a multi-cell selection it acts
on the active cell only. Applying a link across a range is out of scope for this round.

**F5.7** The dialog refuses a Target whose scheme is refused under F3, inline, before storing —
FreeCell will not *author* what it will not open. (F3.4's tolerance applies to links that
arrived from a file, not to ones typed here.)

**F5.8** A Target with no scheme is normalized on entry using the same rules the engine uses for
auto-linking: `www.x.y` → `https://www.x.y`, a bare `local@domain.tld` → `mailto:…`. A leading
`#` makes it an internal link.

**F5.9** ⌘K is disabled in degraded mode and on the Welcome window.

**F5.10** ⌘K pressed while the in-cell editor or the find field has focus first commits the
in-flight edit, then opens the dialog on the resulting cell — mirroring how `toggle_style`
already behaves.

## F6 — Removing a link

**F6.1** **Remove link** appears in the Edit menu, the cell context menu, and the ⌘K dialog, and
is enabled only when the active cell has a **worksheet** link.

**F6.2** Removing is one undo step and leaves the cell's content and formatting untouched (F1.6).

**F6.3** A dynamic link cannot be removed — only its formula can be edited. The Remove affordance
is disabled on such a cell.

## F7 — Auto-linking

**F7.1** Typing a URL or email address into a cell creates a link automatically. This is
**engine** behaviour (`Model::set_user_input` → `auto_link_cell`); FreeCell inherits it and must
not reimplement URL detection.

**F7.2** Accepted forms: `http://`, `https://`, `ftp://`, `ftps://`, `mailto:` prefixes
(case-insensitive, original case preserved); `www.x.y` gains `https://`; a bare
`local@domain.tld` becomes `mailto:`. Whitespace anywhere in the value disqualifies it.

**F7.3** Auto-linking fires only for text input — never for numbers, dates, booleans, errors, or
formulas.

**F7.4** A leading apostrophe (`'http://x.com`) suppresses it. This is the user's escape hatch
and FreeCell surfaces it in documentation rather than inventing a second one.

**F7.5** Auto-linking is undoable as part of the edit that caused it. **FreeCell's batch
`set_user_inputs` currently bypasses the engine's link-diff wrapper**, so an auto-created link
would survive undo. Fixing that is fork work in this project (architecture §2).

**F7.6** Pasting values that look like URLs auto-links them, per the same engine path.

## F8 — Links and other operations

All of these are engine behaviour; FreeCell's job is to not break them and to prove they work.

**F8.1** Insert/delete rows and columns displace links correctly; deleted rows drop their links,
and undo restores them.

**F8.2** Moving a row or column carries its links.

**F8.3** Copy/paste within FreeCell carries links, because they ride in IronCalc's
`ClipboardData`. Cut clears the source link. Autofill copies the source cell's link.

**F8.4** **Paste Values drops links**, matching Excel.

**F8.5** Clearing a cell's contents removes its link.

**F8.6** External (TSV) paste does not carry links. This is the existing Excel-clipboard-interop
gap, not new.

## F9 — `HYPERLINK()`

**F9.1** `HYPERLINK(link_location, [friendly_name])` evaluates: the cell **displays**
`friendly_name` when given, otherwise `link_location`.

**F9.2** The resulting cell is clickable exactly like a worksheet link (F2), and validated
exactly like one (F3).

**F9.3** A `#` prefix on `link_location` makes it an internal link (F4).

**F9.4** Wrong argument count is `#ERROR!`. If the display value is itself an error, no link is
attached and the error shows.

**F9.5** Being a text result, a `HYPERLINK()` cell left-aligns under the existing type-aware
alignment rules. That is correct and is not to be "fixed".

## F10 — File round-trip

**F10.1** Opening an `.xlsx` with hyperlinks imports them; they render and open.

**F10.2** Saving preserves them, including tooltips (ScreenTips) and internal links, and writes
the required per-sheet relationship parts.

**F10.3** A link created in FreeCell survives a save → reopen cycle.

**F10.4** An imported range hyperlink (`<hyperlink ref="B2:C3">`) becomes one link per cell,
capped at 10 000 cells by the engine's guard; beyond the cap only the top-left cell is linked.

**F10.5** `projects/xlsx-preservation.md` currently lists hyperlinks among the parts FreeCell
drops. That is no longer true once this ships and the note is corrected.

## Out of scope

Deliberately excluded from this round; each becomes a `GAPS.md` row or a `projects/` note rather
than an "accepted limitation":

- **Applying one link across a multi-cell range** (F5.6).
- **A workbook-wide link manager** (list/jump/bulk-remove). Upstream's `get_links_list` makes it
  cheap later, but there is no Excel-parity pressure for it.
- **Rich in-cell link text** — a link is a whole-cell property. `InputState` exposes no range
  styling (`projects/styled-text-input-control.md`), so part-of-a-cell links are not possible.
- **External-workbook links** — a separate, later `GAPS.md` row; not to be conflated with this one.
- **Links carried through the system clipboard to/from Excel** — the existing Excel-clipboard
  interop gap.

## Done means

- Every F-item above holds, with tests.
- `GAPS.md`'s **Hyperlinks** row is **deleted** (repo policy: a fixed gap is removed, not
  annotated), and the `HYPERLINK()` clause goes with it.
- `projects/xlsx-preservation.md`'s "hyperlinks dropped" bullet is corrected.
- New `GAPS.md` rows exist for the out-of-scope items that are genuine holes (range-apply,
  `file:` links if F3.2 lands as a block rather than a confirm).
- The full pixel suite is green with eyeballed `link_*` baselines, and the CI `render` gate
  passes on the branch.
