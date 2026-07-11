//! Integration tests for the Phase-4 eval worker seam (`components/engine_worker.md §Test
//! plan`). These drive the **public** `DocumentClient` + `WorkerEvent` surface exactly as the
//! window would — spawn a real worker on its 64 MiB thread, send `Command`s, await
//! `WorkerEvent`s, and read the shared publication/generation. No IronCalc type is reachable
//! here; that is the point of the seam.
//!
//! Determinism: every wait is bounded by a timeout so a stuck worker fails the test instead
//! of hanging CI. Publication reads are wait-free `arc_swap` loads.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use freecell_chart_model::{Anchor, AnchorCell, ChartId, SeriesData};
use freecell_core::{CellRange, CellRef, SheetId};
use freecell_engine::{
    fixtures, ChartAxisKind, ChartChromeEdit, ChartInsertKind, ChartSnapshot, Command,
    DataLabelToggles, DocumentClient, DocumentSource, EditRejectedReason, SheetMeta, StyleAttr,
    WorkerEvent, WorkerEventReceiver,
};
use tempfile::tempdir;

const TIMEOUT: Duration = Duration::from_secs(10);

/// Wait for the first event matching `pred` (or `None` on timeout / channel close).
fn wait_for(
    rx: &WorkerEventReceiver,
    mut pred: impl FnMut(&WorkerEvent) -> bool,
) -> Option<WorkerEvent> {
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match rx.recv_timeout(remaining) {
            Some(ev) if pred(&ev) => return Some(ev),
            Some(_) => continue,
            None => return None,
        }
    }
}

/// Poll `cond` until it returns true, or panic with `msg` at the [`TIMEOUT`] deadline.
///
/// Robust to worker-thread event ordering: two live back-to-back `client.send()` edits may
/// coalesce into ONE publish or land in TWO (the worker loop is `recv()` + `try_iter()` with
/// no batching window, so coalescing of live sends is opportunistic on scheduling). A single
/// `wait_for(Published)` + immediate `arc_swap` read races the second publish's store; polling
/// the converged read model instead is deterministic without weakening any assertion.
fn poll_until(mut cond: impl FnMut() -> bool, msg: &str) {
    let deadline = std::time::Instant::now() + TIMEOUT;
    while !cond() {
        assert!(std::time::Instant::now() < deadline, "{msg}");
        thread::sleep(Duration::from_millis(2));
    }
}

/// Spawn over a source and return the client, the receiver, and the loaded sheet list.
fn spawn(source: DocumentSource) -> (DocumentClient, WorkerEventReceiver, Vec<SheetMeta>) {
    let (client, rx) = DocumentClient::spawn(source);
    let loaded = wait_for(&rx, |e| matches!(e, WorkerEvent::Loaded { .. }))
        .expect("worker should emit Loaded");
    let sheets = match loaded {
        WorkerEvent::Loaded { sheets } => sheets,
        _ => unreachable!(),
    };
    (client, rx, sheets)
}

/// Spawn a fresh empty workbook; return the client, receiver, and the first sheet's stable id.
fn spawn_new() -> (DocumentClient, WorkerEventReceiver, SheetId) {
    let (client, rx, sheets) = spawn(DocumentSource::NewWorkbook);
    let sheet = sheets[0].id;
    (client, rx, sheet)
}

/// The display text of a cell in the latest publication (empty if the cell isn't published).
fn published_text(client: &DocumentClient, row: u32, col: u32) -> String {
    client
        .publication()
        .cells
        .iter()
        .find(|c| c.row == row && c.col == col)
        .map(|c| c.display_text.clone())
        .unwrap_or_default()
}

fn set_input(sheet: SheetId, row: u32, col: u32, input: &str) -> Command {
    Command::SetCellInput {
        sheet,
        cell: CellRef::new(row, col),
        input: input.to_string(),
    }
}

fn full_viewport(sheet: SheetId) -> Command {
    Command::SetViewport {
        sheet,
        rows: 0..64,
        cols: 0..16,
    }
}

#[test]
fn spawn_new_workbook_emits_loaded() {
    let (_client, _rx, sheets) = spawn(DocumentSource::NewWorkbook);
    assert_eq!(sheets.len(), 1);
    assert_eq!(sheets[0].name, "Sheet1");
}

#[test]
fn spawn_open_bad_files_emit_typed_load_failed() {
    let dir = tempdir().unwrap();
    let cases: &[(&str, &[u8])] = &[
        ("empty.xlsx", b""),
        ("text.xlsx", b"just some text, not a spreadsheet"),
        ("broken.xlsx", b"PK\x03\x04\x00\x00garbage-not-a-real-zip"),
        (
            "locked.xlsx",
            &[0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1, 0, 0],
        ),
    ];
    for (name, bytes) in cases {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let (_client, rx) = DocumentClient::spawn(DocumentSource::OpenFile(path));
        let ev = wait_for(&rx, |e| {
            matches!(
                e,
                WorkerEvent::LoadFailed { .. } | WorkerEvent::Loaded { .. }
            )
        });
        assert!(
            matches!(ev, Some(WorkerEvent::LoadFailed { .. })),
            "{name} should emit LoadFailed, got {ev:?}"
        );
    }
}

#[test]
fn set_viewport_then_edit_publishes_values() {
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());

    client.send(set_input(sheet, 0, 0, "42"));
    client.send(set_input(sheet, 2, 1, "=40+2"));
    // The two live edits may coalesce into one publish or land in two; poll the published
    // snapshot until BOTH values are present rather than racing a single wait_for(Published)
    // (which can catch the first publish and read before the second's arc_swap store).
    poll_until(
        || published_text(&client, 0, 0) == "42" && published_text(&client, 2, 1) == "42",
        "both edits should reach the published viewport",
    );

    assert_eq!(published_text(&client, 0, 0), "42");
    assert_eq!(published_text(&client, 2, 1), "42"); // =40+2 evaluated
    assert!(client.generation() >= 1);
}

#[test]
fn eval_started_and_finished_bracket_an_edit() {
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "1"));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::EvalStarted)).is_some());
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::EvalFinished)).is_some());
}

#[test]
fn sheet_switch_publishes_new_sheet() {
    // Two sheets with different A1 values; switching the viewport republishes the new sheet.
    let dir = tempdir().unwrap();
    let path = dir.path().join("multi.xlsx");
    fixtures::multi_sheet().save(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    assert_eq!(sheets.len(), 3);

    client.send(Command::SetViewport {
        sheet: sheets[0].id,
        rows: 0..2,
        cols: 0..2,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(published_text(&client, 0, 0), "10"); // Sheet1!A1
    assert_eq!(client.publication().sheet, sheets[0].id);

    client.send(Command::SetViewport {
        sheet: sheets[1].id,
        rows: 0..2,
        cols: 0..2,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(published_text(&client, 0, 0), "20"); // Sheet2!A1 = Sheet1!A1 * 2
    assert_eq!(client.publication().sheet, sheets[1].id);
}

#[test]
fn formula_errors_are_published_as_values() {
    // #DIV/0! live, and #CIRC! from a saved circular-ring fixture.
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "=1/0"));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(published_text(&client, 0, 0), "#DIV/0!");

    let dir = tempdir().unwrap();
    let path = dir.path().join("circ.xlsx");
    fixtures::circular_ref(50).save(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    client.send(Command::SetViewport {
        sheet: sheets[0].id,
        rows: 0..3,
        cols: 0..1,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(published_text(&client, 0, 0), "#CIRC!");
}

#[test]
fn get_cell_content_replies_with_raw_formula() {
    let (client, rx, sheet) = spawn_new();
    client.send(set_input(sheet, 0, 0, "=SUM(1,2,3)"));
    client.send(Command::GetCellContent {
        sheet,
        cell: CellRef::new(0, 0),
        req_id: 77,
    });
    let ev = wait_for(&rx, |e| {
        matches!(e, WorkerEvent::CellContent { req_id: 77, .. })
    });
    match ev {
        Some(WorkerEvent::CellContent { raw, .. }) => assert_eq!(raw, "=SUM(1,2,3)"),
        other => panic!("expected CellContent, got {other:?}"),
    }
}

#[test]
fn save_through_worker_roundtrips() {
    let (client, rx, sheet) = spawn_new();
    client.send(set_input(sheet, 0, 0, "hello"));
    let dir = tempdir().unwrap();
    let path = dir.path().join("saved.xlsx");
    client.send(Command::Save {
        path: path.clone(),
        req_id: 1,
    });
    let ev = wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 1, .. }));
    assert!(
        matches!(ev, Some(WorkerEvent::Saved { ops_seen: 1, .. })),
        "got {ev:?}"
    );
    assert!(path.exists());

    // Reopen through a second worker and confirm the value survived.
    let (client2, rx2, sheets2) = spawn(DocumentSource::OpenFile(path));
    client2.send(Command::SetViewport {
        sheet: sheets2[0].id,
        rows: 0..1,
        cols: 0..1,
    });
    assert!(wait_for(&rx2, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(published_text(&client2, 0, 0), "hello");
}

#[test]
fn save_atomic_on_failure_leaves_destination_untouched() {
    let (client, rx, sheet) = spawn_new();
    client.send(set_input(sheet, 0, 0, "x"));

    // Root-proof failure: the destination path is an existing non-empty directory, so the
    // atomic rename (temp file → dir) fails and the directory is left byte-identical.
    let dir = tempdir().unwrap();
    let target = dir.path().join("book.xlsx");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"original").unwrap();

    client.send(Command::Save {
        path: target.clone(),
        req_id: 5,
    });
    let ev = wait_for(&rx, |e| {
        matches!(e, WorkerEvent::SaveFailed { req_id: 5, .. })
    });
    assert!(
        matches!(ev, Some(WorkerEvent::SaveFailed { .. })),
        "got {ev:?}"
    );

    assert!(target.is_dir());
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"original");
    // No temp-file litter beside the target.
    let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

#[test]
fn worker_side_cap_rejects_and_never_evaluates() {
    let (client, rx, sheet) = spawn_new();
    let over_len = format!("={}", "1".repeat(9000)); // > 8192 length cap
    client.send(set_input(sheet, 0, 0, &over_len));
    let ev = wait_for(&rx, |e| matches!(e, WorkerEvent::EditRejected { .. }));
    assert!(
        matches!(
            ev,
            Some(WorkerEvent::EditRejected {
                reason: EditRejectedReason::InputCap(_)
            })
        ),
        "got {ev:?}"
    );
    // The cap short-circuits before any eval, so generation never advanced.
    assert_eq!(client.generation(), 0);
}

#[test]
fn undo_redo_through_worker() {
    let (client, _rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "1"));
    client.send(set_input(sheet, 0, 0, "2"));
    // The two edits may coalesce or split; poll until the publication reflects the final "2"
    // (which implies both undoable ops applied — apply increments committed_ops before the
    // publish), rather than racing a single wait_for(Published).
    poll_until(
        || published_text(&client, 0, 0) == "2",
        "both edits should reach the published viewport",
    );
    let ops_after_edits = client.committed_ops();
    assert!(ops_after_edits >= 2);

    client.send(Command::Undo);
    poll_until(
        || published_text(&client, 0, 0) == "1",
        "undo should reflect the prior value in the publication",
    );
    assert_eq!(
        published_text(&client, 0, 0),
        "1",
        "undo reverts to the prior value"
    );
    // Undo itself is a committed op (dirty stays set — architecture §2).
    assert_eq!(client.committed_ops(), ops_after_edits + 1);

    client.send(Command::Redo);
    poll_until(
        || published_text(&client, 0, 0) == "2",
        "redo should re-apply the value in the publication",
    );
    assert_eq!(published_text(&client, 0, 0), "2", "redo re-applies");
}

#[test]
fn sheet_add_rename_delete_emit_sheets_changed() {
    let (client, rx, sheet) = spawn_new();

    client.send(Command::AddSheet);
    let ev = wait_for(&rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. }));
    let sheets = match ev {
        Some(WorkerEvent::SheetsChanged { sheets }) => sheets,
        other => panic!("expected SheetsChanged, got {other:?}"),
    };
    assert_eq!(sheets.len(), 2, "a sheet was added");

    // Rename the original sheet.
    client.send(Command::RenameSheet {
        sheet,
        name: "Renamed".to_string(),
    });
    let ev = wait_for(&rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. }));
    let sheets = match ev {
        Some(WorkerEvent::SheetsChanged { sheets }) => sheets,
        other => panic!("expected SheetsChanged, got {other:?}"),
    };
    assert!(sheets.iter().any(|s| s.name == "Renamed"));

    // Delete the added second sheet.
    let second = sheets.iter().find(|s| s.id != sheet).unwrap().id;
    client.send(Command::DeleteSheet { sheet: second });
    let ev = wait_for(&rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. }));
    let sheets = match ev {
        Some(WorkerEvent::SheetsChanged { sheets }) => sheets,
        other => panic!("expected SheetsChanged, got {other:?}"),
    };
    assert_eq!(sheets.len(), 1, "back to one sheet after delete");
}

#[test]
fn invalid_sheet_rename_is_rejected() {
    let (client, rx, sheet) = spawn_new();
    client.send(Command::RenameSheet {
        sheet,
        name: "bad/name".to_string(), // illegal '/'
    });
    let ev = wait_for(&rx, |e| matches!(e, WorkerEvent::EditRejected { .. }));
    assert!(
        matches!(
            ev,
            Some(WorkerEvent::EditRejected {
                reason: EditRejectedReason::InvalidSheetName(_)
            })
        ),
        "got {ev:?}"
    );
}

#[test]
fn style_edit_publishes_without_changing_values() {
    let (client, _rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "5"));
    poll_until(
        || published_text(&client, 0, 0) == "5",
        "the seed value should publish",
    );

    client.send(Command::SetStyleAttr {
        sheet,
        range: CellRange::single(CellRef::new(0, 0)),
        attr: StyleAttr::Bold,
    });
    // The style edit commits a second op but leaves the value unchanged, so there is no new
    // display text to poll on. Poll committed_ops instead (apply increments it before the
    // publish), rather than racing a single wait_for(Published) that can catch the seed's
    // publish and read committed_ops before the style edit's op lands.
    poll_until(
        || client.committed_ops() >= 2,
        "the style edit should commit a second op",
    );
    // The value is unchanged by the style edit.
    assert_eq!(published_text(&client, 0, 0), "5");
}

#[test]
fn publish_before_bump_never_shows_a_stale_generation() {
    // A concurrent reader spins on (generation, publication); the published generation must
    // never lag the counter (publish-then-bump ordering). If the store/bump order were
    // reversed, the reader would catch a bump with stale data behind it.
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    let client = Arc::new(client);

    let stop = Arc::new(AtomicBool::new(false));
    let violations = Arc::new(AtomicU64::new(0));
    let samples = Arc::new(AtomicU64::new(0));

    let reader = {
        let client = Arc::clone(&client);
        let stop = Arc::clone(&stop);
        let violations = Arc::clone(&violations);
        let samples = Arc::clone(&samples);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let gen = client.generation();
                let pubn = client.publication();
                if pubn.generation < gen {
                    violations.fetch_add(1, Ordering::Relaxed);
                }
                samples.fetch_add(1, Ordering::Relaxed);
            }
        })
    };

    for i in 0..200u32 {
        client.send(set_input(sheet, 0, 0, &format!("{i}")));
    }
    // Let the worker settle.
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    thread::sleep(Duration::from_millis(50));
    stop.store(true, Ordering::Relaxed);
    reader.join().unwrap();

    assert!(samples.load(Ordering::Relaxed) > 0, "the reader sampled");
    assert_eq!(
        violations.load(Ordering::Relaxed),
        0,
        "a generation bump must always have its publication behind it"
    );
    // The reader thread's Arc clone was released at join; dropping this last one closes the
    // command channel so the worker exits cleanly.
    drop(client);
}

#[test]
fn edit_reflected_after_publish_and_reads_are_wait_free() {
    // Two properties the seam guarantees (SP1):
    //  1. an edit is reflected by the very next publish (the staleness bound = one publish);
    //  2. `publication()` never blocks on the worker — it is a wait-free arc_swap load, so it
    //     returns immediately even while a recompute is in flight.
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());

    // Fire an edit and immediately hammer the read path *before* its publish arrives. Each read
    // must return promptly (a stale-but-consistent snapshot) rather than wait for the worker.
    client.send(set_input(sheet, 4, 4, "=6*7"));
    let before = std::time::Instant::now();
    for _ in 0..10_000 {
        let snap = client.publication();
        // Every observed snapshot's generation matches its own data (never a torn read).
        let _ = snap.generation;
    }
    assert!(
        before.elapsed() < TIMEOUT,
        "publication() reads must not block on the worker"
    );

    // After the edit's publish, the fresh value is visible.
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(published_text(&client, 4, 4), "42");
}

/// After load, the active sheet's style/geometry cache is resident on the **public** surface
/// (`caches()`), so the grid renders styled + sized from the very first frame (Phase 5).
#[test]
fn load_populates_public_style_cache() {
    let (client, _rx, sheet) = spawn_new();
    let caches = client.caches();
    let guard = caches.read();
    let cache = guard
        .get(sheet)
        .expect("the active sheet's cache is resident after Loaded");
    // A new empty sheet: full Excel-max axes, FreeCell default geometry, no styles.
    assert_eq!(
        cache.dims(),
        (
            freecell_core::limits::MAX_ROWS,
            freecell_core::limits::MAX_COLS
        )
    );
    assert!((cache.col_width(0) - 100.0).abs() < 1e-3);
    assert!((cache.row_height(0) - 24.0).abs() < 1e-3);
    assert_eq!(cache.render_style(0, 0), None); // plain
}

/// A style edit mirrors into the **public** resident cache and ships a `StyleCacheUpdated`
/// delta — the seam the grid reacts to (read the cache, repaint) without any engine call.
#[test]
fn style_edit_updates_public_cache_and_emits_delta() {
    let (client, rx, sheet) = spawn_new();
    // The load itself ships a StyleCacheUpdated (the active sheet built on open).
    assert!(
        wait_for(&rx, |e| matches!(e, WorkerEvent::StyleCacheUpdated { .. })).is_some(),
        "load builds + announces the active sheet's cache"
    );

    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 1, 1, "x"));
    client.send(Command::SetStyleAttr {
        sheet,
        range: CellRange::single(CellRef::new(1, 1)),
        attr: StyleAttr::Bold,
    });

    // Poll the public cache until the bold edit lands (robust to worker-thread event ordering).
    let caches = client.caches();
    let deadline = std::time::Instant::now() + TIMEOUT;
    loop {
        let is_bold = caches
            .read()
            .get(sheet)
            .and_then(|c| c.render_style(1, 1).map(|s| s.bold))
            == Some(true);
        if is_bold {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the style edit never reached the public cache"
        );
        thread::sleep(Duration::from_millis(2));
    }

    // And a StyleCacheUpdated delta arrives for the edit.
    assert!(
        wait_for(
            &rx,
            |e| matches!(e, WorkerEvent::StyleCacheUpdated { sheet: s } if *s == sheet)
        )
        .is_some(),
        "the style edit ships a StyleCacheUpdated delta"
    );
}

// ---- Range clipboard (`components/clipboard.md`, `functional_spec.md §2`) -----------------
//
// These drive the FULL public seam — spawn a real worker, send clipboard `Command`s, await the
// `CopyReady` / `Pasted` / `PasteRejected` replies, and read the published values.

/// Send a `CopySelection` and wait for its `CopyReady` reply, returning the TSV.
fn copy_and_wait(
    client: &DocumentClient,
    rx: &WorkerEventReceiver,
    sheet: SheetId,
    range: CellRange,
    cut: bool,
) -> String {
    client.send(Command::CopySelection { sheet, range, cut });
    match wait_for(rx, |e| matches!(e, WorkerEvent::CopyReady { .. })) {
        Some(WorkerEvent::CopyReady { tsv }) => tsv,
        other => panic!("expected CopyReady, got {other:?}"),
    }
}

#[test]
fn copy_paste_through_worker_roundtrips_values() {
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "10"));
    client.send(set_input(sheet, 1, 0, "20"));
    poll_until(
        || published_text(&client, 0, 0) == "10" && published_text(&client, 1, 0) == "20",
        "the seed values should publish",
    );

    let tsv = copy_and_wait(
        &client,
        &rx,
        sheet,
        CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
        false,
    );
    assert_eq!(tsv, "10\n20", "copy reply carries the column's TSV");

    // Paste the A1:A2 payload at C1.
    client.send(Command::PasteInternal {
        sheet,
        target: CellRange::single(CellRef::new(0, 2)),
    });
    let pasted = wait_for(&rx, |e| matches!(e, WorkerEvent::Pasted { .. }));
    assert!(
        matches!(
            pasted,
            Some(WorkerEvent::Pasted { sheet: s, range })
                if s == sheet && range == CellRange::new(CellRef::new(0, 2), CellRef::new(1, 2))
        ),
        "paste replies with the pasted rectangle; got {pasted:?}"
    );
    poll_until(
        || published_text(&client, 0, 2) == "10" && published_text(&client, 1, 2) == "20",
        "the pasted values should publish",
    );
}

#[test]
fn paste_tsv_through_worker_writes_typed_cells() {
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());

    client.send(Command::PasteTsv {
        sheet,
        anchor: CellRef::new(0, 0),
        text: "1\t2\n=1+2\ttrue\n".to_string(),
    });
    assert!(
        wait_for(&rx, |e| matches!(e, WorkerEvent::Pasted { .. })).is_some(),
        "a TSV paste replies Pasted"
    );
    poll_until(
        || {
            published_text(&client, 0, 0) == "1"
                && published_text(&client, 0, 1) == "2"
                && published_text(&client, 1, 0) == "3"
                && published_text(&client, 1, 1) == "TRUE"
        },
        "the TSV cells should publish with evaluated types",
    );
}

#[test]
fn paste_undo_is_a_single_step_through_worker() {
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "5"));
    poll_until(|| published_text(&client, 0, 0) == "5", "seed publishes");

    copy_and_wait(
        &client,
        &rx,
        sheet,
        CellRange::single(CellRef::new(0, 0)),
        false,
    );
    client.send(Command::PasteInternal {
        sheet,
        target: CellRange::single(CellRef::new(0, 2)),
    });
    poll_until(|| published_text(&client, 0, 2) == "5", "paste publishes");

    // One undo reverts the whole paste; the source is untouched.
    client.send(Command::Undo);
    poll_until(
        || published_text(&client, 0, 2).is_empty(),
        "one undo reverts the paste",
    );
    assert_eq!(
        published_text(&client, 0, 0),
        "5",
        "the copy source is intact"
    );
}

#[test]
fn overflow_paste_is_rejected_through_worker() {
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "a"));
    client.send(set_input(sheet, 1, 0, "b"));
    poll_until(|| published_text(&client, 1, 0) == "b", "seed publishes");

    copy_and_wait(
        &client,
        &rx,
        sheet,
        CellRange::new(CellRef::new(0, 0), CellRef::new(1, 0)),
        false,
    );
    // A 2-row payload pasted onto the last row spills past the sheet edge.
    client.send(Command::PasteInternal {
        sheet,
        target: CellRange::single(CellRef::new(freecell_core::limits::MAX_ROWS - 1, 0)),
    });
    let rejected = wait_for(&rx, |e| matches!(e, WorkerEvent::PasteRejected { .. }));
    assert!(
        matches!(
            rejected,
            Some(WorkerEvent::PasteRejected {
                reason: freecell_engine::PasteError::Overflow
            })
        ),
        "an overflowing paste is rejected; got {rejected:?}"
    );
}

// ---------------------------------------------------------------------------------------------
// P9 — live binding: charts ride the publication seam (charts/architecture §4.1, functional_spec §2)
// ---------------------------------------------------------------------------------------------

/// The value points of one series in the latest chart snapshot (category/value → its `values`,
/// scatter → its `y`), for the chart anchored on `sheet`.
fn snapshot_series_values(
    snapshot: &ChartSnapshot,
    sheet: SheetId,
    chart_idx: usize,
    series_idx: usize,
) -> Vec<f64> {
    let specs = &snapshot
        .sheets
        .iter()
        .find(|(s, _)| *s == sheet)
        .expect("the anchor sheet carries charts")
        .1;
    match &specs[chart_idx].chart().unwrap().series[series_idx].data {
        SeriesData::CategoryValue { values, .. } => values.clone(),
        SeriesData::Xy { y, .. } => y.clone(),
    }
}

/// Spawn a worker over a freshly written single-line-chart fixture; returns the client, receiver,
/// and the (first) sheet id. Waits until the worker has discovered + published its charts.
fn spawn_line_fixture() -> (DocumentClient, WorkerEventReceiver, SheetId) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    let sheet = sheets[0].id;
    // Lazy discovery (P11): charts are parsed on the **first paint** of their sheet, not on open, so
    // send a viewport (the real app always does) to trigger it. `poll_until` returns only once
    // discovery (which reads the file) has published the charts, so `dir` stays alive across the
    // read; nothing reads the file after this helper returns.
    client.send(full_viewport(sheet));
    poll_until(
        || client.chart_snapshot().version >= 1,
        "the worker discovers + publishes the file's charts on the first paint",
    );
    (client, rx, sheet)
}

#[test]
fn opened_line_chart_is_published_on_the_seam() {
    let (client, _rx, sheet) = spawn_line_fixture();
    let snap = client.chart_snapshot();
    assert!(snap.version >= 1, "charts publish a non-empty version");
    assert_eq!(snap.sheets.len(), 1, "one anchor sheet");
    let (snap_sheet, specs) = &snap.sheets[0];
    assert_eq!(*snap_sheet, sheet, "anchored to the first sheet (P8/P9)");
    assert_eq!(specs.len(), 1, "the fixture has one line chart");
    // First paint uses the file's cached values (the Widgets series = B2:B5).
    assert_eq!(
        snapshot_series_values(&snap, sheet, 0, 0),
        vec![120.0, 150.0, 90.0, 170.0],
    );
}

#[test]
fn editing_a_source_cell_reresolves_the_chart() {
    let (client, rx, sheet) = spawn_line_fixture();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    let base_version = client.chart_snapshot().version;

    // Edit B2 (the first Widgets value) — a cell inside the chart's B2:B5 value range.
    client.send(set_input(sheet, 1, 1, "999"));
    poll_until(
        || {
            let snap = client.chart_snapshot();
            snapshot_series_values(&snap, sheet, 0, 0).first() == Some(&999.0)
        },
        "editing a source cell re-resolves the line chart's first value",
    );
    let snap = client.chart_snapshot();
    // The rest of the series still tracks its (unedited) cells.
    assert_eq!(
        snapshot_series_values(&snap, sheet, 0, 0),
        vec![999.0, 150.0, 90.0, 170.0],
    );
    assert!(
        snap.version > base_version,
        "a re-resolve bumps the snapshot version"
    );
}

#[test]
fn disjoint_edit_does_not_recompute_charts() {
    let (client, rx, sheet) = spawn_line_fixture();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    let base_version = client.chart_snapshot().version;

    // K9 is outside every chart range (the fixture uses cols A–C, rows 1–5).
    client.send(set_input(sheet, 8, 10, "42"));
    poll_until(
        || published_text(&client, 8, 10) == "42",
        "the disjoint edit itself publishes",
    );
    assert_eq!(
        client.chart_snapshot().version,
        base_version,
        "only intersecting charts recompute — a disjoint edit leaves the chart snapshot untouched",
    );
}

// ---------------------------------------------------------------------------------------------
// P11 — perf: lazy parse off open's critical path + coalesced recompute (charts/architecture §5)
// ---------------------------------------------------------------------------------------------

/// Lazy parse (P11): on open — before any paint — the file's charts are NOT parsed. The snapshot
/// stays the empty version 0 until the sheet is first painted (a `SetViewport`), which then
/// discovers + publishes them. This is what keeps chart parsing off the first-paint critical path.
#[test]
fn charts_are_not_discovered_until_first_paint() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, _rx, sheets) = spawn(DocumentSource::OpenFile(path));
    let sheet = sheets[0].id;

    // No command has been sent since `Loaded`, so the worker is parked — no chart was parsed.
    assert_eq!(
        client.chart_snapshot().version,
        0,
        "charts are not discovered on open (off the critical path) — only on first paint",
    );

    // The first paint of the sheet triggers discovery → the chart publishes.
    client.send(full_viewport(sheet));
    poll_until(
        || client.chart_snapshot().version >= 1,
        "the first paint discovers + publishes the chart",
    );
    let snap = client.chart_snapshot();
    assert_eq!(snap.sheets.len(), 1);
    assert_eq!(
        snap.sheets[0].1.len(),
        1,
        "the fixture's one line chart is now bound"
    );
}

/// Coalesced dirty-set recompute (P11): two edits to two cells in one chart's source range — however
/// the worker batches them (one drained recompute, or two) — converge the chart to reflect BOTH,
/// advancing the snapshot. (Structural coalescing of a single drained batch into one recompute is
/// covered deterministically by `binding::coalesced_multi_edit_recompute_is_one_pass`.)
#[test]
fn coalesced_edits_converge_the_chart() {
    let (client, rx, sheet) = spawn_line_fixture();
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    let base_version = client.chart_snapshot().version;

    // Two edits inside the Widgets B2:B5 value range.
    client.send(set_input(sheet, 1, 1, "111")); // B2
    client.send(set_input(sheet, 2, 1, "222")); // B3
    poll_until(
        || {
            let v = snapshot_series_values(&client.chart_snapshot(), sheet, 0, 0);
            v.first() == Some(&111.0) && v.get(1) == Some(&222.0)
        },
        "both edits converge into the chart's values",
    );
    let snap = client.chart_snapshot();
    assert_eq!(
        snapshot_series_values(&snap, sheet, 0, 0),
        vec![111.0, 222.0, 90.0, 170.0],
    );
    assert!(
        snap.version > base_version,
        "the re-resolve advanced the snapshot version",
    );
}

// ---------------------------------------------------------------------------------------------
// P10 — save/restore: `Command::Save` preserves + reflows charts (charts/architecture §4.1/§5)
// ---------------------------------------------------------------------------------------------

/// The reopened first value of a chart's first series, from `discover_and_parse`.
fn reopened_first_value(path: &std::path::Path, chart_idx: usize) -> f64 {
    let specs = freecell_engine::chart::discover_and_parse(path).unwrap();
    match &specs[chart_idx].chart().unwrap().series[0].data {
        SeriesData::CategoryValue { values, .. } => values[0],
        SeriesData::Xy { y, .. } => y[0],
    }
}

/// Every zip entry's decompressed bytes (name → bytes) — robust content equality that ignores
/// zip framing (timestamps, compression).
fn zip_entry_contents(path: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
    use std::io::Read;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
    let mut map = std::collections::BTreeMap::new();
    for i in 0..zip.len() {
        let mut e = zip.by_index(i).unwrap();
        let name = e.name().to_string();
        let mut bytes = Vec::new();
        e.read_to_end(&mut bytes).unwrap();
        map.insert(name, bytes);
    }
    map
}

/// The app's `Command::Save` on an opened chart workbook preserves untouched charts byte-for-byte
/// and patches an edited one — the end-to-end save the post-P11 human checkpoint reviews, driven
/// through the worker command seam (the closest headless seam to the UI's Save action).
#[test]
fn save_through_worker_preserves_and_patches_charts() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("charts_basic.xlsx");
    freecell_engine::chart::authoring::write_fixture(&path).unwrap();

    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path.clone()));
    let sheet = sheets[0].id;
    // Lazy discovery (P11): the first paint of the sheet discovers + publishes its charts.
    client.send(full_viewport(sheet));
    poll_until(
        || client.chart_snapshot().version >= 1,
        "the worker discovers + publishes the file's charts on the first paint",
    );

    // Edit B2 (Widgets Q1) — feeds the column chart (idx 0) and the line chart (idx 1), NOT the
    // pie (idx 2, which reads column D). Wait until the reflow lands in the snapshot.
    client.send(set_input(sheet, 1, 1, "999"));
    poll_until(
        || snapshot_series_values(&client.chart_snapshot(), sheet, 0, 0).first() == Some(&999.0),
        "editing B2 re-resolves the charts that read column B",
    );

    // Save through the worker's Save command.
    let out = dir.path().join("out.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 11,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 11, .. })).is_some());

    // Reopen: the two edited charts reflect 999; the untouched pie is byte-identical to the
    // original part; IronCalc accepts the saved package.
    assert_eq!(
        reopened_first_value(&out, 0),
        999.0,
        "column chart reflowed"
    );
    assert_eq!(reopened_first_value(&out, 1), 999.0, "line chart reflowed");
    assert_eq!(
        freecell_engine::chart::xlsx::read_entry(&out, "xl/charts/chart3.xml").unwrap(),
        freecell_engine::chart::xlsx::read_entry(&path, "xl/charts/chart3.xml").unwrap(),
        "the untouched pie chart is byte-stable",
    );
    freecell_engine::WorkbookDocument::open(&out).expect("saved workbook reopens in the engine");
}

/// P11 lazy parse + save correctness: saving a chart workbook **without ever painting** its sheet
/// (so lazy discovery never ran) must still preserve the chart — the save forces a full sweep first.
#[test]
fn save_preserves_charts_when_their_sheet_was_never_painted() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, rx, _sheets) = spawn(DocumentSource::OpenFile(path));

    // No viewport is ever sent — the chart's sheet is never painted, so nothing was discovered.
    assert_eq!(
        client.chart_snapshot().version,
        0,
        "nothing discovered before the save"
    );

    let out = dir.path().join("out.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 31,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 31, .. })).is_some());

    // The save's full sweep discovered + preserved the chart despite it never being painted.
    freecell_engine::WorkbookDocument::open(&out).expect("saved workbook reopens");
    let specs = freecell_engine::chart::discover_and_parse(&out).unwrap();
    assert_eq!(
        specs.len(),
        1,
        "the never-painted chart is preserved by the save-time full sweep",
    );
    assert!(matches!(
        specs[0].chart().unwrap().kind,
        freecell_chart_model::ChartKind::Line { .. }
    ));
}

/// P11 CR (Critical, rename-robustness): renaming a chart's host sheet that was **never painted**
/// must NOT hide or drop the chart. Lazy discovery keys on the sheet's **stable file worksheet part**
/// (captured at open), not its mutable live name — so after a rename-before-paint, (a) painting the
/// renamed sheet still discovers its chart, and (b) saving preserves it on the **new** name. The
/// pre-fix name-keyed logic silently lost it (discover-by-current-name found nothing in the file).
#[test]
fn rename_before_paint_still_discovers_and_saves_the_chart() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("two_sheet.xlsx");
    // Data (column chart) + Summary (line chart); Summary is NOT the active sheet.
    freecell_engine::chart::authoring::write_two_sheet_fixture(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path.clone()));
    let summary = sheets.iter().find(|s| s.name == "Summary").unwrap().id;

    // Rename Summary → "Renamed" WITHOUT ever painting it — its line chart was never discovered.
    client.send(Command::RenameSheet {
        sheet: summary,
        name: "Renamed".to_string(),
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. })).is_some());
    assert_eq!(
        client.chart_snapshot().version,
        0,
        "nothing is discovered before the sheet is painted",
    );

    // (a) Painting the renamed sheet discovers its chart — keyed by the stable part, not the name.
    client.send(Command::SetViewport {
        sheet: summary,
        rows: 0..64,
        cols: 0..16,
    });
    poll_until(
        || {
            client
                .chart_snapshot()
                .sheets
                .iter()
                .any(|(s, specs)| *s == summary && !specs.is_empty())
        },
        "painting the renamed-before-paint sheet discovers its chart",
    );

    // (b) Saving preserves that chart on the NEW sheet name.
    let out = dir.path().join("out.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 41,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 41, .. })).is_some());
    freecell_engine::WorkbookDocument::open(&out).expect("saved workbook reopens");
    let groups = freecell_engine::chart::discover_and_parse_by_sheet(&out).unwrap();
    let (name, charts) = groups
        .iter()
        .find(|(n, _)| n == "Renamed")
        .expect("the renamed sheet carries its line chart in the saved file");
    assert_eq!(name, "Renamed");
    assert!(matches!(
        charts[0].1.chart().unwrap().kind,
        freecell_chart_model::ChartKind::Line { .. }
    ));
}

/// A chartless workbook still saves through the **plain** path — the P10 wiring must not change
/// the non-chart save. The worker-saved file's contents equal a direct `WorkbookDocument::save`
/// of the same workbook (the pre-P10 behavior).
#[test]
fn chartless_workbook_save_matches_plain_save() {
    let dir = tempdir().unwrap();
    let src = dir.path().join("plain.xlsx");
    fixtures::multi_sheet().save(&src).unwrap();

    let (client, rx, _sheets) = spawn(DocumentSource::OpenFile(src.clone()));
    let via_worker = dir.path().join("via_worker.xlsx");
    client.send(Command::Save {
        path: via_worker.clone(),
        req_id: 12,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 12, .. })).is_some());

    // Reference: the pre-P10 path — open the same file + plain save.
    let via_plain = dir.path().join("via_plain.xlsx");
    freecell_engine::WorkbookDocument::open(&src)
        .unwrap()
        .save(&via_plain)
        .unwrap();

    assert_eq!(
        zip_entry_contents(&via_worker),
        zip_entry_contents(&via_plain),
        "a chartless workbook saves identically to the plain writer",
    );
    // And it carries no chart machinery.
    assert!(zip_entry_contents(&via_worker)
        .keys()
        .all(|n| !n.starts_with("xl/charts/") && !n.starts_with("xl/drawings/")));
}

/// Spawn a worker over a chart fixture at a **kept-alive** temp path — the returned `TempDir` must
/// outlive the client, because a chart-preserving save re-reads the original file (charts, drawings,
/// content-types live there, not in the model). Waits until the charts are published on load.
fn spawn_over_chart_file(
    write: impl FnOnce(&std::path::Path),
) -> (
    DocumentClient,
    WorkerEventReceiver,
    Vec<SheetMeta>,
    tempfile::TempDir,
) {
    let dir = tempdir().unwrap();
    let path = dir.path().join("book.xlsx");
    write(&path);
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    // Lazy discovery (P11): charts bind on the **first paint** of their sheet, so paint EVERY sheet
    // to discover + bind all charts to their real `SheetId`s — exactly as eager discovery did on
    // load. The rename/delete-host save tests depend on that binding existing *before* the
    // structural sheet op (so a deleted host's chart drops, a renamed host's follows). A trailing
    // `GetCellContent` read is a FIFO fence: its reply proves every viewport (and its lazy
    // discovery) has been processed. A final viewport returns the active sheet to the first one.
    for meta in &sheets {
        client.send(Command::SetViewport {
            sheet: meta.id,
            rows: 0..64,
            cols: 0..16,
        });
    }
    client.send(full_viewport(sheets[0].id));
    client.send(Command::GetCellContent {
        sheet: sheets[0].id,
        cell: CellRef::new(0, 0),
        req_id: u64::MAX,
    });
    assert!(
        wait_for(&rx, |e| matches!(e, WorkerEvent::CellContent { req_id: u64::MAX, .. })).is_some(),
        "the read fence must reply once every sheet's first paint (and lazy discovery) is processed",
    );
    (client, rx, sheets, dir)
}

/// The reopened chart's first value + owning sheet name, via grouped discovery.
fn reopened_group_first_value(path: &std::path::Path, group: usize) -> (String, f64) {
    let groups = freecell_engine::chart::discover_and_parse_by_sheet(path).unwrap();
    let (name, charts) = &groups[group];
    let v = match &charts[0].1.chart().unwrap().series[0].data {
        SeriesData::CategoryValue { values, .. } => values[0],
        SeriesData::Xy { y, .. } => y[0],
    };
    (name.clone(), v)
}

/// P10 Critical fix: renaming a chart's host sheet in-session still SAVES (pre-P10 the plain save
/// succeeded by dropping the chart; the first cut regressed to a total SaveFailed). The chart
/// follows the rename onto the renamed worksheet and keeps its edited value.
#[test]
fn save_after_renaming_the_chart_host_sheet() {
    let (client, rx, sheets, _keep) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_line_fixture(p).unwrap()
    });
    let sheet = sheets[0].id;

    // Edit B2 (feeds the line chart) while the sheet is still "Data" → the reflow lands.
    client.send(full_viewport(sheet));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    client.send(set_input(sheet, 1, 1, "999"));
    poll_until(
        || snapshot_series_values(&client.chart_snapshot(), sheet, 0, 0).first() == Some(&999.0),
        "editing B2 re-resolves the line chart",
    );

    // Rename the host sheet, THEN save.
    client.send(Command::RenameSheet {
        sheet,
        name: "Data2".to_string(),
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. })).is_some());
    let out = tempdir().unwrap();
    let out_path = out.path().join("renamed.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 21,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 21, .. })).is_some());

    // The chart is present on the RENAMED sheet, with the edited value.
    freecell_engine::WorkbookDocument::open(&out_path).expect("saved workbook reopens");
    assert_eq!(
        reopened_group_first_value(&out_path, 0),
        ("Data2".to_string(), 999.0)
    );
}

/// P10 Critical fix: deleting a chart's host sheet in-session SAVES (no SaveFailed); that chart is
/// dropped gracefully while charts on surviving sheets are preserved.
#[test]
fn save_after_deleting_the_chart_host_sheet_succeeds() {
    let (client, rx, sheets, _keep) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_two_sheet_fixture(p).unwrap()
    });
    // Two sheets: Data (column chart) + Summary (line chart). Delete Summary.
    let summary = sheets.iter().find(|s| s.name == "Summary").unwrap().id;
    client.send(Command::DeleteSheet { sheet: summary });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. })).is_some());

    let out = tempdir().unwrap();
    let out_path = out.path().join("deleted.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 22,
    });
    // The save SUCCEEDS (not SaveFailed) despite the deleted chart-bearing sheet.
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 22, .. })).is_some());

    // Only the surviving sheet's chart comes back.
    freecell_engine::WorkbookDocument::open(&out_path).expect("saved workbook reopens");
    let specs = freecell_engine::chart::discover_and_parse(&out_path).unwrap();
    assert_eq!(specs.len(), 1);
}

/// P10/P14 (arch §6, no silent chart drop): saving after editing a SUPPORTED chart must
/// byte-preserve an UNSUPPORTED chart that lives alone on another sheet. As of P14 the surface
/// chart is **retained + bound** (a `chart: None` live descriptor) rather than dropped at load, so
/// it byte-preserves via the bound path (never parsed/patched) while still following its host sheet
/// — driven through the worker's real `Command::Save`.
#[test]
fn save_preserves_an_unsupported_chart_on_a_surviving_sheet() {
    let (client, rx, sheets, fixture_dir) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_two_sheet_supported_plus_unsupported_fixture(p)
            .unwrap()
    });
    let original = fixture_dir.path().join("book.xlsx");
    let data = sheets.iter().find(|s| s.name == "Data").unwrap().id;

    // Edit B2 (feeds the supported column chart on Data); wait for the reflow.
    client.send(full_viewport(data));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    client.send(set_input(data, 1, 1, "999"));
    poll_until(
        || snapshot_series_values(&client.chart_snapshot(), data, 0, 0).first() == Some(&999.0),
        "editing B2 re-resolves the supported chart",
    );

    let out = tempdir().unwrap();
    let out_path = out.path().join("mixed.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 24,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 24, .. })).is_some());

    freecell_engine::WorkbookDocument::open(&out_path).expect("saved workbook reopens");
    // The supported chart is present + patched.
    assert_eq!(
        reopened_group_first_value(&out_path, 0),
        ("Data".to_string(), 999.0)
    );
    // The UNSUPPORTED chart on the OTHER sheet survived byte-identically (no silent drop).
    assert_eq!(
        freecell_engine::chart::xlsx::read_entry(&out_path, "xl/charts/chart2.xml").unwrap(),
        freecell_engine::chart::xlsx::read_entry(&original, "xl/charts/chart2.xml").unwrap(),
    );
}

/// P10: the chart save path keeps the plain path's atomicity — a failure (destination is an
/// existing non-empty directory, so the rename fails) leaves that destination byte-identical and
/// litters no temp file.
#[test]
fn chart_save_atomic_on_failure_leaves_destination_untouched() {
    let (client, rx, _sheets, _keep) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_line_fixture(p).unwrap()
    });

    let target_dir = tempdir().unwrap();
    let target = target_dir.path().join("book.xlsx");
    std::fs::create_dir(&target).unwrap();
    std::fs::write(target.join("keep.txt"), b"original").unwrap();

    client.send(Command::Save {
        path: target.clone(),
        req_id: 23,
    });
    assert!(wait_for(&rx, |e| matches!(
        e,
        WorkerEvent::SaveFailed { req_id: 23, .. }
    ))
    .is_some());

    assert!(target.is_dir());
    assert_eq!(std::fs::read(target.join("keep.txt")).unwrap(), b"original");
    // No temp-file litter beside the target.
    let entries: Vec<_> = std::fs::read_dir(target_dir.path()).unwrap().collect();
    assert_eq!(entries.len(), 1);
}

// ---------------------------------------------------------------------------------------------
// P17 — insert flow: `Command::InsertChart` authors a near-empty chart that renders + saves
// (charts/architecture §4.1, ui_design §3.1, components/write-path §1 mode 3)
// ---------------------------------------------------------------------------------------------

/// A default authored-chart anchor (8 cols × 15 rows from the given top-left cell), matching the
/// chrome's insert placement.
fn chart_anchor(from_col: u32, from_row: u32) -> Anchor {
    Anchor::new(
        AnchorCell::new(from_col, from_row),
        AnchorCell::new(from_col + 8, from_row + 15),
    )
}

/// Add a sheet through the worker and return the new sheet's stable id (the one absent from
/// `existing`).
fn add_sheet_and_id(
    client: &DocumentClient,
    rx: &WorkerEventReceiver,
    existing: SheetId,
) -> SheetId {
    client.send(Command::AddSheet);
    let ev = wait_for(rx, |e| matches!(e, WorkerEvent::SheetsChanged { .. }));
    match ev {
        Some(WorkerEvent::SheetsChanged { sheets }) => {
            sheets.iter().find(|s| s.id != existing).unwrap().id
        }
        other => panic!("expected SheetsChanged, got {other:?}"),
    }
}

/// Inserting a chart through the worker publishes an **authored** (snapshot-but-not-live) chart on
/// the chart seam the grid installs from — the render half of the exit criterion.
#[test]
fn insert_line_chart_publishes_authored_snapshot() {
    let (client, _rx, sheet) = spawn_new();
    let base = client.chart_snapshot().version;

    client.send(Command::InsertChart {
        sheet,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(0, 0),
    });
    poll_until(
        || client.chart_snapshot().version > base,
        "InsertChart publishes the authored chart on the seam",
    );

    let snap = client.chart_snapshot();
    let (snap_sheet, specs) = &snap.sheets[0];
    assert_eq!(*snap_sheet, sheet, "anchored on the active sheet");
    assert_eq!(specs.len(), 1);
    assert!(
        specs[0].is_authored(),
        "the inserted chart is Authored (no retained source, no live binding)"
    );
    assert!(matches!(
        specs[0].chart().unwrap().kind,
        freecell_chart_model::ChartKind::Line { .. }
    ));
    // A near-empty chart still carries one placeholder series so it renders as its real kind.
    assert_eq!(specs[0].chart().unwrap().series.len(), 1);
}

/// Criterion #4 (headline seam): a combined save runs BOTH the loaded re-inject (mode 1/2) AND the
/// authored write-from-model (mode 3) over one workbook. Open a line-chart file, add a second sheet,
/// insert an authored chart on THAT sheet, save → reopen carries BOTH charts.
#[test]
fn combined_save_writes_loaded_and_authored_charts() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    let data_sheet = sheets[0].id;

    // Paint the Data sheet so its loaded line chart is discovered (lazy, P11).
    client.send(full_viewport(data_sheet));
    poll_until(
        || client.chart_snapshot().version >= 1,
        "the loaded line chart is discovered on first paint",
    );

    // Add a SECOND sheet and author a chart onto it (a different sheet from the loaded chart).
    let other_sheet = add_sheet_and_id(&client, &rx, data_sheet);
    client.send(Command::InsertChart {
        sheet: other_sheet,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(1, 1),
    });
    poll_until(
        || {
            client
                .chart_snapshot()
                .sheets
                .iter()
                .any(|(s, specs)| *s == other_sheet && specs.iter().any(|sp| sp.is_authored()))
        },
        "the authored chart is published on the second sheet",
    );

    // Save through the worker's Save command (the combined loaded + authored save).
    let out = dir.path().join("out.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 71,
    });
    assert!(
        wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 71, .. })).is_some(),
        "the combined loaded + authored save succeeds"
    );

    // Reopen: IronCalc accepts it AND our loader re-parses BOTH charts (the loaded line + the
    // authored line, now a Loaded spec).
    freecell_engine::WorkbookDocument::open(&out).expect("combined-save workbook reopens");
    let specs = freecell_engine::chart::discover_and_parse(&out).unwrap();
    assert_eq!(
        specs.len(),
        2,
        "both the loaded and the authored chart survive the combined save"
    );
    assert!(specs.iter().all(|s| matches!(
        s.chart().unwrap().kind,
        freecell_chart_model::ChartKind::Line { .. }
    )));
}

/// Criterion #4 (fail-loud): authoring a chart onto the SAME sheet as a loaded chart is not yet
/// supported (it would mean merging into that sheet's existing `<drawing>`), so the combined save
/// must **fail loudly** (`SaveFailed`) rather than silently drop a chart or emit a double drawing —
/// the two write reports (SaveReport vs AuthoredWriteReport) are not conflated.
#[test]
fn authored_chart_on_a_loaded_charts_sheet_fails_loudly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    let data_sheet = sheets[0].id;

    client.send(full_viewport(data_sheet));
    poll_until(
        || client.chart_snapshot().version >= 1,
        "the loaded line chart is discovered on first paint",
    );

    // Author a chart onto the loaded chart's OWN sheet.
    client.send(Command::InsertChart {
        sheet: data_sheet,
        kind: ChartInsertKind::Column,
        anchor: chart_anchor(6, 6),
    });
    poll_until(
        || {
            client
                .chart_snapshot()
                .sheets
                .iter()
                .any(|(s, specs)| *s == data_sheet && specs.iter().any(|sp| sp.is_authored()))
        },
        "the authored chart is published on the Data sheet",
    );

    let out = dir.path().join("out.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 72,
    });
    assert!(
        wait_for(&rx, |e| matches!(
            e,
            WorkerEvent::SaveFailed { req_id: 72, .. }
        ))
        .is_some(),
        "authoring onto a sheet that already has a loaded chart's drawing fails loudly on save"
    );
    // The atomic save wrote nothing (no partial/corrupt output at the target).
    assert!(!out.exists(), "a failed save leaves no output file");
}

// ---------------------------------------------------------------------------------------------
// P18 — manipulate: move / resize / delete persist to the anchor and round-trip
// (charts/functional_spec §6.A, ui_design §3.2). LibreOffice is env-broken here, so persistence is
// proven via the IronCalc `discover_and_parse` reopen path (open → manipulate → save → reopen).
// ---------------------------------------------------------------------------------------------

/// The stable [`ChartId`] of the first chart published on `sheet` (loaded or authored), if any.
fn first_chart_id(client: &DocumentClient, sheet: SheetId) -> Option<ChartId> {
    client
        .chart_snapshot()
        .sheets
        .iter()
        .find(|(s, _)| *s == sheet)
        .and_then(|(_, specs)| specs.first().map(|sp| sp.id))
}

/// The published anchor of chart `id` on `sheet`, if present.
fn snapshot_anchor(client: &DocumentClient, sheet: SheetId, id: ChartId) -> Option<Anchor> {
    client
        .chart_snapshot()
        .sheets
        .iter()
        .find(|(s, _)| *s == sheet)
        .and_then(|(_, specs)| specs.iter().find(|sp| sp.id == id).map(|sp| sp.anchor))
}

/// The number of charts published on `sheet`.
fn chart_count_on(client: &DocumentClient, sheet: SheetId) -> usize {
    client
        .chart_snapshot()
        .sheets
        .iter()
        .find(|(s, _)| *s == sheet)
        .map_or(0, |(_, specs)| specs.len())
}

/// The anchors of every chart parsed back from a saved workbook.
fn reopened_anchors(path: &std::path::Path) -> Vec<Anchor> {
    freecell_engine::chart::discover_and_parse(path)
        .unwrap()
        .iter()
        .map(|s| s.anchor)
        .collect()
}

/// P18 (authored): move/resize an authored chart → its model anchor persists and round-trips.
#[test]
fn move_authored_chart_roundtrips() {
    let (client, rx, sheet) = spawn_new();
    client.send(Command::InsertChart {
        sheet,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(0, 0),
    });
    poll_until(
        || first_chart_id(&client, sheet).is_some(),
        "the authored chart is published",
    );
    let id = first_chart_id(&client, sheet).unwrap();

    let moved = Anchor::new(AnchorCell::new(5, 10), AnchorCell::new(13, 25));
    client.send(Command::SetChartAnchor {
        sheet,
        id,
        anchor: moved,
    });
    poll_until(
        || snapshot_anchor(&client, sheet, id) == Some(moved),
        "the moved anchor is republished",
    );

    let dir = tempdir().unwrap();
    let out = dir.path().join("moved.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 80,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 80, .. })).is_some());
    freecell_engine::WorkbookDocument::open(&out).expect("reopens");
    assert_eq!(
        reopened_anchors(&out),
        vec![moved],
        "the authored chart's moved anchor round-trips"
    );
}

/// P18 (authored): delete one of two authored charts → it is gone from the saved package.
#[test]
fn delete_authored_chart_roundtrips() {
    let (client, rx, sheet) = spawn_new();
    client.send(Command::InsertChart {
        sheet,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(0, 0),
    });
    client.send(Command::InsertChart {
        sheet,
        kind: ChartInsertKind::Bar,
        anchor: chart_anchor(10, 0),
    });
    poll_until(
        || chart_count_on(&client, sheet) == 2,
        "both authored charts are published",
    );
    let first = first_chart_id(&client, sheet).unwrap();
    client.send(Command::DeleteChart { sheet, id: first });
    poll_until(
        || chart_count_on(&client, sheet) == 1,
        "one authored chart remains after delete",
    );

    let dir = tempdir().unwrap();
    let out = dir.path().join("deleted.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 81,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 81, .. })).is_some());
    freecell_engine::WorkbookDocument::open(&out).expect("reopens");
    assert_eq!(
        freecell_engine::chart::discover_and_parse(&out)
            .unwrap()
            .len(),
        1,
        "exactly one authored chart survives the delete"
    );
}

/// P18 (loaded): move a loaded chart → its retained drawing `twoCellAnchor` is patched and the new
/// anchor round-trips (the source-first save contract for a manipulated loaded chart).
#[test]
fn move_loaded_chart_patches_drawing_and_roundtrips() {
    let (client, rx, sheets, _dir) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_line_fixture(p).unwrap()
    });
    let data = sheets[0].id;
    poll_until(
        || first_chart_id(&client, data).is_some(),
        "the loaded line chart is discovered",
    );
    let id = first_chart_id(&client, data).unwrap();
    let original = snapshot_anchor(&client, data, id).unwrap();

    let moved = Anchor::new(AnchorCell::new(3, 3), AnchorCell::new(11, 20));
    assert_ne!(original, moved, "the test moves the chart to a new anchor");
    client.send(Command::SetChartAnchor {
        sheet: data,
        id,
        anchor: moved,
    });
    poll_until(
        || snapshot_anchor(&client, data, id) == Some(moved),
        "the moved loaded chart is republished",
    );

    let out = tempdir().unwrap();
    let out_path = out.path().join("loaded_moved.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 82,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 82, .. })).is_some());
    freecell_engine::WorkbookDocument::open(&out_path).expect("reopens");
    assert_eq!(
        reopened_anchors(&out_path),
        vec![moved],
        "the loaded chart's drawing anchor was patched and round-trips"
    );
}

/// P18 (loaded): delete a loaded chart → it is dropped from the saved package (the workbook still
/// opens; no chart comes back).
#[test]
fn delete_loaded_chart_drops_it_from_the_package() {
    let (client, rx, sheets, _dir) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_line_fixture(p).unwrap()
    });
    let data = sheets[0].id;
    poll_until(
        || first_chart_id(&client, data).is_some(),
        "the loaded line chart is discovered",
    );
    let id = first_chart_id(&client, data).unwrap();
    client.send(Command::DeleteChart { sheet: data, id });
    poll_until(
        || chart_count_on(&client, data) == 0,
        "the loaded chart is removed from the snapshot",
    );

    let out = tempdir().unwrap();
    let out_path = out.path().join("loaded_deleted.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 83,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 83, .. })).is_some());
    // The workbook still opens, and the deleted chart does not come back.
    freecell_engine::WorkbookDocument::open(&out_path).expect("reopens after a chart delete");
    assert_eq!(
        freecell_engine::chart::discover_and_parse(&out_path)
            .unwrap()
            .len(),
        0,
        "the deleted loaded chart is gone from the saved package"
    );
}

/// P18 (loaded, no-drift): the loaded anchor edit is an ABSOLUTE anchor re-applied against the
/// current source each save and cleared once the source advances — so a re-save doesn't drift the
/// anchor, and a second move is applied absolutely (not stacked on the first). No-authored case
/// (the source advances after each save).
#[test]
fn moved_loaded_chart_resave_is_stable_and_reflects_latest_move() {
    let (client, rx, sheets, _dir) = spawn_over_chart_file(|p| {
        freecell_engine::chart::authoring::write_line_fixture(p).unwrap()
    });
    let data = sheets[0].id;
    poll_until(
        || first_chart_id(&client, data).is_some(),
        "the loaded line chart is discovered",
    );
    let id = first_chart_id(&client, data).unwrap();

    let moved = Anchor::new(AnchorCell::new(3, 3), AnchorCell::new(11, 20));
    client.send(Command::SetChartAnchor {
        sheet: data,
        id,
        anchor: moved,
    });
    poll_until(
        || snapshot_anchor(&client, data, id) == Some(moved),
        "the move is published",
    );

    let out = tempdir().unwrap();
    let p1 = out.path().join("save1.xlsx");
    client.send(Command::Save {
        path: p1.clone(),
        req_id: 84,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 84, .. })).is_some());
    assert_eq!(reopened_anchors(&p1), vec![moved]);

    // (a) Re-save with NO further move → the anchor is stable (source advanced, edit spent; the
    // just-saved file is byte-preserved — no drift, no double-apply).
    let p2 = out.path().join("save2.xlsx");
    client.send(Command::Save {
        path: p2.clone(),
        req_id: 85,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 85, .. })).is_some());
    assert_eq!(
        reopened_anchors(&p2),
        vec![moved],
        "a plain re-save does not drift the moved anchor"
    );

    // (b) Move AGAIN → save → the latest anchor wins (absolute, not additive).
    let moved2 = Anchor::new(AnchorCell::new(6, 7), AnchorCell::new(14, 24));
    client.send(Command::SetChartAnchor {
        sheet: data,
        id,
        anchor: moved2,
    });
    poll_until(
        || snapshot_anchor(&client, data, id) == Some(moved2),
        "the second move is published",
    );
    let p3 = out.path().join("save3.xlsx");
    client.send(Command::Save {
        path: p3.clone(),
        req_id: 86,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 86, .. })).is_some());
    assert_eq!(
        reopened_anchors(&p3),
        vec![moved2],
        "the second move is applied absolutely (no double-apply)"
    );
}

/// P18 (loaded, authored-present no-drift): with an authored chart present the reinject source
/// **stays** the original and `loaded_anchor_edits` persist across saves — re-applying the ABSOLUTE
/// anchor each time, so a re-save keeps the loaded chart's moved anchor stable (no drift, no double
/// apply).
#[test]
fn moved_loaded_chart_with_authored_present_resave_is_stable() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    let data = sheets[0].id;
    client.send(full_viewport(data));
    poll_until(
        || first_chart_id(&client, data).is_some(),
        "the loaded line chart is discovered",
    );
    let id = first_chart_id(&client, data).unwrap();

    // An authored chart on a SECOND sheet keeps the reinject source pinned to the original.
    let other = add_sheet_and_id(&client, &rx, data);
    client.send(Command::InsertChart {
        sheet: other,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(1, 1),
    });
    poll_until(
        || chart_count_on(&client, other) == 1,
        "the authored chart is published",
    );

    let moved = Anchor::new(AnchorCell::new(3, 3), AnchorCell::new(11, 20));
    client.send(Command::SetChartAnchor {
        sheet: data,
        id,
        anchor: moved,
    });
    poll_until(
        || snapshot_anchor(&client, data, id) == Some(moved),
        "the loaded chart move is published",
    );

    // Save twice — the loaded chart keeps its moved anchor across both saves (exactly once each).
    let moved_once =
        |p: &std::path::Path| reopened_anchors(p).iter().filter(|a| **a == moved).count();
    let out1 = dir.path().join("out1.xlsx");
    client.send(Command::Save {
        path: out1.clone(),
        req_id: 87,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 87, .. })).is_some());
    assert_eq!(
        moved_once(&out1),
        1,
        "first save places the loaded chart at `moved`"
    );

    let out2 = dir.path().join("out2.xlsx");
    client.send(Command::Save {
        path: out2.clone(),
        req_id: 88,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 88, .. })).is_some());
    assert_eq!(
        moved_once(&out2),
        1,
        "the re-save (authored present, source pinned) keeps the moved anchor — no drift/double-apply"
    );
}

// ---------------------------------------------------------------------------------------------
// P19 — edit panel range/type: a ranged authored chart binds + saves with `c:f` (not literals), and
// a retype round-trips as the new kind (charts/implementation_plan P19). LibreOffice is env-broken,
// so the reopen proof is the IronCalc `discover_and_parse` path (insert → range/type → save → reopen).
// ---------------------------------------------------------------------------------------------

/// A `spawn_new` workbook seeded with a small data grid (B1 header, A2:A3 categories, B2:B3 values)
/// plus a fresh line chart; returns the sheet + the authored chart id, ready for a range set.
fn spawn_new_with_chart_data() -> (DocumentClient, WorkerEventReceiver, SheetId, ChartId) {
    let (client, rx, sheet) = spawn_new();
    for cmd in [
        set_input(sheet, 0, 1, "Widgets"), // B1 (series name)
        set_input(sheet, 1, 0, "Q1"),      // A2 (category)
        set_input(sheet, 2, 0, "Q2"),      // A3
        set_input(sheet, 1, 1, "10"),      // B2 (value)
        set_input(sheet, 2, 1, "20"),      // B3
    ] {
        client.send(cmd);
    }
    client.send(Command::InsertChart {
        sheet,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(4, 0),
    });
    poll_until(
        || first_chart_id(&client, sheet).is_some(),
        "the authored chart is published",
    );
    let id = first_chart_id(&client, sheet).unwrap();
    (client, rx, sheet, id)
}

/// Exit proof: setting a range binds the authored chart to real cells, and on save it round-trips as
/// a **live-bound** Loaded chart (its `c:f` + cached cell values survive `discover_and_parse`), not
/// as literal placeholder data.
#[test]
fn ranged_authored_chart_saves_cf_and_roundtrips() {
    let (client, rx, sheet, id) = spawn_new_with_chart_data();
    client.send(Command::SetChartRange {
        sheet,
        id,
        data: CellRange::from_a1("A1:B3").unwrap(),
    });
    poll_until(
        || snapshot_series_values(&client.chart_snapshot(), sheet, 0, 0) == vec![10.0, 20.0],
        "the ranged chart re-resolves to the cell values on the seam",
    );

    let dir = tempdir().unwrap();
    let out = dir.path().join("ranged.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 90,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 90, .. })).is_some());
    freecell_engine::WorkbookDocument::open(&out).expect("reopens");

    let specs = freecell_engine::chart::discover_and_parse(&out).unwrap();
    assert_eq!(specs.len(), 1);
    let spec = &specs[0];
    assert!(
        spec.is_loaded(),
        "the reopened authored chart is a Loaded spec"
    );
    let ranges: Vec<&str> = spec.source_ranges.iter().map(|r| r.as_str()).collect();
    assert!(
        ranges.iter().any(|r| r.ends_with("$B$2:$B$3")),
        "the value c:f round-tripped (LIVE binding, not literals): {ranges:?}"
    );
    match &spec.chart().unwrap().series[0].data {
        SeriesData::CategoryValue { values, .. } => assert_eq!(values, &vec![10.0, 20.0]),
        other => panic!("expected CategoryValue, got {other:?}"),
    }
    assert!(matches!(
        spec.chart().unwrap().kind,
        freecell_chart_model::ChartKind::Line { .. }
    ));
}

/// Exit proof: switching a ranged chart's type round-trips as the new kind, keeping the `c:f` binding.
#[test]
fn retyped_authored_chart_roundtrips() {
    let (client, rx, sheet, id) = spawn_new_with_chart_data();
    client.send(Command::SetChartRange {
        sheet,
        id,
        data: CellRange::from_a1("A1:B3").unwrap(),
    });
    poll_until(
        || snapshot_series_values(&client.chart_snapshot(), sheet, 0, 0) == vec![10.0, 20.0],
        "the chart is ranged",
    );
    client.send(Command::SetChartType {
        sheet,
        id,
        kind: ChartInsertKind::Column,
    });
    poll_until(
        || {
            client
                .chart_snapshot()
                .sheets
                .iter()
                .find(|(s, _)| *s == sheet)
                .and_then(|(_, specs)| specs.first())
                .is_some_and(|sp| {
                    matches!(
                        sp.chart().unwrap().kind,
                        freecell_chart_model::ChartKind::Bar { .. }
                    )
                })
        },
        "the retype republishes a column chart",
    );

    let dir = tempdir().unwrap();
    let out = dir.path().join("retyped.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 91,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 91, .. })).is_some());
    freecell_engine::WorkbookDocument::open(&out).expect("reopens");

    let specs = freecell_engine::chart::discover_and_parse(&out).unwrap();
    assert_eq!(specs.len(), 1);
    assert!(
        matches!(
            specs[0].chart().unwrap().kind,
            freecell_chart_model::ChartKind::Bar {
                dir: freecell_chart_model::BarDir::Col,
                ..
            }
        ),
        "the reopened chart is a column (bar/col) chart"
    );
    let ranges: Vec<&str> = specs[0].source_ranges.iter().map(|r| r.as_str()).collect();
    assert!(
        ranges.iter().any(|r| r.ends_with("$B$2:$B$3")),
        "the range binding survived the retype: {ranges:?}"
    );
}

/// Phase-plan item 4 (three-way honesty): one save carrying a LOADED chart + a BOUND (ranged)
/// authored chart + an UNBOUND authored chart keeps all three straight — loaded byte-preserved,
/// bound-authored written with `c:f`, unbound-authored written as literals — and all three reopen.
#[test]
fn combined_save_mixes_loaded_bound_and_unbound_authored_charts() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("line.xlsx");
    freecell_engine::chart::authoring::write_line_fixture(&path).unwrap();
    let (client, rx, sheets) = spawn(DocumentSource::OpenFile(path));
    let data_sheet = sheets[0].id;

    // Discover the loaded line chart on Data (lazy, on first paint).
    client.send(full_viewport(data_sheet));
    poll_until(
        || client.chart_snapshot().version >= 1,
        "the loaded line chart is discovered on first paint",
    );

    // A SECOND sheet with its own data grid, carrying the two authored charts (a separate drawing
    // from the loaded chart's, so the combined save doesn't hit the same-sheet fail-loud precondition).
    let sheet2 = add_sheet_and_id(&client, &rx, data_sheet);
    for cmd in [
        set_input(sheet2, 0, 1, "S"),  // B1 (series name)
        set_input(sheet2, 1, 0, "Q1"), // A2 (category)
        set_input(sheet2, 2, 0, "Q2"), // A3
        set_input(sheet2, 1, 1, "10"), // B2 (value)
        set_input(sheet2, 2, 1, "20"), // B3
    ] {
        client.send(cmd);
    }

    // Chart A: insert + range it (BOUND — saves with c:f).
    client.send(Command::InsertChart {
        sheet: sheet2,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(4, 0),
    });
    poll_until(
        || chart_count_on(&client, sheet2) == 1,
        "the first authored chart is published",
    );
    let bound_id = first_chart_id(&client, sheet2).unwrap();
    client.send(Command::SetChartRange {
        sheet: sheet2,
        id: bound_id,
        data: CellRange::from_a1("A1:B3").unwrap(),
    });
    poll_until(
        || snapshot_series_values(&client.chart_snapshot(), sheet2, 0, 0) == vec![10.0, 20.0],
        "chart A binds to Sheet2's cells",
    );

    // Chart B: insert but never range it (UNBOUND — saves as literals).
    client.send(Command::InsertChart {
        sheet: sheet2,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(14, 0),
    });
    poll_until(
        || chart_count_on(&client, sheet2) == 2,
        "both authored charts are published on Sheet2",
    );

    // The combined save runs loaded re-inject (Data) + authored write-from-model (Sheet2: one bound,
    // one unbound) in one pass.
    let out = dir.path().join("mixed.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 92,
    });
    assert!(
        wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 92, .. })).is_some(),
        "the three-way combined save succeeds"
    );
    freecell_engine::WorkbookDocument::open(&out).expect("reopens");

    let specs = freecell_engine::chart::discover_and_parse(&out).unwrap();
    assert_eq!(
        specs.len(),
        3,
        "loaded + bound-authored + unbound-authored all saved"
    );
    // Exactly one chart has no c:f — the unbound authored one (its literals aren't read back as refs).
    assert_eq!(
        specs.iter().filter(|s| s.source_ranges.is_empty()).count(),
        1,
        "one unbound (literal-only) authored chart"
    );
    let ranges: Vec<String> = specs
        .iter()
        .flat_map(|s| s.source_ranges.iter().map(|r| r.as_str().to_string()))
        .collect();
    assert!(
        ranges.iter().any(|r| r.ends_with("$B$2:$B$5")),
        "the loaded chart's c:f is preserved: {ranges:?}"
    );
    assert!(
        ranges.iter().any(|r| r.ends_with("$B$2:$B$3")),
        "the bound authored chart's c:f survived: {ranges:?}"
    );
    assert!(
        specs.iter().all(|s| matches!(
            s.chart().unwrap().kind,
            freecell_chart_model::ChartKind::Line { .. }
        )),
        "all three reopen as line charts",
    );
}

// ---------------------------------------------------------------------------------------------
// P20 — chrome editing: title / legend / axis title / series color / data labels apply LIVE and
// round-trip; the loaded edit-contract holds (patch preserves unmodeled styling byte-for-byte).
// LibreOffice is env-broken here, so round-trip is proven via the `discover_and_parse` reopen path.
// ---------------------------------------------------------------------------------------------

/// The published title of the first chart anchored on `sheet`, if any.
fn snapshot_chart_title(client: &DocumentClient, sheet: SheetId) -> Option<String> {
    client
        .chart_snapshot()
        .sheets
        .iter()
        .find(|(s, _)| *s == sheet)
        .and_then(|(_, specs)| specs.first())
        .and_then(|sp| sp.chart())
        .and_then(|c| c.title.clone())
}

/// Rewrite one zip entry of an `.xlsx` on disk (test helper for injecting a fixture sentinel).
fn rewrite_zip_entry(path: &std::path::Path, entry: &str, edit: impl Fn(&str) -> String) {
    use std::io::{Read, Write};
    let bytes = std::fs::read(path).unwrap();
    let mut zin = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::<u8>::new()));
    let opts =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for i in 0..zin.len() {
        let mut f = zin.by_index(i).unwrap();
        let name = f.name().to_string();
        zw.start_file(&name, opts).unwrap();
        if name == entry {
            let mut s = String::new();
            f.read_to_string(&mut s).unwrap();
            zw.write_all(edit(&s).as_bytes()).unwrap();
        } else {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            zw.write_all(&buf).unwrap();
        }
    }
    std::fs::write(path, zw.finish().unwrap().into_inner()).unwrap();
}

/// A line-chart fixture whose `chart1.xml` carries an **unmodeled** `<c:roundedCorners>` element —
/// the sentinel the loaded edit contract must preserve byte-for-byte across a chrome edit + save.
fn write_line_fixture_with_sentinel(path: &std::path::Path) {
    freecell_engine::chart::authoring::write_line_fixture(path).unwrap();
    rewrite_zip_entry(path, "xl/charts/chart1.xml", |xml| {
        xml.replace("<c:chart>", "<c:roundedCorners val=\"1\"/><c:chart>")
    });
}

/// **The headline exit proof (loaded chart).** Open a loaded line chart carrying an unmodeled
/// styling element, edit its title through the panel command, save, reopen: (a) the title changed,
/// (b) the unmodeled element is byte-identical in the saved chart part (the edit contract), and the
/// edit was LIVE (the published chart re-rendered before the save).
#[test]
fn loaded_chart_title_edit_is_live_and_preserves_unmodeled_styling() {
    let (client, rx, sheets, fixture_dir) = spawn_over_chart_file(write_line_fixture_with_sentinel);
    let original = fixture_dir.path().join("book.xlsx");
    let sheet = sheets[0].id;

    // The sentinel is present in the loaded chart part before any edit.
    let before =
        freecell_engine::chart::xlsx::read_entry(&original, "xl/charts/chart1.xml").unwrap();
    assert!(
        before.contains("<c:roundedCorners val=\"1\"/>"),
        "sentinel present pre-edit"
    );

    let id = first_chart_id(&client, sheet).expect("the loaded line chart is discovered");

    // Edit the title through the panel command; it applies LIVE to the published snapshot.
    client.send(Command::SetChartChrome {
        sheet,
        id,
        edit: ChartChromeEdit::Title(Some("Reviewed Q Report".into())),
    });
    poll_until(
        || snapshot_chart_title(&client, sheet).as_deref() == Some("Reviewed Q Report"),
        "the loaded chart's title re-renders live on the snapshot",
    );

    // Save + reopen.
    let out = tempdir().unwrap();
    let out_path = out.path().join("edited.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 101,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 101, .. })).is_some());

    // (a) The edited title round-trips.
    freecell_engine::WorkbookDocument::open(&out_path).expect("edited workbook reopens");
    let specs = freecell_engine::chart::discover_and_parse(&out_path).unwrap();
    assert_eq!(specs.len(), 1);
    assert_eq!(
        specs[0].chart().unwrap().title.as_deref(),
        Some("Reviewed Q Report"),
        "the title edit round-trips",
    );

    // (b) The unmodeled styling element is byte-identical in the saved chart part (edit contract).
    let saved =
        freecell_engine::chart::xlsx::read_entry(&out_path, "xl/charts/chart1.xml").unwrap();
    assert!(
        saved.contains("<c:roundedCorners val=\"1\"/>"),
        "the unmodeled OOXML element survives the edit + save byte-for-byte",
    );
    // The chart's c:f binding + its series' colors were untouched (only the title was patched).
    assert!(saved.contains("<c:f>Data!$B$2:$B$5</c:f>"), "c:f preserved");
    assert!(
        saved.contains(r#"<a:srgbClr val="4472C4"/>"#),
        "series color preserved"
    );
}

/// P20: a **loaded** chart's legend / series color / data-label edits also round-trip through the
/// source patch (each splices only its own sub-element).
#[test]
fn loaded_chart_legend_color_and_labels_roundtrip() {
    let (client, rx, sheets, _dir) = spawn_over_chart_file(write_line_fixture_with_sentinel);
    let sheet = sheets[0].id;
    let id = first_chart_id(&client, sheet).expect("chart discovered");

    client.send(Command::SetChartChrome {
        sheet,
        id,
        edit: ChartChromeEdit::Legend(Some(freecell_chart_model::LegendPosition::Bottom)),
    });
    client.send(Command::SetChartChrome {
        sheet,
        id,
        edit: ChartChromeEdit::SeriesColor {
            series: 0,
            color: Some(freecell_core::Rgb::from_hex(0x70AD47)),
        },
    });
    client.send(Command::SetChartChrome {
        sheet,
        id,
        edit: ChartChromeEdit::DataLabels(DataLabelToggles {
            show_value: true,
            show_category_name: false,
            show_percent: false,
        }),
    });
    poll_until(
        || {
            client
                .chart_snapshot()
                .sheets
                .iter()
                .find(|(s, _)| *s == sheet)
                .and_then(|(_, specs)| specs.first())
                .and_then(|sp| sp.chart())
                .map(|c| c.series[0].data_labels.is_some())
                == Some(true)
        },
        "the loaded chart's chrome edits apply live",
    );

    let out = tempdir().unwrap();
    let out_path = out.path().join("chrome.xlsx");
    client.send(Command::Save {
        path: out_path.clone(),
        req_id: 102,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 102, .. })).is_some());

    freecell_engine::WorkbookDocument::open(&out_path).expect("reopens");
    let specs = freecell_engine::chart::discover_and_parse(&out_path).unwrap();
    let chart = specs[0].chart().unwrap();
    assert_eq!(
        chart.legend.map(|l| l.position),
        Some(freecell_chart_model::LegendPosition::Bottom),
    );
    assert_eq!(
        chart.series[0].color,
        Some(freecell_chart_model::ChartColor::Rgb(
            freecell_chart_model::Color::from_hex(0x70AD47)
        )),
    );
    assert!(chart.series[0].data_labels.as_ref().unwrap().show_value);
    // The unmodeled sentinel still survives across the multi-field chrome edit.
    let saved =
        freecell_engine::chart::xlsx::read_entry(&out_path, "xl/charts/chart1.xml").unwrap();
    assert!(saved.contains("<c:roundedCorners val=\"1\"/>"));
}

/// P20 (authored): an authored chart's chrome edits (title / legend / axis title / series color /
/// data labels) survive the write-from-model save + reopen.
#[test]
fn authored_chart_chrome_edits_roundtrip() {
    let (client, rx, sheet) = spawn_new();
    client.send(Command::InsertChart {
        sheet,
        kind: ChartInsertKind::Line,
        anchor: chart_anchor(0, 0),
    });
    poll_until(
        || first_chart_id(&client, sheet).is_some(),
        "the authored chart is published",
    );
    let id = first_chart_id(&client, sheet).unwrap();

    for edit in [
        ChartChromeEdit::Title(Some("Authored & Edited".into())),
        ChartChromeEdit::Legend(Some(freecell_chart_model::LegendPosition::Top)),
        ChartChromeEdit::AxisTitle {
            axis: ChartAxisKind::Value,
            title: Some("Units".into()),
        },
        ChartChromeEdit::SeriesColor {
            series: 0,
            color: Some(freecell_core::Rgb::from_hex(0xED7D31)),
        },
        ChartChromeEdit::DataLabels(DataLabelToggles {
            show_value: true,
            show_category_name: true,
            show_percent: false,
        }),
    ] {
        client.send(Command::SetChartChrome { sheet, id, edit });
    }
    poll_until(
        || snapshot_chart_title(&client, sheet).as_deref() == Some("Authored & Edited"),
        "the authored chart's chrome edits apply",
    );

    let dir = tempdir().unwrap();
    let out = dir.path().join("authored_chrome.xlsx");
    client.send(Command::Save {
        path: out.clone(),
        req_id: 103,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Saved { req_id: 103, .. })).is_some());

    freecell_engine::WorkbookDocument::open(&out).expect("reopens");
    let specs = freecell_engine::chart::discover_and_parse(&out).unwrap();
    let chart = specs[0].chart().unwrap();
    assert_eq!(chart.title.as_deref(), Some("Authored & Edited"));
    assert_eq!(
        chart.legend.map(|l| l.position),
        Some(freecell_chart_model::LegendPosition::Top)
    );
    assert_eq!(chart.val_axis.title.as_deref(), Some("Units"));
    assert_eq!(
        chart.series[0].color,
        Some(freecell_chart_model::ChartColor::Rgb(
            freecell_chart_model::Color::from_hex(0xED7D31)
        )),
    );
    let dl = chart.series[0].data_labels.clone().expect("labels present");
    assert!(dl.show_value && dl.show_category_name);
}
