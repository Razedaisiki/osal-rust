//! Contract tests for POSIX task.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit
//! ```

use osal_backend_posix::task::PosixTaskFactory;

#[test]
fn posix_task_smoke_contracts() {
    let factory = PosixTaskFactory;
    osal_testkit::contract::task::run_smoke_contracts(&factory);
}
