//! The **locked engine↔render interop seam** (functional_spec SP1, architecture §4).
//!
//! ## Why this shape (justified by IronCalc 0.7.1's real API)
//!
//! - `Model::evaluate(&mut self)` is a full-workbook, non-incremental, non-interruptible
//!   recompute with **no** progress/cancel/callback. It takes `&mut self`, so it is
//!   inherently **one-at-a-time** and **cannot overlap a read of the same model** (Rust
//!   aliasing). It therefore must run somewhere the render loop never touches.
//! - `Model<'static>` is **`Send`** (asserted at compile time in [`assert_model_send`]),
//!   so it can be **moved to a worker thread**. This is the vehicle for "non-blocking":
//!   the worker owns the model; the render loop owns nothing that eval touches.
//! - IronCalc exposes **no evaluated-cell diff / change stream** — the `UserModel`
//!   diff-list carries only *edit-sites*, never the cascaded downstream cells (see
//!   [`crate::probes`]). So the renderer cannot be told "these 30 cells changed"; it
//!   must **re-pull the visible viewport** after each eval. The locked change-propagation
//!   is therefore **publish-on-completion**: the worker, immediately after an eval,
//!   reads the current visible viewport and publishes a **small** value snapshot the
//!   render loop consumes.
//!
//! ## The seam
//!
//! ```text
//!   render loop (main)                          eval worker (spawned)
//!   ─────────────────                           ─────────────────────
//!   enqueue EditBatch ───────── channel ──────► drain+coalesce edits
//!   set visible viewport ────── shared slot ──► apply latest inputs
//!   each tick: read latest ◄─── shared slot ─── run ONE evaluate()
//!     published viewport (cheap)                read visible viewport
//!                                               publish snapshot ─────┘
//! ```
//!
//! The render tick's synchronous work is **only** O(viewport): read the published
//! viewport snapshot under a short-held lock and copy it out. It **never** calls
//! `evaluate()`, never reads the big model, and never blocks on the worker — so it stays
//! under one frame even while a 10⁶–10⁷ eval runs (GATE 1). Rapid edits **coalesce**:
//! the worker drains everything queued and runs a single eval per settle (GATE 2).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{mpsc, Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

use ironcalc_base::cell::CellValue;
use ironcalc_base::Model;
use round2_harness::Viewport;

/// Compile-time proof that `Model<'static>` is `Send` — the authoritative answer to the
/// "can eval be moved to a worker thread?" API question. If a future IronCalc version
/// added a non-`Send` field, this stops compiling.
pub fn assert_model_send() {
    fn assert_send<T: Send>() {}
    assert_send::<Model<'static>>();
}

/// One rendered cell value (the small projection the render loop consumes). Kept
/// deliberately minimal — value only — so a published viewport snapshot is cheap to
/// build and copy regardless of the full model's size.
#[derive(Debug, Clone, PartialEq)]
pub enum CellSnapshot {
    Empty,
    Number(f64),
    Text(String),
    Bool(bool),
}

impl CellSnapshot {
    fn from_cell_value(v: CellValue) -> Self {
        match v {
            CellValue::None => CellSnapshot::Empty,
            CellValue::Number(n) => CellSnapshot::Number(n),
            CellValue::String(s) => CellSnapshot::Text(s),
            CellValue::Boolean(b) => CellSnapshot::Bool(b),
        }
    }

    /// Numeric content, if any (used by tests/staleness checks).
    pub fn as_number(&self) -> Option<f64> {
        match self {
            CellSnapshot::Number(n) => Some(*n),
            _ => None,
        }
    }
}

/// A published visible-viewport snapshot: the values the render loop paints, plus the
/// generation (eval count at publish time) so the loop can tell fresh from stale.
#[derive(Debug, Clone, Default)]
pub struct PublishedViewport {
    /// The viewport these values cover.
    pub viewport: Option<Viewport>,
    /// Row-major values for `viewport`.
    pub values: Vec<CellSnapshot>,
    /// The worker's eval generation at publish time.
    pub generation: u64,
}

/// One edit the render loop hands the worker: set an input at a 0-based `(row,col)`.
#[derive(Debug, Clone)]
pub struct Edit {
    pub sheet: u32,
    pub row: u32,
    pub col: u32,
    pub input: String,
    /// When the edit was enqueued (for the staleness-window measurement).
    pub enqueued_at: Instant,
}

/// Commands the render loop sends the worker.
enum Command {
    /// Apply an edit (deferred; coalesced with others before a single eval).
    Edit(Edit),
    /// Change the visible viewport the worker republishes after each eval.
    SetViewport(Viewport),
    /// Stop the worker loop.
    Shutdown,
}

/// Shared state the render loop reads without blocking on the worker.
struct Shared {
    /// The latest published viewport snapshot (small; O(viewport)).
    published: Mutex<PublishedViewport>,
    /// Number of `evaluate()` calls the worker has run (coalescing GATE + generation).
    eval_count: AtomicU64,
    /// True while the worker is inside an `evaluate()` (drives the "recalculating…"
    /// indicator; the render loop never blocks on it).
    evaluating: AtomicBool,
    /// The most recent edit's enqueue time and the generation that first reflects it,
    /// for the staleness-window discovery. `0` generation = not yet reflected.
    last_edit_at: Mutex<Option<Instant>>,
    /// Generation at which the last tracked edit first became visible (0 = pending).
    last_edit_visible_gen: AtomicU64,
}

/// A worker thread that owns the authoritative `Model` and runs all evaluation, keeping
/// the render loop non-blocking. Created with a model + an initial viewport.
pub struct EvalWorker {
    tx: Sender<Command>,
    shared: Arc<Shared>,
    handle: Option<JoinHandle<Model<'static>>>,
}

impl EvalWorker {
    /// Spawns the worker, moving `model` onto it (proof it's `Send`). `viewport` is the
    /// initial visible window republished after each eval. An initial eval + publish
    /// runs so the render loop paints last-known values immediately.
    pub fn spawn(model: Model<'static>, viewport: Viewport) -> Self {
        let (tx, rx) = mpsc::channel::<Command>();
        let shared = Arc::new(Shared {
            published: Mutex::new(PublishedViewport::default()),
            eval_count: AtomicU64::new(0),
            evaluating: AtomicBool::new(false),
            last_edit_at: Mutex::new(None),
            last_edit_visible_gen: AtomicU64::new(0),
        });
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::spawn(move || worker_loop(model, viewport, rx, worker_shared));
        Self {
            tx,
            shared,
            handle: Some(handle),
        }
    }

    /// Enqueues an edit (non-blocking send). The worker coalesces it with any other
    /// queued edits before running a single eval.
    pub fn enqueue_edit(&self, edit: Edit) {
        // Track the latest edit for the staleness window.
        *self.shared.last_edit_at.lock().unwrap() = Some(edit.enqueued_at);
        self.shared.last_edit_visible_gen.store(0, Ordering::SeqCst);
        let _ = self.tx.send(Command::Edit(edit));
    }

    /// Updates the visible viewport the worker republishes.
    pub fn set_viewport(&self, viewport: Viewport) {
        let _ = self.tx.send(Command::SetViewport(viewport));
    }

    /// Reads the latest published viewport snapshot. **Cheap and non-blocking** — this
    /// is the render loop's per-tick read (O(viewport), a short-held lock). Returns a
    /// clone so the loop holds no lock while painting.
    pub fn latest_published(&self) -> PublishedViewport {
        self.shared.published.lock().unwrap().clone()
    }

    /// Number of full `evaluate()` runs so far (the coalescing GATE metric).
    pub fn eval_count(&self) -> u64 {
        self.shared.eval_count.load(Ordering::SeqCst)
    }

    /// Whether the worker is currently inside an eval (for the "recalculating…" UX).
    pub fn is_evaluating(&self) -> bool {
        self.shared.evaluating.load(Ordering::SeqCst)
    }

    /// The generation at which the last tracked edit first became visible (0 = not yet).
    pub fn last_edit_visible_gen(&self) -> u64 {
        self.shared.last_edit_visible_gen.load(Ordering::SeqCst)
    }

    /// The enqueue time of the last tracked edit, if any.
    pub fn last_edit_at(&self) -> Option<Instant> {
        *self.shared.last_edit_at.lock().unwrap()
    }

    /// Shuts the worker down and returns the model (so callers can measure snapshot
    /// cost, etc.). Blocks until the worker's current eval (if any) finishes — used only
    /// at teardown, never on the render path.
    pub fn shutdown(mut self) -> Model<'static> {
        let _ = self.tx.send(Command::Shutdown);
        self.handle
            .take()
            .expect("handle present")
            .join()
            .expect("worker thread panicked")
    }
}

impl Drop for EvalWorker {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = self.tx.send(Command::Shutdown);
            let _ = handle.join();
        }
    }
}

/// Reads the current visible viewport from the model into a small value snapshot.
fn read_viewport(model: &Model<'static>, viewport: Viewport) -> Vec<CellSnapshot> {
    viewport
        .addresses()
        .map(|(r, c)| {
            match model.get_cell_value_by_index(0, (r + 1) as i32, (c + 1) as i32) {
                Ok(v) => CellSnapshot::from_cell_value(v),
                Err(_) => CellSnapshot::Empty,
            }
        })
        .collect()
}

/// Publishes the current visible viewport to the shared slot at the given generation.
fn publish(model: &Model<'static>, viewport: Viewport, generation: u64, shared: &Shared) {
    let values = read_viewport(model, viewport);
    let mut slot = shared.published.lock().unwrap();
    *slot = PublishedViewport {
        viewport: Some(viewport),
        values,
        generation,
    };
}

/// The worker's main loop: apply the current model, and on each settle run **one**
/// coalesced eval, then publish the visible viewport.
fn worker_loop(
    mut model: Model<'static>,
    mut viewport: Viewport,
    rx: Receiver<Command>,
    shared: Arc<Shared>,
) -> Model<'static> {
    // Initial eval + publish so the render loop paints last-known values immediately.
    run_eval_and_publish(&mut model, viewport, &shared);

    loop {
        // Block for the next command (idle worker parks; no busy-wait).
        let first = match rx.recv() {
            Ok(cmd) => cmd,
            Err(_) => break, // all senders dropped
        };

        let mut had_edit = false;
        let mut shutdown = false;
        let mut apply = |cmd: Command,
                         model: &mut Model<'static>,
                         viewport: &mut Viewport,
                         had_edit: &mut bool,
                         shutdown: &mut bool| {
            match cmd {
                Command::Edit(edit) => {
                    let _ = model.set_user_input(
                        edit.sheet,
                        (edit.row + 1) as i32,
                        (edit.col + 1) as i32,
                        edit.input,
                    );
                    *had_edit = true;
                }
                Command::SetViewport(vp) => *viewport = vp,
                Command::Shutdown => *shutdown = true,
            }
        };

        apply(first, &mut model, &mut viewport, &mut had_edit, &mut shutdown);

        // COALESCE: drain everything else already queued before evaluating, so a burst
        // of N rapid edits collapses into a single eval (GATE 2).
        loop {
            match rx.try_recv() {
                Ok(cmd) => apply(cmd, &mut model, &mut viewport, &mut had_edit, &mut shutdown),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    shutdown = true;
                    break;
                }
            }
        }

        if had_edit {
            run_eval_and_publish(&mut model, viewport, &shared);
            // Record the generation that first reflects the tracked edit (staleness).
            let gen = shared.eval_count.load(Ordering::SeqCst);
            // Only set if a pending edit is waiting (0 == pending).
            let _ = shared.last_edit_visible_gen.compare_exchange(
                0,
                gen,
                Ordering::SeqCst,
                Ordering::SeqCst,
            );
        } else {
            // A viewport change with no edit: republish current values at the same gen.
            let gen = shared.eval_count.load(Ordering::SeqCst);
            publish(&model, viewport, gen, &shared);
        }

        if shutdown {
            break;
        }
    }

    model
}

/// Runs one full eval (setting/clearing the `evaluating` flag around it), bumps the
/// generation, and publishes the visible viewport.
fn run_eval_and_publish(model: &mut Model<'static>, viewport: Viewport, shared: &Shared) {
    shared.evaluating.store(true, Ordering::SeqCst);
    model.evaluate();
    let gen = shared.eval_count.fetch_add(1, Ordering::SeqCst) + 1;
    shared.evaluating.store(false, Ordering::SeqCst);
    publish(model, viewport, gen, shared);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes::{build, Shape};
    use std::time::Duration;

    fn wait_until<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if cond() {
                return true;
            }
            std::thread::yield_now();
        }
        cond()
    }

    #[test]
    fn model_send_compiles() {
        assert_model_send(); // presence == proof
    }

    #[test]
    fn worker_publishes_initial_values() {
        let built = build(Shape::DeepSerial, 10);
        let vp = Viewport::new(9, 0, 1, 1); // just the tail cell (A10)
        let worker = EvalWorker::spawn(built.model, vp);
        assert!(
            wait_until(|| worker.eval_count() >= 1, Duration::from_secs(5)),
            "worker should run its initial eval"
        );
        let pub_vp = worker.latest_published();
        assert_eq!(pub_vp.values.len(), 1);
        assert_eq!(pub_vp.values[0].as_number(), Some(10.0));
        let _ = worker.shutdown();
    }

    #[test]
    fn worker_reflects_an_edit() {
        let built = build(Shape::DeepSerial, 10);
        let vp = Viewport::new(9, 0, 1, 1);
        let worker = EvalWorker::spawn(built.model, vp);
        assert!(wait_until(|| worker.eval_count() >= 1, Duration::from_secs(5)));

        // Bump the head A1 (=1) to =5: tail A10 should become 5 + 9 = 14.
        worker.enqueue_edit(Edit {
            sheet: 0,
            row: 0,
            col: 0,
            input: "5".to_string(),
            enqueued_at: Instant::now(),
        });
        assert!(wait_until(
            || worker
                .latest_published()
                .values
                .first()
                .and_then(|c| c.as_number())
                == Some(14.0),
            Duration::from_secs(5)
        ));
        let _ = worker.shutdown();
    }

    #[test]
    fn rapid_edits_coalesce_to_few_evals() {
        // A model big enough that an eval takes long enough for a burst to queue behind
        // it, but small enough for a fast test.
        let built = build(Shape::DeepSerial, 50_000);
        let vp = Viewport::new(49_999, 0, 1, 1);
        let worker = EvalWorker::spawn(built.model, vp);
        assert!(wait_until(|| worker.eval_count() >= 1, Duration::from_secs(10)));
        let base = worker.eval_count();

        // Fire 30 rapid edits.
        for i in 0..30u32 {
            worker.enqueue_edit(Edit {
                sheet: 0,
                row: 0,
                col: 0,
                input: format!("{}", i + 1),
                enqueued_at: Instant::now(),
            });
        }
        // Let the worker settle.
        assert!(wait_until(
            || !worker.is_evaluating()
                && worker.latest_published().generation >= worker.eval_count(),
            Duration::from_secs(20)
        ));
        std::thread::sleep(Duration::from_millis(50));

        let extra_evals = worker.eval_count() - base;
        // Coalescing: far fewer than 30 evals. Bound generously to avoid flakiness on a
        // loaded CI box, but assert it collapsed (the GATE asserts a tighter bound in
        // the binary under controlled load).
        assert!(
            extra_evals <= 5,
            "30 rapid edits should coalesce to <=5 evals, got {extra_evals}"
        );
        let _ = worker.shutdown();
    }

    #[test]
    fn shutdown_returns_model() {
        let built = build(Shape::Sparse, 100);
        let vp = Viewport::new(0, 0, 2, 2);
        let worker = EvalWorker::spawn(built.model, vp);
        assert!(wait_until(|| worker.eval_count() >= 1, Duration::from_secs(5)));
        let model = worker.shutdown();
        // The returned model is usable: to_bytes works (snapshot route).
        assert!(!model.to_bytes().is_empty());
    }
}
