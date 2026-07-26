---
status: complete
---

# Functional Spec: xlsx File Association

## 1. Scope & platforms

Register FreeCell with the OS as a handler for spreadsheet files, so opening such a file from
the desktop (double-click, "Open With… → FreeCell", drag-onto-icon, `open`/`xdg-open`, or a
shell path arg) launches FreeCell with that file open.

- **Extensions registered:** `.xlsx` and `.csv` (decided 2026-07-24).
- **Platforms:** macOS, Windows, Linux — all three are **in scope and first-class**. Priority
  for effort/verification order is macOS + Windows (P1), Linux (P2), but all three ship here.
- **Handler rank:** register as a **candidate** handler only (appears in the OS "Open With"
  list / is a valid double-click target). Making FreeCell the *default* `.xlsx`/`.csv` app is a
  user action; we never seize it automatically, but Phase 5 lets the user opt into it via a welcome-
  screen "Set as default" link (`shell::default_app`). (See §9 Non-goals.)

## 2. Registered types

| Ext | macOS `CFBundleTypeName` | Linux MIME (`mime-type`) | Windows `description` | In-app routing |
|---|---|---|---|---|
| `xlsx` | `Excel Workbook` | `application/vnd.openxmlformats-officedocument.spreadsheetml.sheet` | `Excel Workbook` | opens the workbook (dedupe against open windows) |
| `csv` | `CSV Document` | `text/csv` | `CSV Document` | **imports** into a fresh untitled workbook (existing `open_path` `.csv` branch) |

Both extensions already route correctly through the app's single open funnel
(`FreeCellApp::open_path` → `do_open_path`): `.xlsx` opens/dedupes; `.csv` imports into a new
untitled workbook (`path: None`, never deduped) per `functional_spec.md §2` of the MVP. This
project adds **no** new in-app open/import behavior — it only makes the OS deliver the file to
that funnel.

## 3. User flows (trigger → outcome)

Outcome for every flow below is identical: FreeCell opens (or, if already running, is
activated) and the file opens in a document window (`.xlsx`) or a new untitled import window
(`.csv`), reusing existing canonicalize → dedupe → record-recent → open semantics. If the
welcome window was showing, it closes once the document window loads (existing
`note_window_loaded`).

**macOS** (primary target):
- Double-click an `.xlsx`/`.csv` in Finder.
- Right-click → **Open With → FreeCell**.
- Drag the file onto the FreeCell Dock icon or app icon.
- `open -a FreeCell book.xlsx` / `open book.xlsx` (when FreeCell is the chosen handler).
- Both **cold start** (app not running) and **warm start** (already running) must work.

**Windows**:
- Double-click in Explorer (once FreeCell is chosen as the handler), or right-click → **Open
  with → FreeCell**.
- `freecell.exe book.xlsx` from a shell.

**Linux**:
- Double-click / "Open With FreeCell" in the file manager (Nautilus, Dolphin, etc.).
- `xdg-open book.xlsx` (when FreeCell is the default) or `freecell book.xlsx`.

## 4. Delivery mechanism & entry-point contract (per OS)

The OS delivers the target path to the app differently per platform. Two of three are already
wired in FreeCell today:

| OS | How the path arrives | Status in FreeCell |
|---|---|---|
| **Windows** | process **argv** (`shell\open\command "…\freecell.exe" "%1"`) | ✅ already handled by `main.rs::open_arg` → `open_path` |
| **Linux** | process **argv** (`.desktop` `Exec=… %F`) | ✅ already handled by `open_arg` → `open_path`; window `app_id` already set (`app.rs:756`) |
| **macOS** | **Apple Event** (`application:openURLs:`), *not* argv → gpui `App::on_open_urls` | ❌ **new work** — the bridge in §5 |

**`open_arg` widening (Windows/Linux/CLI).** `open_arg` already accepts `.xlsx` and `.csv`
(the first non-flag path arg). No change needed for the two registered extensions. It opens
only the **first** path arg; `%F`/multi-select that passes several paths opens just the first
on Windows/Linux (accepted; see §7).

## 5. macOS Apple-Event bridge (the only new runtime code)

**Why it's needed.** On macOS a Finder open does not pass argv; it sends an Apple Event that
gpui surfaces via `App::on_open_urls`. At the pinned gpui rev (verified, and unchanged on gpui
`main`) the callback is `FnMut(Vec<String>)` with **no `cx`**, so it cannot open a window from
inside itself. This is the deferral tracked as **GAPS.md #4** and `DECISIONS_TO_REVIEW`
Phase 10.

**Behavior:**

1. **Register early.** Install the `on_open_urls` handler inside `app.run` **before** the
   startup welcome/open decision, so a launch-time open event is not dropped.
2. **Bridge without `cx`.** The callback (no `cx`) pushes the received URL strings onto a
   shared channel (sender captured by the closure). A spawned app task (`cx.spawn`) receives
   from the channel and dispatches each through `cx.update(|cx| FreeCellApp::open_path(&path,
   cx))` — i.e. the existing funnel, so dedupe/recents/error-handling are reused verbatim.
3. **URL → path.** Accept `file://` URLs only; percent-decode and convert to `PathBuf`.
   Silently ignore any non-`file` URL (FreeCell registers no custom URL scheme).
4. **Multiple URLs in one event** (e.g. multi-select drag onto the Dock): open each, in order.
   Dedupe handles repeats.
5. **Cold-start vs welcome — no welcome flash (REQUIREMENT).** On a Finder cold start the
   welcome window MUST NOT flash before the document opens. The macOS startup path therefore
   does **not** show welcome synchronously; it defers the welcome-vs-open decision until any
   launch-time open event has been drained from the bridge channel, then either opens the
   delivered file (skipping welcome entirely) or, if none arrived, shows welcome. Mechanism
   (spike detail): gpui delivers the launch `openURLs` through the event loop shortly after the
   handler is registered — not synchronously in the `app.run` callback — so the macOS arm
   registers `on_open_urls`, yields at least one event-loop turn to collect a launch event, and
   only then decides. Exact deferral/coalescing window is for the spike to pin down. If
   reliable no-flash launch-URL capture turns out **not** to be achievable at the pinned rev,
   that is a **blocking finding to raise**, not something to ship with a flash.
6. **Warm start.** Event arrives on the running instance → `open_path` opens or focuses
   (dedupe), skipping welcome.

**Spike-first risk.** Two things the implementer must confirm on real macOS before hardening
(they can't be exercised under the Linux/Xvfb harness): (a) the mac gpui platform actually
invokes `on_open_urls` at the pinned rev for a Finder open; (b) launch-time events aren't lost
in the cold-start race. If (a) fails at the pinned rev, that is a blocking finding to raise
(the fallback would be dropping to an `NSApplicationDelegate`, which fights gpui and is out of
scope without explicit sign-off).

## 6. Declaration config (cargo-packager)

One shared `file-associations` block in `crates/freecell-app/Cargo.toml`
(`[package.metadata.packager]`), **two entries** (distinct mime-types/names). cargo-packager
(pinned 0.11.8) emits the correct per-format metadata from this single source:

```toml
[[package.metadata.packager.file-associations]]
extensions  = ["xlsx"]
mime-type   = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"  # Linux
description = "Excel Workbook"   # Windows Explorer "Type" column
name        = "Excel Workbook"   # macOS CFBundleTypeName
role        = "editor"           # macOS CFBundleTypeRole (default); lowercase — see note below

[[package.metadata.packager.file-associations]]
extensions  = ["csv"]
mime-type   = "text/csv"
description = "CSV Document"
name        = "CSV Document"
role        = "editor"
```

`role` MUST be lowercase `"editor"`: cargo-packager's `BundleTypeRole` enum is `camelCase` +
`deny_unknown_fields`, so `"Editor"` fails to deserialize and breaks packaging (the `lib.rs`
`packaging_metadata` tests pin this).


What each format emits (verified against cargo-packager source):
- **macOS `.app`:** `CFBundleDocumentTypes` with `CFBundleTypeExtensions` (extension-based;
  no UTI/`LSItemContentTypes`) + `CFBundleTypeName` + `CFBundleTypeRole`.
- **Windows NSIS:** registry ProgId + `shell\open\command` (via the bundler's
  `FileAssociation.nsh`).
- **Linux `.deb`/AppImage:** `.desktop` `MimeType=…` and `Exec=… %F`. cargo-packager ships
  **no** `usr/share/mime` XML — fine, because both MIME types (`…spreadsheetml.sheet`,
  `text/csv`) are already in every desktop's shared-mime-info DB.

**Build-time confirm (cheap):** verify 0.11.8 honors `file-associations` for `app` + `nsis` +
`deb` when the packaged output is first produced.

## 7. Edge cases & error handling

- **File missing / permission denied / bad path:** existing `do_open_path` behavior —
  `canonicalize` failure calls `report_error("Couldn't open the file", …)`. No new UX.
- **Non-`.xlsx`/`.csv` file routed to us** (e.g. user forces "Open With FreeCell" on a `.txt`):
  goes through `open_path`; a non-xlsx load surfaces the existing typed `LoadError::NotXlsx`
  error dialog. No new UX.
- **Multiple files** in one macOS event: open each. On Windows/Linux, argv delivers one path
  (multi-select typically spawns one process per file); `open_arg` opens the first. Accepted.
- **App already running:**
  - macOS: event routes to the running instance; dedupe focuses an already-open window.
  - Windows/Linux: a second double-click spawns a **new process** (no cross-process dedupe) →
    a second window onto the same file is possible. Accepted (single-instance IPC is a
    non-goal; §9).
- **Symlinks/aliases:** `canonicalize` resolves them before dedupe (existing behavior).
- **Unsigned dev build:** Gatekeeper/SmartScreen still gate first launch; associations only
  take effect once the built bundle/installer is **installed** (not from `cargo run`). Not a
  code concern — a testing note (§11).

## 8. Documentation reconciliation

Because Windows is now in scope as a working target, and macOS Finder-open is being wired,
several docs/records currently assert the opposite and must be corrected **as part of this
project** — but only after the corresponding capability is verified (Windows via a green CI
build, §11):

- `app/PACKAGING.md` — the "Windows: what a real port needs" section and the "not guaranteed
  to compile" / `continue-on-error` framing; item 3's "`.xlsx` file associations … want a
  real look" becomes "done."
- `.github/workflows/release.yml` — flip the Windows job off `continue-on-error` **once it's
  green**, and update its explanatory comment.
- `projects/windows-port.md` — reconcile "app build not a real target" and item 3 (file
  associations) against reality; if the port is effectively done, mark accordingly / narrow to
  whatever genuinely remains.
- `GAPS.md` #4 (macOS Finder open-file) → mark resolved with a pointer to this project.
- `specs/projects/mvp/DECISIONS_TO_REVIEW.md` Phase 10, `coverage_matrix.md` §2.1,
  `smoke_checklist.md` M-15 — update the "deferred / known-limitation" status of Finder-open.
- `README.md` / `app/README.md` / `architecture.md` platform-support statements, where they
  say Windows is out of scope or Finder-open is unwired.

**Grounding:** Windows-compiles is confirmed by the product owner (CI has run green and a local
build succeeded, 2026-07-24), so the Windows doc/CI reconciliation proceeds directly — flip
`continue-on-error` off and correct the prose. (An implementer may read the latest CI Windows
run to corroborate, read-only; it does not gate the change.) macOS Finder-open prose is updated
alongside the bridge code; the final hardware smoke (§11) confirms it but, per the cadence
decision, doc updates and code commits are **not blocked** on that smoke.

## 9. Non-goals

- **Silently becoming the default handler.** Packaging registers a **candidate** handler only; we
  never seize the default on the user's behalf, and we set no `LSHandlerRank` to auto-outrank other
  apps. **Now in scope (Phase 5):** an explicit, *user-initiated* "Set as default" action — macOS
  `LSSetDefaultRoleHandlerForContentType` and Linux `xdg-mime default …` set it silently on click;
  Windows cannot (the Win10/11 UserChoice hash), so it opens `ms-settings:defaultapps` as a guided
  hand-off. See the `shell::default_app` module + welcome-screen link.
- **Additional extensions** beyond `.xlsx`/`.csv` (e.g. `.xls` — unreadable by IronCalc;
  `.xlsm` — not requested).
- **Custom URL scheme / deep links** (`freecell://…`). Only `file://` open events are handled.
- **Cross-process de-dupe / single-instance IPC.**
- **Code signing / notarization** (separate deferral:
  `projects/release-signing-and-distribution.md`).
- **AppImage desktop-integration UX** — associations in an AppImage only activate if the user
  integrates it (`appimaged`/Gear Lever); the `.deb` registers on install. We ship the
  metadata; we don't build an integrator.

## 10. Constraints & assumptions

- **No rendering changes.** Opening a file routes through existing `open_path`; no grid/cell/
  sheet/titlebar/chart pixels move. The pixel render suite is **out of scope** (per the repo
  render-scope rule). Verify with the crate's gpui view/unit tests + a smoke launch instead.
- **gpui `on_open_urls` lacks `cx`** at the pinned rev and on `main` (verified) → the §5
  bridge is required; a gpui bump is **not** a fix and is not pursued.
- **cargo-packager 0.11.8** is assumed to honor `file-associations` across `app`/`nsis`/`deb`
  (present in current source; confirmed at first packaged output).
- **Windows build compiles/runs** — confirmed by the product owner (CI green + local build,
  2026-07-24); this project flips the CI gate and corrects the docs to match.
- **macOS association testing requires a real installed `.app`** on macOS hardware; it cannot
  be validated in this Linux/Xvfb environment.

## 11. Verification & acceptance

**Committable, in-environment (gates each coding phase — all phases commit + push):**
- Config: `cargo metadata`/packager reads the `file-associations` block; unit-assert the block
  is present/well-formed if a test seam exists.
- macOS bridge: unit tests for `file://`→`PathBuf` parsing (incl. percent-decoding, non-file
  URL rejection, multi-URL) and that parsed paths route into the existing `open_path` funnel
  (mirroring the existing `open_path_detached` test pattern — no real OS thread).
- Linux: build the `.deb`, assert the generated `.desktop` carries `MimeType=` for both types
  and `Exec=… %F`. (Runnable in this environment.)
- Whole-workspace `cargo fmt --all --check`; crate-scoped `cargo build`/`cargo test -p
  freecell-app` per the repo build-efficiency rule.

**Windows (confirmed working per owner; not gated):**
- Windows builds green (owner-confirmed via CI + local build, 2026-07-24). Flip the `release`
  workflow's Windows job off `continue-on-error` and correct the Windows docs (§8). Optionally
  corroborate by reading the latest CI Windows run (read-only); it does not block.

**Final smoke phase — user-run, macOS hardware, NON-BLOCKING (cadence decision 2026-07-24):**
- All coding phases are committed and pushed **before** this; the smoke does **not** gate
  commits/push.
- Build the `.app`/`.dmg` (`scripts/package.sh`), install, and confirm on real macOS:
  double-click, Open-With, drag-to-Dock, `open -a`, cold + warm start — for `.xlsx` and
  `.csv` (smoke item **M-15**).
- Optional Windows hardware smoke of the installed NSIS build (double-click + Open-With),
  complementary to the CI green build.
