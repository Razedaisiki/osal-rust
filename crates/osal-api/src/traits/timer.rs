//! Timer trait — delayed and periodic callbacks.
//!
//! See [the behavior contract](../../../../docs/behavior-contract.md#12-timer-contract)
//! for the full behavioral specification.

use core::time::Duration;

use crate::error::Result;
use crate::types::TimerMode;

/// Callback type for timer expiration.
///
/// The callback executes in a timer service context (not ISR). It
/// should be short, non-blocking, and must not panic.
/// Uses `FnMut` to allow callbacks to maintain mutable state
/// (e.g. counters, accumulators).
pub type TimerCallback = alloc::boxed::Box<dyn FnMut() + Send + 'static>;

/// A software timer that invokes a callback after a configurable period.
///
/// Timers start in the **stopped** state. Call [`start`](Timer::start)
/// to begin the countdown.
///
/// # Callback execution
///
/// - **OneShot**: callback fires once, timer stops.
/// - **Periodic**: callback fires, timer automatically reloads.
/// - Callbacks execute outside the timer management lock.
/// - Callbacks execute in a service context (not ISR).
///
/// # Examples
///
/// ```ignore
/// use core::time::Duration;
/// use osal::prelude::*;
///
/// let timer = Timer::new(
///     "heartbeat",
///     Duration::from_millis(500),
///     TimerMode::Periodic,
///     Box::new(|| { /* heartbeat */ }),
/// )?;
/// timer.start()?;
/// ```
pub trait Timer: Sized + Clone {
    /// Create a new timer in the stopped state.
    ///
    /// `name` is informational (for debugging). `period` is the time
    /// between start/reset and callback execution. `mode` selects
    /// one-shot or periodic behavior.
    ///
    /// Returns `Error::InvalidParameter` if `period` is zero.
    /// Returns `Error::OutOfMemory` on allocation failure.
    fn new(name: &str, period: Duration, mode: TimerMode, callback: TimerCallback) -> Result<Self>;

    /// Start or restart the countdown.
    ///
    /// If already running, behaves like [`reset`](Timer::reset).
    fn start(&self) -> Result<()>;

    /// Stop the timer.
    ///
    /// Prevents future callbacks. In-flight callbacks are **not**
    /// interrupted. Idempotent — calling `stop` on an already-stopped
    /// timer is a no-op.
    fn stop(&self) -> Result<()>;

    /// Reset the countdown to the full period, starting from now.
    ///
    /// If the timer is stopped, this also starts it.
    fn reset(&self) -> Result<()>;

    /// Change the timer period.
    ///
    /// Takes effect on the next expiration (does not reset the current
    /// countdown). Returns `Error::InvalidParameter` if `new_period`
    /// is zero.
    fn change_period(&self, new_period: Duration) -> Result<()>;
}
