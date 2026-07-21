//! Contract tests for backend runtime lifecycle (ADR 0015 / ADR 0019).
//!
//! Every backend must pass these tests.  They verify that:
//!
//! - State transitions follow the four-state machine.
//! - Idempotent initialize / shutdown return precise errors.
//! - Re-initialization works after clean shutdown.
//!
//! Concurrency contracts (single-winner initialize/shutdown) are
//! backend-specific: POSIX provides them in its `runtime_lifecycle`
//! integration tests; Mock defers them (single-context model).
//!
//! Parameter validation precedence over `NotInitialized` is tested
//! per-object (see `contract::timer`, `contract::queue`, etc.).

use osal_api::error::Error;
use osal_api::runtime::RuntimeState;

use crate::factory::RuntimeFactory;

// ===========================================================================
// Core contracts (all backends)
// ===========================================================================

/// The runtime must start in `Uninitialized`.
pub fn initial_state_is_uninitialized<F: RuntimeFactory>() {
    let _ = F::shutdown(); // no-op if already uninitialized
    assert_eq!(F::state(), RuntimeState::Uninitialized);
}

/// After `initialize()`, state must be `Running`.
pub fn initialize_enters_running<F: RuntimeFactory>() {
    let _ = F::initialize(); // may already be initialized from prior test
    // Ensure clean.
    let _ = F::shutdown();

    F::initialize().unwrap();
    assert_eq!(F::state(), RuntimeState::Running);
    F::shutdown().unwrap();
}

/// Calling `initialize()` again while `Running` must return
/// `AlreadyInitialized` without corrupting state.
pub fn repeated_initialize_returns_already_initialized<F: RuntimeFactory>() {
    let _ = F::initialize();
    let _ = F::shutdown();
    F::initialize().unwrap();

    let result = F::initialize();
    assert_eq!(result, Err(Error::AlreadyInitialized));
    // State must still be Running.
    assert_eq!(F::state(), RuntimeState::Running);

    F::shutdown().unwrap();
}

/// `shutdown()` must return the runtime to `Uninitialized`.
pub fn shutdown_returns_to_uninitialized<F: RuntimeFactory>() {
    let _ = F::initialize();
    let _ = F::shutdown();

    F::initialize().unwrap();
    F::shutdown().unwrap();
    assert_eq!(F::state(), RuntimeState::Uninitialized);
}

/// Calling `shutdown()` while `Uninitialized` must return
/// `NotInitialized`.
pub fn shutdown_before_initialize_returns_not_initialized<F: RuntimeFactory>() {
    let _ = F::initialize();
    let _ = F::shutdown();

    // Already Uninitialized from prior cleanup.
    let result = F::shutdown();
    assert_eq!(result, Err(Error::NotInitialized));
}

/// After a full init → shutdown cycle, the runtime must be
/// re-initializable.
pub fn runtime_can_reinitialize<F: RuntimeFactory>() {
    let _ = F::initialize();
    let _ = F::shutdown();
    F::initialize().unwrap();
    assert_eq!(F::state(), RuntimeState::Running);
    F::shutdown().unwrap();
}

/// Run all core contracts for the given factory type.
pub fn run_core_contracts<F: RuntimeFactory>() {
    initial_state_is_uninitialized::<F>();
    initialize_enters_running::<F>();
    repeated_initialize_returns_already_initialized::<F>();
    shutdown_returns_to_uninitialized::<F>();
    shutdown_before_initialize_returns_not_initialized::<F>();
    runtime_can_reinitialize::<F>();
}
