//! Contract tests for mock semaphores.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-mock --features testkit
//! ```

use osal_backend_mock::semaphore::MockSemaphoreFactory;

#[test]
fn mock_semaphore_core_contracts() {
    let factory = MockSemaphoreFactory;
    osal_testkit::contract::semaphore::run_all(&factory);
}
