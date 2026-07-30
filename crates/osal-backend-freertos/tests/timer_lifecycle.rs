//! FreeRTOS Timer lifecycle tests — deterministic Virtual mode.
//!
//! Replaces the original real-time `thread::sleep` + `try_recv().is_err()`
//! tests with deterministic callback-count assertions.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;
use std::sync::Arc;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{FreeRtosTimer, flush_timer_service, timer_flush_request};
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
            Err(e) => panic!("dirty timer lifecycle setup: {e:?}"),
        }
        fixture::reset();
        fixture::set_wait_mode(FixtureWaitMode::Virtual);
        runtime::initialize().expect("initialize runtime");
        TestGuard
    }
}

impl Drop for TestGuard {
    fn drop(&mut self) {
        runtime::shutdown().expect("timer lifecycle leaked objects");
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_and_flush(ticks: u64) {
    fixture::advance_ticks(ticks);
    let target = timer_flush_request();
    flush_timer_service(target);
}

// ---------------------------------------------------------------------------
// 1. OneShot fires exactly once
// ---------------------------------------------------------------------------

#[test]
fn oneshot_fires_exactly_once() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "oneshot",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();

    timer.start().unwrap();

    advance_and_flush(10);
    assert_eq!(fired.load(Ordering::SeqCst), 1);

    advance_and_flush(100);
    assert_eq!(
        fired.load(Ordering::SeqCst),
        1,
        "OneShot fired more than once"
    );
}

// ---------------------------------------------------------------------------
// 2. Periodic stop prevents future callbacks
// ---------------------------------------------------------------------------

#[test]
fn periodic_stop_prevents_future_callbacks() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "periodic",
        Duration::from_millis(10),
        TimerMode::Periodic,
        Box::new(move || {
            f.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();

    timer.start().unwrap();

    // First callback at 10 ms.  Missed-period coalescing means only
    // one callback fires even though deadline 10, 20, and 30 have all
    // passed — the next deadline advances to 40 ms.
    advance_and_flush(30);
    let count_30 = fired.load(Ordering::SeqCst);
    assert_eq!(count_30, 1, "coalesced: one callback at 30 ms");

    // Second callback at 40 ms.
    advance_and_flush(20); // total 50 ms
    let count_50 = fired.load(Ordering::SeqCst);
    assert_eq!(count_50, 2, "second callback at 40 ms");

    timer.stop().unwrap();

    advance_and_flush(100);
    assert_eq!(
        fired.load(Ordering::SeqCst),
        count_50,
        "periodic timer fired after stop"
    );
}

// ---------------------------------------------------------------------------
// 3. Callback can stop itself
// ---------------------------------------------------------------------------

#[test]
fn callback_can_stop_itself() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);
    let slot: Arc<std::sync::Mutex<Option<FreeRtosTimer>>> = Arc::new(std::sync::Mutex::new(None));
    let s = Arc::clone(&slot);

    let timer = FreeRtosTimer::new(
        "self-stop",
        Duration::from_millis(10),
        TimerMode::Periodic,
        Box::new(move || {
            if let Some(t) = s.lock().unwrap().as_ref() {
                t.stop().unwrap();
            }
            f.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();

    *slot.lock().unwrap() = Some(timer.clone());
    timer.start().unwrap();

    advance_and_flush(10);
    assert_eq!(fired.load(Ordering::SeqCst), 1);

    // Break reference cycle.
    slot.lock().unwrap().take();

    advance_and_flush(100);
    assert_eq!(
        fired.load(Ordering::SeqCst),
        1,
        "self-stopped timer fired again"
    );
}

// ---------------------------------------------------------------------------
// 4. Clone stop controls same timer
// ---------------------------------------------------------------------------

#[test]
fn clone_stop_controls_same_timer() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "clone-stop",
        Duration::from_millis(200),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();

    let clone = timer.clone();
    timer.start().unwrap();
    clone.stop().unwrap();

    advance_and_flush(300);
    assert_eq!(
        fired.load(Ordering::SeqCst),
        0,
        "clone stop did not prevent callback"
    );
}

// ---------------------------------------------------------------------------
// 5. Last drop prevents callback
// ---------------------------------------------------------------------------

#[test]
fn last_drop_prevents_callback() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "last-drop",
        Duration::from_millis(500),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();

    timer.start().unwrap();
    drop(timer);

    advance_and_flush(600);
    assert_eq!(
        fired.load(Ordering::SeqCst),
        0,
        "callback fired after last drop"
    );
}

// ---------------------------------------------------------------------------
// 6. Dropping non-last clone keeps timer alive
// ---------------------------------------------------------------------------

#[test]
fn dropping_nonlast_clone_keeps_timer() {
    let _guard = TestGuard::new();
    let fired = Arc::new(AtomicU32::new(0));
    let f = Arc::clone(&fired);

    let timer = FreeRtosTimer::new(
        "keep-clone",
        Duration::from_millis(20),
        TimerMode::OneShot,
        Box::new(move || {
            f.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();

    let clone = timer.clone();
    timer.start().unwrap();
    drop(clone);

    advance_and_flush(30);
    assert_eq!(
        fired.load(Ordering::SeqCst),
        1,
        "dropping clone killed timer"
    );
}
