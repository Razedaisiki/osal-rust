//! Contract tests for mock timer.
//!
//! Mock passes core + controlled contracts.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-mock --features testkit
//! ```

use osal_backend_mock::clock::MockClockControl;
use osal_backend_mock::timer::MockTimerFactory;

#[test]
fn mock_timer_core_contracts() {
    MockClockControl.reset();
    let factory = MockTimerFactory;
    osal_testkit::contract::timer::run_core_contracts(&factory);
}

#[test]
fn mock_timer_controlled_contracts() {
    MockClockControl.reset();
    let factory = MockTimerFactory;
    osal_testkit::contract::timer::run_controlled_contracts(&factory);
}
