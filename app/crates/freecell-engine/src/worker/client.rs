//! `DocumentClient` — the cheap, `Send`-able handle the window keeps, plus the shared
//! read-surfaces the worker writes and the UI reads (`components/engine_worker.md §Public
//! interface`, `architecture.md §2`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use freecell_core::{CfRuleView, Publication, SheetCaches, SheetId};
use parking_lot::{Mutex, RwLock};

use crate::document::DocumentSource;

use super::charts::ChartSnapshot;
use super::protocol::{Command, WorkerEvent};
use super::run::Worker;

/// The worker thread's stack size: **64 MiB** (`components/engine_worker.md §Main loop`,
/// `architecture.md §5`). IronCalc's formula parser + evaluator are recursive with no depth
/// cap; the input cap eliminates the abort *class*, and this deep stack gives the caught
/// panics (`catch_unwind`) generous headroom over every measured round-3 D ceiling.
pub const WORKER_STACK_SIZE: usize = 64 << 20;

/// The read-surfaces shared between the worker (writer) and the UI (reader). All lock-free or
/// briefly-locked so the render loop never blocks on the worker (`architecture.md §2`).
pub(super) struct Shared {
    /// The latest published viewport snapshot (swapped before the generation bump). Held
    /// behind its own `Arc` so the window can hand the exact swap container to the grid's
    /// `GridDataSources` (the grid loads it wait-free each frame).
    pub(super) publication: Arc<ArcSwap<Publication>>,
    /// Bumped strictly **after** the publication swap — a bump always has fresh data behind
    /// it (SP1's publish-then-bump ordering fix). Read via [`DocumentClient::generation`]; the
    /// grid does not poll it (it re-reads the publication + repaints on `Published`).
    pub(super) generation: AtomicU64,
    /// The count of committed undoable ops (dirty tracking; `architecture.md §2`). The UI's
    /// dirty flag = `committed_ops > last_saved_op`.
    pub(super) committed_ops: AtomicU64,
    /// The resident style/geometry cache. Created empty here; **populated in Phase 5** (the
    /// worker owns the writes, the grid reads per frame).
    pub(super) caches: Arc<RwLock<SheetCaches>>,
    /// The latest published live-bound charts (P9). Rides the same wait-free `arc_swap` path as
    /// [`publication`](Self::publication); stored by the worker before the `Published` bump and
    /// installed UI-side on a version change (charts/architecture §4.1).
    pub(super) chart_snapshot: Arc<ArcSwap<ChartSnapshot>>,
    /// The published conditional-formatting rule list per sheet (`architecture.md §4.5`,
    /// `components/engine_cf.md §5`). The worker writes `document.cond_fmt_rules(sheet)` here after
    /// any CF mutation, on undo/redo of a CF op, and once on open; the UI reads it synchronously via
    /// [`DocumentClient::cond_fmt_rules`] to build the sidebar. A sheet with no CF rule has **no**
    /// entry (never an empty vec), so a non-CF workbook keeps this map empty.
    pub(super) cond_fmt: Arc<RwLock<HashMap<SheetId, Vec<CfRuleView>>>>,
}

impl Shared {
    pub(super) fn new(initial_sheet: SheetId) -> Self {
        Self {
            publication: Arc::new(ArcSwap::from_pointee(Publication::empty(initial_sheet, 0))),
            generation: AtomicU64::new(0),
            committed_ops: AtomicU64::new(0),
            caches: Arc::new(RwLock::new(SheetCaches::new())),
            chart_snapshot: Arc::new(ArcSwap::from_pointee(ChartSnapshot::empty())),
            cond_fmt: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

/// How the worker thread ended (B1, `functional_spec.md F2.3`). Reported by
/// [`DocumentClient::worker_exit`] so a window that loses its worker can say *how* it lost it
/// rather than only *that* the event channel closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerExit {
    /// The thread returned normally (a requested shutdown, a load failure, or a dropped command
    /// channel). Also reported for the `test-support` worker-less client, which has no thread.
    Clean,
    /// The thread unwound out of `load_and_run` — a panic no guard caught.
    Panicked,
    /// The thread has not finished yet, so nothing was joined. Expected right after the event
    /// stream closes: the stream closes when the thread's frame drops `event_tx`, which happens
    /// *before* the thread finishes unwinding the rest of the `Worker` (the `UserModel`, the
    /// caches, the chart bindings) and marks itself finished — so a UI-thread probe at that moment
    /// usually lands here. The caller joins off the UI thread
    /// ([`take_worker_handle`](DocumentClient::take_worker_handle) + [`join_worker`]) to learn the
    /// real outcome.
    Running,
}

/// Joins a worker thread taken with [`DocumentClient::take_worker_handle`] and reports how it
/// ended. **Blocks** until the thread has finished unwinding, so it must not be called on the UI
/// thread — that is the whole reason the handle is handed out rather than joined in place.
pub fn join_worker(handle: JoinHandle<()>) -> WorkerExit {
    match handle.join() {
        Ok(()) => WorkerExit::Clean,
        Err(_) => WorkerExit::Panicked,
    }
}

/// The window's handle to its worker: send commands, read the latest published snapshot,
/// generation, committed-op count, and the resident cache. Cloning is intentionally **not**
/// derived — one window owns one worker; the handle carries `Arc`s internally.
pub struct DocumentClient {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    /// The worker thread's handle, **retained** (B1, `functional_spec.md F2.3`). It used to be
    /// dropped at spawn, which is why a dead worker was indistinguishable from a quiet one: with
    /// the handle gone and `send` swallowing `SendError` by design, nothing anywhere could tell
    /// that the thread had unwound. Taken by the first [`worker_exit`](DocumentClient::worker_exit)
    /// call; `None` for the worker-less test client.
    join: Mutex<Option<JoinHandle<()>>>,
    /// Set when the window itself asked the worker to stop, so an *expected* stream close is not
    /// reported as a crash. Nothing sends `Shutdown` today — which is exactly why the window must
    /// check the flag rather than assume, or an orderly shutdown added later would pop a false
    /// alarm on every window close.
    shutdown_requested: AtomicBool,
    /// Whether this client was built over a real worker thread at all. `false` only for the
    /// worker-less [`detached`](Self::detached) test client, whose event stream is closed from
    /// birth. Kept **separate** from [`shutdown_requested`](Self::shutdown_requested): that flag
    /// answers "did we ask the worker to stop", and having the worker-less constructor store
    /// `true` into it (as it briefly did) was a lie the moment anything else consulted it.
    has_worker: bool,
}

impl DocumentClient {
    /// Spawns the worker on a dedicated 64 MiB-stack thread named `eval-worker`, moving the
    /// document build (new/open — real I/O) onto that thread. Returns the client plus the
    /// event receiver the window's gpui task awaits. The worker emits `Loaded` / `LoadFailed`
    /// as its first event.
    pub fn spawn(source: DocumentSource) -> (DocumentClient, WorkerEventReceiver) {
        let (tx, rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = async_channel::unbounded::<WorkerEvent>();
        // The active sheet defaults to the first; its real stable id is fixed up by the worker
        // after the document loads (before the first publish).
        let shared = Arc::new(Shared::new(SheetId(0)));
        let worker_shared = Arc::clone(&shared);

        let join = std::thread::Builder::new()
            .name("eval-worker".to_string())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || Worker::load_and_run(source, worker_shared, event_tx, rx))
            .expect("spawn eval-worker thread");

        (
            DocumentClient {
                tx,
                shared,
                join: Mutex::new(Some(join)),
                shutdown_requested: AtomicBool::new(false),
                has_worker: true,
            },
            WorkerEventReceiver { rx: event_rx },
        )
    }

    /// A **worker-less** client for headless UI tests: no OS thread is spawned, sent commands go
    /// nowhere (the command receiver is dropped), and the event channel is closed so the window's
    /// event task completes immediately (`recv().await` → `None`). Tests drive folding by
    /// injecting `WorkerEvent`s directly, so no real events are needed. Behind the `test-support`
    /// feature so it can never reach a release build. Reads return the empty initial state.
    #[cfg(feature = "test-support")]
    pub fn detached() -> (DocumentClient, WorkerEventReceiver) {
        let (tx, _rx) = mpsc::channel::<Command>(); // `_rx` dropped → sends are no-ops
        let (_event_tx, event_rx) = async_channel::unbounded::<WorkerEvent>(); // closed → recv None
        let shared = Arc::new(Shared::new(SheetId(0)));
        (
            DocumentClient {
                tx,
                shared,
                join: Mutex::new(None),
                shutdown_requested: AtomicBool::new(false),
                // This client has no worker at all, so its immediately closed event stream is
                // expected rather than a death (B1, `functional_spec.md F2.3`) — without this every
                // detached-client window test would trip the worker-lost dialog on its first frame.
                // It is recorded HERE rather than as a fake "we asked for a shutdown", so the two
                // questions stay separable. A test that WANTS to exercise the worker-lost path uses
                // [`detached_live`](Self::detached_live), which does have `has_worker: true`.
                has_worker: false,
            },
            WorkerEventReceiver { rx: event_rx },
        )
    }

    /// A worker-less client whose event stream stays **open**, plus the sender that feeds it — so
    /// a test can inject `WorkerEvent`s and, by dropping the sender, simulate the worker thread
    /// dying without a requested shutdown (B1, `functional_spec.md F2.3`). Unlike
    /// [`detached`](Self::detached) this client claims a worker (`has_worker: true`) and has not
    /// requested shutdown, so a stream close is fatal exactly as it would be in production.
    #[cfg(feature = "test-support")]
    pub fn detached_live() -> (
        DocumentClient,
        WorkerEventReceiver,
        async_channel::Sender<WorkerEvent>,
    ) {
        let (tx, _rx) = mpsc::channel::<Command>();
        let (event_tx, event_rx) = async_channel::unbounded::<WorkerEvent>();
        let shared = Arc::new(Shared::new(SheetId(0)));
        (
            DocumentClient {
                tx,
                shared,
                join: Mutex::new(None),
                shutdown_requested: AtomicBool::new(false),
                has_worker: true,
            },
            WorkerEventReceiver { rx: event_rx },
            event_tx,
        )
    }

    /// Sends a command to the worker. Non-blocking and infallible to the caller: if the worker
    /// is gone the send is dropped (the UI observes the closed event channel instead).
    pub fn send(&self, cmd: Command) {
        if matches!(cmd, Command::Shutdown) {
            self.shutdown_requested.store(true, Ordering::Release);
        }
        let _ = self.tx.send(cmd);
    }

    /// Whether this window asked the worker to stop. The window checks it when the event stream
    /// closes: a close it did not request means the worker died (B1, `functional_spec.md F2.3`).
    pub fn shutdown_requested(&self) -> bool {
        self.shutdown_requested.load(Ordering::Acquire)
    }

    /// Whether this client was built over a real worker thread. `false` only for the worker-less
    /// [`detached`](Self::detached) test client — whose event stream is closed from birth, so the
    /// window must not read that close as a death.
    pub fn has_worker(&self) -> bool {
        self.has_worker
    }

    /// How the worker thread ended, **without ever blocking** — this is called from the UI thread.
    ///
    /// A thread that has not finished reports [`WorkerExit::Running`] and keeps its handle. That is
    /// the *common* answer right after the event stream closes, not a rare one: the stream closes
    /// when the thread's frame drops `event_tx`, and the thread then still has to unwind the whole
    /// `Worker` before it is finished. Callers that want the real outcome take the handle
    /// ([`take_worker_handle`](Self::take_worker_handle)) and [`join_worker`] it off the UI thread.
    ///
    /// The handle is consumed once the thread has finished; later calls report `Clean` (there is
    /// nothing left to join).
    pub fn worker_exit(&self) -> WorkerExit {
        let mut guard = self.join.lock();
        match guard.take() {
            None => WorkerExit::Clean,
            Some(handle) if !handle.is_finished() => {
                *guard = Some(handle);
                WorkerExit::Running
            }
            Some(handle) => join_worker(handle),
        }
    }

    /// Takes the retained join handle so the caller can [`join_worker`] it **off** the UI thread.
    /// `None` once it has been taken (or for the worker-less test clients). Used by the window when
    /// [`worker_exit`](Self::worker_exit) answers [`WorkerExit::Running`]: the thread is by
    /// construction already on its way out (its event sender is dropped), so the join lands
    /// promptly — but it is still a park, and the UI thread must not take it.
    pub fn take_worker_handle(&self) -> Option<JoinHandle<()>> {
        self.join.lock().take()
    }

    /// The latest published viewport snapshot — a wait-free `arc_swap` load (the render loop's
    /// per-frame read; never blocks on the worker).
    pub fn publication(&self) -> Arc<Publication> {
        self.shared.publication.load_full()
    }

    /// The publication **swap container** itself (not a load) — the shape the grid's
    /// `GridDataSources` needs so the render path loads the latest snapshot wait-free each
    /// frame (`components/grid.md §Public interface`).
    pub fn publication_swap(&self) -> Arc<ArcSwap<Publication>> {
        Arc::clone(&self.shared.publication)
    }

    /// The resident style/geometry cache (populated in Phase 5).
    pub fn caches(&self) -> Arc<RwLock<SheetCaches>> {
        Arc::clone(&self.shared.caches)
    }

    /// The latest published live-bound charts (P9) — a wait-free `arc_swap` load. The UI reads this
    /// on `Loaded` / `Published` and installs it into the grid when its
    /// [`version`](crate::ChartSnapshot::version) changed.
    pub fn chart_snapshot(&self) -> Arc<ChartSnapshot> {
        self.shared.chart_snapshot.load_full()
    }

    /// Test-only: publish a [`ChartSnapshot`] into the shared swap, so a headless window/view test
    /// can drive the seam-fed chart install (its version-gating + dropped-sheet clear) without a
    /// real worker. Behind `test-support`, so it can never reach a release build.
    #[cfg(feature = "test-support")]
    pub fn set_chart_snapshot(&self, snapshot: ChartSnapshot) {
        self.shared.chart_snapshot.store(Arc::new(snapshot));
    }

    /// The published conditional-formatting rules for `sheet` (`architecture.md §4.5`) — a clone of
    /// the worker's latest `document.cond_fmt_rules(sheet)`, read under the shared read lock. Empty
    /// when the sheet carries no CF (the map holds no entry for a non-CF sheet). The window reads
    /// this on `Loaded` / `CondFmtUpdated` / sheet switch to build the sidebar rows.
    pub fn cond_fmt_rules(&self, sheet: SheetId) -> Vec<CfRuleView> {
        self.shared
            .cond_fmt
            .read()
            .get(&sheet)
            .cloned()
            .unwrap_or_default()
    }

    /// The current generation counter — the UI treats a change as "repaint from the
    /// publication".
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }

    /// The count of committed undoable ops (for the dirty flag). Acked against `Saved.ops_seen`
    /// on each save (`architecture.md §2`).
    pub fn committed_ops(&self) -> u64 {
        self.shared.committed_ops.load(Ordering::Acquire)
    }
}

/// The window's end of the worker→UI event channel. A thin wrapper that hides `async_channel`
/// and offers exactly the shapes the callers need: `recv().await` on the gpui foreground task,
/// and blocking / polling forms for headless tests.
pub struct WorkerEventReceiver {
    rx: async_channel::Receiver<WorkerEvent>,
}

impl WorkerEventReceiver {
    /// Awaits the next event (the gpui foreground task's `while let Some(ev) = rx.recv().await`
    /// loop). `None` once the worker has exited and the channel drained.
    pub async fn recv(&self) -> Option<WorkerEvent> {
        self.rx.recv().await.ok()
    }

    /// Blocks the current thread until the next event (or the channel closes → `None`).
    pub fn recv_blocking(&self) -> Option<WorkerEvent> {
        self.rx.recv_blocking().ok()
    }

    /// Returns the next event if one is already queued, else `None` (empty or closed).
    pub fn try_recv(&self) -> Option<WorkerEvent> {
        self.rx.try_recv().ok()
    }

    /// Polls for the next event up to `timeout`, returning `None` on timeout or channel close.
    /// Used by tests so a misbehaving worker fails the test instead of hanging it forever.
    pub fn recv_timeout(&self, timeout: Duration) -> Option<WorkerEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            match self.rx.try_recv() {
                Ok(ev) => return Some(ev),
                Err(async_channel::TryRecvError::Closed) => return None,
                Err(async_channel::TryRecvError::Empty) => {
                    if Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(Duration::from_millis(1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::worker::run::testutil::quiet_panics;
    use freecell_core::{CfPreview, CfRuleView};

    fn sample_rule() -> CfRuleView {
        CfRuleView {
            index: 0,
            range: "A1:A10".to_string(),
            priority: 1,
            editable: true,
            summary: "Cell value > 100".to_string(),
            preview: CfPreview::Highlight {
                fill: None,
                text_color: None,
            },
            spec: None,
        }
    }

    /// B1 (`functional_spec.md F2.3`): the window distinguishes "we asked the worker to stop"
    /// from "the worker died", so only the latter is reported as fatal. `Shutdown` is the only
    /// command that flips the flag.
    #[test]
    fn shutdown_requested_tracks_only_the_shutdown_command() {
        let (client, _rx) = DocumentClient::spawn(DocumentSource::NewWorkbook);
        assert!(!client.shutdown_requested(), "a fresh client has not asked");
        assert!(client.has_worker(), "a spawned client has a worker thread");
        client.send(Command::SetViewport {
            sheet: SheetId(0),
            rows: 0..8,
            cols: 0..8,
        });
        assert!(
            !client.shutdown_requested(),
            "an ordinary command is not a shutdown request"
        );
        client.send(Command::Shutdown);
        assert!(client.shutdown_requested());
    }

    /// The retained `JoinHandle` reports a clean exit after a requested shutdown. Before B1 the
    /// handle was dropped at spawn, so there was nothing to ask.
    #[test]
    fn worker_exit_reports_clean_after_a_requested_shutdown() {
        let (client, rx) = DocumentClient::spawn(DocumentSource::NewWorkbook);
        assert!(rx.recv_timeout(Duration::from_secs(10)).is_some(), "Loaded");
        client.send(Command::Shutdown);
        // The worker drops its sender on the way out, so a closed stream means it has exited.
        while rx.recv_timeout(Duration::from_secs(10)).is_some() {}
        assert_eq!(client.worker_exit(), WorkerExit::Clean);
        // The handle is taken by the first call; a second still answers rather than panicking.
        assert_eq!(client.worker_exit(), WorkerExit::Clean);
    }

    /// Waits (bounded) for `handle` to finish, so a `worker_exit` assertion about a *finished*
    /// thread isn't racing the thread's own unwind.
    fn wait_finished(handle: &JoinHandle<()>) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !handle.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(handle.is_finished(), "the thread did not finish in 10s");
    }

    fn client_over(join: Option<JoinHandle<()>>) -> DocumentClient {
        let (tx, _rx) = mpsc::channel();
        DocumentClient {
            tx,
            shared: Arc::new(Shared::new(SheetId(0))),
            join: Mutex::new(join),
            shutdown_requested: AtomicBool::new(false),
            has_worker: true,
        }
    }

    /// B1 (`functional_spec.md F2.3`): the `Err(_) => Panicked` arm — the one this type exists for
    /// — was never asserted before. Every panic site inside `load_and_run` is guarded today, so
    /// the reachable route to it is a *future* unguarded region; a thread that unwinds stands in
    /// for exactly that, and exercises the same join.
    #[test]
    fn worker_exit_reports_a_thread_that_unwound() {
        // The panic hook is process-global, so it is swapped from the PARENT thread with the
        // shared `quiet_panics` helper, around both the spawn and the wait — a hook swapped
        // inside the spawned thread would race every other test's panics.
        let handle = quiet_panics(|| {
            let handle = std::thread::spawn(|| panic!("stand-in for an unguarded worker panic"));
            wait_finished(&handle);
            handle
        });
        let client = client_over(Some(handle));
        assert_eq!(client.worker_exit(), WorkerExit::Panicked);
    }

    /// A thread that has not finished reports `Running` **and keeps its handle**, so the caller can
    /// take it and join off the UI thread. This is the normal answer at the moment the event stream
    /// closes (the sender drops before the thread finishes unwinding), which is why `Running` must
    /// not be a dead end.
    #[test]
    fn a_running_worker_reports_running_and_stays_joinable() {
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let handle = std::thread::spawn(move || {
            let _ = release_rx.recv();
        });
        let client = client_over(Some(handle));

        assert_eq!(
            client.worker_exit(),
            WorkerExit::Running,
            "a thread still running is reported, not waited on"
        );

        let handle = client
            .take_worker_handle()
            .expect("Running must not consume the handle");
        assert!(
            client.take_worker_handle().is_none(),
            "the handle is taken exactly once"
        );
        let _ = release_tx.send(());
        assert_eq!(
            join_worker(handle),
            WorkerExit::Clean,
            "joining off the UI thread reports the real outcome"
        );
    }

    /// The worker-less test client has no thread, and says so **without** claiming a shutdown was
    /// requested — the two questions the window asks are separate flags. Gated on `test-support`
    /// with the constructors it asserts about (`cargo test --all-features`).
    #[cfg(feature = "test-support")]
    #[test]
    fn the_worker_less_client_reports_no_worker_rather_than_a_shutdown() {
        let (client, _rx) = DocumentClient::detached();
        assert!(!client.has_worker());
        assert!(!client.shutdown_requested());
        let (client, _rx, _tx) = DocumentClient::detached_live();
        assert!(
            client.has_worker(),
            "detached_live simulates a real worker, so its stream close is fatal"
        );
        assert!(!client.shutdown_requested());
    }

    #[test]
    fn cond_fmt_rules_reads_published_map() {
        // A `DocumentClient` reads the CF rules the worker published into `Shared::cond_fmt`.
        let shared = Arc::new(Shared::new(SheetId(0)));
        shared
            .cond_fmt
            .write()
            .insert(SheetId(7), vec![sample_rule()]);
        let (tx, _rx) = mpsc::channel();
        let client = DocumentClient {
            tx,
            shared,
            join: Mutex::new(None),
            shutdown_requested: AtomicBool::new(false),
            has_worker: true,
        };

        let rules = client.cond_fmt_rules(SheetId(7));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].range, "A1:A10");
        // A sheet with no published entry reads empty (a non-CF sheet holds no map entry).
        assert!(client.cond_fmt_rules(SheetId(0)).is_empty());
    }
}
