//! FreeRTOS Timer drop/shutdown/lifecycle race tests.
//!
//! Deterministic Virtual mode.

#![cfg(feature = "testkit")]

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::{
    FreeRtosTimer, fixture_shutdown_waiting, flush_timer_service, timer_flush_request,
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
        if runtime::state() == osal_api::runtime::RuntimeState::Running {
            let target = timer_flush_request();
            flush_timer_service(target);
            match runtime::shutdown() {
                Ok(()) | Err(Error::NotInitialized) => {}
                Err(e) => panic!("dirty test teardown: {e:?}"),
            }
        }
        fixture::set_wait_mode(FixtureWaitMode::Realtime);
    }
}

fn advance_ticks_no_flush(ticks: u64) {
    fixture::advance_ticks(ticks);
}

fn advance_ms(ms: u64) {
    fixture::advance_ticks(ms);
    let target = timer_flush_request();
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
    advance_ticks_no_flush(100);
    entered.wait(); // Barrier confirms callback has entered.

    // Drop in a separate thread with an mpsc watchdog.  The drop must
    // complete within 1 second — otherwise it's waiting for the
    // callback (which is still blocked on `release`).
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        drop(timer);
        tx.send(()).unwrap();
    });
    rx.recv_timeout(Duration::from_secs(1))
        .expect("last Timer drop waited for callback");

    // Release the callback.
    release.wait();
    let target = timer_flush_request();
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
    entered.wait(); // callback started

    // Drop the last public handle, then spawn shutdown in another thread.
    drop(timer);

    let release2 = Arc::clone(&release);
    let shutdown_done = Arc::new(AtomicBool::new(false));
    let sd = Arc::clone(&shutdown_done);

    let handle = thread::spawn(move || {
        runtime::shutdown().expect("shutdown should succeed after callback");
        sd.store(true, Ordering::Relaxed);
    });

    // Poll fixture_shutdown_waiting() — shutdown must have entered
    // the completion EventGroup wait before we release the callback.
    let poll_deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !fixture_shutdown_waiting() {
        assert!(
            std::time::Instant::now() < poll_deadline,
            "shutdown did not enter completion wait"
        );
        std::thread::yield_now();
    }
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
    let (tx, rx) = mpsc::channel();

    let timer = FreeRtosTimer::new(
        "self-shutdown",
        Duration::from_millis(10),
        TimerMode::OneShot,
        Box::new(move || {
            tx.send(runtime::shutdown()).unwrap();
        }),
    )
    .unwrap();

    timer.start().unwrap();
    advance_ms(10); // flush guarantees callback has returned

    let err = rx
        .recv_timeout(Duration::from_secs(1))
        .expect("callback did not run");
    assert_eq!(err, Err(Error::Busy), "self-shutdown must return Busy");

    // Drop the timer so RuntimeLease is released.
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
