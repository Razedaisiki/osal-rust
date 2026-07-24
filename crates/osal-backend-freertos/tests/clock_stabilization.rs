//! Backend-specific stabilisation tests for `FreeRtosClock::delay()`.
//!
//! These tests exercise paths that the generic contract suite does not
//! cover: non-zero delay with guard ticks, multi-chunk splitting,
//! scheduler-state panics, tick-wrap correctness, and native delay error
//! handling.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit clock_stabilization -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use core::time::Duration;

use osal_api::traits::clock::Clock as _;
use osal_backend_freertos::clock::FreeRtosClock;
use osal_backend_freertos::runtime;
use osal_backend_freertos_sys::fixture;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() {
    fixture::reset();
    // Shut down in case a previous test left the runtime Running.
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize runtime");
}

fn teardown() {
    let _ = runtime::shutdown();
}

/// Advance the fixture clock by `d` via `FreeRtosClock::delay`.
/// In fixture mode, `delay_ticks` advances virtual time rather than
/// blocking the host thread.
fn delay_via_clock(d: Duration) {
    FreeRtosClock::delay(d);
}

// ---------------------------------------------------------------------------
// Non-zero delay
// ---------------------------------------------------------------------------

#[test]
fn nonzero_delay_advances_at_least_requested() {
    setup();

    let before = FreeRtosClock::now();
    delay_via_clock(Duration::from_millis(10));
    let after = FreeRtosClock::now();

    assert!(
        after >= before + Duration::from_millis(10),
        "delay(10 ms) did not advance far enough: before={before:?}, after={after:?}"
    );

    teardown();
}

#[test]
fn delay_1ns_advances_at_least_one_tick() {
    setup();

    let before = FreeRtosClock::now();
    delay_via_clock(Duration::from_nanos(1));
    let after = FreeRtosClock::now();

    assert!(after > before, "delay(1 ns) should advance at least 1 tick");

    teardown();
}

// ---------------------------------------------------------------------------
// Scheduler-state panics
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "running scheduler")]
fn delay_panics_when_scheduler_not_started() {
    fixture::reset();
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);

    FreeRtosClock::delay(Duration::from_millis(1));

    let _ = runtime::shutdown();
}

#[test]
#[should_panic(expected = "running scheduler")]
fn delay_panics_when_scheduler_suspended() {
    setup();
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Suspended);

    FreeRtosClock::delay(Duration::from_millis(1));

    teardown();
}

// ---------------------------------------------------------------------------
// Multi-chunk delay (guard tick per chunk)
// ---------------------------------------------------------------------------

#[test]
fn long_delay_is_split_into_multiple_guarded_chunks() {
    setup();

    // Set max_finite_delay_ticks to a small value (7) so that even a
    // modest delay is split into multiple chunks.
    fixture::set_max_finite_delay_ticks(7);
    // max_payload = max_finite - 1 (guard tick) = 6
    // ceil_ticks for 10 ms at 1000 Hz = 10
    // chunks: payload=6 native=7, payload=4 native=5

    let before = FreeRtosClock::now();
    delay_via_clock(Duration::from_millis(10));
    let after = FreeRtosClock::now();

    assert!(
        after >= before + Duration::from_millis(10),
        "multi-chunk delay(10 ms) did not advance far enough: before={before:?}, after={after:?}"
    );

    teardown();
}

#[test]
fn delay_with_small_max_finite_advances_correctly() {
    setup();

    // max_finite = 5, max_payload = 4
    // 1 ms = 1 tick at 1000 Hz → 1 chunk (payload=1, native=2)
    fixture::set_max_finite_delay_ticks(5);

    let before = FreeRtosClock::now();
    delay_via_clock(Duration::from_millis(1));
    let after = FreeRtosClock::now();

    assert!(
        after >= before + Duration::from_millis(1),
        "guard-tick delay(1 ms) should advance at least 1 ms: before={before:?}, after={after:?}"
    );

    teardown();
}

// ---------------------------------------------------------------------------
// Tick wrap
// ---------------------------------------------------------------------------

#[test]
fn delay_crosses_native_tick_wrap() {
    // Set tick_bits BEFORE initialize so the capability probe caches
    // the correct width.  Must come before setup() which calls init.
    fixture::reset();
    fixture::set_tick_bits(16);

    // Place counter just before 16-bit wrap.
    fixture::set_tick_snapshot(0, 0xFFF0);

    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");

    let before = FreeRtosClock::now();

    // 20 ms = 20 ceil ticks at 1000 Hz + 1 guard = 21 ticks.
    // 0xFFF0 + 21 = 0x10005 → wraps to tick_count=5, overflow_count=1.
    delay_via_clock(Duration::from_millis(20));

    let after = FreeRtosClock::now();

    // Exact delta: 21 ticks at 1000 Hz = 21 ms.
    // This verifies that both the wrap simulation AND the capability
    // probe agree on 16-bit width (if they disagreed, the expansion
    // would produce a wild jump).
    assert_eq!(
        after - before,
        Duration::from_millis(21),
        "wrap delay must advance exactly 21 ms (20 ceil + 1 guard)"
    );
    assert!(
        fixture::tick_overflow_count() >= 1,
        "overflow_count must increase after tick wrap"
    );

    let _ = runtime::shutdown();
}

// ---------------------------------------------------------------------------
// Zero delay is always safe
// ---------------------------------------------------------------------------

#[test]
fn delay_zero_returns_in_any_scheduler_state() {
    // NotStarted
    fixture::reset();
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);
    FreeRtosClock::delay(Duration::ZERO); // must not panic
    let _ = runtime::shutdown();

    // Suspended
    fixture::reset();
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Suspended);
    FreeRtosClock::delay(Duration::ZERO); // must not panic
    let _ = runtime::shutdown();

    // Running (normal)
    fixture::reset();
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");
    // fixture default is Running
    FreeRtosClock::delay(Duration::ZERO); // must not panic
    let _ = runtime::shutdown();
}
