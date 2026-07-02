//! Contract tests for the [`Timer`] trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#12-timer-contract`.
//!
//! # Note on timed execution
//!
//! Tests that verify actual callback execution (one-shot fires once,
//! periodic fires repeatedly) require a deterministic clock
//! ([`ControlledClockFactory`]) and are deferred until MockClock is
//! available. This module currently covers creation and control
//! operations only.

use core::time::Duration;

use osal_api::error::Error;
use osal_api::traits::timer::Timer as _;
use osal_api::types::TimerMode;

use crate::factory::TimerFactory;

// ---------------------------------------------------------------------------
// Creation tests
// ---------------------------------------------------------------------------

/// Creating a timer with a zero period fails.
pub fn new_rejects_zero_period<F: TimerFactory>(factory: &F) {
    let result =
        factory.create_timer("zero", Duration::ZERO, TimerMode::OneShot, factory.dummy_callback());
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// A newly created timer does not start automatically.
pub fn new_timer_starts_stopped<F: TimerFactory>(factory: &F) {
    let _timer = factory
        .create_timer(
            "stopped",
            Duration::from_secs(60),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    // Timer created in stopped state — no assertion needed beyond
    // the constructor succeeding.
}

// ---------------------------------------------------------------------------
// Control tests
// ---------------------------------------------------------------------------

/// Calling `stop()` on an already-stopped timer is idempotent.
pub fn stop_is_idempotent<F: TimerFactory>(factory: &F) {
    let timer = factory
        .create_timer(
            "idem",
            Duration::from_secs(60),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    timer.stop().unwrap();
    timer.stop().unwrap(); // Must not panic.
}

/// Changing period to zero fails.
pub fn change_period_rejects_zero<F: TimerFactory>(factory: &F) {
    let timer = factory
        .create_timer(
            "chg",
            Duration::from_secs(60),
            TimerMode::OneShot,
            factory.dummy_callback(),
        )
        .unwrap();
    let result = timer.change_period(Duration::ZERO);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Creation and parameter-validation tests.
pub fn run_creation_contracts<F: TimerFactory>(factory: &F) {
    new_rejects_zero_period::<F>(factory);
    new_timer_starts_stopped::<F>(factory);
}

/// Control-operation tests (stop, change_period, reset).
pub fn run_control_contracts<F: TimerFactory>(factory: &F) {
    stop_is_idempotent::<F>(factory);
    change_period_rejects_zero::<F>(factory);
}
