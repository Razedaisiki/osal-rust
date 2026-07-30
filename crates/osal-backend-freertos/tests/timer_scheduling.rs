//! FreeRTOS Timer scheduling, fairness, and long-wait tests (P7F-S3).
//!
//! Deterministic Virtual mode.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Arc;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{
    FreeRtosTimer, fixture_clear_wake_wait_ticks, fixture_wake_wait_count,
    fixture_wake_wait_max_ticks, flush_timer_service, timer_flush_request,
};
use osal_backend_freertos_sys::fixture;
use osal_backend_freertos_sys::fixture::FixtureWaitMode;

// ---------------------------------------------------------------------------
// TestGuard
// ---------------------------------------------------------------------------

struct TestGuard;

impl TestGuard {
    fn new() -> Self {
        match runtime::shutdown() {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) => panic!("dirty test setup: {e:?}"),
        }
        fixture::reset();
        fixture::set_wait_mode(FixtureWaitMode::Virtual);
        runtime::initialize().expect("initialize");
        TestGuard
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        let target = timer_flush_request();
        flush_timer_service(target);
        match runtime::shutdown() {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) => panic!("dirty test teardown: {e:?}"),
        }
        fixture::set_max_finite_delay_ticks((1u64 << 32) - 2);
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_ms(ms: u64) {
    fixture::advance_ticks(ms);
    let target = timer_flush_request();
    flush_timer_service(target);
}

/// Poll `cond` with `yield_now()` and a 2-second wall-clock watchdog.
fn wait_until(mut cond: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !cond() {
        assert!(
            std::time::Instant::now() < deadline,
            "wait_until: condition not met within 2 seconds"
        );
        std::thread::yield_now();
    }
}

// ---------------------------------------------------------------------------
// 1. Earliest deadline dispatches first
// ---------------------------------------------------------------------------

#[test]
fn earliest_deadline_dispatches_first() {
    let _guard = TestGuard::new();
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));

    // Create A first (deadline 200), then B (deadline 100).
    let o_a = Arc::clone(&order);
    let timer_a = FreeRtosTimer::new(
        "A",
        Duration::from_millis(200),
        TimerMode::OneShot,
        Box::new(move || {
            o_a.lock().unwrap().push("A");
        }),
    )
    .unwrap();

    let o_b = Arc::clone(&order);
    let timer_b = FreeRtosTimer::new(
        "B",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            o_b.lock().unwrap().push("B");
        }),
    )
    .unwrap();

    timer_a.start().unwrap();
    timer_b.start().unwrap();

    // Advance past both deadlines — B must fire before A.
    advance_ms(200);
    let result = order.lock().unwrap().clone();
    assert_eq!(result, vec!["B", "A"], "earliest deadline must fire first");
}

// ---------------------------------------------------------------------------
// 2. Overdue periodic does not starve overdue OneShot
// ---------------------------------------------------------------------------

#[test]
fn overdue_periodic_does_not_starve_overdue_oneshot() {
    let _guard = TestGuard::new();
    let order = Arc::new(std::sync::Mutex::new(Vec::new()));
    let o1 = Arc::clone(&order);

    // Periodic P: period 100, deadline 100.
    let timer_p = FreeRtosTimer::new(
        "P",
        Duration::from_millis(100),
        TimerMode::Periodic,
        Box::new(move || {
            o1.lock().unwrap().push("P");
        }),
    )
    .unwrap();

    let o2 = Arc::clone(&order);
    // OneShot O: deadline 250.
    let timer_o = FreeRtosTimer::new(
        "O",
        Duration::from_millis(250),
        TimerMode::OneShot,
        Box::new(move || {
            o2.lock().unwrap().push("O");
        }),
    )
    .unwrap();

    timer_p.start().unwrap();
    timer_o.start().unwrap();

    // Jump to 550 — P was due at 100, 200, 300, 400, 500; O at 250.
    // P fires once (coalesced), then O must fire before P's next.
    advance_ms(550);

    let result = order.lock().unwrap().clone();
    // P fires first (deadline 100, expired), then O (deadline 250).
    assert_eq!(&result[0..2], &["P", "O"], "P then O must fire");
    // P fired only once (coalesced 5 missed periods).
    let p_count = result.iter().filter(|&&s| s == "P").count();
    assert_eq!(p_count, 1, "P coalesced into exactly 1 callback");
}

// ---------------------------------------------------------------------------
// 3. Earlier timer wakes worker from long deadline wait
// ---------------------------------------------------------------------------

#[test]
fn earlier_timer_wakes_worker_from_long_deadline_wait() {
    let _guard = TestGuard::new();
    let a_fired = Arc::new(AtomicU32::new(0));
    let b_fired = Arc::new(AtomicU32::new(0));
    let af = Arc::clone(&a_fired);
    let bf = Arc::clone(&b_fired);

    // A: long deadline.
    let timer_a = FreeRtosTimer::new(
        "A",
        Duration::from_millis(200),
        TimerMode::OneShot,
        Box::new(move || {
            af.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    timer_a.start().unwrap();

    // Advance a little so worker enters deadline wait for A.
    advance_ms(5);

    // B: shorter deadline, started after A.
    let timer_b = FreeRtosTimer::new(
        "B",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            bf.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    // start(B) signals wake semaphore → worker rescans → picks
    // up B's earlier deadline.
    timer_b.start().unwrap();

    // Advance to B's deadline: 5 + 10 = 15 ms from start.
    advance_ms(10); // total 15 ms
    assert_eq!(b_fired.load(Ordering::Relaxed), 1, "B must fire at 15ms");
    assert_eq!(a_fired.load(Ordering::Relaxed), 0, "A must not fire yet");
}

// ---------------------------------------------------------------------------
// 4. Long deadline uses multiple finite wait chunks
// ---------------------------------------------------------------------------

#[test]
fn long_deadline_uses_multiple_finite_wait_chunks() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    // Shrink max finite delay so 20-tick deadline requires multiple chunks
    // (20 / 7 = 3+ chunks before the deadline is reached).
    fixture::set_max_finite_delay_ticks(7);
    fixture_clear_wake_wait_ticks();

    let timer = FreeRtosTimer::new(
        "chunk",
        Duration::from_millis(20),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    timer.start().unwrap();

    // Wait for the worker to enter its first finite wake-semaphore wait.
    wait_until(|| fixture_wake_wait_count() >= 1);

    fixture::advance_ticks(7);
    wait_until(|| fixture_wake_wait_count() >= 2);
    assert_eq!(fired.load(Ordering::Relaxed), 0);

    fixture::advance_ticks(7);
    wait_until(|| fixture_wake_wait_count() >= 3);
    assert_eq!(fired.load(Ordering::Relaxed), 0);

    // Verify no wait exceeded the configured maximum.
    assert!(
        fixture_wake_wait_max_ticks() <= 7,
        "worker requested an oversized finite wait"
    );

    fixture::advance_ticks(5); // total 19 < 20
    assert_eq!(fired.load(Ordering::Relaxed), 0);

    fixture::advance_ticks(2); // total 21 ≥ 20
    let target = timer_flush_request();
    flush_timer_service(target);
    assert_eq!(fired.load(Ordering::Relaxed), 1);

    // TestGuard::drop restores default max_finite_delay_ticks.
}
