---
status: complete
---

# Recent Files + Welcome Screen

Add a **recent files** list to FreeCell and redesign the **welcome screen** around it.

## What we want

- A list of recently-opened spreadsheets, persisted across app launches.
- Recent files surfaced in two places:
  - On the **welcome window** (the launch screen), in a redesigned two-pane layout.
  - Under **File → Open Recent** in the macOS menu bar.
- The welcome screen gets a fresh design (mockups provided): a left column with the
  FreeCell wordmark, tagline, and the **New Spreadsheet** / **Open…** buttons, and a right
  column showing the recent list (or an empty state when there are none).
- New welcome tagline: **"The open spreadsheet"** (replacing "A fast, Excel-compatible
  spreadsheet.").

## Product decisions (confirmed)

- **Recency** = *last opened in FreeCell* (most-recently-used first), not the file's own
  modified time.
- **Missing/moved files** are **silently dropped** from the list — the welcome pane and the
  menu only ever show files that currently exist on disk.
- **Retention:** the welcome pane shows up to **5** recent files; the **Open Recent** menu
  shows up to **10**.

## Why

The current welcome window is a bare centered stack (name + tagline + two buttons) with no
memory of what you were working on. Every launch starts cold. A recent-files list is table
stakes for a document app and is the fastest path back into prior work.

## Design references

Two mockups were provided: (1) the populated welcome screen with a five-row RECENT list,
and (2) the empty state ("No recent spreadsheets"). Their layout and styling are captured in
`ui_design.md`.
