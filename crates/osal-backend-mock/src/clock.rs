//! Deterministic mock clock for testing.
//!
//! Uses a static atomic counter so that [`MockClock`] can implement
//! the static [`Clock`] trait while supporting per-test time control
//! via [`MockClockControl`].
//!
//! Tests should call [`reset()`] at the start of each test to ensure
//! isolation from other tests that advance the clock.

use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use osal_api::traits::clock::Clock;

// ---------------------------------------------------------------------------
// Clock state (nanoseconds as u64)
// ---------------------------------------------------------------------------

static MOCK_TIME_NS: AtomicU64 = AtomicU64::new(0);

fn get_ns() -> u64 {
    MOCK_TIME_NS.load(Ordering::Relaxed)
}

fn set_ns(ns: u64) {
    MOCK_TIME_NS.store(ns, Ordering::Relaxed);
}

fn add_ns(ns: u64) {
    MOCK_TIME_NS.fetch_add(ns, Ordering::Relaxed);
}

fn ns_to_duration(ns: u64) -> Duration {
    Duration::from_nanos(ns)
}

fn duration_to_ns(d: Duration) -> u64 {
    d.as_nanos() as u64
}

/// Advance the mock clock by `d`.
pub fn advance(d: Duration) {
    add_ns(duration_to_ns(d));
}

/// Reset the mock clock to zero.
pub fn reset() {
    set_ns(0);
}

// ---------------------------------------------------------------------------
// Clock trait implementation
// ---------------------------------------------------------------------------

/// A deterministic clock for testing.
///
/// Implements [`Clock`] using a static atomic counter. Use
/// [`MockClockControl`] to advance time in tests.
pub struct MockClock;

impl Clock for MockClock {
    fn now() -> Duration {
        ns_to_duration(get_ns())
    }

    fn elapsed(since: Duration) -> Duration {
        let now = Self::now();
        if now >= since {
            now - since
        } else {
            Duration::ZERO
        }
    }

    fn delay(duration: Duration) {
        add_ns(duration_to_ns(duration));
    }
}

// ---------------------------------------------------------------------------
// ClockControl implementation
// ---------------------------------------------------------------------------

/// Control interface for the mock clock.
///
/// Provides `advance(d)` and `reset()` for test code.
pub struct MockClockControl;

impl MockClockControl {
    /// Reset the mock clock to zero.
    pub fn reset(&self) {
        reset();
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockControl for MockClockControl {
    fn advance_clock(&self, d: Duration) {
        advance(d);
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockFactory for MockClockControl {
    type Clock = MockClock;
}
