---
status: complete
---

# Architecture: xlsx File Association

Single architecture doc (no component designs) — the project is small: one packaging-config
block, one new ~small runtime module (macOS), and a doc/CI reconciliation pass.

## 0. Shape — three seams

- **Seam A — Declaration (packaging).** cargo-packager `file-associations` metadata. No runtime
  code. Drives macOS `CFBundleDocumentTypes`, Windows NSIS registry, Linux `.desktop`
  `MimeType=`/`Exec=%F`.
- **Seam B — Delivery (runtime entry point).** How the target path reaches
  `FreeCellApp::open_path`:
  - **Windows / Linux / CLI** → process **argv** → already wired (`main.rs::open_arg`). No change.
  - **macOS** → **Apple Event** → gpui `App::on_open_urls` (no `cx`) → **new bridge module**.
- **Seam C — Reconciliation (docs/CI).** Correct the docs/CI that assert Windows can't compile
  and that macOS Finder-open is unwired. Prose + one CI-gate flip. Mechanical, last.

The whole engine of the feature is **reuse**: every path funnels into the existing
`do_open_path` (canonicalize → dedupe → record-recent → open/import). This project only makes
the OS deliver files to that funnel and declares the association; it adds no open/import logic.

## 1. Files touched

| File | Change | Seam |
|---|---|---|
| `crates/freecell-app/Cargo.toml` | add `[[package.metadata.packager.file-associations]]` ×2 (xlsx, csv); promote `url` + `async-channel` to direct deps | A, B |
| `crates/freecell-app/src/shell/open_files.rs` | **NEW** — `file_url_to_path` (cfg-agnostic, unit-tested) + macOS `install_finder_open` bridge | B |
| `crates/freecell-app/src/shell/mod.rs` | declare + re-export the new module's public items | B |
| `crates/freecell-app/src/main.rs` | platform-split the startup welcome-vs-open decision (see §3.2) | B |
| `.github/workflows/release.yml` | drop `continue-on-error` from the Windows job; fix its comment | C |
| `app/PACKAGING.md`, `projects/windows-port.md`, `GAPS.md`, `specs/projects/mvp/DECISIONS_TO_REVIEW.md`, `.../mvp/coverage_matrix.md`, `.../mvp/smoke_checklist.md`, `README.md`, `app/README.md`, `.../mvp/architecture.md` | correct platform/Finder-open statements | C |

`FreeCellApp::open_path` / `do_open_path` (`shell/app.rs`) are **reused unchanged**.

## 2. Seam A — packaging declaration

Add to `[package.metadata.packager]` in `crates/freecell-app/Cargo.toml`, **two entries**
(distinct mime-types/names):

```toml
[[package.metadata.packager.file-associations]]
extensions  = ["xlsx"]
mime-type   = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
description = "Excel Workbook"
name        = "Excel Workbook"
role        = "Editor"

[[package.metadata.packager.file-associations]]
extensions  = ["csv"]
mime-type   = "text/csv"
description = "CSV Document"
name        = "CSV Document"
role        = "Editor"
```

- These entries contain **no file paths**, so cargo-packager's "cd into the crate manifest dir"
  gotcha (relevant to the `icons` paths) does not apply here.
- Per-format emission is cargo-packager's job (verified against its source): macOS
  `CFBundleDocumentTypes`+`CFBundleTypeExtensions`; NSIS registry ProgId + `shell\open\command`;
  deb `.desktop` `MimeType=…;…;` + `Exec=… %F`. cargo-packager ships **no** `usr/share/mime`
  XML — fine, both MIME types are already in every desktop's shared-mime-info DB.
- Placement: TOML array-of-tables must sit with the other `[package.metadata.packager.*]`
  tables; put both `[[…file-associations]]` blocks **before** the `[package.metadata.packager.deb]`
  table header (a new table header ends the packager table), mirroring the existing ordering
  note in the manifest.

## 3. Seam B — runtime delivery

### 3.1 argv path (Windows / Linux / CLI) — unchanged

`main.rs::open_arg()` already returns the first non-flag arg ending in `.xlsx`/`.csv` and
routes it to `FreeCellApp::open_path`. Both registered extensions are already accepted. No
change. (`%F`/`%1` deliver a single path per launch; multi-select spawns one process per file
on Win/Linux — `open_arg` opens the first, per functional spec §7. Accepted.)

### 3.2 macOS Apple-Event bridge — new

**Problem.** gpui `App::on_open_urls`'s callback is `FnMut(Vec<String>)` with **no `cx`**
(verified at the pinned rev and on gpui `main`), so it can't open a window from inside itself.
**Solution** = the Zed-proven pattern: a channel bridges the `cx`-less callback into an app
task, and the startup UI decision reads the channel **before** showing welcome (the no-flash
requirement, functional spec §5.5).

**New module `shell/open_files.rs`:**

```rust
// cfg-agnostic, pure, unit-tested (runs on Linux CI).
pub fn file_url_to_path(raw: &str) -> Option<PathBuf> {
    // 1. Try to parse as a URL; accept only the `file` scheme.
    if let Ok(url) = url::Url::parse(raw) {
        if url.scheme() == "file" {
            return url.to_file_path().ok();   // percent-decoding handled by `url`
        }
        return None;                          // http/https/custom scheme → ignore
    }
    // 2. Defensive fallback: a bare absolute path handed through as-is.
    raw.starts_with('/').then(|| PathBuf::from(raw))
}

#[cfg(target_os = "macos")]
pub fn install_finder_open(cx: &mut App, opened_via_argv: bool) {
    use async_channel::unbounded;
    let (tx, rx) = unbounded::<Vec<String>>();
    // Callback has no cx: it only forwards the raw URL strings.
    cx.on_open_urls(move |urls| { let _ = tx.try_send(urls); });

    cx.spawn(async move |cx| {
        // Let the platform deliver any launch-time openURLs (fired during
        // didFinishLaunching) before we decide welcome-vs-open. See "coalesce" note below.
        cx.background_executor().timer(STARTUP_COALESCE).await;

        let mut opened_any = opened_via_argv;
        while let Ok(urls) = rx.try_recv() {                 // drain launch events
            opened_any |= open_all(&urls, cx);
        }
        if !opened_any {
            cx.update(|cx| FreeCellApp::show_welcome(cx)).ok();   // deferred → NO flash
        }
        while let Ok(urls) = rx.recv().await {               // warm-start events, for app life
            open_all(&urls, cx);
        }
    })
    .detach();
}

// helper: parse+open each, return whether any opened. `cx.update` runs on gpui's foreground.
```

**Startup control flow in `main.rs`** (replaces the current `match open_path { Some→open,
None→show_welcome }`):

```rust
let opened_via_argv = match open_path_arg {
    Some(p) => { FreeCellApp::open_path(&p, cx); true }
    None => false,
};
#[cfg(target_os = "macos")]
shell::install_finder_open(cx, opened_via_argv);
#[cfg(not(target_os = "macos"))]
if !opened_via_argv { FreeCellApp::show_welcome(cx); }
```

Key properties:
- **No welcome flash.** On macOS, welcome is shown **only** from inside the spawned task,
  after the coalesce window + channel drain. It is never shown synchronously, so a Finder cold
  start that carries a file never flashes welcome. A normal (fileless) cold start shows welcome
  `STARTUP_COALESCE` later — imperceptible.
- **Single owner of `rx`.** One task handles both the launch drain and the warm-start loop, so
  there's no split-ownership race.
- **Platform isolation.** The bridge is `cfg(target_os = "macos")`; Windows/Linux keep today's
  synchronous decision and spawn no idle task. `file_url_to_path` stays cfg-agnostic so it's
  unit-tested on the Linux CI.

**`STARTUP_COALESCE` (robustness margin, not a guess).** If gpui flushes buffered launch URLs
synchronously when `on_open_urls` registers (Zed relies on a synchronous `try_recv` at
startup), the URLs are already queued before the task's first line runs and the timer could be
zero. If gpui instead delivers them via the event loop shortly after registration, a small
window (start at **50 ms**) catches them. The **spike** confirms which and tunes/removes the
timer; either way the design is correct because welcome is gated behind the drain. Define it as
a named `const STARTUP_COALESCE: Duration` so the spike can adjust one value.

**Spike, blocking findings.** Before hardening, confirm on real macOS: (a) the mac gpui
platform actually invokes `on_open_urls` for a Finder open at the pinned rev; (b) a launch-time
event is captured within the coalesce window (no-flash holds). If (a) fails at the pinned rev,
**stop and raise it** — the only fallback is an `NSApplicationDelegate`, which fights gpui and
is out of scope without explicit sign-off (do not silently add it).

### 3.3 URL/path shapes to handle (drives `file_url_to_path` tests)

- `file:///Users/x/My%20Book.xlsx` → `/Users/x/My Book.xlsx` (percent-decoded).
- `file:///Users/x/データ.xlsx` (unicode) → decoded path.
- `https://…`, `freecell://…`, `mailto:…` → `None` (no scheme registered).
- `/Users/x/book.csv` (bare abs path fallback) → `PathBuf`.
- `""`, `"not a url"`, `"C:\\x"` → `None` (macOS-only module; Windows uses argv, not this).

## 4. Data model

None persistent. Transient: an `async_channel` of `Vec<String>` (raw URL strings) → parsed to
`PathBuf` via `file_url_to_path` → existing `open_path`. No new entities, no storage.

## 5. Error handling

- **Unparseable / non-file URL:** `file_url_to_path` returns `None`; the bridge skips it (log at
  `debug`). Not user-facing — there's no file to open.
- **Valid path, open fails:** existing `do_open_path` handles it — missing/denied →
  `report_error("Couldn't open the file", …)`; non-xlsx content → the typed
  `LoadError::NotXlsx` loading-window error dialog. No new UX.
- **Channel `try_send` fails** (receiver dropped at shutdown): ignored (`let _`).

## 6. Testing strategy

- **`file_url_to_path` unit tests** (Linux CI, no gpui) — the cases in §3.3: percent-encoded
  spaces, unicode, non-file schemes → `None`, bare-abs-path fallback, empty/garbage → `None`,
  and a multi-URL vector mapping. This is the bulk of the automated coverage and it runs
  everywhere.
- **Bridge routing** — where feasible, a `TestAppContext` test that pushes a `Vec<String>` into
  the channel and asserts it routes into the open funnel, mirroring the existing
  `open_path_detached` worker-less test harness in `shell/app.rs`. Keep the gpui-touching
  wiring thin; the deep verification of Finder events is the manual smoke.
- **Packaging** — build the `.deb` (runnable here) and assert the generated `freecell.desktop`
  carries `MimeType=…spreadsheetml.sheet;text/csv;` and `Exec=… %F`. Optionally a test that
  reads the crate manifest and asserts both `file-associations` entries are present/well-formed.
- **Checks** — whole-workspace `cargo fmt --all --check`; crate-scoped `cargo build`/`cargo test
  -p freecell-app`. **No pixel render suite** — opening a file changes no grid/cell/sheet/
  titlebar/chart pixels (render-scope rule); validate instead via the tests above + the smoke
  launch.

## 7. Dependencies

Both already resolved in `Cargo.lock` — this only **promotes transitives to direct deps** of
`freecell-app` (adds nothing to the lock, same pattern as the existing `dirs` dep note; both are
MIT/Apache, so no new `cargo-deny` surface):

- `url = "2"` — `Url::parse` / `to_file_path` for `file_url_to_path`.
- `async-channel = "2"` — the callback→task bridge (`Sender: Clone + Send + 'static`, `try_recv`
  + async `recv`).

No new crates. No signing/notarization deps (out of scope).

## 8. Seam C — docs/CI reconciliation (mechanical, last)

After A+B land (and given Windows is owner-confirmed working), correct — per functional spec
§8, with its guardrail — the statements that assert the opposite:

- `.github/workflows/release.yml`: remove `continue-on-error: true` from the `windows` job;
  rewrite the comment (no longer "may fail to compile").
- `app/PACKAGING.md`: revise "Windows: what a real port needs" (esp. item 3 file-associations →
  done) and the unsigned-note framing that calls Windows non-compiling.
- `projects/windows-port.md`: reconcile "app build not a real target" / item 3; narrow to
  whatever genuinely remains (or mark the association + build items done).
- `GAPS.md` #4 (macOS Finder open-file): mark **resolved**, pointer to this project.
- `specs/projects/mvp/DECISIONS_TO_REVIEW.md` Phase 10, `coverage_matrix.md` §2.1,
  `smoke_checklist.md` M-15: update the deferred/known-limitation status.
- `README.md`, `app/README.md`, `specs/projects/mvp/architecture.md`: platform-support and
  Finder-open lines.

Guardrail: don't claim a capability not delivered here. Windows-works is owner-confirmed (CI
green + local build, 2026-07-24); macOS Finder-open is delivered by seam B (final smoke
confirms, but doc edits + commits are **not** blocked on the smoke, per the cadence decision).

## 9. Risks / open questions

1. **macOS `on_open_urls` actually fires at the pinned rev** — the spike's first check; a
   negative is a blocking finding (fallback `NSApplicationDelegate` is out of scope w/o sign-off).
2. **Launch-event timing vs `STARTUP_COALESCE`** — the spike confirms the no-flash window; the
   const localizes the tuning.
3. **cargo-packager 0.11.8 honors `file-associations`** for app/nsis/deb — present in current
   source; confirmed when the packaged output is first produced.

## 10. Build order → implementation plan

1. **Declaration** (seam A) — packaging config; verify via `.deb` `.desktop`. Enables
   Windows/Linux end-to-end immediately (argv already wired).
2. **macOS entry-point bridge** (seam B) — new module + `file_url_to_path` + tests + main.rs
   startup split (the spike + the substantive code).
3. **Docs/CI reconciliation** (seam C) — after 1+2, so all claims are true.
4. **Release smoke** (user-run, NON-BLOCKING) — build `.app`/`.dmg` + NSIS; macOS smoke M-15
   (double-click / Open-With / drag-to-Dock / `open -a`, cold+warm, xlsx+csv) + optional
   Windows smoke. Phases 1–3 are already committed/pushed before this.
