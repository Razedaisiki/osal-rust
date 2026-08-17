//! Managed-object real-kernel validation suite dispatcher (P7G Step 4).

#[cfg(feature = "suite-mixed")]
use osal_api::error::Error;

#[cfg(any(
    feature = "suite-aggregate",
    feature = "suite-queue-blocking",
    feature = "suite-task",
    feature = "suite-timer",
))]
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
        c"OSAL_OBJECT_PASS profile=queue-blocking queue_blocking=true queue_forever=true queue_multi_waiter=true queue_close_broadcast=true queue_stack_margin=true queue_payload_accounting=true queue_timeout_race=true queue_close_timeout_priority=true helper_self_delete=true idle_cleanup=true heap_recovered=true",
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
        c"OSAL_OBJECT_PASS profile=task task=true task_builder=true task_identity=true task_count=true task_join=true task_timeout=true task_self_join=true task_concurrent_join=true task_cached_join=true task_drop=true task_scheduler=true task_mapping=true task_rollback=true task_lease=true task_self_delete=true task_stack_margin=true helper_self_delete=true idle_cleanup=true heap_recovered=true",
    );
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}

// ------------------------------------------------------------------
// Timer suite (suite-timer feature)
// ------------------------------------------------------------------

#[cfg(feature = "suite-timer")]
pub fn run_object_suite(tick_bits: u8) -> i32 {
    const SUITE_RUNTIME_INIT_FAILED: i32 = -180;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -181;
    const SUITE_FINAL_HEAP_LEAK: i32 = -182;

    harness::console_line(c"OSAL_OBJECT_BEGIN profile=timer");

    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    let profile_baseline = sys::heap_free();

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    let timer_result = cases::timer_contracts::run_timer_cases(tick_bits);

    if let Err(e) = timer_result {
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
    // Worker TCB+stack reclaimed asynchronously by Idle after
    // task_delete_current.  Bounded wait, not a fixed sleep.
    if harness::wait_until_heap_recovered(profile_baseline, 100, tick_bits).is_err() {
        return SUITE_FINAL_HEAP_LEAK;
    }

    harness::console_line(
        c"OSAL_OBJECT_PASS profile=timer timer=true timer_worker=true timer_identity=true timer_stack_margin=true timer_builder=true timer_one_shot=true timer_periodic=true timer_control=true timer_change_period=true timer_coalescing=true timer_order=true timer_reentry=true timer_callback_unlock=true timer_drop=true timer_scheduler=true timer_shutdown=true timer_self_shutdown=true timer_lease=true timer_stress=true timer_self_delete=true helper_self_delete=true idle_cleanup=true heap_recovered=true",
    );
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}

// ------------------------------------------------------------------
// Mixed suite (suite-mixed feature)
// ------------------------------------------------------------------

#[cfg(feature = "suite-mixed")]
pub fn run_object_suite(tick_bits: u8) -> i32 {
    const SUITE_RUNTIME_INIT_FAILED: i32 = -190;
    const SUITE_RUNTIME_SHUTDOWN_FAILED: i32 = -191;
    const SUITE_FINAL_HEAP_LEAK: i32 = -192;

    harness::console_line(c"OSAL_OBJECT_BEGIN profile=mixed");

    if let Err(e) = harness::run_harness_case(tick_bits) {
        return e as i32;
    }

    let profile_baseline = sys::heap_free();

    if osal::initialize().is_err() {
        return SUITE_RUNTIME_INIT_FAILED;
    }

    let mixed_result = cases::mixed::run_mixed_cases(tick_bits, profile_baseline);

    if let Err(e) = mixed_result {
        return -(e as i32);
    }

    // mixed_shutdown_accounting already verified clean lease accounting
    // and dropped the last task handle — the final shutdown must
    // succeed directly.  No retry loop.
    if !matches!(osal::shutdown(), Ok(())) {
        return SUITE_RUNTIME_SHUTDOWN_FAILED;
    }
    if harness::wait_until_heap_recovered(profile_baseline, 100, tick_bits).is_err() {
        return SUITE_FINAL_HEAP_LEAK;
    }

    harness::console_line(
        c"OSAL_OBJECT_PASS profile=mixed mixed=true mixed_rollback=true mixed_pressure=true mixed_pipeline=true mixed_stress=true mixed_shutdown=true task_self_delete=true timer_self_delete=true helper_self_delete=true idle_cleanup=true heap_recovered=true",
    );
    harness::console_line(c"OSAL_OBJECT_END status=pass");

    0
}
