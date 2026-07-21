//! Contract tests for PosixQueue.
//!
//! POSIX passes both `QueueCoreContract` and `QueueBlockingContract`.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit
//! ```

use osal_backend_posix::queue::PosixQueueFactory;
use osal_backend_posix::runtime;

// ---------------------------------------------------------------------------
// Queue core contracts
// ---------------------------------------------------------------------------

#[test]
fn posix_queue_core_contracts() {
    runtime::initialize().unwrap();
    let factory = PosixQueueFactory;
    osal_testkit::contract::queue::run_core_contracts(&factory);
    runtime::shutdown().unwrap();
}

#[test]
fn posix_queue_clone_lifetime_contracts() {
    runtime::initialize().unwrap();
    let factory = PosixQueueFactory;
    osal_testkit::contract::lifetime::run_clone_contracts(&factory);
    runtime::shutdown().unwrap();
}
