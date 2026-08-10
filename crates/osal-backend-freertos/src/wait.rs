//! Common blocking-wait engine for FreeRTOS mutex, semaphore, and queue.
//!
//! Implements the absolute-deadline loop with per-chunk guard ticks
//! defined in ADR 0025, plus [`WaitBudget`] for operations that may
//! require multiple wait attempts within one API call (Queue send/recv).
//!
//! # Core types
//!
//! - [`WaitOutcome`] — result of a wait attempt (`Acquired` or `Unavailable`).
//! - [`WaitBudget`] — stateful budget that preserves a single absolute
//!   deadline across repeated waits.  Used by Queue operations that may
//!   experience spurious wakeups or condition changes.
//! - [`wait_native`] — convenience wrapper that creates a one-shot
//!   `WaitBudget`.  Used by Mutex and Semaphore (single-acquisition ops).
//!
//! # Algorithm (ADR 0025 §2-3)
//!
//! - `NoWait`: single `take(0)`, maps `Timeout` → `Unavailable`.
//! - `After(ZERO)`: same `take(0)`; the caller maps `Unavailable` to
//!   `Error::Timeout`.
//! - `After(d > 0)`: absolute-deadline loop. On each iteration:
//!     1. Opportunistic `take(0)` (resource may be free).
//!     2. If deadline passed → `Unavailable`.
//!     3. Convert remaining time to payload ticks, add per-chunk guard.
//!     4. `take(payload + 1)` — if acquired, done.
//!     5. Otherwise re-read the clock (spurious wakeups / early returns
//!        from tick-phase misalignment).
//! - `Forever`: loop `take(max_finite)` until acquired.  Does NOT use
//!   `portMAX_DELAY` (avoids depending on `INCLUDE_vTaskSuspend`).

use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::clock::Clock as _;
use osal_portable::tick_time;

use crate::clock::FreeRtosClock;
use crate::runtime;
use osal_backend_freertos_sys as sys;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Outcome of a wait attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The resource was acquired.
    Acquired,
    /// The resource was not acquired (timeout, locked, or empty).
    Unavailable,
}

// ---------------------------------------------------------------------------
// WaitBudget — reusable across multiple wait attempts (ADR 0027 §5)
// ---------------------------------------------------------------------------

/// A stateful timeout budget that preserves a single absolute deadline
/// across repeated wait attempts.
///
/// Queue operations may need to wait multiple times within one API call:
/// a waiter wakes, reacquires the mutex, finds the condition has changed
/// (another consumer took the message, or the queue was closed), and must
/// wait again without resetting the original deadline.
///
/// The deadline is computed **lazily** on the first blocking call to
/// [`wait_once`][WaitBudget::wait_once].  If the operation succeeds on
/// the first opportunistic check (resource immediately available), no
/// deadline is ever computed — avoiding spurious `Overflow` from
/// `checked_add` on very large durations.
#[derive(Debug)]
pub(crate) enum WaitBudget {
    /// Single attempt — never blocks.
    NoWait,

    /// Single attempt — never blocks; caller maps to `Error::Timeout`.
    Zero,

    /// Finite duration with optional lazily-computed absolute deadline.
    Finite {
        duration: Duration,
        deadline: Option<Duration>,
        /// Set by [`prepare_blocking`](WaitBudget::prepare_blocking).
        /// When true, `wait_once` skips the scheduler-state check
        /// (already passed) and deadline computation (already done).
        prepared: bool,
    },

    /// Block indefinitely (loops max-finite chunks, not `portMAX_DELAY`).
    Forever {
        /// Set by [`prepare_blocking`](WaitBudget::prepare_blocking).
        /// When true, `wait_once` skips the scheduler-state check
        /// (already passed).
        prepared: bool,
    },
}

impl WaitBudget {
    /// Create a budget from a [`Timeout`].
    pub fn new(timeout: Timeout) -> Self {
        match timeout {
            Timeout::NoWait => WaitBudget::NoWait,
            Timeout::After(d) => {
                if d == Duration::ZERO {
                    WaitBudget::Zero
                } else {
                    WaitBudget::Finite {
                        duration: d,
                        deadline: None,
                        prepared: false,
                    }
                }
            }
            Timeout::Forever => WaitBudget::Forever { prepared: false },
        }
    }

    /// Validate that this budget can enter a blocking wait.
    ///
    /// Checks the scheduler state and eagerly computes the absolute
    /// deadline for `Finite` budgets.  Call this **before** registering
    /// as a waiter — if it returns `Err`, no waiter state has been
    /// modified and the caller can propagate the error directly.
    ///
    /// After `prepare_blocking()` returns `Ok(())`, subsequent
    /// [`wait_once`] calls are infallible (they will not return
    /// configuration errors like `NotInitialized` or `Overflow`).
    pub fn prepare_blocking(&mut self) -> Result<()> {
        match self {
            WaitBudget::NoWait | WaitBudget::Zero => {
                // These never block — caller should not have called this.
                Ok(())
            }
            WaitBudget::Finite {
                duration,
                deadline,
                prepared,
            } => {
                ensure_blocking_allowed()?;
                if deadline.is_none() {
                    let value = FreeRtosClock::now()
                        .checked_add(*duration)
                        .ok_or(Error::Overflow)?;
                    *deadline = Some(value);
                }
                *prepared = true;
                Ok(())
            }
            WaitBudget::Forever { prepared } => {
                ensure_blocking_allowed()?;
                *prepared = true;
                Ok(())
            }
        }
    }

    /// Execute one wait attempt using the supplied native `take` closure.
    ///
    /// `take(ticks)` must return `TakeStatus::Acquired` on success and
    /// `TakeStatus::Timeout` on failure.  `TakeStatus::Invalid` triggers
    /// a fatal panic (invariant violation).
    ///
    /// # Budget consumption
    ///
    /// - `NoWait` / `Zero`: one attempt, returns immediately.
    /// - `Finite`: lazily computes an absolute deadline on first call,
    ///   then enters the ADR 0025 absolute-deadline loop.  Each subsequent
    ///   call resumes with the same (remaining) deadline.
    /// - `Forever`: enters the max-finite chunk loop; never returns
    ///   `Unavailable`.
    pub fn wait_once(
        &mut self,
        mut take: impl FnMut(u64) -> sys::TakeStatus,
    ) -> Result<WaitOutcome> {
        match self {
            WaitBudget::NoWait => match take(0) {
                sys::TakeStatus::Acquired => Ok(WaitOutcome::Acquired),
                sys::TakeStatus::Timeout => Ok(WaitOutcome::Unavailable),
                sys::TakeStatus::Invalid => {
                    panic!("FreeRTOS take returned Invalid on a live handle")
                }
            },
            WaitBudget::Zero => match take(0) {
                sys::TakeStatus::Acquired => Ok(WaitOutcome::Acquired),
                sys::TakeStatus::Timeout => Ok(WaitOutcome::Unavailable),
                sys::TakeStatus::Invalid => {
                    panic!("FreeRTOS take returned Invalid on a live handle")
                }
            },
            WaitBudget::Finite {
                duration,
                deadline,
                prepared,
            } => {
                // If prepare_blocking() was called, skip checks — they
                // already passed //.
                if !*prepared {
                    ensure_blocking_allowed()?;
                }
                // Lazily compute the absolute deadline on first blocking entry.
                let dl = match deadline {
                    Some(value) => *value,
                    None => {
                        let value = FreeRtosClock::now()
                            .checked_add(*duration)
                            .ok_or(Error::Overflow)?;
                        *deadline = Some(value);
                        value
                    }
                };

                wait_deadline_loop(dl, take)
            }
            WaitBudget::Forever { prepared } => {
                if !*prepared {
                    ensure_blocking_allowed()?;
                }
                wait_forever(take)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Convenience wrapper — one-shot WaitBudget //
// ---------------------------------------------------------------------------

/// Run a blocking wait using the supplied native `take` closure.
///
/// Convenience wrapper that creates a one-shot [`WaitBudget`] and calls
/// [`WaitBudget::wait_once`] once.  Suitable for single-acquisition
/// operations (Mutex, Semaphore) where retry is not needed.
///
/// The caller is responsible for mapping `WaitOutcome::Unavailable` to
/// the appropriate error variant (`LockFailed` for mutex `NoWait`,
/// `Timeout` for everything else).
pub fn wait_native(
    timeout: Timeout,
    take: impl FnMut(u64) -> sys::TakeStatus,
) -> Result<WaitOutcome> {
    let mut budget = WaitBudget::new(timeout);
    budget.wait_once(take)
}

// ---------------------------------------------------------------------------
// Scheduler-state precondition (ADR 0025 §4)
// ---------------------------------------------------------------------------

/// Check that the scheduler is in a state that permits blocking.
///
/// `NoWait` and `After(Duration::ZERO)` do NOT call this — they are
/// non-blocking operations that work regardless of scheduler state.
fn ensure_blocking_allowed() -> Result<()> {
    match sys::scheduler_state() {
        sys::SchedulerState::Running => Ok(()),
        sys::SchedulerState::NotStarted => Err(Error::NotInitialized),
        sys::SchedulerState::Suspended => Err(Error::Busy),
        sys::SchedulerState::Unknown(_) => Err(Error::Internal("unknown FreeRTOS scheduler state")),
    }
}

// ---------------------------------------------------------------------------
// Internal strategies
// ---------------------------------------------------------------------------

/// Absolute-deadline loop with a pre-computed deadline.
///
/// Used by `WaitBudget::Finite` after the deadline has been resolved.
fn wait_deadline_loop(
    deadline: Duration,
    mut take: impl FnMut(u64) -> sys::TakeStatus,
) -> Result<WaitOutcome> {
    let caps = runtime::capabilities().expect("wait requires osal::initialize()");
    let tick_rate = caps.tick_rate_hz;
    let max_native = sys::max_finite_delay_ticks() as u128;
    let max_payload = max_native
        .checked_sub(1)
        .expect("max_finite_delay_ticks too small for guard tick");

    loop {
        // Opportunistic immediate attempt.
        if take(0) == sys::TakeStatus::Acquired {
            return Ok(WaitOutcome::Acquired);
        }

        let now = FreeRtosClock::now();
        if now >= deadline {
            return Ok(WaitOutcome::Unavailable);
        }

        let remaining = deadline.saturating_sub(now);
        let payload_ticks =
            tick_time::duration_to_ticks_ceil(remaining, tick_rate).map_err(|_| Error::Overflow)?;

        let payload = payload_ticks.min(max_payload);
        let native_ticks = payload
            .checked_add(1) // per-chunk guard tick (ADR 0023 §4)
            .expect("guard tick overflowed u128");

        match take(native_ticks as u64) {
            sys::TakeStatus::Acquired => return Ok(WaitOutcome::Acquired),
            sys::TakeStatus::Timeout => {
                // May have returned early due to tick-phase alignment.
                // Re-read the absolute clock — only timeout when the
                // deadline actually passes.
                continue;
            }
            sys::TakeStatus::Invalid => {
                panic!("FreeRTOS take returned Invalid on a live handle")
            }
        }
    }
}

fn wait_forever(mut take: impl FnMut(u64) -> sys::TakeStatus) -> Result<WaitOutcome> {
    let max_finite = sys::max_finite_delay_ticks();
    loop {
        match take(max_finite) {
            sys::TakeStatus::Acquired => return Ok(WaitOutcome::Acquired),
            sys::TakeStatus::Timeout => continue, // wake and retry
            sys::TakeStatus::Invalid => {
                panic!("FreeRTOS take returned Invalid on a live handle")
            }
        }
    }
}
