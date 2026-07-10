//! `DocumentClient` — the cheap, `Send`-able handle the window keeps, plus the shared
//! read-surfaces the worker writes and the UI reads (`components/engine_worker.md §Public
//! interface`, `architecture.md §2`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use arc_swap::ArcSwap;
use freecell_core::{Publication, SheetCaches, SheetId};
use parking_lot::RwLock;

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
}

impl Shared {
    pub(super) fn new(initial_sheet: SheetId) -> Self {
        Self {
            publication: Arc::new(ArcSwap::from_pointee(Publication::empty(initial_sheet, 0))),
            generation: AtomicU64::new(0),
            committed_ops: AtomicU64::new(0),
            caches: Arc::new(RwLock::new(SheetCaches::new())),
            chart_snapshot: Arc::new(ArcSwap::from_pointee(ChartSnapshot::empty())),
        }
    }
}

/// The window's handle to its worker: send commands, read the latest published snapshot,
/// generation, committed-op count, and the resident cache. Cloning is intentionally **not**
/// derived — one window owns one worker; the handle carries `Arc`s internally.
pub struct DocumentClient {
    tx: Sender<Command>,
    shared: Arc<Shared>,
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

        std::thread::Builder::new()
            .name("eval-worker".to_string())
            .stack_size(WORKER_STACK_SIZE)
            .spawn(move || Worker::load_and_run(source, worker_shared, event_tx, rx))
            .expect("spawn eval-worker thread");

        (
            DocumentClient { tx, shared },
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
            DocumentClient { tx, shared },
            WorkerEventReceiver { rx: event_rx },
        )
    }

    /// Sends a command to the worker. Non-blocking and infallible to the caller: if the worker
    /// is gone the send is dropped (the UI observes the closed event channel instead).
    pub fn send(&self, cmd: Command) {
        let _ = self.tx.send(cmd);
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
