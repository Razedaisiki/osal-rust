//! Clock trait — monotonic time measurement and delay.
//!
//! See [the backend contract](../../../docs/behavior-contract.md#6-time-and-timeout-semantics)
//! for the full behavioral specification.

use core::time::Duration;

/// Monotonic clock for time measurement and blocking delays.
///
/// All time values are expressed as [`core::time::Duration`] from an
/// arbitrary epoch (typically process start or system boot). The clock
/// is guaranteed never to go backward.
///
/// # Precision
///
/// Resolution is backend-dependent. Portable code should not assume
/// sub-millisecond precision.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// let start = Clock::now();
/// do_work();
/// let elapsed = Clock::elapsed(start);
/// println!("Work took {} ms", elapsed.as_millis());
///
/// Clock::delay(Duration::from_millis(100));
/// ```
pub trait Clock {
    /// Return the current monotonic time.
    fn now() -> Duration;

    /// Return the duration elapsed since `since`.
    ///
    /// Equivalent to `now() - since`, saturating at zero if `since` is
    /// in the future (which should not happen with monotonic time).
    fn elapsed(since: Duration) -> Duration;

    /// Block the calling task for at least `duration`.
    ///
    /// `delay(Duration::ZERO)` must return immediately.
    /// The actual delay may be longer due to scheduling granularity.
    fn delay(duration: Duration);
}
