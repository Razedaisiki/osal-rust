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
/// Returns `Error::Overflow` if the computation overflows.
pub fn next_periodic_deadline(
    previous_deadline: Duration,
    period: Duration,
    now: Duration,
) -> Result<Duration> {
    let mut next = previous_deadline;
    loop {
        next = next.checked_add(period).ok_or(Error::Overflow)?;
        if next > now {
            return Ok(next);
        }
    }
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
    fn periodic_overflow() {
        assert_eq!(
            next_periodic_deadline(Duration::MAX, Duration::from_secs(1), Duration::ZERO).unwrap_err(),
            Error::Overflow
        );
    }
}
