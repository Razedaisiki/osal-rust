//! Mutex contract tests for FreeRTOS backend (fixture).
//!
//! Uses the host sync fixture to simulate mutex operations.  The
//! fixture provides real waiter/wake-one behaviour via
//! `std::sync::Mutex` + `Condvar`, so the contract tests exercise
//! the full blocking and timeout paths.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit mutex_contract -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_backend_freertos::mutex::FreeRtosMutexFactory;
use osal_backend_freertos::runtime;
use osal_backend_freertos_sys::fixture;
use osal_testkit::contract::mutex;

#[test]
fn freertos_mutex_core_contracts() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize runtime");

    mutex::run_core_contracts(&FreeRtosMutexFactory);

    let _ = runtime::shutdown();
}
