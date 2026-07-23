//! Checked time arithmetic for deadlines and periodic scheduling.
//!
//! All arithmetic uses checked operations — overflow returns an error
//! rather than silently wrapping.

use core::time::Duration;

use osal_api::error::{Error, Result};

/// Compute an absolute deadline: `now + after`.
///
/// Returns `Error::Overflow` if the result would overflow `Duration`.
pub fn checked_deadline(now: Duration, after: Duration) -> Result<Duration> {
    now.checked_add(after).ok_or(Error::Overflow)
}

/// Remaining time until `deadline` from `now`.
///
/// Returns `Duration::ZERO` if the deadline has already passed.
pub fn remaining_until(now: Duration, deadline: Duration) -> Duration {
    deadline.saturating_sub(now)
}

/// Compute the next periodic deadline after a timer expires.
///
/// Given the previous scheduled deadline and the period, advance to
/// the first multiple of `period` that is strictly greater than `now`.
/// Merges missed periods into a single advance.
///
/// Uses O(1) arithmetic instead of iterating period-by-period.
/// Handles arbitrarily large gaps (e.g. system sleep / debug halt).
///
/// Returns `Error::Overflow` if the computation overflows.
pub fn next_periodic_deadline(
    previous_deadline: Duration,
    period: Duration,
    now: Duration,
) -> Result<Duration> {
    const NANOS_PER_SEC: u128 = 1_000_000_000;

    // Guard: period must be non-zero (validated by caller).
    if period.is_zero() {
        return Err(Error::InvalidParameter);
    }

    // If already ahead of now, return as-is.
    if previous_deadline > now {
        return Ok(previous_deadline);
    }

    // O(1): compute how many periods have elapsed since the previous
    // deadline, then advance by (missed + 1).
    let elapsed_ns = now
        .checked_sub(previous_deadline)
        .unwrap_or(Duration::ZERO)
        .as_nanos();
    let period_ns = period.as_nanos();

    // missed = floor(elapsed / period) + 1  → next after now
    let missed = elapsed_ns
        .checked_div(period_ns)
        .and_then(|n| n.checked_add(1))
        .ok_or(Error::Overflow)?;

    let advance_ns = period_ns.checked_mul(missed).ok_or(Error::Overflow)?;

    // Convert u128 nanos to Duration (u64 secs + u32 nanos).
    let advance_secs = advance_ns / NANOS_PER_SEC;
    let advance_subsec = (advance_ns % NANOS_PER_SEC) as u32;
    let advance_secs_u64 = u64::try_from(advance_secs).map_err(|_| Error::Overflow)?;
    let advance = Duration::new(advance_secs_u64, advance_subsec);

    previous_deadline
        .checked_add(advance)
        .ok_or(Error::Overflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_normal() {
        let d = checked_deadline(Duration::from_secs(1), Duration::from_millis(500)).unwrap();
        assert_eq!(d, Duration::from_micros(1_500_000));
    }

    #[test]
    fn deadline_zero() {
        let d = checked_deadline(Duration::from_secs(1), Duration::ZERO).unwrap();
        assert_eq!(d, Duration::from_secs(1));
    }

    #[test]
    fn deadline_overflow() {
        assert_eq!(
            checked_deadline(Duration::MAX, Duration::from_secs(1)).unwrap_err(),
            Error::Overflow
        );
    }

    #[test]
    fn remaining_unexpired() {
        let r = remaining_until(Duration::from_secs(1), Duration::from_secs(2));
        assert_eq!(r, Duration::from_secs(1));
    }

    #[test]
    fn remaining_expired() {
        let r = remaining_until(Duration::from_secs(3), Duration::from_secs(2));
        assert_eq!(r, Duration::ZERO);
    }

    #[test]
    fn periodic_normal() {
        let n = next_periodic_deadline(
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(150),
        )
        .unwrap();
        assert_eq!(n, Duration::from_millis(200));
    }

    #[test]
    fn periodic_missed_one() {
        let n = next_periodic_deadline(
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(350),
        )
        .unwrap();
        // 100+100=200, 200+100=300, 300+100=400 > 350
        assert_eq!(n, Duration::from_millis(400));
    }

    #[test]
    fn periodic_at_deadline() {
        // exactly at the deadline — next must be after now
        let n = next_periodic_deadline(
            Duration::from_millis(100),
            Duration::from_millis(100),
            Duration::from_millis(200),
        )
        .unwrap();
        assert_eq!(n, Duration::from_millis(300));
    }

    #[test]
    fn periodic_not_passed_yet() {
        // Deadline is already ahead of now — no advance needed.
        let n =
            next_periodic_deadline(Duration::MAX, Duration::from_secs(1), Duration::ZERO).unwrap();
        assert_eq!(n, Duration::MAX);
    }

    #[test]
    fn periodic_large_but_valid_duration() {
        // Period of > u64::MAX nanos but within valid Duration range.
        let period = Duration::new(u64::MAX / 2 + 1, 0);
        let result = next_periodic_deadline(Duration::ZERO, period, Duration::ZERO).unwrap();
        assert_eq!(result, period);
    }

    #[test]
    fn periodic_overflow_on_advance() {
        // Genuine overflow: deadline is near u64::MAX seconds, and
        // the next period would exceed Duration::MAX.
        let deadline = Duration::new(u64::MAX - 1, 0);
        let period = Duration::from_secs(2);
        let now = deadline; // exactly at deadline — next must be after now
        let result = next_periodic_deadline(deadline, period, now);
        assert_eq!(result, Err(Error::Overflow));
    }

    #[test]
    fn periodic_zero_period_defensive() {
        assert_eq!(
            next_periodic_deadline(
                Duration::from_secs(1),
                Duration::ZERO,
                Duration::from_secs(2)
            )
            .unwrap_err(),
            Error::InvalidParameter
        );
    }
}
