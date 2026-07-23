//! Clock contract tests for FreeRTOS backend (fixture).
//!
//! Uses the test-fixture controlled tick counter to verify Clock behaviour
//! deterministically.  No real FreeRTOS kernel required.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit clock_contract -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use core::time::Duration;

use osal_backend_freertos::clock::FreeRtosClock;
use osal_backend_freertos::runtime;
use osal_backend_freertos_sys::fixture;
use osal_testkit::contract::clock;
use osal_testkit::factory::{ClockControl, ClockFactory};

// ---------------------------------------------------------------------------
// Fixture factory
// ---------------------------------------------------------------------------

/// Controlled-clock fixture for FreeRTOS contract tests.
///
/// Implements both `ClockFactory` (provides `FreeRtosClock`) and
/// `ClockControl` (advances the virtual tick counter via fixture).
pub struct FreeRtosClockFixture;

impl ClockFactory for FreeRtosClockFixture {
    type Clock = FreeRtosClock;
}

impl ClockControl for FreeRtosClockFixture {
    fn advance_clock(&self, duration: Duration) {
        // Use the fixture's tick infrastructure: convert Duration to
        // ticks and advance the virtual counter via delay_ticks (which
        // in fixture mode advances rather than sleeping).
        let caps = runtime::capabilities_for_test().expect("runtime must be initialised");
        let ticks = osal_portable::tick_time::duration_to_ticks_ceil(duration, caps.tick_rate_hz)
            .expect("duration → ticks overflow");
        if ticks > 0 {
            osal_backend_freertos_sys::delay_ticks(ticks as u64);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn clock_basic_contracts() {
    fixture::reset();
    runtime::initialize().expect("initialize runtime");

    clock::run_basic_contracts(&FreeRtosClockFixture);

    runtime::shutdown().expect("shutdown runtime");
}

#[test]
fn clock_controlled_contracts() {
    fixture::reset();
    runtime::initialize().expect("initialize runtime");

    clock::run_controlled_contracts(&FreeRtosClockFixture);

    runtime::shutdown().expect("shutdown runtime");
}
