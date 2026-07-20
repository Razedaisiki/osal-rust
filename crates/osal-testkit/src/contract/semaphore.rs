//! Contract tests for the `CountingSemaphore` and `BinarySemaphore` traits.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#10-semaphore-contract`.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::semaphore::{BinarySemaphore as _, CountingSemaphore as _};

use crate::factory::{SemaphoreFactory, TaskFactory};

// ===========================================================================
// CountingSemaphore — Core
// ===========================================================================

pub fn counting_create_valid<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 2).unwrap();
    assert_eq!(sem.max_count(), 5);
    assert_eq!(sem.count().unwrap(), 2);
}

pub fn counting_reject_max_zero<F: SemaphoreFactory>(factory: &F) {
    let result = factory.create_counting_semaphore(0, 0);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

pub fn counting_reject_initial_gt_max<F: SemaphoreFactory>(factory: &F) {
    let result = factory.create_counting_semaphore(3, 5);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

pub fn counting_max_count_is_fixed<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 3).unwrap();
    sem.acquire(Timeout::NoWait).unwrap();
    sem.release().unwrap();
    assert_eq!(sem.max_count(), 5);
}

pub fn counting_acquire_decrements<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 3).unwrap();
    sem.acquire(Timeout::NoWait).unwrap();
    assert_eq!(sem.count().unwrap(), 2);
}

pub fn counting_release_increments<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(5, 0).unwrap();
    sem.release().unwrap();
    assert_eq!(sem.count().unwrap(), 1);
}

pub fn counting_release_at_max_overflows<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(1, 1).unwrap();
    let result = sem.release();
    assert!(matches!(result, Err(Error::Overflow)));
}

pub fn counting_overflow_preserves_count<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(2, 2).unwrap();
    let _ = sem.release();
    assert_eq!(sem.count().unwrap(), 2);
}

pub fn counting_empty_no_wait_times_out<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 0).unwrap();
    let result = sem.acquire(Timeout::NoWait);
    assert!(matches!(result, Err(Error::Timeout)));
}

pub fn counting_empty_after_zero_times_out<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 0).unwrap();
    let result = sem.acquire(Timeout::After(core::time::Duration::ZERO));
    assert!(matches!(result, Err(Error::Timeout)));
}

pub fn counting_available_after_zero_succeeds<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 1).unwrap();
    sem.acquire(Timeout::After(core::time::Duration::ZERO))
        .unwrap();
    assert_eq!(sem.count().unwrap(), 0);
}

pub fn counting_failed_acquire_preserves_count<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(3, 1).unwrap();
    let _ = sem.acquire(Timeout::After(core::time::Duration::from_millis(10)));
    // Should succeed (count > 0), not block
    assert_eq!(sem.count().unwrap(), 0);
}

pub fn counting_clone_shares_state<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone,
{
    let s1 = factory.create_counting_semaphore(3, 1).unwrap();
    let s2 = s1.clone();
    s2.acquire(Timeout::NoWait).unwrap();
    // Both handles see count 0
    assert_eq!(s1.count().unwrap(), 0);
    assert_eq!(s2.count().unwrap(), 0);
}

pub fn counting_drop_one_clone_preserves_resource<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone,
{
    let s1 = factory.create_counting_semaphore(3, 1).unwrap();
    let s2 = s1.clone();
    drop(s1);
    // s2 still works
    s2.acquire(Timeout::NoWait).unwrap();
    assert_eq!(s2.count().unwrap(), 0);
}

// ===========================================================================
// BinarySemaphore — Core
// ===========================================================================

pub fn binary_create_unsignaled<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    assert!(!sem.is_signaled().unwrap());
}

pub fn binary_release_signals<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.release().unwrap();
    assert!(sem.is_signaled().unwrap());
}

pub fn binary_acquire_clears_signal<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.release().unwrap();
    sem.acquire(Timeout::NoWait).unwrap();
    assert!(!sem.is_signaled().unwrap());
}

pub fn binary_double_release_overflows<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.release().unwrap();
    let result = sem.release();
    assert!(matches!(result, Err(Error::Overflow)));
}

pub fn binary_overflow_preserves_signal<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    sem.release().unwrap();
    let _ = sem.release();
    assert!(sem.is_signaled().unwrap());
}

pub fn binary_empty_no_wait_times_out<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    let result = sem.acquire(Timeout::NoWait);
    assert!(matches!(result, Err(Error::Timeout)));
}

pub fn binary_empty_after_zero_times_out<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_binary_semaphore().unwrap();
    let result = sem.acquire(Timeout::After(core::time::Duration::ZERO));
    assert!(matches!(result, Err(Error::Timeout)));
}

pub fn binary_clone_shares_state<F: SemaphoreFactory>(factory: &F)
where
    F::BinarySemaphore: Clone,
{
    let s1 = factory.create_binary_semaphore().unwrap();
    let s2 = s1.clone();
    s2.release().unwrap();
    assert!(s1.is_signaled().unwrap());
}

pub fn binary_drop_one_clone_preserves_resource<F: SemaphoreFactory>(factory: &F)
where
    F::BinarySemaphore: Clone,
{
    let s1 = factory.create_binary_semaphore().unwrap();
    let s2 = s1.clone();
    drop(s1);
    s2.release().unwrap();
    assert!(s2.is_signaled().unwrap());
}

// ===========================================================================
// Grouped entry points
// ===========================================================================

/// Counting semaphore core contracts — all backends must pass.
pub fn run_counting_core_contracts<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone,
{
    counting_create_valid::<F>(factory);
    counting_reject_max_zero::<F>(factory);
    counting_reject_initial_gt_max::<F>(factory);
    counting_max_count_is_fixed::<F>(factory);
    counting_acquire_decrements::<F>(factory);
    counting_release_increments::<F>(factory);
    counting_release_at_max_overflows::<F>(factory);
    counting_overflow_preserves_count::<F>(factory);
    counting_empty_no_wait_times_out::<F>(factory);
    counting_empty_after_zero_times_out::<F>(factory);
    counting_available_after_zero_succeeds::<F>(factory);
    counting_failed_acquire_preserves_count::<F>(factory);
    counting_clone_shares_state::<F>(factory);
    counting_drop_one_clone_preserves_resource::<F>(factory);
}

/// Binary semaphore core contracts — all backends must pass.
pub fn run_binary_core_contracts<F: SemaphoreFactory>(factory: &F)
where
    F::BinarySemaphore: Clone,
{
    binary_create_unsignaled::<F>(factory);
    binary_release_signals::<F>(factory);
    binary_acquire_clears_signal::<F>(factory);
    binary_double_release_overflows::<F>(factory);
    binary_overflow_preserves_signal::<F>(factory);
    binary_empty_no_wait_times_out::<F>(factory);
    binary_empty_after_zero_times_out::<F>(factory);
    binary_clone_shares_state::<F>(factory);
    binary_drop_one_clone_preserves_resource::<F>(factory);
}

/// Blocking / concurrency contract tests (POSIX only).
///
/// Requires `TaskFactory`. Currently a placeholder — tests are
/// implemented in the POSIX backend's integration tests.
pub fn run_blocking_contracts<F: SemaphoreFactory + TaskFactory>(_factory: &F) {}

/// All core contracts.
pub fn run_all<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone,
    F::BinarySemaphore: Clone,
{
    run_counting_core_contracts(factory);
    run_binary_core_contracts(factory);
}
