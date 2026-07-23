//! System contract tests for FreeRTOS backend (fixture).
//!
//! Uses the test-fixture controlled heap-free value and critical-section
//! depth counter to verify System behaviour deterministically.  No real
//! FreeRTOS kernel required.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit system_contract -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_backend_freertos::runtime;
use osal_backend_freertos::system::FreeRtosSystemFactory;
use osal_backend_freertos_sys::fixture;
use osal_testkit::contract::system;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn system_contracts() {
    fixture::reset();
    runtime::initialize().expect("initialize runtime");

    system::run_all(&FreeRtosSystemFactory);

    runtime::shutdown().expect("shutdown runtime");
}
