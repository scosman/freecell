//! The recent-files store's *app-layer* seams (`architecture.md §2.3, §3.1`): where the store
//! persists on disk, and the single wall-clock read the feature needs.
//!
//! All the recent-list *logic* lives in the GPUI-free [`freecell_core::recent`]
//! (`RecentList`, dedupe/cap/prune, display-row building, formatters). This module is the thin
//! app-only glue that `freecell-core` can't own: resolving the per-user data directory (via the
//! `dirs` crate) and turning the system clock into Unix seconds. Keeping the clock read here —
//! never in `freecell-core` — is what lets the core stay a pure function of injected `now`
//! values (`architecture.md §3.1`).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// The on-disk recent-files store: `<data_dir>/FreeCell/recents.json`, or `None` when no
/// per-user data directory resolves (a headless environment with no `HOME`). With `None` the
/// list is kept in memory only — never an error, never a dialog (`functional_spec.md §1.5`).
///
/// - macOS: `~/Library/Application Support/FreeCell/recents.json`
/// - Linux: `${XDG_DATA_HOME:-~/.local/share}/FreeCell/recents.json`
pub fn recents_store_path() -> Option<PathBuf> {
    dirs::data_dir().map(|dir| dir.join("FreeCell").join("recents.json"))
}

/// The current time as Unix seconds — the **only** wall-clock read in the recents feature
/// (`architecture.md §3.1`: it must never live in `freecell-core`, so every time-dependent core
/// function takes `now` as an injected argument). A clock before the Unix epoch (which
/// `duration_since` reports as an error) degrades to `0`; the relative-time formatter then reads
/// such an entry as far in the past, which is harmless for a "recently opened" cache.
pub(crate) fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}
