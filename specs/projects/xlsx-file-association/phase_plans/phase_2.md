---
status: complete
---

# Phase 2: macOS entry-point bridge (the substantive phase + spike)

## Overview

On macOS a Finder open does not pass the file via argv — AppKit delivers it as an Apple Event
(`application:openURLs:`) that gpui surfaces through `App::on_open_urls`, whose callback is
`FnMut(Vec<String>)` with **no `cx`** (so it cannot open a window from inside itself). This phase
adds the Zed-proven bridge that adapts that cx-less callback into the existing
`FreeCellApp::open_path` funnel, plus the cfg-agnostic `file:// → PathBuf` helper (the bulk of the
automated coverage), and splits the `main.rs` startup welcome-vs-open decision so the welcome
window never flashes on a Finder cold-start (functional §5.5 REQUIREMENT).

**Spike result (source-verified at pinned gpui rev `1d217ee`):** the wiring the architecture
assumes is real and achievable — see the roadblock-guardrail note at the bottom. No
`NSApplicationDelegate` fallback is needed; proceed.

## Steps

1. **`crates/freecell-app/Cargo.toml`** — promote two transitives to direct deps (both already in
   `Cargo.lock`; no lock churn): `url = "2"` (2.5.8) and `async-channel = "2"` (2.5.0). Add under
   `[dependencies]` with a short comment tying them to the xlsx-file-association project.

2. **`crates/freecell-app/src/shell/open_files.rs`** (NEW):
   - `pub fn file_url_to_path(raw: &str) -> Option<PathBuf>` — cfg-agnostic, pure. Parse via
     `url::Url::parse`; accept only `file` scheme (`url.to_file_path().ok()`, percent-decoding
     handled by `url`); any other scheme → `None`; on parse failure, defensive bare-absolute-path
     fallback (`raw.starts_with('/')`).
   - `#[cfg(target_os = "macos")] const STARTUP_COALESCE: Duration = Duration::from_millis(50)` —
     the robustness-margin coalesce window (spike may tune this one value).
   - `#[cfg(target_os = "macos")] pub fn install_finder_open(cx: &mut App, opened_via_argv: bool)`
     — `async_channel::unbounded::<Vec<String>>()`; register cx-less `cx.on_open_urls(move |urls|
     { let _ = tx.try_send(urls); })`; `cx.spawn` a detached task that (a) awaits
     `STARTUP_COALESCE`, (b) drains launch events via `rx.try_recv()` routing each through
     `open_all`, (c) if nothing opened (incl. argv), shows welcome *from inside the task* (no
     flash), (d) loops `rx.recv().await` for warm-start events for app life.
   - `#[cfg(target_os = "macos")] fn open_all(urls: &[String], cx: &AsyncApp) -> bool` — parse each
     via `file_url_to_path`, route `Some` through `cx.update(|cx| FreeCellApp::open_path(&p, cx))`,
     `debug!`-log + skip `None`; return whether any file URL was routed (mirrors `opened_via_argv`
     suppression semantics).
   - `#[cfg(test)] mod tests` — cfg-agnostic `file_url_to_path` cases (run on Linux CI).

3. **`crates/freecell-app/src/shell/mod.rs`** — `mod open_files;` + `pub use
   open_files::file_url_to_path;` and `#[cfg(target_os = "macos")] pub use
   open_files::install_finder_open;`.

4. **`crates/freecell-app/src/main.rs`** — replace the `match open_path { Some→open,
   None→show_welcome }` with: compute `opened_via_argv` (still opens the argv path synchronously);
   then `#[cfg(target_os = "macos")] freecell_app::shell::install_finder_open(cx,
   opened_via_argv);` and `#[cfg(not(target_os = "macos"))] if !opened_via_argv {
   FreeCellApp::show_welcome(cx); }`. Use an inline cfg'd full-path call for the macOS arm so no
   unused import lands on Linux.

## Tests

- `file_url_to_path_decodes_percent_encoded_space` — `file:///Users/x/My%20Book.xlsx` →
  `/Users/x/My Book.xlsx`.
- `file_url_to_path_decodes_unicode` — percent-encoded unicode (データ) → the unicode path.
- `file_url_to_path_rejects_non_file_schemes` — `https://…`, `freecell://…`, `mailto:…` → `None`.
- `file_url_to_path_bare_absolute_path_fallback` — `/Users/x/book.csv` → `PathBuf`.
- `file_url_to_path_rejects_garbage` — `""`, `"not a url"`, `"C:\\x"` → `None`.
- `file_url_to_path_multi_url_vector` — a `Vec<String>` of mixed file/non-file URLs maps to the
  expected filtered `Vec<PathBuf>` (the multi-select-in-one-event shape).

## Render / build validation

No pixel suite (opening a file moves no grid/cell/sheet/titlebar/chart pixels — functional §10,
architecture §6). Crate-scoped `cargo build -p freecell-app` + `cargo test -p freecell-app --lib`
(Linux, non-macOS arm) + whole-workspace `cargo fmt --all --check`. The macOS-cfg code cannot
compile on this Linux host; correctness of that arm rests on the source-verified gpui API contract
and is confirmed live in Phase 4 (non-blocking real-macOS smoke, M-15).
