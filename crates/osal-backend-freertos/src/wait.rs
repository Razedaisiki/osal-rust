//! Common blocking-wait engine for FreeRTOS mutex and semaphore.
//!
//! Implements the absolute-deadline loop with per-chunk guard ticks
//! defined in ADR 0025.  Shared by [`FreeRtosMutex`], [`FreeRtosCountingSemaphore`],
//! and [`FreeRtosBinarySemaphore`] — no duplicated timeout logic.
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
// Core wait engine
// ---------------------------------------------------------------------------

/// Run a blocking wait using the supplied native `take` closure.
///
/// `take(ticks)` must return `TakeStatus::Acquired` on success and
/// `TakeStatus::Timeout` on failure.  `TakeStatus::Invalid` triggers
/// a fatal panic (invariant violation).
///
/// The caller is responsible for mapping `WaitOutcome::Unavailable` to
/// the appropriate error variant (`LockFailed` for mutex `NoWait`,
/// `Timeout` for everything else).
pub fn wait_native(
    timeout: Timeout,
    mut take: impl FnMut(u64) -> sys::TakeStatus,
) -> Result<WaitOutcome> {
    match timeout {
        Timeout::NoWait => {
            match take(0) {
                sys::TakeStatus::Acquired => Ok(WaitOutcome::Acquired),
                sys::TakeStatus::Timeout => Ok(WaitOutcome::Unavailable),
                sys::TakeStatus::Invalid => {
                    panic!("FreeRTOS take returned Invalid on a live handle")
                }
            }
        }
        Timeout::After(d) => {
            if d == Duration::ZERO {
                match take(0) {
                    sys::TakeStatus::Acquired => Ok(WaitOutcome::Acquired),
                    sys::TakeStatus::Timeout => Ok(WaitOutcome::Unavailable),
                    sys::TakeStatus::Invalid => {
                        panic!("FreeRTOS take returned Invalid on a live handle")
                    }
                }
            } else {
                wait_absolute_deadline(d, take)
            }
        }
        Timeout::Forever => wait_forever(take),
    }
}

// ---------------------------------------------------------------------------
// Internal strategies
// ---------------------------------------------------------------------------

fn wait_absolute_deadline(
    duration: Duration,
    mut take: impl FnMut(u64) -> sys::TakeStatus,
) -> Result<WaitOutcome> {
    let deadline = FreeRtosClock::now()
        .checked_add(duration)
        .ok_or(Error::Overflow)?;

    let caps = runtime::capabilities()
        .expect("wait requires osal::initialize()");
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
            tick_time::duration_to_ticks_ceil(remaining, tick_rate)
                .map_err(|_| Error::Overflow)?;

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

fn wait_forever(
    mut take: impl FnMut(u64) -> sys::TakeStatus,
) -> Result<WaitOutcome> {
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
