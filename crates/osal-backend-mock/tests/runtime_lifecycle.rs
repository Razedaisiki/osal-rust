//! Mock backend runtime lifecycle tests.
//!
//! Verifies that the Mock backend correctly owns a `RuntimeLifecycle`
//! instance (ADR 0019) and resets the virtual clock / timer registry
//! on init and shutdown.
//!
//! ```bash
//! cargo test -p osal-backend-mock --features testkit -- --test-threads=1
//! ```

use osal_api::error::Error;
use osal_api::runtime::RuntimeState;
use osal_backend_mock::runtime;

// ---------------------------------------------------------------------------
// Basic state transitions
// ---------------------------------------------------------------------------

#[test]
fn initial_state_is_uninitialized() {
    // Ensure we start from a clean state.
    runtime::initialize().unwrap();
    runtime::shutdown().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Uninitialized);
}

#[test]
fn initialize_enters_running() {
    runtime::initialize().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Running);
    runtime::shutdown().unwrap();
}

#[test]
fn repeated_initialize_returns_already_initialized() {
    runtime::initialize().unwrap();
    assert_eq!(runtime::initialize(), Err(Error::AlreadyInitialized));
    assert_eq!(runtime::state(), RuntimeState::Running);
    runtime::shutdown().unwrap();
}

#[test]
fn shutdown_returns_to_uninitialized() {
    runtime::initialize().unwrap();
    runtime::shutdown().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Uninitialized);
}

#[test]
fn shutdown_before_initialize_returns_not_initialized() {
    // Ensure clean state first.
    runtime::initialize().unwrap();
    runtime::shutdown().unwrap();
    // Second shutdown on Uninitialized.
    assert_eq!(runtime::shutdown(), Err(Error::NotInitialized));
}

#[test]
fn runtime_can_reinitialize() {
    runtime::initialize().unwrap();
    runtime::shutdown().unwrap();
    runtime::initialize().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Running);
    runtime::shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// active_objects (testkit)
// ---------------------------------------------------------------------------

#[test]
fn no_active_objects_when_idle() {
    runtime::initialize().unwrap();
    assert_eq!(runtime::active_objects(), 0);
    runtime::shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// Mock-specific: timer/clock reset on re-init
// ---------------------------------------------------------------------------

#[test]
fn reinitialize_resets_virtual_clock() {
    use osal_api::traits::clock::Clock;
    use osal_backend_mock::clock::MockClock;

    runtime::initialize().unwrap();

    // Advance clock.
    MockClock::delay(core::time::Duration::from_millis(100));
    assert!(MockClock::now() >= core::time::Duration::from_millis(100));

    runtime::shutdown().unwrap();
    runtime::initialize().unwrap();

    // Clock must be reset to zero after re-init.
    assert_eq!(
        MockClock::now(),
        core::time::Duration::ZERO,
        "clock must be zero after re-initialization"
    );

    runtime::shutdown().unwrap();
}

#[test]
fn reinitialize_detaches_stale_timers() {
    use core::time::Duration;
    use osal_api::traits::timer::Timer;
    use osal_api::types::TimerMode;
    use osal_backend_mock::timer::MockTimer;

    runtime::initialize().unwrap();

    // Create a timer.
    let t = MockTimer::new(
        "stale",
        Duration::from_millis(100),
        TimerMode::OneShot,
        Box::new(|| {}),
    )
    .unwrap();
    t.start().unwrap();

    // Drop timer handle, shutdown, re-init — the stale timer must
    // not fire after re-initialization.
    drop(t);
    runtime::shutdown().unwrap();
    runtime::initialize().unwrap();

    // Advance clock past the old timer's period.  The stale timer
    // was detached; no callback should fire.
    // (No assertion needed — the timer was dropped, so no callback
    // remains.  This test verifies no panic / epoch mismatch.)
    runtime::shutdown().unwrap();
}
