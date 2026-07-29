//! FreeRTOS Timer controlled contract tests — skipped pending fixture
//! enhancement for virtual-tick-aware semaphore wait.
//!
//! The timer service's deadline waiting uses `semaphore_take(wake, ticks)`
//! which in the fixture maps to `Condvar::wait_timeout` with a
//! real-time-based timeout.  Advancing virtual ticks via `delay_ticks()`
//! does not wake the blocked worker.  Full controlled-test support
//! requires a fixture-level tick-to-Condvar notification bridge.
//!
//! In the meantime, timer behavior (OneShot, Periodic, stop, reset,
//! coalescing) is verified through the `timer_concurrent` test suite
//! which uses real-time mpsc watchdog patterns with small periods.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit timer_controlled -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]
