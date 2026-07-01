//! # Investigation D — engine-robustness probes (functional_spec §6-D, architecture §5)
//!
//! FreeCell recomputes the **whole workbook** on every edit, on the SP1 worker thread
//! that owns the `Model` (`experiments/round-2/01-async-interop`). So three engine
//! behaviors gate the build:
//!
//! 1. **Circular references** must return a typed error and **not hang / stack-overflow**
//!    (a cycle mid-recompute would lock the app). GATE.
//! 2. **Malformed / pathological input** must yield a typed error, **not a panic**.
//! 3. **Worker-panic recovery** — if `evaluate()` (or an apply) can die on bad input,
//!    does the worker survive? What recovery is needed?
//!
//! This module exposes small, directly-assertable probes over `ironcalc_base::Model`
//! (0.7.1) plus two isolation primitives — a bounded-**thread** runner (contains unwind
//! panics) and a **child-process** runner (contains *aborts* like a stack overflow, which
//! `catch_unwind` cannot catch). The tests in `tests/robustness.rs` assert against these;
//! the binary (`src/main.rs`) runs them foreground and writes an env-stamped summary.
//!
//! ## Empirically-confirmed IronCalc 0.7.1 behavior (see `findings.md` for citations)
//! - Circular refs (self / mutual / long ring) → `#CIRC!`, `CellType::ErrorValue`, no hang
//!   (marker-based cycle guard, `model.rs:801-848`).
//! - Invalid formulas → `#ERROR!` / typed error on `evaluate()`, **no unwind panic**
//!   observed across an adversarial corpus. `set_user_input` accepts them without `Err`.
//! - The **one** crash mode is a **stack overflow** from deep recursion in the parser — via
//!   deeply-nested parentheses **or** a very long flat operator chain (`=1+1+…`); the
//!   recursive-descent parser has no depth cap. It is a process **abort** (uncatchable by
//!   `catch_unwind`). The ceiling scales ~linearly with stack size: nested parens ~340
//!   depth/MiB (~490 on a default 2 MiB worker thread, ~2637 on the 8 MiB main thread);
//!   flat chains ~1465 terms/MiB (~2832 @2 MiB, ~11.8k @8 MiB). Mitigation = a pre-eval
//!   input cap (+ optionally a larger worker stack); see `findings.md`.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use ironcalc_base::types::CellType;
use ironcalc_base::Model;

/// Sheet 0, 1-based IronCalc coordinates for A1.
const SHEET: u32 = 0;

/// What one probed cell looked like after `evaluate()`.
#[derive(Debug, Clone, PartialEq)]
pub struct CellOutcome {
    /// `TYPE()`-style classification. `ErrorValue` is the typed-error signal.
    pub is_error: bool,
    /// The cell's read-back string (an error cell reads back its `#…!` token).
    pub value_string: String,
}

impl CellOutcome {
    fn read(model: &Model, row: i32, col: i32) -> Self {
        let cell_type = model
            .get_cell_type(SHEET, row, col)
            .expect("get_cell_type on a valid sheet");
        let value = model
            .get_cell_value_by_index(SHEET, row, col)
            .expect("get_cell_value_by_index on a valid sheet");
        CellOutcome {
            is_error: cell_type == CellType::ErrorValue,
            value_string: value_to_string(value),
        }
    }
}

fn value_to_string(v: ironcalc_base::cell::CellValue) -> String {
    use ironcalc_base::cell::CellValue;
    match v {
        CellValue::None => String::new(),
        CellValue::String(s) => s,
        CellValue::Number(n) => format!("{n}"),
        CellValue::Boolean(b) => b.to_string(),
    }
}

fn fresh_model() -> Model<'static> {
    Model::new_empty("robustness", "en", "UTC", "en").expect("new_empty")
}

/// Writes `input` at A1 on a fresh model, evaluates, and reads A1 back. This is the core
/// "does bad input become a typed error, not a panic?" probe (malformed / pathological).
pub fn error_probe(input: &str) -> CellOutcome {
    let mut model = fresh_model();
    // `set_user_input` accepts malformed formulas without `Err` (stored as a formula that
    // parses to an error node — confirmed empirically); the error surfaces on evaluate().
    let _ = model.set_user_input(SHEET, 1, 1, input.to_string());
    model.evaluate();
    CellOutcome::read(&model, 1, 1)
}

/// The three circular-reference shapes the spec names.
#[derive(Debug, Clone, Copy)]
pub enum CycleKind {
    /// `A1 = A1`.
    SelfRef,
    /// `A1 = B1`, `B1 = A1`.
    Mutual,
    /// A ring of `n` cells: `A1=A2, A2=A3, …, A(n)=A1` (`n >= 2`).
    Ring(usize),
}

/// Builds the cycle, evaluates, and returns the outcome at the head cell (A1). The GATE
/// property is: `is_error == true` (a `#CIRC!`) and the call **returns at all** (no hang —
/// the caller runs this under an isolation deadline to make a hang observable).
pub fn cycle_probe(kind: CycleKind) -> CellOutcome {
    let mut model = fresh_model();
    match kind {
        CycleKind::SelfRef => {
            model
                .set_user_input(SHEET, 1, 1, "=A1".to_string())
                .unwrap();
        }
        CycleKind::Mutual => {
            model
                .set_user_input(SHEET, 1, 1, "=B1".to_string())
                .unwrap();
            model
                .set_user_input(SHEET, 1, 2, "=A1".to_string())
                .unwrap();
        }
        CycleKind::Ring(n) => {
            assert!(n >= 2, "a ring needs >= 2 cells");
            for i in 1..=n as i32 {
                let next = if i == n as i32 { 1 } else { i + 1 };
                model
                    .set_user_input(SHEET, i, 1, format!("=A{next}"))
                    .unwrap();
            }
        }
    }
    model.evaluate();
    CellOutcome::read(&model, 1, 1)
}

/// A giant flat formula `=1+1+…+1` with `terms` ones (evaluates to `terms`).
pub fn wide_add(terms: usize) -> String {
    let mut s = String::with_capacity(terms * 2 + 1);
    s.push('=');
    for i in 0..terms {
        if i > 0 {
            s.push('+');
        }
        s.push('1');
    }
    s
}

/// A deeply-nested-parentheses formula `=((((1))))` at the given nesting `depth`. The
/// prime stack-overflow candidate (the parser recurses per level, no depth cap).
pub fn nested_parens(depth: usize) -> String {
    format!("={}1{}", "(".repeat(depth), ")".repeat(depth))
}

// ---------------------------------------------------------------------------------------
// Isolation primitives: make a hang or an abort an *observed finding*, not a wedged run.
// ---------------------------------------------------------------------------------------

/// Outcome of running a closure under an isolation boundary.
#[derive(Debug, Clone, PartialEq)]
pub enum Isolated<T> {
    /// Finished within the deadline and did not panic/abort. Carries the value.
    Completed(T),
    /// Did not finish within the deadline (a hang).
    TimedOut,
    /// Unwound with a panic that `catch_unwind` caught (message best-effort).
    Panicked(String),
}

/// Runs `f` on a spawned thread with an explicit `stack_bytes` stack and a join `deadline`.
/// - A **panic** inside `f` is caught (→ `Panicked`) — this is what an unwinding
///   `evaluate()` would produce (none observed, but we prove the containment works).
/// - A **hang** shows up as `TimedOut` (the thread is detached and leaked; acceptable in a
///   one-shot probe — the point is the *finding*, and the harness exits after reporting).
///
/// NOTE: a **stack overflow is an abort, not a panic** — it is NOT contained here. Use
/// [`run_nested_parens_in_subprocess`] for inputs that can overflow (deep nesting).
pub fn run_in_bounded_thread<T, F>(stack_bytes: usize, deadline: Duration, f: F) -> Isolated<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = mpsc::channel::<Result<T, String>>();
    let builder = thread::Builder::new().stack_size(stack_bytes);
    let _handle = builder
        .spawn(move || {
            let r = catch_unwind(AssertUnwindSafe(f)).map_err(|e| panic_message(&e));
            let _ = tx.send(r);
        })
        .expect("spawn bounded thread");

    match rx.recv_timeout(deadline) {
        Ok(Ok(v)) => Isolated::Completed(v),
        Ok(Err(msg)) => Isolated::Panicked(msg),
        Err(_) => Isolated::TimedOut, // hang: thread leaked, reported as a finding
    }
}

fn panic_message(e: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = e.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = e.downcast_ref::<String>() {
        s.clone()
    } else {
        "<non-string panic payload>".to_string()
    }
}

/// Result of a child-process isolation run: `true` == the child exited successfully
/// (no abort), plus the raw status text for the record.
#[derive(Debug, Clone, PartialEq)]
pub struct SubprocessOutcome {
    pub survived: bool,
    pub status: String,
}

/// The two recursion-vector shapes we probe for stack overflow. Both drive IronCalc's
/// recursive-descent parser (and recursive evaluator): nesting recurses via the
/// paren/primary rule; a long flat operator chain recurses via `parse_expr`/`parse_term`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecursionShape {
    /// `=((((1))))` at `size` nesting depth.
    NestedParens,
    /// `=1+1+…+1` with `size` terms (a long left-associative operator chain).
    WideFlat,
}

impl RecursionShape {
    fn arg(self) -> &'static str {
        match self {
            RecursionShape::NestedParens => "--nested-parens",
            RecursionShape::WideFlat => "--wide-flat",
        }
    }
    fn build(self, size: usize) -> String {
        match self {
            RecursionShape::NestedParens => nested_parens(size),
            RecursionShape::WideFlat => wide_add(size),
        }
    }
}

/// Runs the `robustness` binary with the shape's subcommand + `size` in a child process
/// and reports whether it survived. This is the ONLY safe way to probe a stack-overflow
/// (abort) input: an abort in-process would kill the caller; in a child it is just a
/// non-success exit we observe. The child (`main.rs`) dispatches the subcommand to
/// [`recursion_child`].
///
/// **The child MUST be the `robustness` binary, not `current_exe()`** — under `cargo test`
/// `current_exe()` is the *test* harness, which has no subcommand dispatch. Callers pass
/// the binary path: the binary uses its own `current_exe()`; the integration test passes
/// `env!("CARGO_BIN_EXE_robustness")` (see the `robustness_bin()` helper in the test).
pub fn run_recursion_in_subprocess_with(
    exe: &std::path::Path,
    shape: RecursionShape,
    size: usize,
) -> SubprocessOutcome {
    let status = Command::new(exe)
        .arg(shape.arg())
        .arg(size.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("spawn child process");
    SubprocessOutcome {
        survived: status.success(),
        status: format!("{status}"),
    }
}

/// Convenience wrapper: re-execute `current_exe()`. Correct **only when the caller is the
/// `robustness` binary itself** (whose `current_exe()` has the subcommand dispatch). The
/// integration test must NOT use this (its `current_exe()` is the test harness) — it calls
/// [`run_recursion_in_subprocess_with`] passing the built binary path instead.
pub fn run_recursion_in_subprocess(shape: RecursionShape, size: usize) -> SubprocessOutcome {
    let exe = std::env::current_exe().expect("current_exe");
    run_recursion_in_subprocess_with(&exe, shape, size)
}

/// The child-process body: parse+evaluate the shape at `size`. An overflow aborts the child
/// (observed by the parent); otherwise it returns after forcing a read.
pub fn recursion_child(shape: RecursionShape, size: usize) {
    let mut model = fresh_model();
    let _ = model.set_user_input(SHEET, 1, 1, shape.build(size));
    model.evaluate();
    // Force a read so the parse/eval isn't optimized away, then report.
    let out = CellOutcome::read(&model, 1, 1);
    println!("{shape:?} child size={size} is_error={}", out.is_error);
}

// ---------------------------------------------------------------------------------------
// Worker-panic-recovery experiment: the SP1-shaped seam under adversarial input.
// ---------------------------------------------------------------------------------------

/// Result of driving the SP1-shaped worker with adversarial then good input.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkerRecovery {
    /// Did the adversarial `evaluate()` unwind-panic (caught by the worker)?
    pub adversarial_panicked: bool,
    /// After the adversarial input, did a subsequent GOOD edit still evaluate correctly?
    /// (Proves the worker thread + model survived / recovered.)
    pub recovered: bool,
    /// The value the good edit produced (for assertion).
    pub post_recovery_value: String,
}

/// A minimal SP1-shaped worker: it owns a `Model` on a spawned thread and applies edits
/// (`set_user_input` + `evaluate`) **inside `catch_unwind`**. We feed it `adversarial`
/// first, then a known-good `=2+3`, and check the good result comes back — i.e. a bad
/// eval did not poison the worker.
///
/// This mirrors the real seam's guarantee we want: an unwinding eval must not take down
/// the worker. (Empirically no user input unwinds `evaluate()`, so `recovered` is true and
/// `adversarial_panicked` is false — evidence for the "evaluate can't panic on user input,
/// but wrap it in catch_unwind for defense-in-depth" recommendation in `findings.md`.)
pub fn worker_recovery_probe(adversarial: &str) -> WorkerRecovery {
    struct Job {
        input: String,
        // Where to write the (row, col) result read-back; None => shutdown.
        reply: Option<Sender<WorkerReply>>,
    }
    struct WorkerReply {
        panicked: bool,
        value: String,
    }

    let (job_tx, job_rx): (Sender<Job>, Receiver<Job>) = mpsc::channel();
    // Track whether the worker thread is still alive to serve a second job.
    let alive = Arc::new(Mutex::new(true));
    let alive_worker = Arc::clone(&alive);

    let handle = thread::spawn(move || {
        let mut model = fresh_model();
        while let Ok(job) = job_rx.recv() {
            let Some(reply) = job.reply else { break };
            // Apply + evaluate inside catch_unwind: an unwinding eval is contained here,
            // the worker loop lives on to serve the next job.
            let input = job.input.clone();
            let result = catch_unwind(AssertUnwindSafe(|| {
                let _ = model.set_user_input(SHEET, 1, 1, input);
                model.evaluate();
                CellOutcome::read(&model, 1, 1).value_string
            }));
            let (panicked, value) = match result {
                Ok(v) => (false, v),
                Err(_) => (true, String::new()),
            };
            let _ = reply.send(WorkerReply { panicked, value });
        }
        *alive_worker.lock().unwrap() = false;
    });

    // 1) adversarial job
    let (r1_tx, r1_rx) = mpsc::channel();
    job_tx
        .send(Job {
            input: adversarial.to_string(),
            reply: Some(r1_tx),
        })
        .expect("send adversarial job");
    let r1 = r1_rx.recv().expect("worker replies to adversarial job");

    // 2) known-good job: =2+3 => 5. If the worker survived, this comes back.
    let (r2_tx, r2_rx) = mpsc::channel();
    job_tx
        .send(Job {
            input: "=2+3".to_string(),
            reply: Some(r2_tx),
        })
        .expect("send good job");
    let good = r2_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worker still alive to serve the good job");

    // shutdown
    let _ = job_tx.send(Job {
        input: String::new(),
        reply: None,
    });
    let _ = handle.join();

    WorkerRecovery {
        adversarial_panicked: r1.panicked,
        recovered: good.value == "5",
        post_recovery_value: good.value,
    }
}

/// Bisects the `size` at which `shape` overflows the child process's **default** stack,
/// using child-process isolation for each probe. Returns `(ok_upto, aborts_by)`. If `hi`
/// still survives, returns `(hi, usize::MAX)` (a lower bound). Cost-bounded: the bisection
/// makes O(log(hi-lo)) child launches so it stays foreground.
pub fn find_overflow_ceiling(
    shape: RecursionShape,
    lo_ok: usize,
    hi_start: usize,
) -> (usize, usize) {
    let mut lo = lo_ok;
    let mut hi = hi_start;
    if run_recursion_in_subprocess(shape, hi).survived {
        return (hi, usize::MAX);
    }
    while hi - lo > 50 {
        let mid = lo + (hi - lo) / 2;
        if run_recursion_in_subprocess(shape, mid).survived {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (lo, hi)
}

/// A tiny stopwatch wrapper so the binary can report that cycles resolve fast.
pub fn time_ms<T, F: FnOnce() -> T>(f: F) -> (T, f64) {
    let start = Instant::now();
    let v = f();
    (v, start.elapsed().as_secs_f64() * 1000.0)
}
