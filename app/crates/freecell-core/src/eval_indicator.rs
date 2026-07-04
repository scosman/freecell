//! The action-row **evaluating spinner** state machine as a pure reducer
//! (`components/app_shell.md §Data row` "Evaluating spinner", `ui_design.md §3.1`,
//! `functional_spec.md §4`).
//!
//! The spinner shows only when an evaluation has been in flight **> 250 ms** — the same
//! no-flash rule as the data-row content-fetch spinner ([`crate::data_row`]): a fast
//! recalc never flashes it, a slow one shows it until the eval finishes. Extracted here
//! (GPUI-free) so the timing logic is table-tested headless on Linux; the window arms a
//! 250 ms one-shot gpui timer on [`EvalEffect::ArmTimer`] and toggles the widget on
//! [`EvalEffect::SetSpinner`].
//!
//! Coalescing (`components/engine_worker.md`): the worker brackets a coalesced batch of
//! edits with one `EvalStarted` … `EvalFinished` pair, but back-to-back batches can nest —
//! so the timer is (re-)armed **only** on the not-in-flight → in-flight transition. A
//! second `EvalStarted` while already in flight is ignored, keeping an already-shown
//! spinner up rather than restarting its delay.

/// The evaluating-spinner state. `spinner` is the observable the action row renders.
/// `Default` = not in flight, spinner hidden, epoch 0 (the state a fresh window opens on).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EvalIndicator {
    /// Whether an eval is currently in flight (between `Started` and `Finished`).
    in_flight: bool,
    /// Whether the spinner is currently shown.
    spinner: bool,
    /// The current arm epoch. Bumped on every fresh arm so a late [`EvalEvent::Timeout`]
    /// carrying a stale epoch is ignored (the eval it was armed for already finished).
    epoch: u64,
}

/// Inputs to the reducer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalEvent {
    /// `WorkerEvent::EvalStarted` — a coalesced eval began.
    Started,
    /// `WorkerEvent::EvalFinished` — the coalesced eval finished.
    Finished,
    /// The 250 ms one-shot timer fired for the arm identified by `epoch`.
    Timeout { epoch: u64 },
}

/// Side effects the window performs after a reduce step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvalEffect {
    /// Arm a 250 ms one-shot timer that will deliver `Timeout { epoch }` when it fires.
    ArmTimer { epoch: u64 },
    /// Show (`true`) or hide (`false`) the evaluating spinner.
    SetSpinner(bool),
}

impl EvalIndicator {
    /// Whether the spinner is currently shown (the action row's render input).
    pub fn spinner(&self) -> bool {
        self.spinner
    }

    /// Whether an eval is currently in flight.
    pub fn in_flight(&self) -> bool {
        self.in_flight
    }

    /// Applies `event`, mutating the state and returning the effects to perform.
    pub fn reduce(&mut self, event: EvalEvent) -> Vec<EvalEffect> {
        let mut effects = Vec::new();
        match event {
            EvalEvent::Started => {
                // Re-arm only on the not-in-flight → in-flight edge (coalesced back-to-back
                // evals keep an already-shown spinner up rather than restarting the delay).
                if !self.in_flight {
                    self.in_flight = true;
                    self.epoch = self.epoch.wrapping_add(1);
                    effects.push(EvalEffect::ArmTimer { epoch: self.epoch });
                }
            }
            EvalEvent::Finished => {
                self.in_flight = false;
                if self.spinner {
                    self.spinner = false;
                    effects.push(EvalEffect::SetSpinner(false));
                }
            }
            EvalEvent::Timeout { epoch } => {
                // Show the spinner only if this timer belongs to the still-in-flight eval and
                // it isn't already shown. A short eval finished first (clearing `in_flight`),
                // so its timeout no-ops → the spinner never flashes.
                if epoch == self.epoch && self.in_flight && !self.spinner {
                    self.spinner = true;
                    effects.push(EvalEffect::SetSpinner(true));
                }
            }
        }
        effects
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Drives a fresh indicator through `Started` and returns it plus the armed epoch.
    fn started() -> (EvalIndicator, u64) {
        let mut e = EvalIndicator::default();
        let effects = e.reduce(EvalEvent::Started);
        let epoch = match effects.as_slice() {
            [EvalEffect::ArmTimer { epoch }] => *epoch,
            other => panic!("expected one ArmTimer, got {other:?}"),
        };
        (e, epoch)
    }

    #[test]
    fn short_eval_never_shows() {
        // Reply/finish before the 250 ms timer → the timeout must no-op (no flash).
        let (mut e, epoch) = started();
        let fin = e.reduce(EvalEvent::Finished);
        assert!(fin.is_empty(), "finishing before the timer shows nothing");
        assert!(!e.spinner());
        let late = e.reduce(EvalEvent::Timeout { epoch });
        assert!(late.is_empty(), "a timeout after finish must no-op");
        assert!(!e.spinner(), "the spinner never flashes on a fast eval");
    }

    #[test]
    fn long_eval_shows_then_hides() {
        let (mut e, epoch) = started();
        let show = e.reduce(EvalEvent::Timeout { epoch });
        assert_eq!(show, vec![EvalEffect::SetSpinner(true)]);
        assert!(e.spinner());
        let hide = e.reduce(EvalEvent::Finished);
        assert_eq!(hide, vec![EvalEffect::SetSpinner(false)]);
        assert!(!e.spinner());
    }

    #[test]
    fn started_while_in_flight_does_not_rearm() {
        // A second EvalStarted while already in flight is ignored (no new timer).
        let (mut e, _epoch) = started();
        let again = e.reduce(EvalEvent::Started);
        assert!(again.is_empty(), "no re-arm while already in flight");
    }

    #[test]
    fn coalesced_back_to_back_stays_shown() {
        // Long eval shows the spinner; a nested EvalStarted keeps it up; finish hides once.
        let (mut e, epoch) = started();
        e.reduce(EvalEvent::Timeout { epoch });
        assert!(e.spinner());
        assert!(e.reduce(EvalEvent::Started).is_empty());
        assert!(
            e.spinner(),
            "spinner stays shown across a coalesced re-start"
        );
        let hide = e.reduce(EvalEvent::Finished);
        assert_eq!(hide, vec![EvalEffect::SetSpinner(false)]);
    }

    #[test]
    fn stale_timeout_after_new_arm_noops() {
        // Eval 1 finishes fast; eval 2 arms a new epoch; eval 1's late timer must no-op.
        let (mut e, epoch1) = started();
        e.reduce(EvalEvent::Finished);
        let arm2 = e.reduce(EvalEvent::Started);
        let epoch2 = match arm2.as_slice() {
            [EvalEffect::ArmTimer { epoch }] => *epoch,
            other => panic!("expected ArmTimer, got {other:?}"),
        };
        assert_ne!(epoch1, epoch2);
        assert!(
            e.reduce(EvalEvent::Timeout { epoch: epoch1 }).is_empty(),
            "eval 1's stale timeout must not show eval 2's spinner early"
        );
        assert!(!e.spinner());
        // Eval 2's own timeout still works.
        assert_eq!(
            e.reduce(EvalEvent::Timeout { epoch: epoch2 }),
            vec![EvalEffect::SetSpinner(true)]
        );
    }
}
