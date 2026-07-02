//! Fault injection contract tests.
//!
//! These tests verify that fault injection is correctly reported
//! by Mock backends. They use [`FaultFactory`] and are intended
//! to be invoked only by backends that support fault injection.
//! These are **not** part of any default `run_all()`.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;

use crate::factory::{FaultFactory, QueueFactory};

// ---------------------------------------------------------------------------
// Queue fault tests
// ---------------------------------------------------------------------------

/// Queue creation fault is reported.
pub fn queue_create_fault_is_reported<F: QueueFactory + FaultFactory>(factory: &F) {
    factory.clear_faults();
    factory.fail_next_queue_create(Error::OutOfMemory);
    let result = factory.create_queue(4, 2);
    assert!(matches!(result, Err(Error::OutOfMemory)));
}

/// Queue send fault is reported.
pub fn queue_send_fault_is_reported<F: QueueFactory + FaultFactory>(factory: &F) {
    factory.clear_faults();
    let q = factory.create_queue(4, 2).unwrap();
    factory.fail_next_queue_send(Error::QueueFull);
    let result = q.send(&[1, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueFull)));
}

/// Faults can be cleared.
pub fn faults_can_be_cleared<F: QueueFactory + FaultFactory>(factory: &F) {
    factory.clear_faults();
    factory.fail_next_queue_create(Error::OutOfMemory);
    factory.clear_faults();
    // Should succeed after clearing.
    let result = factory.create_queue(4, 2);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Grouped entry point
// ---------------------------------------------------------------------------

/// All queue fault contract tests.
pub fn run_queue_fault_contracts<F: QueueFactory + FaultFactory>(factory: &F) {
    queue_create_fault_is_reported::<F>(factory);
    queue_send_fault_is_reported::<F>(factory);
    faults_can_be_cleared::<F>(factory);
}
