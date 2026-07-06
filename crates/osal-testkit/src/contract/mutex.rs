//! Contract tests for the [`Mutex`] trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#9-mutex-contract`.
//!
//! The mutex is **non-recursive** (see ADR 0007).

use osal_api::error::Error;
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
// Non-recursive: second lock while held must fail
// ---------------------------------------------------------------------------

/// Locking again while holding a guard returns LockFailed.
pub fn no_second_guard<F: MutexFactory>(factory: &F) {
    let m = factory.create_mutex(0).unwrap();
    let _guard = m.lock(Timeout::NoWait).unwrap();
    let result = m.lock(Timeout::NoWait);
    assert!(matches!(result, Err(Error::LockFailed)));
}

// ---------------------------------------------------------------------------
// Clone / handle sharing
// ---------------------------------------------------------------------------

/// Cloned handles share the same data.
pub fn clone_shares_state<F: MutexFactory>(factory: &F)
where
    F::Mutex: Clone,
{
    let m1 = factory.create_mutex(0).unwrap();
    let m2 = m1.clone();
    {
        let mut guard = m1.lock(Timeout::NoWait).unwrap();
        *guard = 99;
    }
    let guard = m2.lock(Timeout::NoWait).unwrap();
    assert_eq!(*guard, 99);
}

/// Dropping one clone does not destroy the shared resource.
pub fn drop_clone_keeps_alive<F: MutexFactory>(factory: &F)
where
    F::Mutex: Clone,
{
    let m1 = factory.create_mutex(0).unwrap();
    let m2 = m1.clone();
    {
        let mut guard = m1.lock(Timeout::NoWait).unwrap();
        *guard = 55;
    }
    drop(m1);
    let guard = m2.lock(Timeout::NoWait).unwrap();
    assert_eq!(*guard, 55);
}

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Core contract tests — all backends must pass.
///
/// Covers creation, uncontended lock/unlock, non-recursive guard
/// exclusivity, and clone handle sharing.
pub fn run_core_contracts<F: MutexFactory>(factory: &F)
where
    F::Mutex: Clone,
{
    create::<F>(factory);
    lock_unlock::<F>(factory);
    guard_deref_mut::<F>(factory);
    lock_forever::<F>(factory);
    lock_no_wait::<F>(factory);
    no_second_guard::<F>(factory);
    clone_shares_state::<F>(factory);
    drop_clone_keeps_alive::<F>(factory);
}

/// Blocking / concurrency contract tests.
///
/// Requires [`TaskFactory`] for cross-task testing. Currently a
/// placeholder — these tests are implemented in the POSIX backend's
/// integration tests using std::thread.
///
/// Future tests:
/// - mutex_excludes_other_task (NoWait → LockFailed)
/// - mutex_after_returns_timeout
/// - mutex_forever_woken_by_guard_drop
pub fn run_blocking_contracts<F: MutexFactory + TaskFactory>(_factory: &F) {}

/// All contracts except blocking.
pub fn run_all<F: MutexFactory>(factory: &F)
where
    F::Mutex: Clone,
{
    run_core_contracts(factory);
}
