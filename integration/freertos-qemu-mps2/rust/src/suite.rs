//! Managed-object real-kernel validation suite (P7G Step 4).
//!
//! Owns the object protocol envelope (`OSAL_OBJECT_BEGIN` /
//! `OSAL_OBJECT_PASS` / `OSAL_OBJECT_END`) and delegates individual
//! cases to their respective modules.

use osal_api::runtime::RuntimeState;
use osal_backend_freertos_sys as sys;

use crate::cases;
use crate::harness;

/// Top-level entry point for all managed-object real-kernel tests.
pub fn run_object_suite(tick_bits: u8) -> i32 {
    const SUITE_RUNTIME_INIT_FAILED: i32 = -150;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -151;
    const SUITE_FINAL_HEAP_LEAK: i32 = -152;

    // --- begin object protocol ---
    harness::console_line(c"OSAL_OBJECT_BEGIN");

    // ------------------------------------------------------------------
    // Step 4-0 — native helper-task harness smoke.
    // ------------------------------------------------------------------
    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    // Suite baseline: heap before any OSAL objects are created.
    let suite_baseline = sys::heap_free();

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    // ------------------------------------------------------------------
    // Step 4A — Mutex real-kernel contracts.
    // ------------------------------------------------------------------
    let mutex_result = cases::mutex::run_mutex_cases(tick_bits, suite_baseline);

    // Primary: case errors.
    if let Err(e) = mutex_result {
        return -(e as i32);
    }

    // ------------------------------------------------------------------
    // Step 4B — Semaphore real-kernel contracts.
    // ------------------------------------------------------------------
    let semaphore_result = cases::semaphore::run_semaphore_cases(tick_bits, suite_baseline);

    if let Err(e) = semaphore_result {
        return -(e as i32);
    }

    // ------------------------------------------------------------------
    // Step 4C — Queue real-kernel contracts.
    // ------------------------------------------------------------------
    let queue_result = cases::queue::run_queue_cases(tick_bits);

    if let Err(e) = queue_result {
        return -(e as i32);
    }

    // ------------------------------------------------------------------
    // Final shutdown and heap gate (Mutex lifecycle + Semaphore
    // lifecycle cases have reinitialized; one final shutdown).
    // ------------------------------------------------------------------
    let runtime_state = osal::runtime_state();
    let shutdown_ok = match runtime_state {
        RuntimeState::Running => osal::shutdown().is_ok(),
        RuntimeState::Uninitialized => true,
        _ => false,
    };

    if !shutdown_ok {
        return SUITE_RUNTIME_SHUTDOWN_FAILED;
    }

    if sys::heap_free() != suite_baseline {
        return SUITE_FINAL_HEAP_LEAK;
    }

    // ------------------------------------------------------------------
    // Step 4C–4E — to be added.
    // ------------------------------------------------------------------

    // --- object pass ---
    harness::console_line(
        c"OSAL_OBJECT_PASS harness=true helper_self_delete=true idle_cleanup=true heap_recovered=true multi_helper=true tick_advance=true mutex=true mutex_clone=true mutex_timeout=true mutex_nowait=true mutex_blocking=true mutex_suspended=true mutex_lease=true semaphore=true counting=true binary=true semaphore_timeout=true semaphore_blocking=true semaphore_multi_waiter=true semaphore_suspended=true semaphore_lease=true queue=true queue_fifo=true",
    );

    // --- end object protocol ---
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}
