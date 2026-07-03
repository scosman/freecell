---
status: complete
---

# MVP

Read the existing specs for context. We're building a fast, GPU-rendered spreadsheet app, with near parity to the big-name spreadsheets.

This project is to build the MVP. A workable:

- Scaffolding
  - a root "app" folder, cargo setup, etc.
  - A typical "great modern Rust project" stack: linters, formatters, CI, etc.
- App/UI
  - Welcome screen when you first open with "Open" and "New" options. Open launches a file picker which can open xlsx files; New creates a new spreadsheet.
  - Spreadsheets open in their own window. See details below.
  - Menu: Apple menu with basic File: Save/Open/etc.
- Spreadsheet window
  - Each spreadsheet gets its own window. Minimal shared app state across this boundary.
  - Render tabs for each sheet, can switch sheets.
  - Vertically on page:
    - shared grey background
      - action row: a row of buttons that impact the currently selected cell. For MVP: bold, italic, underline (text), and fill button (background color of cell)
      - Data row: text box showing the content of the current cell
    - Spreadsheet area: full bleed. Very similar to demo app.
    - Tab control: list each sheet, plus icon to add another sheet. Double-clicking title of sheet makes it editable.
  - Cell selection:
    - outline cells when selected
    - single cell selection: value shows in "data row", action-row buttons apply to this cell only
    - multi-cell selection: data-row is disabled, but actions all work for all cells.
  - Scrolling: both directions like demo app
  - P2: dragging to change height of row or width of cols
- Rendering support:
  - a reasonable set of cell formatting: bold/italic/underline, background color
- Data updates
  - when you hit enter on data row, update the cell's value, and run our evaluate loop, refreshing the cells (per plan)
- File support:
  - Opening and saving xlsx files. No CSV support yet. Support only features IronCalc has native support for; this should be relatively full.
  - Accessed via "File" menu options

What else should our MVP have? Feature overview should flesh this out.

## UI

Don't invest heavily in UI design, we're aiming for functional proof of concept.

- Use pretty much stock gpui-component controls.
- Take some care with spacing: it should be usable/nice. But don't go writing a ton of opinionated style code. We'll do design later.
- Do nail the design for the spreadsheet component: we're building that one, let's do it right. It should look… like a good spreadsheet.

## Tech

- gpui-component for most components. Build them ourselves in rare cases (like the core spreadsheet).
- See planning docs, lots of decisions there.
- Rich testing investment: each phase is tested well, doesn't need human review.
- UI testing: when doing cell rendering, I want a "cell render" test suite:
  - Many tests like "bold", "bold-italic", etc. — every feature and most permutations. Good names so my visual review and CI failures are super interpretable.
  - Renders the cell, and compares to a reference PNG for correctness. Completely automated.
  - A manual script to "generate_baselines", creating the reference images. Explain in README the human process: run that, verify results are good before committing files.
  - Extensible: add tests for every new cell formatting/rendering feature. Big suite but reusable infra.

## Implementation Plan Guidance

- Many small phases expected. This is a huge project. Best to have well-defined separate phases.
- In the plan, note when areas can be run in parallel. Common to build many parts in parallel, and have a few integration phases.
- P2/P3 happen later.

## Autonomy

After planning, we expect the rest to complete with a single "run /spec implement all". The spec should have all information needed to make decisions. Where it's missing, the agent should decide. It can track "DECISIONS_TO_REVIEW.md" in the spec folder if needed, but should not stop.
