//! Mock clock — deterministic virtual time via spin::Mutex-protected runtime.

use core::time::Duration;

use osal_api::traits::clock::Clock;

use crate::time_runtime::MockTimeRuntime;

static RUNTIME: spin::Mutex<Option<MockTimeRuntime>> = spin::Mutex::new(None);

fn with_runtime<F, R>(f: F) -> R
where
    F: FnOnce(&mut MockTimeRuntime) -> R,
{
    let mut guard = RUNTIME.lock();
    let rt = guard.get_or_insert_with(MockTimeRuntime::new);
    f(rt)
}

/// Advance time and dispatch one callback at a time.
/// Each callback executes outside the mutex lock.
pub(crate) fn advance_and_dispatch(d: Duration) {
    with_runtime(|rt| rt.advance_time(d));
    loop {
        let action = {
            let mut guard = RUNTIME.lock();
            guard.as_mut().and_then(|rt| rt.take_next_expired())
        };
        match action {
            Some((key, mut cb)) => {
                cb();
                let mut guard = RUNTIME.lock();
                if let Some(rt) = guard.as_mut() {
                    rt.restore_callback(key, cb);
                }
            }
            None => break,
        }
    }
}

/// Reset the runtime between tests.
pub fn reset_runtime() {
    let mut guard = RUNTIME.lock();
    if let Some(rt) = guard.as_mut() {
        rt.reset();
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
