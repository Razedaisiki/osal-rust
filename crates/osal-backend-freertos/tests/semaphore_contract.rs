//! Semaphore contract tests for FreeRTOS backend (fixture).
//!
//! Uses the host sync fixture to simulate semaphore operations.  The
//! fixture provides real waiter/wake-one behaviour via
//! `std::sync::Mutex` + `Condvar`, so the contract tests exercise
//! the full blocking and timeout paths.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit semaphore_contract -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_backend_freertos::runtime;
use osal_backend_freertos::semaphore::FreeRtosSemaphoreFactory;
use osal_backend_freertos_sys::fixture;
use osal_testkit::contract::semaphore;

#[test]
fn freertos_semaphore_core_contracts() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize runtime");

    semaphore::run_all(&FreeRtosSemaphoreFactory);

    let _ = runtime::shutdown();
}
