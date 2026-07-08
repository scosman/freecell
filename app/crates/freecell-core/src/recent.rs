//! The recent-files store + its display formatters (`functional_spec.md §1`,
//! `architecture.md §2`). Pure and GPUI-/IronCalc-free: the model, JSON (de)serialize,
//! record/dedupe/cap/prune, display-row building, and the size/folder/relative-time
//! formatters all live here and are unit-tested headless.
//!
//! **No wall-clock reads.** Every time-dependent function takes `now` as an explicit Unix
//! seconds argument, so buckets and labels are pure functions of their inputs and tests
//! inject fixed instants (`functional_spec.md §1.7`, `architecture.md §2.2`). The only impure
//! inputs are `std::fs` reads (existence/size stats and the JSON load/save) — deliberately
//! kept best-effort here so the app layer never has to touch the store's shape.
//!
//! Persistence is a small JSON cache (`{ "entries": [...] }`). Only `path` + `last_opened`
//! are stored; a row's size and parent-folder label are re-derived from disk at display time
//! (`display_entries`) so they can never go stale (`architecture.md §2`).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Maximum entries retained in the persisted store; older distinct files fall off the end
/// (`functional_spec.md §1.3`).
pub const STORE_CAP: usize = 10;

/// Rows shown on the welcome pane (`functional_spec.md §1.3`).
pub const WELCOME_LIMIT: usize = 5;

/// Items shown in the macOS **Open Recent** menu (`functional_spec.md §1.3`).
pub const MENU_LIMIT: usize = 10;

const SECONDS_PER_MINUTE: i64 = 60;
const SECONDS_PER_HOUR: i64 = 60 * 60;
const SECONDS_PER_DAY: i64 = 24 * 60 * 60;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// Weekday abbreviations indexed with **0 = Sunday** (see [`weekday_name`]).
const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

/// One recorded file. `last_opened` is Unix seconds — "last opened/saved **in FreeCell**",
/// not the file's own mtime (`functional_spec.md §1.2`).
///
/// Only `path` + `last_opened` persist; `path` (a `PathBuf`) serializes as its string form
/// via serde. That is acceptable for a local cache — a non-UTF-8 path cannot serialize, but
/// such paths never round-trip through the `.xlsx` open flow in practice (`architecture.md §2`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RecentEntry {
    pub path: PathBuf,
    pub last_opened: i64,
}

/// The most-recent-first recent-files list. Serialized as `{ "entries": [...] }`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RecentList {
    pub entries: Vec<RecentEntry>,
}

/// A ready-to-render row: everything the UI needs, with no further disk access
/// (`architecture.md §2`, `functional_spec.md §1.6`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayEntry {
    /// Absolute path of the file (the click/menu target).
    pub path: PathBuf,
    /// The file name, e.g. `Q3 Revenue Forecast.xlsx`.
    pub name: String,
    /// `"{size} · {folder}"`, or just `"{size}"` when the path has no parent-folder name.
    pub subtitle: String,
    /// Relative "last opened in FreeCell" label (`functional_spec.md §1.7`).
    pub relative_time: String,
}

impl RecentList {
    /// Parse from JSON bytes. Any error — malformed JSON, a missing/renamed field, a single
    /// bad entry — yields an empty list rather than propagating (`functional_spec.md §1.5`:
    /// corrupt/unreadable ⇒ empty, no crash). Never panics.
    pub fn from_json(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }

    /// Serialize to pretty JSON bytes. Serialization of `PathBuf` + `i64` is infallible for
    /// UTF-8 paths (the only paths that reach the store — see [`RecentEntry`]); the
    /// theoretically-impossible error yields empty bytes rather than panicking, keeping the
    /// `-> Vec<u8>` signature total.
    pub fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    /// Read + parse the store at `path`. A missing, unreadable, or corrupt file is treated as
    /// an empty list — best-effort, never an error to the caller (`functional_spec.md §1.5`).
    pub fn load(path: &Path) -> Self {
        match fs::read(path) {
            Ok(bytes) => Self::from_json(&bytes),
            Err(_) => Self::default(),
        }
    }

    /// Serialize + write the store to `path`, creating parent directories as needed. Returns
    /// the raw `io::Result`; the caller (app layer) logs and swallows a failure so a
    /// read-only disk never blocks an open/save (`functional_spec.md §1.5`,
    /// `architecture.md §6`).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(path, self.to_json())
    }

    /// Record `path` as opened/saved at `now` (Unix seconds). The caller passes an
    /// already-canonical path (`architecture.md §2.1`), so dedupe is by exact path equality:
    ///
    /// 1. remove any existing entry for `path` (dedupe — no second row),
    /// 2. push the fresh entry to the front (most-recent-first; a re-open updates its time),
    /// 3. prune entries whose file no longer exists (keeps the store tidy —
    ///    `functional_spec.md §1.4`),
    /// 4. truncate to [`STORE_CAP`] (older distinct files fall off the end —
    ///    `functional_spec.md §1.3`).
    ///
    /// Pruning runs before truncation so a stale (missing) entry can never keep a live one out
    /// of the capped list.
    pub fn record(&mut self, path: PathBuf, now: i64) {
        self.entries.retain(|entry| entry.path != path);
        self.entries.insert(
            0,
            RecentEntry {
                path,
                last_opened: now,
            },
        );
        self.entries.retain(|entry| entry.path.exists());
        self.entries.truncate(STORE_CAP);
    }

    /// Empty the list (**Clear Recent Files**, `functional_spec.md §3`).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Up to `limit` display rows for **existing** files, most-recent-first. Each row is stat'd
    /// for its size; any entry whose file no longer exists (stat fails) is silently dropped
    /// (`functional_spec.md §1.4`), so the visible count may be fewer than `limit` or than the
    /// stored count. Stats are lazy — evaluation stops once `limit` live rows are collected, so
    /// at most a bounded number of files are touched (`functional_spec.md §5`).
    pub fn display_entries(&self, now: i64, limit: usize) -> Vec<DisplayEntry> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let size = format_size(fs::metadata(&entry.path).ok()?.len());
                Some(DisplayEntry {
                    path: entry.path.clone(),
                    name: file_name_label(&entry.path),
                    subtitle: subtitle(&size, &parent_folder_label(&entry.path)),
                    relative_time: format_relative_time(now, entry.last_opened),
                })
            })
            .take(limit)
            .collect()
    }
}

/// The file name of `path` as a lossy `String`, or `""` if it has none.
fn file_name_label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A display row's subtitle: `"{size} · {folder}"`, or just `"{size}"` when `folder` is empty
/// (a path with no parent-folder name — `functional_spec.md §1.6`). Split out so the
/// folder-absent branch is directly unit-testable (existing files in a sandboxed temp dir
/// always have a named parent, so it can't be reached through `display_entries`).
fn subtitle(size_label: &str, folder: &str) -> String {
    if folder.is_empty() {
        size_label.to_string()
    } else {
        format!("{size_label} · {folder}")
    }
}

/// Human-readable file size using binary (1024) units (`functional_spec.md §1.6`,
/// `architecture.md §2.2`):
///
/// - `< 1 KiB` → whole bytes, e.g. `512 B`,
/// - `< 1 MiB` → whole KB rounded to nearest, e.g. `12 KB`, `480 KB`,
/// - `< 1 GiB` → one decimal, e.g. `1.2 MB`, `12.0 MB`,
/// - otherwise → one decimal GB, e.g. `1.0 GB`.
///
/// A value that *rounds up* to a full 1024 of its unit is promoted to the next unit, so the
/// label never reads `1024 KB` / `1024.0 MB`: e.g. `1 MiB − 1` → `1.0 MB`, `1 GiB − 1` →
/// `1.0 GB`.
pub fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    let bytes_f = bytes as f64;
    if bytes < KB {
        return format!("{bytes} B");
    }
    // Whole KB, rounded to nearest; promote to MB if it reaches a full 1024 KB.
    let kb = (bytes_f / KB as f64).round() as u64;
    if bytes < MB && kb < KB {
        return format!("{kb} KB");
    }
    // One-decimal MB; promote to GB if it rounds up to a full 1024.0 MB (10_240 tenths).
    let mb = bytes_f / MB as f64;
    if bytes < GB && (mb * 10.0).round() < (KB * 10) as f64 {
        return format!("{mb:.1} MB");
    }
    format!("{:.1} GB", bytes_f / GB as f64)
}

/// The immediate parent directory's name (e.g. `.../Downloads/x.xlsx` → `Downloads`), or `""`
/// when the path has no named parent (a filesystem-root child, or a bare file name).
pub fn parent_folder_label(path: &Path) -> String {
    path.parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The relative "last opened in FreeCell" label for `then`, viewed from `now` (both Unix
/// seconds), bucketed per `functional_spec.md §1.7`:
///
/// | Condition | Label |
/// |---|---|
/// | future (clock skew) or `< 1 min` | `Just now` |
/// | `< 1 hour` | `{n}m ago` |
/// | same calendar day | `{n}h ago` |
/// | previous calendar day | `Yesterday` |
/// | 2–6 days ago | weekday name (e.g. `Mon`) |
/// | same calendar year | `{Mon} {D}` (e.g. `Jul 1`) |
/// | earlier year | `{Mon} {D}, {YYYY}` (e.g. `Dec 3, 2024`) |
///
/// Calendar buckets compare civil days (UTC) via [`civil_from_days`]; the earlier time-delta
/// buckets are checked first, so e.g. a 45-minute span that happens to cross midnight is
/// `45m ago`, not `Yesterday` — matching the table's evaluation order.
pub fn format_relative_time(now: i64, then: i64) -> String {
    let delta = now - then;
    // Future (negative delta) and sub-minute both collapse to "Just now".
    if delta < SECONDS_PER_MINUTE {
        return "Just now".to_string();
    }
    if delta < SECONDS_PER_HOUR {
        return format!("{}m ago", delta / SECONDS_PER_MINUTE);
    }

    let now_day = now.div_euclid(SECONDS_PER_DAY);
    let then_day = then.div_euclid(SECONDS_PER_DAY);
    let day_diff = now_day - then_day;

    if day_diff == 0 {
        return format!("{}h ago", delta / SECONDS_PER_HOUR);
    }
    if day_diff == 1 {
        return "Yesterday".to_string();
    }
    if (2..=6).contains(&day_diff) {
        return weekday_name(then_day).to_string();
    }

    let (now_year, _, _) = civil_from_days(now_day);
    let (then_year, then_month, then_day_of_month) = civil_from_days(then_day);
    let month = MONTHS[(then_month - 1) as usize];
    if now_year == then_year {
        format!("{month} {then_day_of_month}")
    } else {
        format!("{month} {then_day_of_month}, {then_year}")
    }
}

/// Civil (Gregorian) `(year, month, day)` for a count of days since the Unix epoch
/// (1970-01-01), via Howard Hinnant's `civil_from_days`
/// (<https://howardhinnant.github.io/date_algorithms.html#civil_from_days>). Dependency-free
/// so the relative-time formatter needs no `chrono`/`time` dependency (`architecture.md §2.2`).
/// Handles negative days (pre-epoch) correctly.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Shift the epoch to 0000-03-01 so leap days sit at the end of each 400-year era.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let month_shifted = (5 * day_of_year + 2) / 153; // [0, 11] (0 = March)
    let day = (day_of_year - (153 * month_shifted + 2) / 5 + 1) as u32; // [1, 31]
    let month = if month_shifted < 10 {
        month_shifted + 3
    } else {
        month_shifted - 9
    }; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month as u32, day)
}

/// Weekday abbreviation for a Unix-epoch day count. Index base is **0 = Sunday**: day 0
/// (1970-01-01) is a Thursday, so `(0 + 4) % 7 = 4 = Thu`. `rem_euclid` keeps the index in
/// `[0, 6]` for negative (pre-epoch) days too.
fn weekday_name(days: i64) -> &'static str {
    let index = (days.rem_euclid(7) + 4) % 7;
    WEEKDAYS[index as usize]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    /// Create a real file `name` (with `bytes` of content) inside `dir` and return its path.
    fn touch(dir: &Path, name: &str, bytes: usize) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, vec![0u8; bytes]).expect("write temp file");
        path
    }

    /// Seconds at `hour:00:00` UTC on Unix-epoch `day` — a readable way to build injected
    /// instants for the relative-time tests.
    fn at(day: i64, hour: i64) -> i64 {
        day * SECONDS_PER_DAY + hour * SECONDS_PER_HOUR
    }

    #[test]
    fn record_front_inserts() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "a.xlsx", 1);
        let b = touch(dir.path(), "b.xlsx", 1);
        let mut list = RecentList::default();
        list.record(a.clone(), 100);
        list.record(b.clone(), 200);
        // Most-recent-first: the last recorded is at the front.
        assert_eq!(
            list.entries.iter().map(|e| &e.path).collect::<Vec<_>>(),
            vec![&b, &a]
        );
    }

    #[test]
    fn record_dedupes_and_moves_to_front_updating_time() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "a.xlsx", 1);
        let b = touch(dir.path(), "b.xlsx", 1);
        let mut list = RecentList::default();
        list.record(a.clone(), 100);
        list.record(b.clone(), 200);
        list.record(a.clone(), 300);
        // One row for `a` (deduped), now at the front, with its timestamp refreshed to 300.
        assert_eq!(list.entries.len(), 2);
        assert_eq!(list.entries[0].path, a);
        assert_eq!(list.entries[0].last_opened, 300);
        assert_eq!(list.entries[1].path, b);
    }

    #[test]
    fn record_caps_at_store_cap() {
        let dir = TempDir::new().unwrap();
        let paths: Vec<PathBuf> = (0..=STORE_CAP)
            .map(|i| touch(dir.path(), &format!("f{i}.xlsx"), 1))
            .collect();
        let mut list = RecentList::default();
        for (i, p) in paths.iter().enumerate() {
            list.record(p.clone(), 100 + i as i64);
        }
        // STORE_CAP + 1 distinct files recorded ⇒ exactly STORE_CAP retained.
        assert_eq!(list.entries.len(), STORE_CAP);
        // Newest at the front, oldest (f0) dropped off the end.
        assert_eq!(list.entries[0].path, *paths.last().unwrap());
        assert!(!list.entries.iter().any(|e| e.path == paths[0]));
    }

    #[test]
    fn record_prunes_missing_files() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "a.xlsx", 1);
        let b = touch(dir.path(), "b.xlsx", 1);
        let c = touch(dir.path(), "c.xlsx", 1);
        let mut list = RecentList::default();
        list.record(a.clone(), 100);
        list.record(b.clone(), 200);
        list.record(c.clone(), 300);
        // b's file vanishes; the next record prunes it from the store.
        fs::remove_file(&b).unwrap();
        let d = touch(dir.path(), "d.xlsx", 1);
        list.record(d.clone(), 400);
        assert_eq!(
            list.entries.iter().map(|e| &e.path).collect::<Vec<_>>(),
            vec![&d, &c, &a]
        );
    }

    #[test]
    fn clear_empties() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "a.xlsx", 1);
        let mut list = RecentList::default();
        list.record(a, 100);
        list.clear();
        assert!(list.entries.is_empty());
    }

    #[test]
    fn json_round_trips() {
        let list = RecentList {
            entries: vec![
                RecentEntry {
                    path: PathBuf::from("/tmp/one.xlsx"),
                    last_opened: 1_700_000_000,
                },
                RecentEntry {
                    path: PathBuf::from("/tmp/two.xlsx"),
                    last_opened: 1_700_000_500,
                },
            ],
        };
        let parsed = RecentList::from_json(&list.to_json());
        assert_eq!(parsed.entries, list.entries);
    }

    #[test]
    fn from_json_on_garbage_is_empty() {
        // Malformed, empty, and structurally-incomplete inputs all degrade to empty, no panic.
        assert!(RecentList::from_json(b"not json at all").entries.is_empty());
        assert!(RecentList::from_json(b"").entries.is_empty());
        assert!(RecentList::from_json(b"{}").entries.is_empty());
        assert!(RecentList::from_json(b"{\"entries\": 3}")
            .entries
            .is_empty());
    }

    #[test]
    fn load_absent_is_empty() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("does-not-exist.json");
        assert!(RecentList::load(&missing).entries.is_empty());
    }

    #[test]
    fn save_then_load_round_trips() {
        let dir = TempDir::new().unwrap();
        // A nested path exercises the parent-dir creation in `save`.
        let store = dir.path().join("FreeCell").join("recents.json");
        let list = RecentList {
            entries: vec![RecentEntry {
                path: PathBuf::from("/tmp/saved.xlsx"),
                last_opened: 42,
            }],
        };
        list.save(&store).expect("save should succeed");
        let loaded = RecentList::load(&store);
        assert_eq!(loaded.entries, list.entries);
    }

    #[test]
    fn display_entries_builds_rows() {
        let dir = TempDir::new().unwrap();
        let folder = dir.path().join("Downloads");
        fs::create_dir_all(&folder).unwrap();
        let path = touch(&folder, "Report.xlsx", 12 * 1024); // 12 KB
        let mut list = RecentList::default();
        let now = at(20_642, 12); // 2026-07-08 12:00 UTC
        list.record(path.clone(), now); // opened "now"
        let rows = list.display_entries(now, WELCOME_LIMIT);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, path);
        assert_eq!(rows[0].name, "Report.xlsx");
        assert_eq!(rows[0].subtitle, "12 KB · Downloads");
        assert_eq!(rows[0].relative_time, "Just now");
    }

    #[test]
    fn display_entries_drops_missing() {
        let dir = TempDir::new().unwrap();
        let a = touch(dir.path(), "a.xlsx", 1);
        let b = touch(dir.path(), "b.xlsx", 1);
        let mut list = RecentList::default();
        list.record(a.clone(), 100);
        list.record(b.clone(), 200);
        // Delete b *without* re-recording; display_entries must filter it out at render time.
        fs::remove_file(&b).unwrap();
        let rows = list.display_entries(1_000, WELCOME_LIMIT);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path, a);
    }

    #[test]
    fn display_entries_honors_limit() {
        let dir = TempDir::new().unwrap();
        let mut list = RecentList::default();
        for i in 0..3 {
            let p = touch(dir.path(), &format!("f{i}.xlsx"), 1);
            list.record(p, 100 + i as i64);
        }
        assert_eq!(list.display_entries(1_000, 2).len(), 2);
    }

    #[test]
    fn format_size_boundaries() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(512), "512 B");
        assert_eq!(format_size(1023), "1023 B");
        assert_eq!(format_size(1024), "1 KB");
        assert_eq!(format_size(12 * 1024), "12 KB");
        assert_eq!(format_size(480 * 1024), "480 KB");
        // 1.5 KB rounds to nearest whole KB.
        assert_eq!(format_size(1536), "2 KB");
        assert_eq!(format_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_size(12 * 1024 * 1024), "12.0 MB");
        assert_eq!(format_size(3 * 1024 * 1024 / 2), "1.5 MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_size(5 * 1024 * 1024 * 1024), "5.0 GB");
    }

    #[test]
    fn format_size_rolls_over_at_unit_boundaries() {
        // One byte below a unit boundary rounds up to a full 1024 of the lower unit, which is
        // promoted to the next unit rather than rendered as "1024 KB" / "1024.0 MB".
        assert_eq!(format_size(1024 * 1024 - 1), "1.0 MB"); // 1 MiB − 1
        assert_eq!(format_size(1024 * 1024 * 1024 - 1), "1.0 GB"); // 1 GiB − 1
    }

    #[test]
    fn subtitle_omits_folder_when_absent() {
        // With a folder name the subtitle joins size + folder; without one it is just the size
        // (the folder-empty branch of `display_entries`, unreachable via a sandboxed temp dir).
        assert_eq!(subtitle("12 KB", "Downloads"), "12 KB · Downloads");
        assert_eq!(subtitle("12 KB", ""), "12 KB");
    }

    #[test]
    fn parent_folder_label_cases() {
        assert_eq!(
            parent_folder_label(Path::new("/Users/me/Downloads/x.xlsx")),
            "Downloads"
        );
        assert_eq!(parent_folder_label(Path::new("/a/b/c/file")), "c");
        assert_eq!(parent_folder_label(Path::new("/x.xlsx")), ""); // parent is the root "/"
        assert_eq!(parent_folder_label(Path::new("bare.xlsx")), ""); // parent is "" (no name)
    }

    #[test]
    fn civil_from_days_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1)); // epoch (a Thursday)
        assert_eq!(civil_from_days(-1), (1969, 12, 31)); // one day pre-epoch
        assert_eq!(civil_from_days(19_723), (2024, 1, 1)); // leap-year start
        assert_eq!(civil_from_days(20_060), (2024, 12, 3)); // mid leap year
        assert_eq!(civil_from_days(20_642), (2026, 7, 8)); // the tests' "today" anchor
    }

    #[test]
    fn weekday_name_known_days() {
        assert_eq!(weekday_name(0), "Thu"); // 1970-01-01
        assert_eq!(weekday_name(20_642), "Wed"); // 2026-07-08
        assert_eq!(weekday_name(20_635), "Wed"); // 2026-07-01
        assert_eq!(weekday_name(20_639), "Sun"); // 2026-07-05
    }

    #[test]
    fn format_relative_time_buckets() {
        // Anchor "now" at 2026-07-08 12:00 UTC (day 20_642).
        let now = at(20_642, 12);

        // Future (clock skew) and sub-minute both clamp to "Just now".
        assert_eq!(format_relative_time(now, now + 500), "Just now");
        assert_eq!(format_relative_time(now, now), "Just now");
        assert_eq!(format_relative_time(now, now - 30), "Just now");

        // Minutes, then hours (same calendar day).
        assert_eq!(
            format_relative_time(now, now - 5 * SECONDS_PER_MINUTE),
            "5m ago"
        );
        assert_eq!(
            format_relative_time(now, now - 59 * SECONDS_PER_MINUTE),
            "59m ago"
        );
        assert_eq!(
            format_relative_time(now, now - 2 * SECONDS_PER_HOUR),
            "2h ago"
        );
        assert_eq!(format_relative_time(now, at(20_642, 1)), "11h ago"); // 01:00 same day

        // Previous calendar day ⇒ "Yesterday" (even a 13h span across midnight).
        assert_eq!(format_relative_time(now, at(20_641, 23)), "Yesterday");
        assert_eq!(format_relative_time(now, at(20_641, 12)), "Yesterday");

        // A sub-hour span that crosses midnight is still minutes, not "Yesterday" (order):
        // 00:10 today vs 23:55 yesterday is 15 minutes apart across the day boundary.
        let just_after_midnight = at(20_642, 0) + 10 * SECONDS_PER_MINUTE;
        assert_eq!(
            format_relative_time(
                just_after_midnight,
                at(20_641, 23) + 55 * SECONDS_PER_MINUTE
            ),
            "15m ago"
        );

        // 2–6 days ago ⇒ weekday name of `then`.
        assert_eq!(format_relative_time(now, at(20_640, 12)), "Mon"); // 2 days (2026-07-06)
        assert_eq!(format_relative_time(now, at(20_639, 12)), "Sun"); // 3 days (2026-07-05)
        assert_eq!(format_relative_time(now, at(20_636, 12)), "Thu"); // 6 days (2026-07-02)

        // 7+ days, same calendar year ⇒ "{Mon} {D}".
        assert_eq!(format_relative_time(now, at(20_635, 12)), "Jul 1"); // 2026-07-01

        // Earlier year ⇒ "{Mon} {D}, {YYYY}".
        assert_eq!(format_relative_time(now, at(20_060, 12)), "Dec 3, 2024");
    }
}
