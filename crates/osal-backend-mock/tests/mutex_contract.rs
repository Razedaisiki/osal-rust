//! Contract tests for MockMutex.
//!
//! Mock passes `MutexCoreContract` but not `MutexBlockingContract`
//! (cross-task blocking is deferred until the mock scheduler is
//! implemented).
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-mock --features testkit
//! ```

use osal_backend_mock::mutex::MockMutexFactory;
use osal_backend_mock::runtime;

#[test]
fn mock_mutex_core_contracts() {
    runtime::initialize().unwrap();
    let factory = MockMutexFactory;
    osal_testkit::contract::mutex::run_core_contracts(&factory);
    runtime::shutdown().unwrap();
}
