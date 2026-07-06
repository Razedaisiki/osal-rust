//! Contract tests for the [`Mutex`] trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#9-mutex-contract`.

use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex as _;

use crate::factory::{MutexFactory, TaskFactory};

// ---------------------------------------------------------------------------
// Creation
// ---------------------------------------------------------------------------

/// Mutex can be created with an initial value.
pub fn create<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(42).unwrap();
    let guard = m.lock(Timeout::NoWait).unwrap();
    assert_eq!(*guard, 42);
}

// ---------------------------------------------------------------------------
// Uncontended lock / unlock
// ---------------------------------------------------------------------------

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

/// Guard provides mutable access via DerefMut.
pub fn guard_deref_mut<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    {
        let mut guard = m.lock(Timeout::NoWait).unwrap();
        *guard += 1;
        assert_eq!(*guard, 1);
    }
    // Value persists across lock/unlock cycles.
    let guard = m.lock(Timeout::NoWait).unwrap();
    assert_eq!(*guard, 1);
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

// ---------------------------------------------------------------------------
// Recursive locking
// ---------------------------------------------------------------------------

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

/// Three levels of recursive locking.
pub fn recursive_lock_three_levels<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    let mut g1 = m.lock(Timeout::NoWait).unwrap();
    *g1 = 1;
    let mut g2 = m.lock(Timeout::NoWait).unwrap();
    *g2 = 2;
    let g3 = m.lock(Timeout::NoWait).unwrap();
    assert_eq!(*g3, 2);
    drop(g3);
    // g2 still holds; value unchanged.
    assert_eq!(*g2, 2);
    drop(g2);
    // g1 still holds; value unchanged.
    assert_eq!(*g1, 1);
    drop(g1);
    // All released; re-lock and verify final value.
    let guard = m.lock(Timeout::NoWait).unwrap();
    assert_eq!(*guard, 1);
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

/// Core contract tests — all backends must pass.
///
/// Covers creation, uncontended lock/unlock, and recursive semantics.
pub fn run_core_contracts<F: MutexFactory>(factory: &F) {
    create::<F>(factory);
    lock_unlock::<F>(factory);
    guard_deref_mut::<F>(factory);
    lock_forever::<F>(factory);
    lock_no_wait::<F>(factory);
    recursive_lock::<F>(factory);
    recursive_lock_three_levels::<F>(factory);
    guard_drop_releases_one_level::<F>(factory);
}

/// Blocking / concurrency contract tests.
///
/// Requires [`TaskFactory`] for cross-task testing. Currently a
/// placeholder — these tests will be filled when the POSIX Mutex
/// backend is implemented.
///
/// Future tests:
/// - mutex_excludes_other_task
/// - mutex_timeout_when_held_by_other_task (NoWait → LockFailed)
/// - mutex_after_returns_timeout
/// - mutex_forever_woken_by_guard_drop
/// - guard_drop_wakes_waiter
pub fn run_blocking_contracts<F: MutexFactory + TaskFactory>(_factory: &F) {}

/// All contracts except blocking.
pub fn run_all<F: MutexFactory>(factory: &F) {
    run_core_contracts(factory);
}
