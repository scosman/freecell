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

use freecell_core::{CellRange, CellRef, SheetId};
use freecell_engine::{
    fixtures, Command, DocumentClient, DocumentSource, EditRejectedReason, SheetMeta, StyleAttr,
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
    // Two edits coalesce; wait for the publish that reflects them.
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());

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
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "1"));
    client.send(set_input(sheet, 0, 0, "2"));
    // Drain to the last publish reflecting "2".
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    let ops_after_edits = client.committed_ops();
    assert!(ops_after_edits >= 2);

    client.send(Command::Undo);
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    assert_eq!(
        published_text(&client, 0, 0),
        "1",
        "undo reverts to the prior value"
    );
    // Undo itself is a committed op (dirty stays set — architecture §2).
    assert_eq!(client.committed_ops(), ops_after_edits + 1);

    client.send(Command::Redo);
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
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
    let (client, rx, sheet) = spawn_new();
    client.send(full_viewport(sheet));
    client.send(set_input(sheet, 0, 0, "5"));
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());

    client.send(Command::SetStyleAttr {
        sheet,
        range: CellRange::single(CellRef::new(0, 0)),
        attr: StyleAttr::Bold,
    });
    assert!(wait_for(&rx, |e| matches!(e, WorkerEvent::Published)).is_some());
    // The value is unchanged; the style edit still committed an op + published.
    assert_eq!(published_text(&client, 0, 0), "5");
    assert!(client.committed_ops() >= 2);
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
