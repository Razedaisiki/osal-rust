//! Contract tests for POSIX timer.
//!
//! POSIX passes core contracts.
//!
//! Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit
//! ```

use osal_backend_posix::timer::PosixTimerFactory;

#[test]
fn posix_timer_core_contracts() {
    let factory = PosixTimerFactory;
    osal_testkit::contract::timer::run_core_contracts(&factory);
}
