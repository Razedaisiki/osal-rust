//! Managed-object real-kernel validation suite (P7G Step 4).
//!
//! Owns the object protocol envelope (`OSAL_OBJECT_BEGIN` /
//! `OSAL_OBJECT_PASS` / `OSAL_OBJECT_END`) and delegates individual
//! cases to their respective modules.
//!
//! Cases are added incrementally:
//!
//!   Step 4-0 — harness (harness.rs)
//!   Step 4A  — Mutex   (cases/mutex.rs)
//!   Step 4B  — Semaphore
//!   Step 4C  — Queue
//!   Step 4D  — Task
//!   Step 4E  — Timer

use crate::cases;
use crate::harness;

/// Top-level entry point for all managed-object real-kernel tests.
///
/// Called from the C boot task after Step 3C smoke and boot protocol
/// markers have been emitted.
pub fn run_object_suite(tick_bits: u8) -> i32 {
    // --- begin object protocol ---
    harness::console_line(c"OSAL_OBJECT_BEGIN");

    // ------------------------------------------------------------------
    // Step 4-0 — native helper-task harness smoke.
    // ------------------------------------------------------------------
    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    // ------------------------------------------------------------------
    // Step 4A — Mutex real-kernel contracts.
    // ------------------------------------------------------------------
    // Runtime errors — fatal to the object suite.
    const SUITE_RUNTIME_INIT_FAILED: i32 = -150;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -151;

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    let result = cases::mutex::run_mutex_cases(tick_bits);

    if osal::shutdown().is_err() {
        return SUITE_RUNTIME_SHUTDOWN_FAILED;
    }

    if let Err(e) = result {
        return -(e as i32);
    }

    // ------------------------------------------------------------------
    // Step 4B–4E — to be added.
    // ------------------------------------------------------------------

    // --- object pass ---
    harness::console_line(
        c"OSAL_OBJECT_PASS harness=true helper_self_delete=true idle_cleanup=true heap_recovered=true multi_helper=true tick_advance=true mutex=true mutex_clone=true mutex_timeout=true mutex_nowait=true mutex_blocking=true",
    );

    // --- end object protocol ---
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}
