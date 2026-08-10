//! Timer real-kernel contracts (P7G Step 4E).
//!
//! Commit 3 — callback reentry, drop, and cross-timer contracts
//! on real FreeRTOS V11.3.0 (QEMU mps2-an385).

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::ptr::null_mut;
use core::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, Ordering};
use core::time::Duration;

use osal_api::error::Error;
use osal_api::traits::task::Task;
use osal_api::traits::timer::{Timer, TimerCallback};
use osal_api::types::TimerMode;
use osal_backend_freertos::task::FreeRtosTask;
use osal_backend_freertos::timer::FreeRtosTimer;
use osal_backend_freertos_sys as sys;

use crate::harness;

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------
#[repr(i32)]
pub enum TimerContractError {
    WorkerCreatedTooEarly = 600,
    WorkerNotCreated = 601,
    WorkerCreatedMultiple = 602,
    WorkerNativeHandleZero = 603,
    WorkerPriorityWrong = 604,
    WorkerStackWrong = 605,
    WorkerTaskCurrentNotNone = 606,
    WorkerTaskCountIncreased = 607,
    WorkerHwmTooLow = 608,
    WorkerCleanupFailed = 609,

    TimerBuilderZeroPeriodNotRejected = 610,
    TimerBuilderCreatedWorker = 611,

    TimerCallbackNotFired = 620,
    TimerCallbackTooEarly = 621,
    TimerOneShotRearmed = 622,
    TimerPeriodicCountTooLow = 623,
    TimerPeriodicTicksNonMonotonic = 624,
    TimerPeriodicFirstCallbackTooEarly = 625,

    TimerResetOnStoppedNoCallback = 630,
    TimerResetOnRunningEarlyCallback = 631,
    TimerStartOnRunningNotReset = 632,

    TimerStopCallbackFired = 640,
    TimerStopNotIdempotent = 641,

    TimerChangePeriodEarlyCallback = 650,
    TimerChangePeriodFirstDeadlineWrong = 651,
    TimerChangePeriodOnStoppedFiredEarly = 652,
    TimerChangePeriodZeroNotRejected = 653,
    TimerChangePeriodNewPeriodNotEffective = 654,

    TimerCoalescingBurstDetected = 660,

    // ---- Commit 3: reentry / drop ----
    SelfStopFailed = 700,
    SelfStopExtraCallback = 701,
    SelfResetFailed = 702,
    SelfResetMissingSecondCallback = 703,
    SelfResetSecondCallbackTooEarly = 704,

    CrossTimerStartFailed = 710,
    CrossTimerBCallbackMissing = 711,

    CloneDropNonLastCanceled = 720,
    CloneDropLastDropLeakedCallback = 721,

    InflightDropBlocked = 730,
    InflightExtraCallback = 731,
    InflightDropProbeMissing = 732,

    CallbackDropOutsideLockNotDropped = 740,
    CallbackDropOutsideLockDeadlock = 741,

    HeapNotRecovered = 690,
}

type TestResult = Result<(), TimerContractError>;

// ------------------------------------------------------------------
// Diagnostics FFI
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
fn diag_attempts() -> u32 { unsafe { osal_test_diag_internal_task_create_attempts() } }
fn diag_successes() -> u32 { unsafe { osal_test_diag_internal_task_create_successes() } }
fn diag_last_prio() -> u32 { unsafe { osal_test_diag_last_internal_priority() } }
fn diag_last_stack() -> u32 { unsafe { osal_test_diag_last_internal_stack_words() } }
fn diag_last_hdl() -> u32 { unsafe { osal_test_diag_last_internal_handle() } }
fn task_stack_hwm() -> u32 { unsafe { osal_test_task_stack_hwm() } }

// ------------------------------------------------------------------
// Constants
// ------------------------------------------------------------------
const HWM_MIN_WORDS: u32 = 64;

/// Reconstruct a TickSnapshot from a stored 32-bit raw tick value.
fn raw_tick_snap(raw: u32) -> sys::TickSnapshot {
    sys::TickSnapshot { overflow_count: 0, tick_count: raw as u64 }
}

// ------------------------------------------------------------------
// TimerCaseState — shared by simple record-and-count cases.
// ------------------------------------------------------------------
pub struct TimerCaseState {
    /// Publish-store: written LAST, after all per-callback data is
    /// visible.  Controller polls this with Acquire.
    pub callback_count: AtomicU32,
    pub callback_tick_0: AtomicU32,
    pub callback_tick_1: AtomicU32,
    pub callback_tick_2: AtomicU32,
    pub callback_hwm: AtomicU32,
    pub callback_task_current_some: AtomicU32,
}

impl TimerCaseState {
    pub fn new() -> Self {
        Self {
            callback_count: AtomicU32::new(0),
            callback_tick_0: AtomicU32::new(0),
            callback_tick_1: AtomicU32::new(0),
            callback_tick_2: AtomicU32::new(0),
            callback_hwm: AtomicU32::new(0),
            callback_task_current_some: AtomicU32::new(0),
        }
    }

    pub fn record(&self) {
        if FreeRtosTask::current().is_some() {
            self.callback_task_current_some.store(1, Ordering::Relaxed);
        }

        let raw = sys::tick_snapshot().tick_count as u32;
        let idx = self.callback_count.load(Ordering::Relaxed);

        match idx {
            0 => self.callback_tick_0.store(raw, Ordering::Relaxed),
            1 => self.callback_tick_1.store(raw, Ordering::Relaxed),
            2 => self.callback_tick_2.store(raw, Ordering::Relaxed),
            _ => {}
        }

        self.callback_hwm.store(task_stack_hwm(), Ordering::Relaxed);

        self.callback_count.store(idx + 1, Ordering::Release);
    }
}

fn make_record_callback(state: &Arc<TimerCaseState>) -> TimerCallback {
    let s = Arc::clone(state);
    Box::new(move || s.record())
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn wait_for_callback_count(state: &TimerCaseState, target: u32, deadline_ticks: u32) -> TestResult {
    let start = sys::tick_snapshot();
    loop {
        if state.callback_count.load(Ordering::Acquire) >= target {
            return Ok(());
        }
        let elapsed = harness::total_ticks_diff(sys::tick_snapshot(), start, 32);
        if elapsed >= deadline_ticks as u128 {
            return Err(TimerContractError::TimerCallbackNotFired);
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(TimerContractError::TimerCallbackNotFired);
        }
    }
}

fn assert_worker_identity(state: &TimerCaseState) -> TestResult {
    if state.callback_task_current_some.load(Ordering::Acquire) != 0 {
        return Err(TimerContractError::WorkerTaskCurrentNotNone);
    }
    if state.callback_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
        return Err(TimerContractError::WorkerHwmTooLow);
    }
    Ok(())
}

fn assert_heap_recovery(expected: u64) -> TestResult {
    let start = sys::tick_snapshot();
    loop {
        if sys::heap_free() == expected {
            return Ok(());
        }
        let elapsed = harness::total_ticks_diff(sys::tick_snapshot(), start, 32);
        if elapsed >= 50 {
            return Err(TimerContractError::HeapNotRecovered);
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            break;
        }
    }
    if sys::heap_free() == expected {
        Ok(())
    } else {
        Err(TimerContractError::HeapNotRecovered)
    }
}

// ------------------------------------------------------------------
// Internal diagnostics
// ------------------------------------------------------------------

struct InternalDiagSnapshot {
    create_attempts: u32,
    create_successes: u32,
}

fn read_internal_diag() -> InternalDiagSnapshot {
    InternalDiagSnapshot {
        create_attempts: diag_attempts(),
        create_successes: diag_successes(),
    }
}

fn internal_create_attempt_delta(after: &InternalDiagSnapshot, before: &InternalDiagSnapshot) -> u32 {
    after.create_attempts.wrapping_sub(before.create_attempts)
}

fn internal_create_success_delta(after: &InternalDiagSnapshot, before: &InternalDiagSnapshot) -> u32 {
    after.create_successes.wrapping_sub(before.create_successes)
}

fn expected_worker_stack_words(caps: &sys::Capabilities) -> u32 {
    let raw = (4096u32 + caps.stack_word_size as u32 - 1) / caps.stack_word_size as u32;
    raw.max(caps.minimal_stack_depth_words).min(caps.max_stack_depth_words)
}

// ==================================================================
// Entry point
// ==================================================================
pub fn run_timer_cases(tick_bits: u8) -> Result<(), i32> {
    diag_reset();

    if let Err(e) = case_timer_worker_lazy_identity() {
        return Err(-(e as i32));
    }
    let worker_baseline = sys::heap_free();

    let caps = sys::capabilities();

    macro_rules! run_case {
        ($state:ident, $case_fn:expr, $name:expr) => {{
            let $state = Arc::new(TimerCaseState::new());
            let inner = $case_fn(&caps, &$state);
            drop($state);
            if let Err(e) = inner {
                return Err(-(e as i32));
            }
            assert_heap_recovery(worker_baseline).map_err(|e| -(e as i32))?;
            harness::console_line($name);
        }};
    }

    // Cases that manage their own state (no TimerCaseState argument).
    macro_rules! run_custom_case {
        ($case_fn:expr, $name:expr) => {{
            let inner = $case_fn(&caps);
            if let Err(e) = inner {
                return Err(-(e as i32));
            }
            assert_heap_recovery(worker_baseline).map_err(|e| -(e as i32))?;
            harness::console_line($name);
        }};
    }

    // --- Commit 2 cases ---
    run_case!(s, case_timer_builder_core, c"OSAL_CASE_PASS name=timer_builder_core");
    run_case!(s, case_timer_one_shot, c"OSAL_CASE_PASS name=timer_one_shot");
    run_case!(s, case_timer_periodic, c"OSAL_CASE_PASS name=timer_periodic");
    run_case!(s, case_timer_start_reset, c"OSAL_CASE_PASS name=timer_start_reset");
    run_case!(s, case_timer_stop, c"OSAL_CASE_PASS name=timer_stop");
    run_case!(s, case_timer_change_period_stopped, c"OSAL_CASE_PASS name=timer_change_period_stopped");
    run_case!(s, case_timer_change_period_running, c"OSAL_CASE_PASS name=timer_change_period_running");
    run_case!(s, case_timer_periodic_coalescing, c"OSAL_CASE_PASS name=timer_periodic_coalescing");
    // --- Commit 3 cases ---
    run_custom_case!(case_timer_callback_self_control, c"OSAL_CASE_PASS name=timer_callback_self_control");
    run_custom_case!(case_timer_callback_cross_timer, c"OSAL_CASE_PASS name=timer_callback_cross_timer");
    run_case!(s, case_timer_clone_last_drop, c"OSAL_CASE_PASS name=timer_clone_last_drop");
    run_custom_case!(case_timer_inflight_last_drop, c"OSAL_CASE_PASS name=timer_inflight_last_drop");
    run_custom_case!(case_timer_callback_drop_outside_lock, c"OSAL_CASE_PASS name=timer_callback_drop_outside_lock");

    let _ = tick_bits;
    Ok(())
}

// ==================================================================
// case_timer_worker_lazy_identity
// ==================================================================
fn case_timer_worker_lazy_identity() -> TestResult {
    let caps = sys::capabilities();
    let expected_priority = caps.max_priorities.saturating_sub(1);
    let expected_stack = expected_worker_stack_words(&caps);

    let cb_cur = Arc::new(AtomicU32::new(0));
    let cb_cnt = Arc::new(AtomicU32::new(0));
    let cb_hwm = Arc::new(AtomicU32::new(0));

    let c_cur = Arc::clone(&cb_cur);
    let c_cnt = Arc::clone(&cb_cnt);
    let c_hwm = Arc::clone(&cb_hwm);

    let test_callback: TimerCallback = Box::new(move || {
        if FreeRtosTask::current().is_some() {
            c_cur.store(1, Ordering::Release);
        }
        c_cnt.store(FreeRtosTask::count() as u32, Ordering::Release);
        c_hwm.store(task_stack_hwm(), Ordering::Release);
    });

    let period = Duration::from_millis(50);

    let diag_before = read_internal_diag();
    let t = FreeRtosTimer::new("test-wid", period, TimerMode::OneShot, test_callback)
        .map_err(|_| TimerContractError::WorkerCreatedTooEarly)?;

    t.stop().map_err(|_| TimerContractError::WorkerCreatedTooEarly)?;
    t.change_period(Duration::from_millis(100))
        .map_err(|_| TimerContractError::WorkerCreatedTooEarly)?;

    let diag_still_before = read_internal_diag();
    if internal_create_attempt_delta(&diag_still_before, &diag_before) != 0 {
        return Err(TimerContractError::WorkerCreatedTooEarly);
    }

    let public_task_count_baseline = FreeRtosTask::count() as u32;
    t.start().map_err(|_| TimerContractError::WorkerNotCreated)?;
    let diag_after_start = read_internal_diag();
    if internal_create_attempt_delta(&diag_after_start, &diag_still_before) != 1
        || internal_create_success_delta(&diag_after_start, &diag_still_before) != 1
    {
        return Err(TimerContractError::WorkerNotCreated);
    }

    if diag_last_hdl() == 0 {
        return Err(TimerContractError::WorkerNativeHandleZero);
    }
    if diag_last_prio() != expected_priority {
        return Err(TimerContractError::WorkerPriorityWrong);
    }
    if diag_last_stack() != expected_stack {
        return Err(TimerContractError::WorkerStackWrong);
    }

    let diag_before_second = read_internal_diag();
    t.reset().map_err(|_| TimerContractError::WorkerCreatedMultiple)?;
    let diag_after_second = read_internal_diag();
    if internal_create_attempt_delta(&diag_after_second, &diag_before_second) != 0 {
        return Err(TimerContractError::WorkerCreatedMultiple);
    }

    let deadline_ticks: u32 = 200;
    let start_tick = sys::tick_snapshot();
    loop {
        if cb_hwm.load(Ordering::Acquire) != 0 {
            break;
        }
        let elapsed = harness::total_ticks_diff(sys::tick_snapshot(), start_tick, caps.tick_bits);
        if elapsed >= deadline_ticks as u128 {
            if cb_hwm.load(Ordering::Acquire) != 0 { break; }
            return Err(TimerContractError::WorkerNotCreated);
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(TimerContractError::WorkerNotCreated);
        }
    }

    if cb_cur.load(Ordering::Acquire) != 0 {
        return Err(TimerContractError::WorkerTaskCurrentNotNone);
    }
    if cb_cnt.load(Ordering::Acquire) != public_task_count_baseline {
        return Err(TimerContractError::WorkerTaskCountIncreased);
    }
    if cb_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
        return Err(TimerContractError::WorkerHwmTooLow);
    }

    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t);
    sys::delay_ticks(10);

    harness::console_line(c"OSAL_CASE_PASS name=timer_worker_lazy_identity");
    Ok(())
}

// ==================================================================
// case_timer_builder_core
// ==================================================================
fn case_timer_builder_core(_caps: &sys::Capabilities, _state: &Arc<TimerCaseState>) -> TestResult {
    let empty_cb: TimerCallback = Box::new(|| {});
    let task_count_before = FreeRtosTask::count() as u32;
    let heap_before = sys::heap_free();
    let diag_before = read_internal_diag();

    match FreeRtosTimer::new("test-zp", Duration::ZERO, TimerMode::OneShot, empty_cb) {
        Err(Error::InvalidParameter) => {}
        _ => return Err(TimerContractError::TimerBuilderZeroPeriodNotRejected),
    }

    let diag_after = read_internal_diag();
    if internal_create_attempt_delta(&diag_after, &diag_before) != 0 {
        return Err(TimerContractError::TimerBuilderCreatedWorker);
    }
    if FreeRtosTask::count() as u32 != task_count_before {
        return Err(TimerContractError::WorkerTaskCountIncreased);
    }
    if sys::heap_free() != heap_before {
        return Err(TimerContractError::HeapNotRecovered);
    }

    let diag_before_os = read_internal_diag();
    let empty_cb2: TimerCallback = Box::new(|| {});
    let t1 = FreeRtosTimer::new(
        "test-os", Duration::from_millis(10), TimerMode::OneShot, empty_cb2,
    ).map_err(|_| TimerContractError::TimerCallbackNotFired)?;
    if internal_create_attempt_delta(&read_internal_diag(), &diag_before_os) != 0 {
        return Err(TimerContractError::TimerBuilderCreatedWorker);
    }

    let empty_cb3: TimerCallback = Box::new(|| {});
    let t2 = FreeRtosTimer::new(
        "test-per", Duration::from_millis(10), TimerMode::Periodic, empty_cb3,
    ).map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    drop(t1);
    drop(t2);
    Ok(())
}

// ==================================================================
// case_timer_one_shot
// ==================================================================
fn case_timer_one_shot(caps: &sys::Capabilities, state: &Arc<TimerCaseState>) -> TestResult {
    let period = Duration::from_millis(5);
    let cb = make_record_callback(state);

    let t = FreeRtosTimer::new("test-os1", period, TimerMode::OneShot, cb)
        .map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    let start_tick = sys::tick_snapshot();
    t.start().map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    wait_for_callback_count(state, 1, 100)?;

    let cb_tick = state.callback_tick_0.load(Ordering::Acquire);
    let elapsed = harness::total_ticks_diff(raw_tick_snap(cb_tick), start_tick, caps.tick_bits);
    if elapsed < 5 {
        return Err(TimerContractError::TimerCallbackTooEarly);
    }

    sys::delay_ticks(period.as_millis() as u64 + 5);
    if state.callback_count.load(Ordering::Acquire) != 1 {
        return Err(TimerContractError::TimerOneShotRearmed);
    }

    assert_worker_identity(state)?;

    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t);
    Ok(())
}

// ==================================================================
// case_timer_periodic
// ==================================================================
fn case_timer_periodic(caps: &sys::Capabilities, state: &Arc<TimerCaseState>) -> TestResult {
    let period = Duration::from_millis(4);
    let cb = make_record_callback(state);

    let t = FreeRtosTimer::new("test-per1", period, TimerMode::Periodic, cb)
        .map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    let start_tick = sys::tick_snapshot();
    t.start().map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    wait_for_callback_count(state, 3, 200)?;
    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;

    if state.callback_count.load(Ordering::Acquire) < 3 {
        return Err(TimerContractError::TimerPeriodicCountTooLow);
    }

    let t0 = state.callback_tick_0.load(Ordering::Acquire);
    let t1 = state.callback_tick_1.load(Ordering::Acquire);
    let t2 = state.callback_tick_2.load(Ordering::Acquire);

    let d0 = harness::total_ticks_diff(raw_tick_snap(t0), start_tick, caps.tick_bits);
    if d0 < 4 {
        return Err(TimerContractError::TimerPeriodicFirstCallbackTooEarly);
    }

    let d01 = harness::total_ticks_diff(raw_tick_snap(t1), raw_tick_snap(t0), caps.tick_bits);
    let d12 = harness::total_ticks_diff(raw_tick_snap(t2), raw_tick_snap(t1), caps.tick_bits);
    if d01 == 0 || d12 == 0 {
        return Err(TimerContractError::TimerPeriodicTicksNonMonotonic);
    }

    assert_worker_identity(state)?;
    drop(t);
    Ok(())
}

// ==================================================================
// case_timer_start_reset
// ==================================================================
fn case_timer_start_reset(caps: &sys::Capabilities, state: &Arc<TimerCaseState>) -> TestResult {
    // ---- Sub-case A: reset on stopped timer starts it. ----
    let cb_a = make_record_callback(state);
    let t_a = FreeRtosTimer::new(
        "test-rstA", Duration::from_millis(5), TimerMode::OneShot, cb_a,
    ).map_err(|_| TimerContractError::TimerResetOnStoppedNoCallback)?;

    let start_a = sys::tick_snapshot();
    t_a.reset().map_err(|_| TimerContractError::TimerResetOnStoppedNoCallback)?;
    wait_for_callback_count(state, 1, 100)?;

    let cb_a_raw = state.callback_tick_0.load(Ordering::Acquire);
    let elapsed_a = harness::total_ticks_diff(raw_tick_snap(cb_a_raw), start_a, caps.tick_bits);
    if elapsed_a < 5 {
        return Err(TimerContractError::TimerCallbackTooEarly);
    }
    t_a.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t_a);

    // ---- Sub-case B: reset on running timer discards old deadline. ----
    let state_b = Arc::new(TimerCaseState::new());
    let cb_b = make_record_callback(&state_b);
    let t_b = FreeRtosTimer::new(
        "test-rstB", Duration::from_millis(10), TimerMode::OneShot, cb_b,
    ).map_err(|_| TimerContractError::TimerResetOnRunningEarlyCallback)?;

    t_b.start().map_err(|_| TimerContractError::TimerResetOnRunningEarlyCallback)?;
    sys::delay_ticks(2);
    let reset_tick = sys::tick_snapshot();
    t_b.reset().map_err(|_| TimerContractError::TimerResetOnRunningEarlyCallback)?;

    wait_for_callback_count(&state_b, 1, 100)?;
    let cb_b_raw = state_b.callback_tick_0.load(Ordering::Acquire);
    let elapsed_from_reset = harness::total_ticks_diff(raw_tick_snap(cb_b_raw), reset_tick, caps.tick_bits);
    if elapsed_from_reset < 10 {
        return Err(TimerContractError::TimerResetOnRunningEarlyCallback);
    }

    assert_worker_identity(&state_b)?;
    t_b.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t_b);

    // ---- Sub-case C: start on already-running timer acts as reset. ----
    let state_c = Arc::new(TimerCaseState::new());
    let cb_c = make_record_callback(&state_c);
    let t_c = FreeRtosTimer::new(
        "test-rstC", Duration::from_millis(10), TimerMode::OneShot, cb_c,
    ).map_err(|_| TimerContractError::TimerStartOnRunningNotReset)?;

    t_c.start().map_err(|_| TimerContractError::TimerStartOnRunningNotReset)?;
    sys::delay_ticks(2);
    let second_start_tick = sys::tick_snapshot();
    t_c.start().map_err(|_| TimerContractError::TimerStartOnRunningNotReset)?;

    wait_for_callback_count(&state_c, 1, 100)?;
    let cb_c_raw = state_c.callback_tick_0.load(Ordering::Acquire);
    let elapsed_c = harness::total_ticks_diff(raw_tick_snap(cb_c_raw), second_start_tick, caps.tick_bits);
    if elapsed_c < 10 {
        return Err(TimerContractError::TimerStartOnRunningNotReset);
    }

    assert_worker_identity(&state_c)?;
    t_c.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t_c);

    Ok(())
}

// ==================================================================
// case_timer_stop
// ==================================================================
fn case_timer_stop(_caps: &sys::Capabilities, state: &Arc<TimerCaseState>) -> TestResult {
    let cb = make_record_callback(state);
    let t = FreeRtosTimer::new(
        "test-stp", Duration::from_millis(10), TimerMode::OneShot, cb,
    ).map_err(|_| TimerContractError::TimerStopCallbackFired)?;

    t.start().map_err(|_| TimerContractError::TimerStopCallbackFired)?;
    sys::delay_ticks(3);
    t.stop().map_err(|_| TimerContractError::TimerStopNotIdempotent)?;
    sys::delay_ticks(20);

    if state.callback_count.load(Ordering::Acquire) != 0 {
        return Err(TimerContractError::TimerStopCallbackFired);
    }

    t.stop().map_err(|_| TimerContractError::TimerStopNotIdempotent)?;
    drop(t);
    Ok(())
}

// ==================================================================
// case_timer_change_period_stopped
// ==================================================================
fn case_timer_change_period_stopped(
    caps: &sys::Capabilities, state: &Arc<TimerCaseState>,
) -> TestResult {
    let cb = make_record_callback(state);
    let t = FreeRtosTimer::new(
        "test-cps", Duration::from_millis(50), TimerMode::OneShot, cb,
    ).map_err(|_| TimerContractError::TimerChangePeriodEarlyCallback)?;

    // --- change_period(ZERO) must be rejected ---
    match t.change_period(Duration::ZERO) {
        Err(Error::InvalidParameter) => {}
        _ => return Err(TimerContractError::TimerChangePeriodZeroNotRejected),
    }

    // --- change_period on stopped timer updates period but does NOT start it ---
    let diag_before = read_internal_diag();
    t.change_period(Duration::from_millis(5))
        .map_err(|_| TimerContractError::TimerChangePeriodEarlyCallback)?;
    if internal_create_attempt_delta(&read_internal_diag(), &diag_before) != 0 {
        return Err(TimerContractError::TimerBuilderCreatedWorker);
    }

    // Prove the timer is still stopped: wait well past the new period.
    sys::delay_ticks(7);
    if state.callback_count.load(Ordering::Acquire) != 0 {
        return Err(TimerContractError::TimerChangePeriodOnStoppedFiredEarly);
    }

    // Now start — the new period must take effect.
    let start_tick = sys::tick_snapshot();
    t.start().map_err(|_| TimerContractError::TimerChangePeriodEarlyCallback)?;
    wait_for_callback_count(state, 1, 50)?;

    let cb_raw = state.callback_tick_0.load(Ordering::Acquire);
    let elapsed = harness::total_ticks_diff(raw_tick_snap(cb_raw), start_tick, caps.tick_bits);
    if elapsed < 5 {
        return Err(TimerContractError::TimerChangePeriodEarlyCallback);
    }

    assert_worker_identity(state)?;
    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t);
    Ok(())
}

// ==================================================================
// case_timer_change_period_running
// ==================================================================
fn case_timer_change_period_running(
    caps: &sys::Capabilities, state: &Arc<TimerCaseState>,
) -> TestResult {
    let old_period = Duration::from_millis(40);
    let new_period = Duration::from_millis(5);

    let cb = make_record_callback(state);
    let t = FreeRtosTimer::new(
        "test-cpr", old_period, TimerMode::Periodic, cb,
    ).map_err(|_| TimerContractError::TimerChangePeriodFirstDeadlineWrong)?;

    let start_tick = sys::tick_snapshot();
    t.start().map_err(|_| TimerContractError::TimerChangePeriodFirstDeadlineWrong)?;
    sys::delay_ticks(2);
    t.change_period(new_period)
        .map_err(|_| TimerContractError::TimerChangePeriodFirstDeadlineWrong)?;

    wait_for_callback_count(state, 1, 80)?;
    let t0 = state.callback_tick_0.load(Ordering::Acquire);
    let elapsed_0 = harness::total_ticks_diff(raw_tick_snap(t0), start_tick, caps.tick_bits);
    if elapsed_0 < 40 {
        return Err(TimerContractError::TimerChangePeriodFirstDeadlineWrong);
    }

    let second_deadline = 20u32;
    let wait_start = sys::tick_snapshot();
    loop {
        if state.callback_count.load(Ordering::Acquire) >= 2 {
            break;
        }
        let waited = harness::total_ticks_diff(sys::tick_snapshot(), wait_start, caps.tick_bits);
        if waited >= second_deadline as u128 {
            return Err(TimerContractError::TimerChangePeriodNewPeriodNotEffective);
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(TimerContractError::TimerChangePeriodNewPeriodNotEffective);
        }
    }

    let t1 = state.callback_tick_1.load(Ordering::Acquire);
    let elapsed_between = harness::total_ticks_diff(raw_tick_snap(t1), raw_tick_snap(t0), caps.tick_bits);
    if elapsed_between < 5 {
        return Err(TimerContractError::TimerChangePeriodEarlyCallback);
    }

    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    assert_worker_identity(state)?;
    drop(t);
    Ok(())
}

// ==================================================================
// case_timer_periodic_coalescing
// ==================================================================
fn case_timer_periodic_coalescing(
    _caps: &sys::Capabilities, state: &Arc<TimerCaseState>,
) -> TestResult {
    let period_ticks: u32 = 5;
    let block_ticks = period_ticks * 3 + 1; // 16

    let entered = Arc::new(AtomicU32::new(0));
    let s = Arc::clone(state);
    let e = Arc::clone(&entered);
    let cb: TimerCallback = Box::new(move || {
        let index = e.fetch_add(1, Ordering::AcqRel);
        s.record();
        if index == 0 {
            sys::delay_ticks(block_ticks as u64);
        }
    });

    let t = FreeRtosTimer::new(
        "test-coal", Duration::from_millis(period_ticks as u64),
        TimerMode::Periodic, cb,
    ).map_err(|_| TimerContractError::TimerCoalescingBurstDetected)?;
    t.start().map_err(|_| TimerContractError::TimerCoalescingBurstDetected)?;

    wait_for_callback_count(state, 2, 80)?;
    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;

    let count = state.callback_count.load(Ordering::Acquire);
    if count > 2 {
        return Err(TimerContractError::TimerCoalescingBurstDetected);
    }
    if count < 2 {
        return Err(TimerContractError::TimerCallbackNotFired);
    }

    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    assert_worker_identity(state)?;
    drop(t);
    Ok(())
}

// ==================================================================
// Commit-3 state structs
// ==================================================================

/// State for self-control callbacks.
/// controller polls control_result (not just callback_count) after
/// waiting for the callback to ensure the re-entrant API call completed.
struct SelfControlState {
    callback_count: AtomicU32,
    callback_tick_0: AtomicU32,
    callback_tick_1: AtomicU32,
    callback_hwm: AtomicU32,
    callback_task_current_some: AtomicU32,
    /// 0 = not called yet, 1 = Ok, 2 = Err.
    /// Published with Release AFTER the re-entrant API call completes.
    control_result: AtomicU32,
    /// Raw tick captured during self-reset (sub-case B only).
    reset_tick: AtomicU32,
}

impl SelfControlState {
    fn new() -> Self {
        Self {
            callback_count: AtomicU32::new(0),
            callback_tick_0: AtomicU32::new(0),
            callback_tick_1: AtomicU32::new(0),
            callback_hwm: AtomicU32::new(0),
            callback_task_current_some: AtomicU32::new(0),
            control_result: AtomicU32::new(0),
            reset_tick: AtomicU32::new(0),
        }
    }

    fn record(&self) {
        if FreeRtosTask::current().is_some() {
            self.callback_task_current_some.store(1, Ordering::Relaxed);
        }
        let raw = sys::tick_snapshot().tick_count as u32;
        let idx = self.callback_count.load(Ordering::Relaxed);
        match idx {
            0 => self.callback_tick_0.store(raw, Ordering::Relaxed),
            1 => self.callback_tick_1.store(raw, Ordering::Relaxed),
            _ => {}
        }
        self.callback_hwm.store(task_stack_hwm(), Ordering::Relaxed);
        self.callback_count.store(idx + 1, Ordering::Release);
    }
}

/// State for the cross-timer callback case.
///
/// PUBLISH ORDERING: every callback stores per-call data with Relaxed,
/// then publishes its count with Release **last**.  The controller polls
/// the count with Acquire, which guarantees all Relaxed stores above it
/// are visible.
struct CrossTimerState {
    a_count: AtomicU32,
    a_tick: AtomicU32,
    /// Timer B start result: 0=not called, 1=Ok, 2=Err.
    b_start_result: AtomicU32,
    b_count: AtomicU32,
    b_tick: AtomicU32,
    hwm: AtomicU32,
}

impl CrossTimerState {
    fn new() -> Self {
        Self {
            a_count: AtomicU32::new(0),
            a_tick: AtomicU32::new(0),
            b_start_result: AtomicU32::new(0),
            b_count: AtomicU32::new(0),
            b_tick: AtomicU32::new(0),
            hwm: AtomicU32::new(0),
        }
    }
}

/// State for the inflight last-drop case.
struct InflightState {
    started: AtomicBool,
    release: AtomicBool,
    completed: AtomicBool,
    callback_count: AtomicU32,
    hwm: AtomicU32,
}

impl InflightState {
    fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            release: AtomicBool::new(false),
            completed: AtomicBool::new(false),
            callback_count: AtomicU32::new(0),
            hwm: AtomicU32::new(0),
        }
    }
}

/// Drop-probe: records exactly-one destruction of the callback closure.
struct CallbackDropProbe {
    drops: Arc<AtomicU32>,
}

impl Drop for CallbackDropProbe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
    }
}

// ==================================================================
// case_timer_callback_self_control
// ==================================================================
fn case_timer_callback_self_control(caps: &sys::Capabilities) -> TestResult {
    // ---- Sub-case A: callback self-stop on Periodic ----
    let state_a = Arc::new(SelfControlState::new());
    let slot_a = Arc::new(AtomicPtr::<FreeRtosTimer>::new(null_mut()));

    {
        let s = Arc::clone(&state_a);
        let sl = Arc::clone(&slot_a);
        let cb_a: TimerCallback = Box::new(move || {
            s.record();
            // SAFETY: controller publishes pointer before start() and
            // nulls it before dropping the Box — the pointee outlives
            // every callback invocation.
            let ptr = sl.load(Ordering::Acquire);
            if !ptr.is_null() {
                let t = unsafe { &*ptr };
                match t.stop() {
                    Ok(()) => s.control_result.store(1, Ordering::Release),
                    Err(_) => s.control_result.store(2, Ordering::Release),
                }
            }
        });

        // RAII Box — error-path cleanup is automatic.
        let timer = Box::new(
            FreeRtosTimer::new(
                "test-slfA", Duration::from_millis(4), TimerMode::Periodic, cb_a,
            ).map_err(|_| TimerContractError::SelfStopFailed)?
        );

        // Publish non-owning pointer.  The Box heap address is stable.
        slot_a.store(
            (&*timer as *const FreeRtosTimer).cast_mut(),
            Ordering::Release,
        );

        timer.start().map_err(|_| TimerContractError::SelfStopFailed)?;

        // Wait for the callback to fire.
        wait_self_control_count(&state_a, 1, 80)?;

        // Now wait for the re-entrant self-stop to complete.
        // callback_count==1 only means the callback *entered*, not
        // that self-stop finished.  control_result is the real signal.
        bounded_wait_u32(&state_a.control_result, |v| v != 0, 50)
            .map_err(|_| TimerContractError::SelfStopFailed)?;
        if state_a.control_result.load(Ordering::Acquire) != 1 {
            return Err(TimerContractError::SelfStopFailed);
        }

        // Wait > 3 periods; self-stop must prevent further callbacks.
        sys::delay_ticks(20);
        if state_a.callback_count.load(Ordering::Acquire) != 1 {
            return Err(TimerContractError::SelfStopExtraCallback);
        }

        // Null the pointer before dropping the Box so a stray
        // callback (impossible after verified stop, but defense in
        // depth) cannot dereference freed memory.
        slot_a.store(null_mut(), Ordering::Release);
        timer.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
        drop(timer);
    }

    // ---- Sub-case B: callback self-reset on OneShot ----
    let state_b = Arc::new(SelfControlState::new());
    let slot_b = Arc::new(AtomicPtr::<FreeRtosTimer>::new(null_mut()));

    {
        let s = Arc::clone(&state_b);
        let sl = Arc::clone(&slot_b);
        let cb_b: TimerCallback = Box::new(move || {
            let index = s.callback_count.load(Ordering::Relaxed);
            s.record();

            // Only the first callback performs self-reset.
            if index == 0 {
                let reset_raw = sys::tick_snapshot().tick_count as u32;
                s.reset_tick.store(reset_raw, Ordering::Relaxed);

                let ptr = sl.load(Ordering::Acquire);
                if !ptr.is_null() {
                    let t = unsafe { &*ptr };
                    match t.reset() {
                        Ok(()) => s.control_result.store(1, Ordering::Release),
                        Err(_) => s.control_result.store(2, Ordering::Release),
                    }
                }
            }
        });

        let timer = Box::new(
            FreeRtosTimer::new(
                "test-slfB", Duration::from_millis(4), TimerMode::OneShot, cb_b,
            ).map_err(|_| TimerContractError::SelfResetFailed)?
        );

        slot_b.store(
            (&*timer as *const FreeRtosTimer).cast_mut(),
            Ordering::Release,
        );

        timer.start().map_err(|_| TimerContractError::SelfResetFailed)?;

        // Wait for second callback — self-reset must have re-armed.
        wait_self_control_count(&state_b, 2, 80)
            .map_err(|_| TimerContractError::SelfResetMissingSecondCallback)?;

        // Verify self-reset returned Ok.
        if state_b.control_result.load(Ordering::Acquire) != 1 {
            return Err(TimerContractError::SelfResetFailed);
        }

        // Second callback must arrive >= one period after the real
        // reset tick captured inside the first callback.
        let reset_tick = state_b.reset_tick.load(Ordering::Acquire);
        let t1 = state_b.callback_tick_1.load(Ordering::Acquire);
        let d_reset_to_cb2 = harness::total_ticks_diff(
            raw_tick_snap(t1), raw_tick_snap(reset_tick), caps.tick_bits,
        );
        if d_reset_to_cb2 < 4 {
            return Err(TimerContractError::SelfResetSecondCallbackTooEarly);
        }

        assert_self_control_worker(&state_b)?;

        slot_b.store(null_mut(), Ordering::Release);
        timer.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
        drop(timer);
    }

    Ok(())
}

fn wait_self_control_count(state: &SelfControlState, target: u32, deadline_ticks: u32) -> TestResult {
    let start = sys::tick_snapshot();
    loop {
        if state.callback_count.load(Ordering::Acquire) >= target {
            return Ok(());
        }
        let elapsed = harness::total_ticks_diff(sys::tick_snapshot(), start, 32);
        if elapsed >= deadline_ticks as u128 {
            return Err(TimerContractError::TimerCallbackNotFired);
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(TimerContractError::TimerCallbackNotFired);
        }
    }
}

fn assert_self_control_worker(state: &SelfControlState) -> TestResult {
    if state.callback_task_current_some.load(Ordering::Acquire) != 0 {
        return Err(TimerContractError::WorkerTaskCurrentNotNone);
    }
    if state.callback_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
        return Err(TimerContractError::WorkerHwmTooLow);
    }
    Ok(())
}

/// Bounded poll for an AtomicU32 to satisfy `pred`.
fn bounded_wait_u32<F: Fn(u32) -> bool>(
    atom: &AtomicU32,
    pred: F,
    deadline_ticks: u32,
) -> Result<(), ()> {
    let start = sys::tick_snapshot();
    loop {
        let v = atom.load(Ordering::Acquire);
        if pred(v) {
            return Ok(());
        }
        let elapsed = harness::total_ticks_diff(sys::tick_snapshot(), start, 32);
        if elapsed >= deadline_ticks as u128 {
            return Err(());
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(());
        }
    }
}

/// Bounded poll for an AtomicBool to reach expected.
fn bounded_wait_bool(
    atom: &AtomicBool,
    expected: bool,
    deadline_ticks: u32,
    tick_bits: u8,
) -> Result<(), ()> {
    let start = sys::tick_snapshot();
    loop {
        if atom.load(Ordering::Acquire) == expected {
            return Ok(());
        }
        let elapsed = harness::total_ticks_diff(sys::tick_snapshot(), start, tick_bits);
        if elapsed >= deadline_ticks as u128 {
            return Err(());
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return Err(());
        }
    }
}

// ==================================================================
// case_timer_callback_cross_timer
// ==================================================================
fn case_timer_callback_cross_timer(_caps: &sys::Capabilities) -> TestResult {
    let state = Arc::new(CrossTimerState::new());

    // --- Timer B: OneShot, initially stopped. ---
    let b_slot = Arc::new(AtomicPtr::<FreeRtosTimer>::new(null_mut()));

    let s_b = Arc::clone(&state);
    let cb_b: TimerCallback = Box::new(move || {
        let raw = sys::tick_snapshot().tick_count as u32;
        // All per-call data stored Relaxed; count published Release LAST.
        s_b.b_tick.store(raw, Ordering::Relaxed);
        s_b.hwm.store(task_stack_hwm(), Ordering::Relaxed);
        s_b.b_count.store(1, Ordering::Release);
    });

    let t_b = Box::new(
        FreeRtosTimer::new(
            "test-xb", Duration::from_millis(4), TimerMode::OneShot, cb_b,
        ).map_err(|_| TimerContractError::CrossTimerStartFailed)?
    );

    b_slot.store(
        (&*t_b as *const FreeRtosTimer).cast_mut(),
        Ordering::Release,
    );

    // --- Timer A: OneShot, callback starts B. ---
    let s_a = Arc::clone(&state);
    let sl_b = Arc::clone(&b_slot);
    let cb_a: TimerCallback = Box::new(move || {
        let raw = sys::tick_snapshot().tick_count as u32;
        // All per-call data stored Relaxed.
        s_a.a_tick.store(raw, Ordering::Relaxed);
        s_a.hwm.store(task_stack_hwm(), Ordering::Relaxed);

        // Cross-timer: start B from within A's callback.
        let b_start_result = {
            let ptr = sl_b.load(Ordering::Acquire);
            if !ptr.is_null() {
                let b = unsafe { &*ptr };
                match b.start() {
                    Ok(()) => 1,
                    Err(_) => 2,
                }
            } else {
                2
            }
        };
        s_a.b_start_result.store(b_start_result, Ordering::Relaxed);
        // Publish LAST — all Relaxed stores above are now visible.
        s_a.a_count.store(1, Ordering::Release);
    });

    let t_a = FreeRtosTimer::new(
        "test-xa", Duration::from_millis(3), TimerMode::OneShot, cb_a,
    ).map_err(|_| TimerContractError::CrossTimerStartFailed)?;

    t_a.start().map_err(|_| TimerContractError::CrossTimerStartFailed)?;

    // Wait for A's callback.  Acquire on a_count guarantees visibility
    // of a_tick, hwm, and b_start_result.
    bounded_wait_u32(&state.a_count, |v| v >= 1, 50)
        .map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    // B.start() must have succeeded.
    if state.b_start_result.load(Ordering::Acquire) != 1 {
        return Err(TimerContractError::CrossTimerStartFailed);
    }

    // Wait for B's callback.  Acquire on b_count guarantees hwm.
    bounded_wait_u32(&state.b_count, |v| v >= 1, 80)
        .map_err(|_| TimerContractError::CrossTimerBCallbackMissing)?;

    if state.hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
        return Err(TimerContractError::WorkerHwmTooLow);
    }

    drop(t_a);
    b_slot.store(null_mut(), Ordering::Release);
    drop(t_b);

    Ok(())
}

// ==================================================================
// case_timer_clone_last_drop
// ==================================================================
fn case_timer_clone_last_drop(
    _caps: &sys::Capabilities, state: &Arc<TimerCaseState>,
) -> TestResult {
    let cb = make_record_callback(state);
    let t1 = FreeRtosTimer::new(
        "test-cld", Duration::from_millis(4), TimerMode::OneShot, cb,
    ).map_err(|_| TimerContractError::CloneDropNonLastCanceled)?;
    let t2 = t1.clone();

    t1.start().map_err(|_| TimerContractError::CloneDropNonLastCanceled)?;

    // Non-last clone drop: timer must keep running.
    drop(t1);
    wait_for_callback_count(state, 1, 50)?;

    // Re-arm as OneShot so there is exactly one pending future
    // callback to cancel.  This avoids conflating in-flight
    // completion (verified by timer_inflight_last_drop) with
    // last-drop cancellation.
    t2.reset().map_err(|_| TimerContractError::CloneDropLastDropLeakedCallback)?;

    // Last handle drop: must cancel the pending callback.
    let count_before = state.callback_count.load(Ordering::Acquire);
    drop(t2);

    sys::delay_ticks(20);
    let count_after = state.callback_count.load(Ordering::Acquire);
    if count_after != count_before {
        return Err(TimerContractError::CloneDropLastDropLeakedCallback);
    }

    assert_worker_identity(state)?;
    Ok(())
}

// ==================================================================
// case_timer_inflight_last_drop
// ==================================================================
fn case_timer_inflight_last_drop(caps: &sys::Capabilities) -> TestResult {
    let state = Arc::new(InflightState::new());
    let drops = Arc::new(AtomicU32::new(0));

    let probe = CallbackDropProbe {
        drops: Arc::clone(&drops),
    };

    // Build callback: signals started, blocks until release, records
    // and signals completed.  Uses delay_ticks(1) polling so the
    // controller (lower priority) isn't starved.
    // CallbackDropProbe is captured to verify the closure is dropped
    // exactly once after in-flight completion.
    let s = Arc::clone(&state);
    let cb: TimerCallback = Box::new(move || {
        let _probe = &probe;

        s.started.store(true, Ordering::Release);

        while !s.release.load(Ordering::Acquire) {
            sys::delay_ticks(1);
        }

        s.callback_count.fetch_add(1, Ordering::Relaxed);
        s.hwm.store(task_stack_hwm(), Ordering::Relaxed);
        s.completed.store(true, Ordering::Release);
    });

    let t = FreeRtosTimer::new(
        "test-ifd", Duration::from_millis(3), TimerMode::Periodic, cb,
    ).map_err(|_| TimerContractError::InflightDropBlocked)?;

    t.start().map_err(|_| TimerContractError::InflightDropBlocked)?;

    // Wait for callback to start.
    bounded_wait_bool(&state.started, true, 100, caps.tick_bits)
        .map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    // Drop the last handle while callback is in-flight.
    drop(t);

    // Drop must have returned (we're here!). Callback is still running.
    if state.completed.load(Ordering::Acquire) {
        return Err(TimerContractError::InflightDropBlocked);
    }

    // Release the callback.
    state.release.store(true, Ordering::Release);

    // Wait for callback body to complete.
    bounded_wait_bool(&state.completed, true, 100, caps.tick_bits)
        .map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    // completed is visible, but the callback closure may not be dropped
    // yet — dispatch_one() must re-lock the registry, detect the
    // deleted entry, and drop the closure.  Bounded-wait for exactly
    // one drop.
    {
        let start = sys::tick_snapshot();
        loop {
            let c = drops.load(Ordering::Acquire);
            if c == 1 {
                break;
            }
            if c > 1 {
                return Err(TimerContractError::InflightDropProbeMissing);
            }
            if harness::total_ticks_diff(sys::tick_snapshot(), start, caps.tick_bits) >= 50 {
                return Err(TimerContractError::InflightDropProbeMissing);
            }
            if sys::delay_ticks(1) != sys::DelayStatus::Ok {
                return Err(TimerContractError::InflightDropProbeMissing);
            }
        }
    }

    // After the in-flight callback finished and its closure was
    // dropped, no more callbacks should fire.
    let count = state.callback_count.load(Ordering::Acquire);
    sys::delay_ticks(20);
    if state.callback_count.load(Ordering::Acquire) != count {
        return Err(TimerContractError::InflightExtraCallback);
    }
    if count < 1 {
        return Err(TimerContractError::TimerCallbackNotFired);
    }

    if state.hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
        return Err(TimerContractError::WorkerHwmTooLow);
    }

    Ok(())
}

// ==================================================================
// case_timer_callback_drop_outside_lock
// ==================================================================

/// Probe that re-enters the Timer API during its own Drop.
///
/// If the registry lock is held when the callback closure is dropped
/// (during `deregister`), calling `t.stop()` on another timer would
/// deadlock.  A successful stop proves the destructor runs outside
/// any internal lock.
struct ReentrantDropProbe {
    other_timer: Option<FreeRtosTimer>,
    drops: Arc<AtomicU32>,
    reentry_ok: Arc<AtomicU32>,
}

impl Drop for ReentrantDropProbe {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::Release);
        if let Some(ref t) = self.other_timer {
            match t.stop() {
                Ok(()) => self.reentry_ok.store(1, Ordering::Release),
                Err(_) => self.reentry_ok.store(2, Ordering::Release),
            }
        }
    }
}

fn case_timer_callback_drop_outside_lock(_caps: &sys::Capabilities) -> TestResult {
    let drops = Arc::new(AtomicU32::new(0));
    let reentry_ok = Arc::new(AtomicU32::new(0));

    // Timer B (stopped, never started) — target of re-entrant stop.
    let cb_b: TimerCallback = Box::new(|| {});
    let t_b = FreeRtosTimer::new(
        "test-dolB", Duration::from_millis(5), TimerMode::OneShot, cb_b,
    ).map_err(|_| TimerContractError::CallbackDropOutsideLockNotDropped)?;

    // ReentrantDropProbe captures another timer handle.  When this
    // probe is dropped (inside the callback closure's destructor),
    // it calls t.stop() to prove it's not inside the registry lock.
    let probe = ReentrantDropProbe {
        other_timer: Some(t_b.clone()),
        drops: Arc::clone(&drops),
        reentry_ok: Arc::clone(&reentry_ok),
    };

    // Timer A: never started.  The callback captures `probe`.
    // Dropping the last handle triggers deregister which takes the
    // callback and drops it (and thus drops probe) outside the lock.
    let cb_a: TimerCallback = Box::new(move || {
        let _ = &probe; // keep probe alive in closure
    });
    let t_a = FreeRtosTimer::new(
        "test-dolA", Duration::from_millis(10), TimerMode::OneShot, cb_a,
    ).map_err(|_| TimerContractError::CallbackDropOutsideLockNotDropped)?;

    // Drop the last handle to A — triggers deregister → drop callback
    // → drop probe → ReentrantDropProbe::drop() → t_b.stop()
    drop(t_a);

    // Probe must have been dropped exactly once.
    if drops.load(Ordering::Acquire) != 1 {
        return Err(TimerContractError::CallbackDropOutsideLockNotDropped);
    }

    // Re-entrant stop must have succeeded (no deadlock).
    if reentry_ok.load(Ordering::Acquire) != 1 {
        return Err(TimerContractError::CallbackDropOutsideLockDeadlock);
    }

    // Timer B must still be usable after the probe drop.
    t_b.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;
    drop(t_b);

    Ok(())
}
