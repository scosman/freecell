---
status: draft
---

# FreeCell

The project is simple: build a better Spreadsheet app.

Essentials done right:
- GPU based rendering engine (a la Zed/Ghostty)
- Stupid fast / tested: Rust + GPU
- Resource efficient: can handle huge spreadsheets.
- Excel compatibility required: exact feature set we match will be documented, tested, and can grow over time without regressions.
- Deep testing: we assume we're going to find bugs. Goal is to make a test suite that grows over time: test all core features, add tests for every bug fix to prevent regression (expect many excel compatibility bugs), expand compatible tests (XLOOKUP), rendering tests compare to known-good PNG (rendering cells with highlight/bold/italic, and 100 combinations).
- Great UI: excellent menus, dark mode. Powerful, but approachable.

## Tech Direction

Needs confirmation, but this is the current lean.

- **Core engine:** [PSU3D0/formualizer — Embeddable spreadsheet engine: parse, evaluate & mutate Excel workbooks from Rust, Python, or the browser. Arrow-powered, 320+ functions.](https://github.com/psu3d0/formualizer)
  - Actually has a really great API. This acts as a pretty clear data model layer for us.
- **UI:** Zed's library is Apache: GPUI.

Chats:
- "gpui-component" has web-component, so could use e-charts.
- "gpui-component" has charts, but prob not good enough.

## Process

This whole project will be agentic using the /spec tooling.

- **Stage 1:** outlined below. Validate core technical questions, remove technical uncertainty. All work in "experiments" folder, no app exists yet.
- **Stage 2:** validate more technical uncertainty.
- **Stage 3:** decide if we want to continue…

This spec project covers **Stage 1** (Phase 1).

## Sub-projects: Phase 1

### Challenge design direction

Is Formualizer + GPUI a great base tech stack? Any alternatives I should consider? Research using web agents, hypothesize alternatives. Suggest a few alternatives and come up with a ranked suggestion list with reasoning.

### File support

Confirm Formualizer can read/write modern Excel files and CSV. Need to be able to load and save files. If not native to Formualizer, then need a plan for doing this in our app. Research, and propose.

### Datamodel Binding & Perf Testing

Interconnect between Formualizer and a UI layer is a core technical design element, that will drive a lot of our perf/scalability.

- Setting values seems easy enough: `set_value`, I don't see concerns (but challenge that).
- Mostly concerned about binding from data model to UI: how do we pull all values for current view, as I scroll around quickly, and update as data changes.
  - Need to design the binding layer: UI just calling `evaluate_cell` to load each as I scroll. Is that fast enough? Range access APIs? Parallelism for `evaluate_cell`? How do I subscribe to updates for the visible cells? Caching needed or internal? And if caching needed, how do we handle invalidation?
  - Understand the underlying data model (Apache Arrow), and how it impacts performance, and what makes the ideal load/subscribe patterns.
  - See: [Large Workbook Performance](https://www.formualizer.dev/docs/guides/large-workbook-performance)
  - Basically we are validating the performance characteristics of: will Formualizer work for our app (UI spreadsheet, with visible pane). Design the access pattern, then design some pure-code benchmarks for the access pattern. Want to verify it's stupid fast on large sheets, across a range of cases.
  - Scrolling around: benchmark read access similar to scrolling around rapidly.
  - Updates: I update a cell, but it impacts other cells in view (maybe via cells that are on different sheets/offscreen): benchmark the "change cascade, then get updates for visible" use case.
- General performance validation:
  - Make sure Formualizer is fast.
  - Example: cascading changes are fast. 1M rows of "=LAST_CELL+1" -> update first cell, and propagation should be lightning fast. This is just an example: propose a few more.
- Memory / RAM:
  - Loading a reasonably large sheet and making changes: is memory reasonable? Should be okay I assume, but let's validate with real tests.
- Anything else:
  - What are the other perf-critical areas we should validate at the beginning?

We probably should compare a few different designs/approaches, and measure what's best. Expect to have to iterate on perf in a research loop.

### Formatting: Research & Pre-Validations

Spreadsheets have formatting: row widths, bold, lines, font-size, etc.

Formatting:
- Does Formualizer (or the engines it uses under hood for XLSX) expose formatting information?
- Does Formualizer offer format storage fields? General metadata storage we could use that uses the same Arrow backend? Nothing and we need to build it?
- Writing files: if I load a file, edit formatting, then want to save: easy? Hard?

Research what Formualizer has, and propose a design, and next best alternative.

### UI Technical Test

Validate our core concept of using GPUI to build a crazy fast spreadsheet app.

Best way to do this: build a proof of concept!

Build a basic GPUI app that has a giant grid, and let me play with it to see speed. Doesn't need to be polished, but not ugly as sin either. It's a spreadsheet: headers, columns, data, formatting, etc. But big (not sure what "really big sheet" is nowadays, but pick a "if this works we're good" size for this test).

Data:
- Don't need to connect a real datamodel, but have a static datamodel provider (code that returns values for each cell, making a reasonable proxy for a big difficult spreadsheet).
- Do need a variety of formatting to test it's fast: highlighting, variable row/col widths, bold, italic.

Not sure if there are valuable automation tests here. But would be ideal to:
- human sanity check rendering is correct enough to evaluate
- agent loops on performance using automated testing for common cases: scroll, scroll fast, jump to cell, etc.

Expect to have to iterate on perf in a research loop.

### Round 2 technical exploration

Propose a list of technical explores to do next to de-risk this project.
