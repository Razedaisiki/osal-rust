//! FreeRTOS Timer controlled contract tests — deterministic virtual-tick
//! timer verification via the fixture Virtual wait mode.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit timer_controlled -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use osal_backend_freertos::runtime;
use osal_backend_freertos::timer::FreeRtosTimerFactory;
use osal_backend_freertos_sys::fixture;
use osal_backend_freertos_sys::fixture::FixtureWaitMode;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup_virtual() {
    let _ = runtime::shutdown();
    fixture::reset();
    fixture::set_wait_mode(FixtureWaitMode::Virtual);
    runtime::initialize().expect("initialize runtime");
}

fn teardown_virtual() {
    let _ = runtime::shutdown();
    fixture::set_wait_mode(FixtureWaitMode::Realtime);
}

// ---------------------------------------------------------------------------
// Controlled contracts (5 tests) — Virtual wait mode, deterministic
// ---------------------------------------------------------------------------

#[test]
fn controlled_oneshot_fires_once() {
    setup_virtual();
    let factory = FreeRtosTimerFactory;
    osal_testkit::contract::timer::oneshot_fires_once(&factory);
    teardown_virtual();
}

#[test]
fn controlled_periodic_fires_multiple() {
    setup_virtual();
    let factory = FreeRtosTimerFactory;
    osal_testkit::contract::timer::periodic_fires_multiple(&factory);
    teardown_virtual();
}

#[test]
fn controlled_stop_prevents_callback() {
    setup_virtual();
    let factory = FreeRtosTimerFactory;
    osal_testkit::contract::timer::stop_prevents_callback(&factory);
    teardown_virtual();
}

#[test]
fn controlled_reset_restarts_deadline() {
    setup_virtual();
    let factory = FreeRtosTimerFactory;
    osal_testkit::contract::timer::reset_restarts_deadline(&factory);
    teardown_virtual();
}

#[test]
fn controlled_missed_expiration_coalesced() {
    setup_virtual();
    let factory = FreeRtosTimerFactory;
    osal_testkit::contract::timer::missed_expiration_coalesced(&factory);
    teardown_virtual();
}
