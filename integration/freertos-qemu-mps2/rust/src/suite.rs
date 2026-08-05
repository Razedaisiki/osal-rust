//! Managed-object real-kernel validation suite dispatcher (P7G Step 4).

use osal_api::runtime::RuntimeState;
use osal_backend_freertos_sys as sys;

use crate::cases;
use crate::harness;

// ------------------------------------------------------------------
// Aggregate suite (suite-aggregate feature)
// ------------------------------------------------------------------

#[cfg(feature = "suite-aggregate")]
pub fn run_object_suite(tick_bits: u8) -> i32 {
    const SUITE_RUNTIME_INIT_FAILED: i32 = -150;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -151;
    const SUITE_FINAL_HEAP_LEAK: i32 = -152;

    harness::console_line(c"OSAL_OBJECT_BEGIN");

    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    let suite_baseline = sys::heap_free();

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    let mutex_result = cases::mutex::run_mutex_cases(tick_bits, suite_baseline);
    if let Err(e) = mutex_result {
        return -(e as i32);
    }

    let semaphore_result = cases::semaphore::run_semaphore_cases(tick_bits, suite_baseline);
    if let Err(e) = semaphore_result {
        return -(e as i32);
    }

    let queue_result = cases::queue::run_queue_cases(tick_bits, suite_baseline);
    if let Err(e) = queue_result {
        return -(e as i32);
    }

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

    harness::console_line(
        c"OSAL_OBJECT_PASS harness=true helper_self_delete=true idle_cleanup=true heap_recovered=true multi_helper=true tick_advance=true mutex=true mutex_clone=true mutex_timeout=true mutex_nowait=true mutex_blocking=true mutex_suspended=true mutex_lease=true semaphore=true counting=true binary=true semaphore_timeout=true semaphore_blocking=true semaphore_multi_waiter=true semaphore_suspended=true semaphore_lease=true queue=true queue_fifo=true queue_timeout=true queue_close=true queue_suspended=true queue_lease=true",
    );
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}

// ------------------------------------------------------------------
// Queue-blocking suite (suite-queue-blocking feature)
// ------------------------------------------------------------------

#[cfg(feature = "suite-queue-blocking")]
pub fn run_object_suite(tick_bits: u8) -> i32 {
    const SUITE_RUNTIME_INIT_FAILED: i32 = -160;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -161;
    const SUITE_FINAL_HEAP_LEAK: i32 = -162;

    harness::console_line(c"OSAL_OBJECT_BEGIN profile=queue-blocking");

    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    let profile_baseline = sys::heap_free();

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    let queue_result = cases::queue_blocking::run_queue_blocking_cases(tick_bits);

    if let Err(e) = queue_result {
        return -(e as i32);
    }

    let runtime_state = osal::runtime_state();
    let shutdown_ok = match runtime_state {
        RuntimeState::Running => osal::shutdown().is_ok(),
        RuntimeState::Uninitialized => true,
        _ => false,
    };
    if !shutdown_ok {
        return SUITE_RUNTIME_SHUTDOWN_FAILED;
    }
    if sys::heap_free() != profile_baseline {
        return SUITE_FINAL_HEAP_LEAK;
    }

    harness::console_line(
        c"OSAL_OBJECT_PASS profile=queue-blocking queue_blocking=true queue_forever=true queue_multi_waiter=true queue_close_broadcast=true queue_stack_margin=true queue_payload_accounting=true helper_self_delete=true idle_cleanup=true heap_recovered=true",
    );
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}

// ------------------------------------------------------------------
// Task suite (suite-task feature)
// ------------------------------------------------------------------

#[cfg(feature = "suite-task")]
pub fn run_object_suite(tick_bits: u8) -> i32 {
    const SUITE_RUNTIME_INIT_FAILED: i32 = -170;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -171;
    const SUITE_FINAL_HEAP_LEAK: i32 = -172;

    harness::console_line(c"OSAL_OBJECT_BEGIN profile=task");

    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    let profile_baseline = sys::heap_free();

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    let task_result = cases::task_contracts::run_task_cases(tick_bits);

    if let Err(e) = task_result {
        return -(e as i32);
    }

    let runtime_state = osal::runtime_state();
    let shutdown_ok = match runtime_state {
        RuntimeState::Running => osal::shutdown().is_ok(),
        RuntimeState::Uninitialized => true,
        _ => false,
    };
    if !shutdown_ok {
        return SUITE_RUNTIME_SHUTDOWN_FAILED;
    }
    if sys::heap_free() != profile_baseline {
        return SUITE_FINAL_HEAP_LEAK;
    }

    harness::console_line(
        c"OSAL_OBJECT_PASS profile=task task=true task_builder=true task_identity=true task_count=true task_join=true task_timeout=true task_self_join=true task_concurrent_join=true task_cached_join=true task_drop=true task_scheduler=true task_mapping=true task_lease=true task_self_delete=true task_stack_margin=true helper_self_delete=true idle_cleanup=true heap_recovered=true",
    );
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}
