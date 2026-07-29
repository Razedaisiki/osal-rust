//! FreeRTOS Timer contract tests — shared core and controlled contracts.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit timer_contract -- --test-threads=1
//! cargo test -p osal-backend-freertos --features testkit timer_controlled -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::FreeRtosTimerFactory;
use osal_backend_freertos_sys::fixture;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize runtime");
}

fn teardown() {
    let _ = runtime::shutdown();
}

// ---------------------------------------------------------------------------
// Core contracts (6 tests)
// ---------------------------------------------------------------------------

#[test]
fn freertos_timer_core_contracts() {
    setup();
    let factory = FreeRtosTimerFactory;
    osal_testkit::contract::timer::run_core_contracts(&factory);
    teardown();
}
