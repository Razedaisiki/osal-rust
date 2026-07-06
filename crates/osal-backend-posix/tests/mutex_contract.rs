//! Contract tests for PosixMutexImpl.
//!
//! POSIX passes both core and blocking contracts.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit
//! ```

use osal_backend_posix::mutex::PosixMutexFactory;

#[test]
fn posix_mutex_core_contracts() {
    let factory = PosixMutexFactory;
    osal_testkit::contract::mutex::run_core_contracts(&factory);
}
