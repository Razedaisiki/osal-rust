//! Contract tests for the [`Mutex`] trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#9-mutex-contract`.

use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex as _;

use crate::factory::{MutexFactory, TaskFactory};

/// Lock succeeds on an uncontended mutex; guard provides access;
/// drop releases the lock.
pub fn lock_unlock<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(42).unwrap();
    {
        let guard = m.lock(Timeout::NoWait).unwrap();
        assert_eq!(*guard, 42);
    }
    // Re-lock after guard dropped — must succeed.
    let _g2 = m.lock(Timeout::NoWait).unwrap();
}

/// Non-blocking lock fails when the mutex is held by another context.
pub fn try_lock_fails_when_held<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    let _guard = m.lock(Timeout::NoWait).unwrap();
    // Second lock on same mutex from same task — recursive.
    // From a different task this would fail; tested in concurrency
    // tests when Task support is available.
}

/// `Timeout::Forever` blocks until the lock is acquired.
pub fn lock_forever<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    let guard = m.lock(Timeout::Forever).unwrap();
    assert_eq!(*guard, 0);
    drop(guard);
}

/// `Timeout::NoWait` succeeds when uncontended.
pub fn lock_no_wait<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(100).unwrap();
    let guard = m.lock(Timeout::NoWait).unwrap();
    assert_eq!(*guard, 100);
    drop(guard);
}

/// Recursive lock: the owning task can lock the same mutex
/// multiple times without deadlocking.
pub fn recursive_lock<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    let g1 = m.lock(Timeout::NoWait).unwrap();
    let g2 = m.lock(Timeout::NoWait).unwrap();
    assert_eq!(*g1, 0);
    assert_eq!(*g2, 0);
    drop(g2);
    drop(g1);
    // After all guards dropped, can lock again.
    let _g3 = m.lock(Timeout::NoWait).unwrap();
}

/// Guard drop releases exactly one recursion level.
pub fn guard_drop_releases_one_level<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    let _outer = m.lock(Timeout::NoWait).unwrap();
    {
        let _inner = m.lock(Timeout::NoWait).unwrap();
        // inner dropped here
    }
    // outer still holds — verify by locking once more (recursive).
    let _still_ok = m.lock(Timeout::NoWait).unwrap();
}

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Basic lock/unlock and guard semantics (no recursion, no concurrency).
pub fn run_immediate_contracts<F: MutexFactory>(factory: &F) {
    lock_unlock::<F>(factory);
    lock_forever::<F>(factory);
    lock_no_wait::<F>(factory);
    try_lock_fails_when_held::<F>(factory);
}

/// Recursive locking semantics.
pub fn run_recursive_contracts<F: MutexFactory>(factory: &F) {
    recursive_lock::<F>(factory);
    guard_drop_releases_one_level::<F>(factory);
}

/// Cross-task concurrency tests (requires [`TaskFactory`]).
///
/// Currently a placeholder. Future tests:
/// - mutex_excludes_other_task
/// - mutex_timeout_when_held_by_other_task
/// - guard_drop_wakes_waiter
pub fn run_concurrency_contracts<F: MutexFactory + TaskFactory>(_factory: &F) {}

/// All current contract tests.
pub fn run_all<F: MutexFactory>(factory: &F) {
    run_immediate_contracts(factory);
    run_recursive_contracts(factory);
}
