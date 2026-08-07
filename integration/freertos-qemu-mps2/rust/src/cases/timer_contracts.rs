//! Timer real-kernel contracts (P7G Step 4E).
//!
//! Commit 1 — suite-timer profile, internal-task diagnostics,
//! worker identity probe (timer_worker_lazy_identity).

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};
use core::time::Duration;

use osal_api::traits::task::Task;
use osal_api::traits::timer::{Timer, TimerCallback};
use osal_api::types::TimerMode;
use osal_backend_freertos::task::FreeRtosTask;
use osal_backend_freertos::timer::FreeRtosTimer;
use osal_backend_freertos_sys as sys;

use crate::harness;

// ------------------------------------------------------------------
// Error codes — returned as negative i32 through run_timer_cases.
// ------------------------------------------------------------------
#[repr(i32)]
pub enum TimerContractError {
    // Worker identity
    WorkerCreatedTooEarly = 600,
    WorkerNotCreated = 601,
    WorkerCreatedMultiple = 602,
    WorkerNativeHandleZero = 603,
    WorkerPriorityWrong = 604,
    WorkerStackWrong = 605,
    WorkerTaskCurrentNotNone = 606,
    WorkerTaskCountIncreased = 607,
    WorkerHwmTooLow = 608,

    // Cleanup
    WorkerCleanupFailed = 609,
}

type TestResult = Result<(), TimerContractError>;

// ------------------------------------------------------------------
// Diagnostics FFI — C observer getters (linked in suite-timer).
// ------------------------------------------------------------------
unsafe extern "C" {
    fn osal_test_diag_reset();
    fn osal_test_diag_internal_task_create_attempts() -> u32;
    fn osal_test_diag_internal_task_create_successes() -> u32;
    fn osal_test_diag_last_internal_stack_words() -> u32;
    fn osal_test_diag_last_internal_priority() -> u32;
    fn osal_test_diag_last_internal_handle() -> u32;
    fn osal_test_task_stack_hwm() -> u32;
}

fn diag_reset() { unsafe { osal_test_diag_reset() } }
fn diag_internal_task_create_attempts() -> u32 { unsafe { osal_test_diag_internal_task_create_attempts() } }
fn diag_internal_task_create_successes() -> u32 { unsafe { osal_test_diag_internal_task_create_successes() } }
fn diag_last_internal_stack_words() -> u32 { unsafe { osal_test_diag_last_internal_stack_words() } }
fn diag_last_internal_priority() -> u32 { unsafe { osal_test_diag_last_internal_priority() } }
fn diag_last_internal_handle() -> u32 { unsafe { osal_test_diag_last_internal_handle() } }
fn task_stack_hwm() -> u32 { unsafe { osal_test_task_stack_hwm() } }

// ------------------------------------------------------------------
// Constants
// ------------------------------------------------------------------
const HWM_MIN_WORDS: u32 = 64;

// ------------------------------------------------------------------
// Entry point — called by suite::run_object_suite.
//
// Heap baseline management (P7G Step 4E):
//   profile_baseline  — after initialize(), before any case
//   worker_baseline   — after first start() creates the worker
//   case_baseline     — after test state allocations
//
// Each case must revert to worker_baseline.  The suite handles
// shutdown → worker self-delete → heap == profile_baseline.
// ------------------------------------------------------------------
pub fn run_timer_cases(_tick_bits: u8) -> Result<(), i32> {
    diag_reset();

    // Case 1: worker lazy identity.
    // This case creates the worker (lazy), so it establishes worker_baseline.
    if let Err(e) = case_timer_worker_lazy_identity() {
        return Err(-(e as i32));
    }

    Ok(())
}
fn case_timer_worker_lazy_identity() -> TestResult {
    let caps = sys::capabilities();
    let expected_priority = caps.max_priorities.saturating_sub(1);
    // Expected stack: 4096 bytes → words (round up, clamp to min/max).
    let expected_stack_words = {
        let raw = (4096u32 + caps.stack_word_size as u32 - 1) / caps.stack_word_size as u32;
        raw.max(caps.minimal_stack_depth_words).min(caps.max_stack_depth_words)
    };

    // Shared slot: callback records Task::current(), Task::count(), and HWM here.
    let callback_current_some: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
    let callback_count: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
    let callback_hwm: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));

    let cb_current = Arc::clone(&callback_current_some);
    let cb_count = Arc::clone(&callback_count);
    let cb_hwm = Arc::clone(&callback_hwm);

    let test_callback: TimerCallback = Box::new(move || {
        // Inside the Timer worker context.
        // Task::current() must be None (worker is not an OSAL Task).
        if FreeRtosTask::current().is_some() {
            cb_current.store(1, Ordering::Release);
        }
        // Task::count() should not include the worker.
        cb_count.store(FreeRtosTask::count() as u32, Ordering::Release);
        // Stack HWM from within the worker.
        let hwm = task_stack_hwm();
        cb_hwm.store(hwm, Ordering::Release);
    });

    let period = Duration::from_millis(50); // 50 ticks

    // Phase 1: before first start, no worker should exist.
    let diag_before = read_internal_diag();

    // Creating a timer, stopping it, changing period — none create the worker.
    let t = FreeRtosTimer::new(
        "test-wid",
        period,
        TimerMode::OneShot,
        test_callback,
    ).map_err(|_| TimerContractError::WorkerCreatedTooEarly)?;

    t.stop().map_err(|_| TimerContractError::WorkerCreatedTooEarly)?;
    t.change_period(Duration::from_millis(100))
        .map_err(|_| TimerContractError::WorkerCreatedTooEarly)?;

    // Worker must still not exist.
    let diag_still_before = read_internal_diag();
    if internal_create_attempt_delta(&diag_still_before, &diag_before) != 0 {
        return Err(TimerContractError::WorkerCreatedTooEarly);
    }

    // Phase 2: first start creates the worker.
    // Record public Task::count() baseline before the worker exists.
    let public_task_count_baseline = FreeRtosTask::count() as u32;
    t.start().map_err(|_| TimerContractError::WorkerNotCreated)?;
    let diag_after_start = read_internal_diag();
    let attempts = internal_create_attempt_delta(&diag_after_start, &diag_still_before);
    let successes = internal_create_success_delta(&diag_after_start, &diag_still_before);
    if attempts != 1 || successes != 1 {
        return Err(TimerContractError::WorkerNotCreated);
    }

    // Verify diagnostics: native handle, priority, stack.
    if diag_last_internal_handle() == 0 {
        return Err(TimerContractError::WorkerNativeHandleZero);
    }
    if diag_last_internal_priority() != expected_priority {
        return Err(TimerContractError::WorkerPriorityWrong);
    }
    if diag_last_internal_stack_words() != expected_stack_words {
        return Err(TimerContractError::WorkerStackWrong);
    }

    // Phase 3: second start does NOT create a second worker.
    let diag_before_second = read_internal_diag();
    t.reset().map_err(|_| TimerContractError::WorkerCreatedMultiple)?;
    let diag_after_second = read_internal_diag();
    if internal_create_attempt_delta(&diag_after_second, &diag_before_second) != 0 {
        return Err(TimerContractError::WorkerCreatedMultiple);
    }

    // Phase 4: wait for the callback to fire, then stop.
    // OneShot timer with 50ms period — callback fires once, then stops.
    // Poll until callback has run (HWM recorded).
    let deadline_ticks: u32 = 200; // 200ms should be more than enough
    let start_tick = sys::tick_snapshot();
    loop {
        if callback_hwm.load(Ordering::Acquire) != 0 {
            break;
        }
        let now = sys::tick_snapshot();
        let elapsed = harness::total_ticks_diff(now, start_tick, caps.tick_bits);
        if elapsed >= deadline_ticks as u128 {
            // Callback may still fire; check one more time.
            if callback_hwm.load(Ordering::Acquire) != 0 {
                break;
            }
            // Timer is stopped; if callback hasn't fired by now, something is wrong.
            return Err(TimerContractError::WorkerNotCreated);
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(TimerContractError::WorkerNotCreated);
        }
    }

    // Verify worker identity from inside the callback.
    if callback_current_some.load(Ordering::Acquire) != 0 {
        return Err(TimerContractError::WorkerTaskCurrentNotNone);
    }
    // Task::count() must not include the worker — exact baseline equality.
    let count = callback_count.load(Ordering::Acquire);
    if count != public_task_count_baseline {
        return Err(TimerContractError::WorkerTaskCountIncreased);
    }
    let hwm = callback_hwm.load(Ordering::Acquire);
    if hwm < HWM_MIN_WORDS {
        return Err(TimerContractError::WorkerHwmTooLow);
    }

    // Cleanup: stop and drop timer.
    t.stop()
        .map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t);

    // Wait for heap recovery (timer deregistered, but worker stays alive).
    // The registry should be empty; worker_baseline remains.
    // Give Idle time to process any cleanup.
    sys::delay_ticks(10);

    harness::console_line(c"OSAL_CASE_PASS name=timer_worker_lazy_identity");
    Ok(())
}

// ------------------------------------------------------------------
// Internal diagnostics helpers
// ------------------------------------------------------------------

struct InternalDiagSnapshot {
    create_attempts: u32,
    create_successes: u32,
}

fn read_internal_diag() -> InternalDiagSnapshot {
    InternalDiagSnapshot {
        create_attempts: diag_internal_task_create_attempts(),
        create_successes: diag_internal_task_create_successes(),
    }
}

fn internal_create_attempt_delta(after: &InternalDiagSnapshot, before: &InternalDiagSnapshot) -> u32 {
    after.create_attempts.wrapping_sub(before.create_attempts)
}

fn internal_create_success_delta(after: &InternalDiagSnapshot, before: &InternalDiagSnapshot) -> u32 {
    after.create_successes.wrapping_sub(before.create_successes)
}

