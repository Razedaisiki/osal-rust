//! FreeRTOS Timer drop/shutdown/lifecycle race tests.
//!
//! Deterministic Virtual mode.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{FreeRtosTimer, flush_request, flush_timer_service};
use osal_backend_freertos_sys::fixture;
use osal_backend_freertos_sys::fixture::FixtureWaitMode;

// ---------------------------------------------------------------------------
// TestGuard
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
        if runtime::state() == osal_api::runtime::RuntimeState::Running {
            let target = flush_request();
            flush_timer_service(target);
            assert!(runtime::shutdown().is_ok(), "runtime shutdown failed");
        }
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_ticks_no_flush(ticks: u64) {
    fixture::advance_ticks(ticks);
}

fn advance_ms(ms: u64) {
    let target = flush_request();
    fixture::advance_ticks(ms);
    flush_timer_service(target);
}

// ---------------------------------------------------------------------------
// 1. Last drop during in-flight callback
// ---------------------------------------------------------------------------

#[test]
fn last_drop_during_callback_does_not_wait() {
    let _guard = TestGuard::new();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let e = Arc::clone(&entered);
    let r = Arc::clone(&release);

    let timer = FreeRtosTimer::new(
        "drop-during-cb",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            e.wait();
            r.wait();
        }),
    )
    .unwrap();

    timer.start().unwrap();
    // Advance ticks without flush — callback will fire but block on barrier.
    advance_ticks_no_flush(100);
    // Let worker get CPU time to process the callback.
    std::thread::sleep(Duration::from_millis(10));

    // Verify callback has entered.
    entered.wait();

    // Drop the last public handle.  Must return immediately.
    let start = std::time::Instant::now();
    drop(timer);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(1),
        "last handle drop blocked waiting for in-flight callback"
    );

    // Release the callback.
    release.wait();
    std::thread::sleep(Duration::from_millis(10));
    let target = flush_request();
    flush_timer_service(target);
}

// ---------------------------------------------------------------------------
// 2. Shutdown waits for slow callback
// ---------------------------------------------------------------------------

#[test]
fn shutdown_waits_for_inflight_callback() {
    let _guard = TestGuard::new();
    let entered = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let e = Arc::clone(&entered);
    let r = Arc::clone(&release);

    let timer = FreeRtosTimer::new(
        "shutdown-wait",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(move || {
            e.wait();
            r.wait();
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ticks_no_flush(100);
    std::thread::sleep(Duration::from_millis(10));
    entered.wait(); // callback started

    // Drop the last public handle under the fixture lock so it completes.
    drop(timer);

    // Shutdown from another thread — must wait for callback.
    let release2 = Arc::clone(&release);
    let shutdown_done = Arc::new(AtomicBool::new(false));
    let sd = Arc::clone(&shutdown_done);

    let handle = thread::spawn(move || {
        runtime::shutdown().expect("shutdown should succeed after callback");
        sd.store(true, Ordering::Relaxed);
    });

    // Shutdown should be blocked on the callback.
    thread::sleep(Duration::from_millis(100));
    assert!(
        !shutdown_done.load(Ordering::Relaxed),
        "shutdown should wait for callback"
    );

    // Release callback → worker exits → shutdown completes.
    release2.wait();
    handle.join().unwrap();
    assert!(shutdown_done.load(Ordering::Relaxed));

    // Re-init for clean teardown.
    fixture::set_wait_mode(FixtureWaitMode::Virtual);
    runtime::initialize().unwrap();
}

// ---------------------------------------------------------------------------
// 3. Callback self-shutdown returns Busy
// ---------------------------------------------------------------------------

#[test]
fn callback_self_shutdown_returns_busy() {
    let _guard = TestGuard::new();
    let result: Arc<Mutex<Option<Error>>> = Arc::new(Mutex::new(None));
    let r = Arc::clone(&result);

    let timer = FreeRtosTimer::new(
        "self-shutdown",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            let err = runtime::shutdown().unwrap_err();
            *r.lock().unwrap() = Some(err);
        }),
    )
    .unwrap();

    timer.start().unwrap();
    // Advance ticks — worker will dispatch the callback, which calls
    // shutdown and stores the error.
    advance_ticks_no_flush(10);
    // Give worker CPU time to process the callback.
    std::thread::sleep(Duration::from_millis(50));

    let err = result.lock().unwrap().take().expect("callback did not run");
    assert_eq!(err, Error::Busy, "self-shutdown must return Busy");

    // Drop the timer so RuntimeLease is released.  Runtime must still
    // be Running after the failed self-shutdown.
    drop(timer);
    assert!(
        runtime::shutdown().is_ok(),
        "runtime should still be alive after Busy"
    );
    runtime::initialize().unwrap();
}

// ---------------------------------------------------------------------------
// 4. Shutdown succeeds after scheduler restored
// ---------------------------------------------------------------------------

#[test]
fn shutdown_suspended_retry_succeeds() {
    let _guard = TestGuard::new();
    let timer = FreeRtosTimer::new(
        "suspend",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();

    // Create the worker.
    timer.start().unwrap();
    advance_ms(100);
    // Timer fired, auto-stopped.
    drop(timer);

    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Suspended);
    assert_eq!(runtime::shutdown().unwrap_err(), Error::Busy);

    // Restore and retry.
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    assert!(runtime::shutdown().is_ok());
    runtime::initialize().unwrap();
}
