//! Task contract tests for the FreeRTOS backend.
//!
//! Runs the full TaskCoreContract suite from `osal-testkit`.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit task_contract -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_backend_freertos::runtime;
use osal_backend_freertos::task::FreeRtosTaskFactory;
use osal_backend_freertos_sys::fixture;

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize");
}

#[test]
fn task_core_contracts() {
    setup();
    let factory = FreeRtosTaskFactory;
    osal_testkit::contract::task::run_core_contracts(&factory);
    let _ = runtime::shutdown();
}
