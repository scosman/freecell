//! OS-delivered file opens (`specs/projects/xlsx-file-association/`, functional_spec.md §5,
//! architecture.md §3.2–§3.3).
//!
//! Windows and Linux deliver the target path via process **argv**, already handled by
//! `main.rs::open_arg` → [`FreeCellApp::open_path`](super::FreeCellApp::open_path). macOS instead
//! delivers a Finder open as an Apple Event (`application:openURLs:`) that gpui surfaces through
//! [`gpui::App::on_open_urls`], whose callback is `FnMut(Vec<String>)` with **no `cx`** — so it
//! cannot open a window from inside itself. [`install_finder_open`] bridges that cx-less callback
//! into the existing open funnel via an `async-channel` + a spawned app task (the Zed-proven
//! pattern), and gates the welcome window behind the channel drain so a Finder cold-start never
//! flashes welcome (functional_spec.md §5.5 REQUIREMENT).
//!
//! [`file_url_to_path`] — the `file://` → [`PathBuf`] conversion — is intentionally cfg-agnostic
//! and pure, so it is unit-tested on the Linux CI (it carries the bulk of this feature's automated
//! coverage; the live Finder-event behavior is the manual macOS smoke, architecture.md §6).

use std::path::PathBuf;

/// Converts one raw open-event URL string into a filesystem path, or `None` if it is not a local
/// file we should open (architecture.md §3.3).
///
/// macOS `openURLs` delivers each entry as an `NSURL.absoluteString`, i.e. a percent-encoded
/// `file://` URL. We accept **only** the `file` scheme (FreeCell registers no custom URL scheme,
/// functional_spec.md §5.3): the `url` crate handles percent-decoding and unicode. Any other
/// scheme (`https`, `mailto`, a hypothetical `freecell://`) yields `None` and is silently ignored.
/// A string that does not parse as a URL at all falls back to being treated as a bare absolute
/// path (defensive — this module is macOS-only, where absolute paths start with `/`).
///
/// This function is cfg-agnostic and side-effect-free so it can be exercised by the Linux unit
/// suite; the macOS-only bridge in this module is what actually feeds it real events.
pub fn file_url_to_path(raw: &str) -> Option<PathBuf> {
    // 1. Try to parse as a URL; accept only the `file` scheme.
    if let Ok(url) = url::Url::parse(raw) {
        if url.scheme() == "file" {
            return url.to_file_path().ok(); // percent-decoding + unicode handled by `url`
        }
        return None; // http/https/mailto/custom scheme → not ours, ignore
    }
    // 2. Defensive fallback: a bare absolute path handed through as-is.
    raw.starts_with('/').then(|| PathBuf::from(raw))
}

/// The startup coalesce window: after registering [`gpui::App::on_open_urls`], the bridge task
/// yields this long before deciding welcome-vs-open, so a launch-time Finder `openURLs` (delivered
/// through the event loop shortly *after* the `app.run` callback registers the handler — not
/// synchronously inside it) is captured first and welcome never flashes (functional_spec.md §5.5,
/// architecture.md §3.2). A small robustness margin, not a guess; the macOS spike (Phase 4) can
/// tune this one value. A fileless cold start merely shows welcome this much later — imperceptible.
#[cfg(target_os = "macos")]
const STARTUP_COALESCE: std::time::Duration = std::time::Duration::from_millis(50);

/// Installs the macOS Finder open-file bridge and owns the deferred welcome-vs-open decision
/// (functional_spec.md §5, architecture.md §3.2). Call **once**, inside `app.run`, right after the
/// argv open is dispatched and before any synchronous welcome — welcome is shown only from inside
/// the spawned task here, so a Finder cold-start that carries a file never flashes welcome.
///
/// `opened_via_argv` reports whether `main.rs` already opened a CLI/argv path; if so (or if a
/// launch-time `openURLs` arrives), welcome is suppressed.
///
/// gpui's `on_open_urls` callback is cx-less, so it only forwards the raw URL strings onto a
/// channel; the spawned task (which *does* have a `cx`) drains them through the existing
/// [`FreeCellApp::open_path`](super::FreeCellApp::open_path) funnel — reusing
/// canonicalize/dedupe/recents/error-handling verbatim — for both the launch drain and warm-start
/// events over the app's lifetime.
#[cfg(target_os = "macos")]
pub fn install_finder_open(cx: &mut gpui::App, opened_via_argv: bool) {
    use super::FreeCellApp;
    use gpui::AsyncApp;

    let (tx, rx) = async_channel::unbounded::<Vec<String>>();
    // The callback has no `cx`: it only forwards the raw URL strings onto the channel. `try_send`
    // on an unbounded channel only fails if the receiver was dropped (app shutting down) — ignore.
    cx.on_open_urls(move |urls| {
        let _ = tx.try_send(urls);
    });

    cx.spawn(async move |cx: &mut AsyncApp| {
        // Let the platform deliver any launch-time `openURLs` (fired through the event loop just
        // after the handler is registered) before we decide welcome-vs-open.
        cx.background_executor().timer(STARTUP_COALESCE).await;

        // Drain launch events. `opened_via_argv` seeds the decision so a CLI open also suppresses
        // welcome, exactly like the non-macOS arm.
        let mut opened_any = opened_via_argv;
        while let Ok(urls) = rx.try_recv() {
            opened_any |= open_all(&urls, cx);
        }
        if !opened_any {
            // Deferred (never synchronous) → no welcome flash on a Finder cold-start.
            cx.update(|cx| FreeCellApp::show_welcome(cx));
        }

        // Warm-start events, for the life of the app (a single owner of `rx` — no split-ownership
        // race with the launch drain above). The loop ends when the sender is dropped at shutdown.
        while let Ok(urls) = rx.recv().await {
            open_all(&urls, cx);
        }
    })
    .detach();
}

/// Parses and opens each URL in one open event, in order (dedupe in `open_path` collapses repeats,
/// functional_spec.md §5.4). Returns whether any entry was a local file we routed to the open
/// funnel — mirroring `opened_via_argv` so a delivered-but-failing open (missing file → error
/// dialog) still suppresses welcome, exactly like the argv path.
#[cfg(target_os = "macos")]
fn open_all(urls: &[String], cx: &gpui::AsyncApp) -> bool {
    use super::FreeCellApp;

    let mut opened = false;
    for raw in urls {
        match file_url_to_path(raw) {
            Some(path) => {
                cx.update(|cx| FreeCellApp::open_path(&path, cx));
                opened = true;
            }
            None => tracing::debug!(url = %raw, "ignoring non-file open URL"),
        }
    }
    opened
}

#[cfg(test)]
mod tests {
    use super::file_url_to_path;
    use std::path::PathBuf;

    #[test]
    fn file_url_to_path_decodes_percent_encoded_space() {
        assert_eq!(
            file_url_to_path("file:///Users/x/My%20Book.xlsx"),
            Some(PathBuf::from("/Users/x/My Book.xlsx")),
        );
    }

    #[test]
    fn file_url_to_path_decodes_unicode() {
        // Percent-encoded UTF-8 for "データ" — the shape macOS `absoluteString` delivers.
        assert_eq!(
            file_url_to_path("file:///Users/x/%E3%83%87%E3%83%BC%E3%82%BF.xlsx"),
            Some(PathBuf::from("/Users/x/データ.xlsx")),
        );
        // A raw (already-decoded) unicode path must round-trip too (defensive).
        assert_eq!(
            file_url_to_path("file:///Users/x/データ.xlsx"),
            Some(PathBuf::from("/Users/x/データ.xlsx")),
        );
    }

    #[test]
    fn file_url_to_path_rejects_non_file_schemes() {
        // FreeCell registers no custom URL scheme — anything but `file` is not ours.
        assert_eq!(file_url_to_path("https://example.com/a.xlsx"), None);
        assert_eq!(file_url_to_path("freecell:///Users/x/a.xlsx"), None);
        assert_eq!(file_url_to_path("mailto:foo@bar.com"), None);
    }

    #[test]
    fn file_url_to_path_bare_absolute_path_fallback() {
        // A non-URL that is a bare absolute path (defensive fallback).
        assert_eq!(
            file_url_to_path("/Users/x/book.csv"),
            Some(PathBuf::from("/Users/x/book.csv")),
        );
    }

    #[test]
    fn file_url_to_path_rejects_garbage() {
        assert_eq!(file_url_to_path(""), None);
        assert_eq!(file_url_to_path("not a url"), None);
        // Windows-style path: this module is macOS-only (Windows uses argv, not this).
        assert_eq!(file_url_to_path("C:\\x"), None);
    }

    #[test]
    fn file_url_to_path_multi_url_vector() {
        // The multi-select-drag-onto-the-Dock shape: one event, several URLs. Non-file entries
        // drop out; the file entries map to paths, in order.
        let event = [
            "file:///Users/x/a.xlsx".to_string(),
            "https://example.com/skip".to_string(),
            "file:///Users/x/b.csv".to_string(),
        ];
        let paths: Vec<PathBuf> = event.iter().filter_map(|u| file_url_to_path(u)).collect();
        assert_eq!(
            paths,
            vec![
                PathBuf::from("/Users/x/a.xlsx"),
                PathBuf::from("/Users/x/b.csv"),
            ],
        );
    }
}
