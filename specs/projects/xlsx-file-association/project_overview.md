---
status: complete
---

# xlsx File Association

## Goal

Make FreeCell register with the OS as an application that can open `.xlsx` files, so
double-clicking a workbook in Finder / Explorer / a Linux file manager (or "Open With… →
FreeCell", or `open -a FreeCell book.xlsx`) launches FreeCell and opens that file.

## Priority

- **P1: macOS and Windows.**
- **P2: Linux.**

("Register as *a* handler" is the goal — appearing in the OS's Open-With list and being a
valid double-click target. Becoming the user's *default* `.xlsx` app is a separate,
user-controlled action on every OS and is **not** in scope; see Non-goals.)

## Why

FreeCell's whole pitch is "open and edit Excel files." Today the only wired way to open a
file is the in-app **Open…** panel, a welcome-screen click, or a CLI path argument. The most
natural way a user reaches a spreadsheet app — double-clicking the file — does nothing for
FreeCell. On macOS (the primary design target) this is a conspicuous gap.

## Background — what already exists

This project builds on prior research (this session) and existing repo tracking. Two
separable jobs:

1. **Declaration** (tell the OS the app handles `.xlsx`). FreeCell already packages with
   **cargo-packager** (pinned 0.11.8), which supports a `file-associations` config block that
   emits the right per-OS metadata from one place:
   - macOS `.app`: `CFBundleDocumentTypes` (extension-based `CFBundleTypeExtensions`).
   - Windows NSIS: registry ProgId + `shell\open\command "…\freecell.exe" "%1"`.
   - Linux `.deb`/AppImage: `.desktop` `MimeType=` + `Exec=… %F`.

2. **Delivery / entry point** (receive the path when the OS launches the app for a file).
   This differs by OS:
   - **Windows & Linux**: the path arrives as a normal **argv**, which FreeCell already
     routes (`main.rs::open_arg` → `FreeCellApp::open_path` → `do_open_path`:
     canonicalize → dedupe → open). So these are essentially **packaging-config-only**.
   - **macOS**: the path arrives as an **Apple Event**, *not* argv. gpui surfaces it via
     `App::on_open_urls`, but at our pinned rev (and on gpui `main`) that callback is
     `FnMut(Vec<String>)` with **no `cx`**, so it can't open a window from inside itself.
     This is the one real piece of engineering, already tracked as **GAPS.md #4** and
     `DECISIONS_TO_REVIEW` Phase 10, and overlapping `projects/windows-port.md` item 3.

So the P1 split is inverted from intuition: **Windows is nearly free** (config + existing
argv path); **macOS is the actual work** (the Apple-Event bridge to reuse `open_path`).

## Non-goals

- **Becoming the default `.xlsx` handler.** We register as a candidate; making FreeCell the
  default (over Excel / Numbers / LibreOffice) is a user action per OS, and cargo-packager
  exposes no force-default knob.
- **Code signing / notarization.** Unsigned dev builds still trip Gatekeeper/SmartScreen on
  first launch; that's the separate `projects/release-signing-and-distribution.md` deferral.
- **Cross-process de-dupe / single-instance.** On Windows/Linux each double-click spawns a new
  process; FreeCell's de-dupe is in-process. Single-instance IPC is a separate future nicety.
- **Additional extensions** (`.csv`, `.xls`, `.xlsm`). `.csv` already routes through `open_arg`
  so it could ride along cheaply, but the ask is `.xlsx`; treat others as an optional extra to
  decide during functional-spec, not a commitment here.

## Constraints / notes

- One shared cargo-packager `file-associations` block serves all three OSes.
- No new rendering: opening a file routes through existing `open_path`, so the pixel render
  suite is out of scope.
- macOS verification requires a real installed `.app` bundle (Launch Services doesn't register a
  bare `cargo run` binary) on real macOS hardware — it can't be exercised under the Linux/Xvfb
  harness (smoke item **M-15**).
