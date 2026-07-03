//! Contract tests for PosixQueue.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit
//! ```

use osal_backend_posix::queue::PosixQueueFactory;

#[test]
fn posix_queue_immediate_contracts() {
    let factory = PosixQueueFactory;
    osal_testkit::contract::queue::run_immediate_contracts(&factory);
}

#[test]
fn posix_queue_lifetime_contracts() {
    let factory = PosixQueueFactory;
    osal_testkit::contract::queue::run_lifetime_contracts(&factory);
}

#[test]
fn posix_queue_clone_lifetime_contracts() {
    let factory = PosixQueueFactory;
    osal_testkit::contract::lifetime::run_clone_contracts(&factory);
}
