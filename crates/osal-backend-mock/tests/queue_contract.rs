//! Contract tests for MockQueue.
//!
//! Runs osal-testkit contract suites against the mock backend.

use osal_backend_mock::clock::MockClockControl;
use osal_backend_mock::queue::MockQueueFactory;

// ---------------------------------------------------------------------------
// Immediate contracts
// ---------------------------------------------------------------------------

#[test]
fn mock_queue_immediate_contracts() {
    let factory = MockQueueFactory;
    osal_testkit::contract::queue::run_immediate_contracts(&factory);
}

#[test]
fn mock_queue_lifetime_contracts() {
    let factory = MockQueueFactory;
    osal_testkit::contract::queue::run_lifetime_contracts(&factory);
}

#[test]
fn mock_queue_clone_lifetime_contracts() {
    let factory = MockQueueFactory;
    osal_testkit::contract::lifetime::run_clone_contracts(&factory);
}

// ---------------------------------------------------------------------------
// Mutex contracts
// ---------------------------------------------------------------------------

// Skipped: MockQueue does not yet implement MutexFactory.

// ---------------------------------------------------------------------------
// Semaphore contracts
// ---------------------------------------------------------------------------

// Skipped: MockQueue does not yet implement SemaphoreFactory.

// ---------------------------------------------------------------------------
// Clock contracts
// ---------------------------------------------------------------------------

#[test]
fn mock_clock_basic_contracts() {
    let factory = MockClockControl;
    osal_testkit::contract::clock::run_basic_contracts(&factory);
}

#[test]
fn mock_clock_controlled_contracts() {
    let factory = MockClockControl;
    osal_testkit::contract::clock::run_controlled_contracts(&factory);
}

// ---------------------------------------------------------------------------
// Fault contracts
// ---------------------------------------------------------------------------

// Skipped: requires MockBackend that integrates fault state with queue.
