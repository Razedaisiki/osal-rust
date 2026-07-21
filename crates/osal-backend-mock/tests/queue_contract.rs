//! Contract tests for MockQueue.
//!
//! Mock passes `QueueCoreContract` but not `QueueBlockingContract`
//! (blocking is deferred until the mock scheduler is implemented).
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-mock --features testkit
//! ```

use osal_backend_mock::clock::MockClockControl;
use osal_backend_mock::queue::{MockFaultyQueueFactory, MockQueueFactory};
use osal_backend_mock::runtime;

// ---------------------------------------------------------------------------
// Queue contracts — Core only
// ---------------------------------------------------------------------------

#[test]
fn mock_queue_core_contracts() {
    runtime::initialize().unwrap();
    let factory = MockQueueFactory;
    osal_testkit::contract::queue::run_core_contracts(&factory);
    runtime::shutdown().unwrap();
}

#[test]
fn mock_queue_clone_lifetime_contracts() {
    runtime::initialize().unwrap();
    let factory = MockQueueFactory;
    osal_testkit::contract::lifetime::run_clone_contracts(&factory);
    runtime::shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// Clock contracts
// ---------------------------------------------------------------------------

#[test]
fn mock_clock_basic_contracts() {
    let factory = MockClockControl;
    factory.reset();
    osal_testkit::contract::clock::run_basic_contracts(&factory);
}

#[test]
fn mock_clock_controlled_contracts() {
    let factory = MockClockControl;
    factory.reset();
    osal_testkit::contract::clock::run_controlled_contracts(&factory);
}

// ---------------------------------------------------------------------------
// Fault contracts
// ---------------------------------------------------------------------------

#[test]
fn mock_queue_fault_contracts() {
    runtime::initialize().unwrap();
    let factory = MockFaultyQueueFactory::new();
    osal_testkit::contract::fault::run_queue_fault_contracts(&factory);
    runtime::shutdown().unwrap();
}
