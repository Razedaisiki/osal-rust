//! FreeRTOS Timer concurrency and boundary tests.
//!
//! Tests are split across files to ensure clean fixture state between
//! test binaries (Cargo runs each test file as a separate binary).
//!
//! Worker-lifecycle tests (ones that start the timer worker) are in
//! `timer_lifecycle.rs`.

#![cfg(feature = "testkit")]

use core::time::Duration;

use osal_api::error::Error;
use osal_api::traits::timer::Timer;
use osal_api::types::TimerMode;
use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::FreeRtosTimer;
use osal_backend_freertos_sys::fixture;

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize runtime");
}

fn teardown() {
    let _ = runtime::shutdown();
}

// ---------------------------------------------------------------------------
// These tests do NOT start the timer worker — they only test state
// that doesn't require the service task.
// ---------------------------------------------------------------------------

#[test]
fn reject_zero_period() {
    setup();
    let result = FreeRtosTimer::new("t", Duration::ZERO, TimerMode::OneShot, Box::new(|| {}));
    assert_eq!(result.err().unwrap(), Error::InvalidParameter);
    teardown();
}

#[test]
fn stop_idempotent() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();
    assert!(timer.stop().is_ok());
    assert!(timer.stop().is_ok());
    teardown();
}

#[test]
fn change_period_rejects_zero() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();
    assert_eq!(
        timer.change_period(Duration::ZERO).unwrap_err(),
        Error::InvalidParameter
    );
    teardown();
}

#[test]
fn change_period_zero_rejected_by_trait() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();
    assert_eq!(
        <FreeRtosTimer as Timer>::change_period(&timer, Duration::ZERO).unwrap_err(),
        Error::InvalidParameter
    );
    teardown();
}

#[test]
fn start_before_scheduler_returns_not_initialized() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();

    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);
    assert_eq!(timer.start().unwrap_err(), Error::NotInitialized);
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    teardown();
}

#[test]
fn start_while_suspended_returns_busy() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();

    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Suspended);
    assert_eq!(timer.start().unwrap_err(), Error::Busy);
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    teardown();
}

#[test]
fn stop_while_suspended_succeeds() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();

    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Suspended);
    assert!(timer.stop().is_ok());
    fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    teardown();
}

#[test]
fn shutdown_busy_with_timer_handle() {
    setup();
    let timer = FreeRtosTimer::new(
        "t",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();

    assert_eq!(runtime::shutdown().unwrap_err(), Error::Busy);
    drop(timer);
    assert!(runtime::shutdown().is_ok());
    runtime::initialize().unwrap();
    teardown();
}
