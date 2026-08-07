//! Timer real-kernel contracts (P7G Step 4E).
//!
//! Commit 2 — Timer core, one-shot, periodic, deadline, and
//! coalescing contracts on real FreeRTOS V11.3.0 (QEMU mps2-an385).

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicU32, Ordering};
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

    TimerResetOnStoppedNoCallback = 630,
    TimerResetOnRunningEarlyCallback = 631,

    TimerStopCallbackFired = 640,
    TimerStopNotIdempotent = 641,

    TimerChangePeriodEarlyCallback = 650,
    TimerChangePeriodFirstDeadlineWrong = 651,

    TimerCoalescingBurstDetected = 660,

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
// TimerCaseState
// ------------------------------------------------------------------
pub struct TimerCaseState {
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
            self.callback_task_current_some.store(1, Ordering::Release);
        }
        let raw = sys::tick_snapshot().tick_count as u32;
        let idx = self.callback_count.fetch_add(1, Ordering::AcqRel);
        match idx {
            0 => self.callback_tick_0.store(raw, Ordering::Release),
            1 => self.callback_tick_1.store(raw, Ordering::Release),
            2 => self.callback_tick_2.store(raw, Ordering::Release),
            _ => {}
        }
        self.callback_hwm.store(task_stack_hwm(), Ordering::Release);
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

    run_case!(s, case_timer_builder_core, c"OSAL_CASE_PASS name=timer_builder_core");
    run_case!(s, case_timer_one_shot, c"OSAL_CASE_PASS name=timer_one_shot");
    run_case!(s, case_timer_periodic, c"OSAL_CASE_PASS name=timer_periodic");
    run_case!(s, case_timer_start_reset, c"OSAL_CASE_PASS name=timer_start_reset");
    run_case!(s, case_timer_stop, c"OSAL_CASE_PASS name=timer_stop");
    run_case!(s, case_timer_change_period_stopped, c"OSAL_CASE_PASS name=timer_change_period_stopped");
    run_case!(s, case_timer_change_period_running, c"OSAL_CASE_PASS name=timer_change_period_running");
    run_case!(s, case_timer_periodic_coalescing, c"OSAL_CASE_PASS name=timer_periodic_coalescing");

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
    let start_tick = sys::tick_snapshot();

    let t = FreeRtosTimer::new("test-os1", period, TimerMode::OneShot, cb)
        .map_err(|_| TimerContractError::TimerCallbackNotFired)?;
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
    t.start().map_err(|_| TimerContractError::TimerCallbackNotFired)?;

    wait_for_callback_count(state, 3, 200)?;
    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;

    if state.callback_count.load(Ordering::Acquire) < 3 {
        return Err(TimerContractError::TimerPeriodicCountTooLow);
    }

    let t0 = state.callback_tick_0.load(Ordering::Acquire);
    let t1 = state.callback_tick_1.load(Ordering::Acquire);
    let t2 = state.callback_tick_2.load(Ordering::Acquire);
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
    // Sub-case A: reset on stopped timer starts it.
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

    // Sub-case B: reset on running timer discards old deadline.
    let state_b = Arc::new(TimerCaseState::new());
    let cb_b = make_record_callback(&state_b);
    let t_b = FreeRtosTimer::new(
        "test-rstB", Duration::from_millis(10), TimerMode::OneShot, cb_b,
    ).map_err(|_| TimerContractError::TimerResetOnRunningEarlyCallback)?;

    let _start_b = sys::tick_snapshot();
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

    let diag_before = read_internal_diag();
    t.change_period(Duration::from_millis(5))
        .map_err(|_| TimerContractError::TimerChangePeriodEarlyCallback)?;
    if internal_create_attempt_delta(&read_internal_diag(), &diag_before) != 0 {
        return Err(TimerContractError::TimerBuilderCreatedWorker);
    }

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
    let cb = make_record_callback(state);
    let t = FreeRtosTimer::new(
        "test-cpr", Duration::from_millis(10), TimerMode::Periodic, cb,
    ).map_err(|_| TimerContractError::TimerChangePeriodFirstDeadlineWrong)?;

    let start_tick = sys::tick_snapshot();
    t.start().map_err(|_| TimerContractError::TimerChangePeriodFirstDeadlineWrong)?;
    sys::delay_ticks(2);
    t.change_period(Duration::from_millis(4))
        .map_err(|_| TimerContractError::TimerChangePeriodFirstDeadlineWrong)?;

    wait_for_callback_count(state, 2, 80)?;
    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;

    let t0 = state.callback_tick_0.load(Ordering::Acquire);
    let t1 = state.callback_tick_1.load(Ordering::Acquire);

    let elapsed_0 = harness::total_ticks_diff(raw_tick_snap(t0), start_tick, caps.tick_bits);
    if elapsed_0 < 10 {
        return Err(TimerContractError::TimerChangePeriodFirstDeadlineWrong);
    }
    let elapsed_between = harness::total_ticks_diff(raw_tick_snap(t1), raw_tick_snap(t0), caps.tick_bits);
    if elapsed_between < 4 {
        return Err(TimerContractError::TimerChangePeriodEarlyCallback);
    }

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
    let block_ticks = period_ticks * 3 + 1;

    let s = Arc::clone(state);
    let cb: TimerCallback = Box::new(move || {
        s.record();
        sys::delay_ticks(block_ticks as u64);
    });

    let t = FreeRtosTimer::new(
        "test-coal", Duration::from_millis(period_ticks as u64),
        TimerMode::Periodic, cb,
    ).map_err(|_| TimerContractError::TimerCoalescingBurstDetected)?;
    t.start().map_err(|_| TimerContractError::TimerCoalescingBurstDetected)?;

    sys::delay_ticks(block_ticks as u64 + period_ticks as u64 * 2 + 10);
    t.stop().map_err(|_| TimerContractError::WorkerCleanupFailed)?;

    let count = state.callback_count.load(Ordering::Acquire);
    if count >= 4 {
        return Err(TimerContractError::TimerCoalescingBurstDetected);
    }
    if count < 1 {
        return Err(TimerContractError::TimerCallbackNotFired);
    }

    assert_worker_identity(state)?;
    drop(t);
    Ok(())
}
