//! Queue contract tests for the FreeRTOS backend.
//!
//! Runs the full QueueCoreContract and clone lifetime contract suites
//! from `osal-testkit`.  Blocking and concurrency tests live in
//! `queue_concurrent.rs`.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit queue_contract -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_api::error::Error;
use osal_backend_freertos::queue::FreeRtosQueueFactory;
use osal_backend_freertos::runtime;
use osal_backend_freertos_sys::fixture;

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize");
}

fn teardown() {
    match runtime::shutdown() {
        Ok(()) | Err(Error::NotInitialized) => {}
        Err(e) => panic!("test leaked runtime lease or object: {e:?}"),
    }
    fixture::reset();
}

// ---------------------------------------------------------------------------
// QueueCoreContract — 18 tests covering creation, FIFO, error precedence,
// close, and timeout
// ---------------------------------------------------------------------------

#[test]
fn queue_core_contracts() {
    setup();
    {
        let factory = FreeRtosQueueFactory;
        osal_testkit::contract::queue::run_core_contracts(&factory);
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Clone lifetime contracts — 3 tests
// ---------------------------------------------------------------------------

#[test]
fn queue_clone_lifetime_contracts() {
    setup();
    {
        let factory = FreeRtosQueueFactory;
        osal_testkit::contract::lifetime::run_clone_contracts(&factory);
    }
    teardown();
}
