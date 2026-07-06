//! Mock clock — deterministic virtual time via unified runtime.

use core::time::Duration;

use osal_api::traits::clock::Clock;

use crate::time_runtime::{reset_runtime, restore_callback, take_next_expired, with_runtime};

/// Advance time and dispatch one callback at a time outside the runtime lock.
pub(crate) fn advance_and_dispatch(d: Duration) {
    with_runtime(|rt| rt.advance_time(d));
    loop {
        let Some((key, mut cb)) = take_next_expired() else {
            break;
        };
        cb();
        restore_callback(key, cb);
    }
}

// ---------------------------------------------------------------------------
// MockClock
// ---------------------------------------------------------------------------

pub struct MockClock;

impl Clock for MockClock {
    fn now() -> Duration {
        with_runtime(|rt| rt.now())
    }
    fn delay(duration: Duration) {
        advance_and_dispatch(duration);
    }
}

// ---------------------------------------------------------------------------
// MockClockControl
// ---------------------------------------------------------------------------

pub struct MockClockControl;

impl MockClockControl {
    pub fn reset(&self) {
        reset_runtime();
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockControl for MockClockControl {
    fn advance_clock(&self, d: Duration) {
        advance_and_dispatch(d);
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockFactory for MockClockControl {
    type Clock = MockClock;
}
