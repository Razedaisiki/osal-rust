//! Contract tests for MockQueue.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-mock --features testkit
//! ```

use osal_backend_mock::clock::MockClockControl;
use osal_backend_mock::queue::MockQueueFactory;

// ---------------------------------------------------------------------------
// Queue contracts
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
