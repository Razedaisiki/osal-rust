//! Timer demo — one-shot timer observed via `Clock::delay`.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::fmt;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};
use crate::runtime::with_runtime;

/// Timer period; the wait below is deliberately much longer so scheduler
/// jitter cannot change the expected fire count.
const TIMER_PERIOD_MS: u64 = 10;
const TIMER_WAIT_MS: u64 = 30;

/// Outcome of the timer demo.
pub struct TimerReport {
    /// Number of callback invocations observed.
    pub callback_count: u32,
}

impl fmt::Display for TimerReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[timer] callback_count={}", self.callback_count)
    }
}

/// Run the timer demo against the active backend.
pub fn run() -> DemoResult<TimerReport> {
    with_runtime(|| {
        let fired = Arc::new(AtomicU32::new(0));
        let counter = Arc::clone(&fired);

        let timer = Timer::new(
            "demo",
            Duration::from_millis(TIMER_PERIOD_MS),
            TimerMode::OneShot,
            Box::new(move || {
                counter.fetch_add(1, Ordering::Release);
            }),
        )
        .demo_context("timer.create")?;

        timer.start().demo_context("timer.start")?;
        Clock::delay(Duration::from_millis(TIMER_WAIT_MS));

        let callback_count = fired.load(Ordering::Acquire);
        if callback_count != 1 {
            return Err(DemoError::Check("one-shot timer did not fire exactly once"));
        }

        timer.stop().demo_context("timer.stop")?;
        drop(timer);

        Ok(TimerReport { callback_count })
    })
}
