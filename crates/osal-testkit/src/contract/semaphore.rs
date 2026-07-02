//! Contract tests for the [`CountingSemaphore`] and [`BinarySemaphore`] traits.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#10-semaphore-contract`.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::semaphore::{BinarySemaphore as _, CountingSemaphore as _};

use crate::factory::{ClockFactory, SemaphoreFactory, TaskFactory};

// ---------------------------------------------------------------------------
// CountingSemaphore
// ---------------------------------------------------------------------------

/// Create with valid bounds; verify initial count.
pub fn counting_create<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 2).unwrap();
    assert_eq!(sem.count(), 2);
    assert_eq!(sem.max_count(), 5);
}

/// Reject initial count greater than max count.
pub fn counting_reject_initial_gt_max<F: SemaphoreFactory>(factory: &F) {
    let result = factory.create_counting_semaphore(3, 5);
    assert!(result.is_err());
}

/// Reject max count of zero.
pub fn counting_reject_max_zero<F: SemaphoreFactory>(factory: &F) {
    let result = factory.create_counting_semaphore(0, 0);
    assert!(result.is_err());
}

/// Acquire decrements the count.
pub fn counting_acquire_decrements<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 3).unwrap();
    sem.acquire(Timeout::NoWait).unwrap();
    assert_eq!(sem.count(), 2);
}

/// Release increments the count.
pub fn counting_release_increments<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 0).unwrap();
    sem.release().unwrap();
    assert_eq!(sem.count(), 1);
}

/// Release at max count returns Error::Overflow.
pub fn counting_release_at_max_fails<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(1, 1).unwrap();
    let result = sem.release();
    assert!(matches!(result, Err(Error::Overflow)));
}

/// Acquire on empty with NoWait returns Error::Timeout.
pub fn counting_acquire_empty_no_wait<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 0).unwrap();
    let result = sem.acquire(Timeout::NoWait);
    assert!(matches!(result, Err(Error::Timeout)));
}

/// Acquire on empty with After times out.
pub fn counting_acquire_timeout<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 0).unwrap();
    let result = sem.acquire(Timeout::After(core::time::Duration::from_millis(10)));
    assert!(matches!(result, Err(Error::Timeout)));
}

/// Non-blocking ISR acquire returns immediately.
pub fn counting_isr_acquire<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 1).unwrap();
    sem.isr_acquire().unwrap();
    assert_eq!(sem.count(), 0);
}

/// Non-blocking ISR release returns immediately.
pub fn counting_isr_release<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 0).unwrap();
    sem.isr_release().unwrap();
    assert_eq!(sem.count(), 1);
}

// ---------------------------------------------------------------------------
// BinarySemaphore
// ---------------------------------------------------------------------------

/// Create binary semaphore with count 0.
pub fn binary_create<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    assert!(!sem.is_acquired());
}

/// Acquire blocks until release.
pub fn binary_acquire_release<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    // Release first, then acquire.
    sem.release().unwrap();
    assert!(sem.is_acquired());
    sem.acquire(Timeout::NoWait).unwrap();
    assert!(!sem.is_acquired());
}

/// Release when already signaled returns Error::Overflow.
pub fn binary_double_release_fails<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.release().unwrap();
    let result = sem.release();
    assert!(matches!(result, Err(Error::Overflow)));
}

/// ISR acquire on signaled semaphore succeeds.
pub fn binary_isr_acquire<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.release().unwrap();
    sem.isr_acquire().unwrap();
    assert!(!sem.is_acquired());
}

/// ISR release succeeds.
pub fn binary_isr_release<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.isr_release().unwrap();
    assert!(sem.is_acquired());
}

// ---------------------------------------------------------------------------
// Aggregator
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Counting semaphore immediate tests (no wait/concurrency).
pub fn run_counting_immediate_contracts<F: SemaphoreFactory>(factory: &F) {
    counting_create::<F>(factory);
    counting_reject_initial_gt_max::<F>(factory);
    counting_reject_max_zero::<F>(factory);
    counting_acquire_decrements::<F>(factory);
    counting_release_increments::<F>(factory);
    counting_release_at_max_fails::<F>(factory);
    counting_acquire_empty_no_wait::<F>(factory);
}

/// Counting semaphore timeout-based tests.
pub fn run_counting_wait_contracts<F: SemaphoreFactory>(factory: &F) {
    counting_acquire_timeout::<F>(factory);
}

/// Binary semaphore immediate tests.
pub fn run_binary_immediate_contracts<F: SemaphoreFactory>(factory: &F) {
    binary_create::<F>(factory);
    binary_acquire_release::<F>(factory);
    binary_double_release_fails::<F>(factory);
}

/// ISR-variant tests for both semaphore types.
pub fn run_isr_contracts<F: SemaphoreFactory>(factory: &F) {
    counting_isr_acquire::<F>(factory);
    counting_isr_release::<F>(factory);
    binary_isr_acquire::<F>(factory);
    binary_isr_release::<F>(factory);
}

/// Cross-task concurrency tests (requires [`TaskFactory`] + [`ClockFactory`]).
///
/// Currently a placeholder. Future tests:
/// - counting_release_wakes_one_waiter
/// - counting_timeout_when_empty
/// - binary_release_wakes_one_waiter
pub fn run_wait_contracts<F: SemaphoreFactory + TaskFactory + ClockFactory>(_factory: &F) {}

/// All current contract tests.
pub fn run_all<F: SemaphoreFactory>(factory: &F) {
    run_counting_immediate_contracts(factory);
    run_counting_wait_contracts(factory);
    run_binary_immediate_contracts(factory);
    run_isr_contracts(factory);
}
