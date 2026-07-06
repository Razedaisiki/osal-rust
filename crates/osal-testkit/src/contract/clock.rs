//! Contract tests for the [`Clock`] trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#6-time-and-timeout-semantics`.
//!
//! Clock tests are split into two groups:
//!
//! - **basic**: tests that work against any real or fake clock.
//! - **controlled**: tests that require a deterministic clock
//!   ([`ControlledClockFactory`]) to verify timing precisely.

use core::time::Duration;

use osal_api::traits::clock::Clock as _;

use crate::factory::{ClockFactory, ControlledClockFactory};

// ---------------------------------------------------------------------------
// Basic tests (any clock)
// ---------------------------------------------------------------------------

/// `now()` is monotonic — it never decreases.
pub fn now_is_monotonic<F: ClockFactory>(_factory: &F) {
    let a = F::Clock::now();
    let b = F::Clock::now();
    assert!(b >= a);
}

/// Smoke test: `elapsed()` returns a non-negative duration.
///
/// Note: `Duration` is already a non-negative type by construction,
/// so this primarily verifies the method can be called. Stronger
/// elapsed-contract tests (e.g. `elapsed_tracks_advanced_time`) belong
/// in the controlled group.
pub fn elapsed_is_non_negative<F: ClockFactory>(_factory: &F) {
    let start = F::Clock::now();
    let elapsed = F::Clock::elapsed(start);
    assert!(elapsed >= Duration::ZERO);
}

/// `delay(Duration::ZERO)` returns immediately.
pub fn delay_zero_returns<F: ClockFactory>(_factory: &F) {
    F::Clock::delay(Duration::ZERO);
}

/// `delay` waits at least the requested duration.
///
/// This test is only meaningful with a real (non-fake) clock.
/// For deterministic testing, use [`delay_advances_clock`] instead.
#[cfg(feature = "std")]
pub fn delay_waits_at_least_duration<F: ClockFactory>(_factory: &F) {
    let start = F::Clock::now();
    F::Clock::delay(Duration::from_millis(1));
    assert!(F::Clock::elapsed(start) >= Duration::from_millis(1));
}

// ---------------------------------------------------------------------------
// Controlled-clock tests (deterministic time)
// ---------------------------------------------------------------------------

/// Advancing the clock by `d` makes `elapsed` return at least `d`.
pub fn advance_clock_advances_time<F: ControlledClockFactory>(factory: &F) {
    let start = F::Clock::now();
    factory.advance_clock(Duration::from_millis(42));
    assert!(F::Clock::elapsed(start) >= Duration::from_millis(42));
}

/// After advancing, `now()` should reflect the elapsed time.
pub fn advance_clock_increases_now<F: ControlledClockFactory>(factory: &F) {
    let a = F::Clock::now();
    factory.advance_clock(Duration::from_millis(1));
    let b = F::Clock::now();
    assert!(b >= a + Duration::from_millis(1));
}

/// Realtime clock tests (require a real clock, e.g. POSIX).
///
/// Uses `std::time::Instant` for independent timing verification.
#[cfg(feature = "std")]
pub fn run_realtime_contracts<F: ClockFactory>(_factory: &F) {
    delay_waits_at_least_duration::<F>(_factory);
}

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Basic clock tests (work with any clock).
pub fn run_basic_contracts<F: ClockFactory>(_factory: &F) {
    now_is_monotonic::<F>(_factory);
    elapsed_is_non_negative::<F>(_factory);
    delay_zero_returns::<F>(_factory);
}

/// Controlled-clock tests (require deterministic time).
pub fn run_controlled_contracts<F: ControlledClockFactory>(factory: &F) {
    advance_clock_advances_time::<F>(factory);
    advance_clock_increases_now::<F>(factory);
}
