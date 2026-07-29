//! FreeRTOS Timer controlled contract tests — deferred pending fixture
//! enhancement for virtual-tick-aware semaphore wait.
//!
//! The timer service's deadline waiting uses `semaphore_take(wake, ticks)`
//! which in the fixture maps to `Condvar::wait_timeout` with a
//! real-time-based timeout.  Advancing virtual ticks via `delay_ticks()`
//! does not wake the blocked worker.  Full controlled-test support
//! requires a fixture-level tick-to-Condvar notification bridge.
//!
//! Deferred controlled contracts:
//! - oneshot_fires_once (timing-precise)
//! - periodic_fires_multiple (timing-precise)
//! - stop_prevents_callback (timing-precise)
//! - reset_restarts_deadline (timing-precise)
//! - missed_expiration_coalesced (timing-precise)
//!
//! Basic lifecycle verification (OneShot, Periodic, self-stop, clone,
//! last-drop) is in `timer_lifecycle.rs` (real-time mpsc watchdog,
//! no deterministic clock control).
//!
//! State-only tests (parameter validation, scheduler preconditions) are
//! in `timer_concurrent.rs` (no worker started).
//!
//! Remaining gaps:
//! - running timer change_period keeps current deadline
//! - reset restarts deadline from now
//! - fixed-rate (not fixed-delay) periodic reload
//! - missed-period coalescing (N missed → 1 callback)
//! - callback self-reset/restart/change-period
//! - last drop during in-flight callback
//! - callback capture lock-free destruction regression test
//! - callback self-shutdown returns Busy
//! - worker creation failure rollback
//! - shutdown waits for slow in-flight callback
//! - earliest-deadline dispatch order / starvation resistance
//! - long-deadline multi-chunk wait

#![cfg(feature = "testkit")]
