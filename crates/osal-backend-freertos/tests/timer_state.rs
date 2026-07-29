//! FreeRTOS TimerState semantic tests — deterministic Virtual-mode.
//!
//! Verifies change_period, reset, fixed-rate reload, and missed-period
//! coalescing.  All use the virtual-tick fixture bridge.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Arc;

use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{FreeRtosTimer, flush_request, flush_timer_service};
use osal_backend_freertos_sys::fixture;
use osal_backend_freertos_sys::fixture::FixtureWaitMode;

// ---------------------------------------------------------------------------
// TestGuard — strict Virtual-mode setup/teardown
// ---------------------------------------------------------------------------

struct TestGuard;

impl TestGuard {
    fn new() -> Self {
        let _ = runtime::shutdown();
        fixture::reset();
        fixture::set_wait_mode(FixtureWaitMode::Virtual);
        runtime::initialize().expect("initialize");
        TestGuard
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        let target = flush_request();
        flush_timer_service(target);
        assert!(
            runtime::shutdown().is_ok(),
            "TestGuard: runtime shutdown failed — leaked timer or stuck worker"
        );
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_ms(ms: u64) {
    let target = flush_request();
    fixture::advance_ticks(ms); // 1000 Hz → 1 tick = 1 ms
    flush_timer_service(target);
}

// ---------------------------------------------------------------------------
// 1. change_period preserves current deadline
// ---------------------------------------------------------------------------

#[test]
fn change_period_preserves_current_deadline() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "cp",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ms(40);

    // Change period — current deadline must stay at 100 ms.
    timer.change_period(Duration::from_millis(250)).unwrap();

    advance_ms(59); // total 99 ms
    assert_eq!(fired.load(Ordering::Relaxed), 0);

    advance_ms(1); // total 100 ms → deadline reached
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    advance_ms(249); // total 349 ms
    assert_eq!(fired.load(Ordering::Relaxed), 1, "not yet 100+250=350");

    advance_ms(1); // total 350 ms
    assert_eq!(
        fired.load(Ordering::Relaxed),
        2,
        "next deadline should be 350, not 290 or 250"
    );
}

// ---------------------------------------------------------------------------
// 2. reset restarts from now
// ---------------------------------------------------------------------------

#[test]
fn reset_restarts_deadline_from_now() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "rst",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ms(60);
    timer.reset().unwrap();

    advance_ms(99); // total 159 ms from original start, 99 from reset
    assert_eq!(fired.load(Ordering::Relaxed), 0);

    advance_ms(1); // total 160 ms → 100 ms after reset
    assert_eq!(fired.load(Ordering::Relaxed), 1);
}

#[test]
fn start_on_running_is_equivalent_to_reset() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "str",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ms(60);
    timer.start().unwrap(); // running → equivalent to reset

    advance_ms(99); // 99 from second start
    assert_eq!(fired.load(Ordering::Relaxed), 0);

    advance_ms(1); // 100 from second start
    assert_eq!(fired.load(Ordering::Relaxed), 1);
}

// ---------------------------------------------------------------------------
// 3. Missed-period coalescing
// ---------------------------------------------------------------------------

#[test]
fn missed_periods_coalesce_into_one_callback() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "coal",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    timer.start().unwrap();

    // Advance past 5 periods (100, 200, 300, 400, 500).
    advance_ms(550);
    // All 5 missed periods coalesce into exactly 1 callback.
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    // Next deadline is 600.
    advance_ms(49); // total 599 ms
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    advance_ms(1); // total 600 ms
    assert_eq!(fired.load(Ordering::Relaxed), 2);
}

// ---------------------------------------------------------------------------
// 4. Fixed-rate (not fixed-delay)
// ---------------------------------------------------------------------------

#[test]
fn periodic_uses_fixed_rate_not_fixed_delay() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "fr",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(move || {
            // Simulate a slow callback by advancing time.
            // In Virtual mode, the callback runs in the worker thread;
            // advancing ticks here simulates callback work.
            fixture::advance_ticks(40);
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    timer.start().unwrap();

    // First expiry at 100 ms.
    advance_ms(100);
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    // Callback took 40 ms of virtual time.  If fixed-delay, next
    // deadline would be 100 + 40 + 100 = 240.
    // If fixed-rate, next deadline is 200 regardless.
    advance_ms(59); // total: 100 + 40(in callback) + 59 = 199 ms
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    advance_ms(1); // total 200 ms → next deadline
    assert_eq!(
        fired.load(Ordering::Relaxed),
        2,
        "should be fixed-rate, not fixed-delay"
    );
}
