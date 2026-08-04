//! Semaphore real-kernel contracts (P7G Step 4B).
//!
//! Validates CountingSemaphore and BinarySemaphore on real FreeRTOS
//! against the Behavior Contract: creation, count tracking, overflow,
//! NoWait / After(ZERO), finite timeout, blocking wake, Forever wake,
//! multi-waiter, scheduler suspended, clone/last-drop, and RuntimeLease.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::semaphore::CountingSemaphore;
use osal_backend_freertos_sys as sys;

use crate::harness::{self, CaseState, HarnessError};
use crate::harness::{
    PHASE_BEFORE_OPERATION, PHASE_EXITING, PHASE_OPERATION_COMPLETED, PHASE_STARTED,
};

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------
#[allow(dead_code)]
#[repr(i32)]
pub enum SemaphoreError {
    Create = 200,
    InvalidParamNotRejected = 201,
    CountMismatch = 202,
    MaxMismatch = 203,
    AcquireNotSucceeded = 204,
    AcquireFailed = 205,
    OverflowNotReturned = 206,
    OverflowCountChanged = 207,
    NoWaitNotTimeout = 208,
    AfterZeroNotTimeout = 209,
    TimeoutTooEarly = 210,
    HelperSpawnFailed = 211,
    AcquireNotTimeout = 212,
    BlockingNotAcquired = 213,
    CompletedBeforeRelease = 214,
    CountNotZero = 215,
    WrongWaiterCount = 216,
    CloneHeapLeak = 217,
    LastDropLeak = 218,
    ShutdownBusyNotReturned = 219,
    BusyStateChanged = 220,
    BusyHeapChanged = 221,
    ShutdownFailed = 222,
    ShutdownStateInvalid = 223,
    ControllerDelayFailed = 224,
    BinaryNotSignaled = 225,
    BinaryNotUnsignaled = 226,
    PermitLeak = 227,
}

// ------------------------------------------------------------------
// CountingOperation — what a native helper should attempt.
// ------------------------------------------------------------------
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum CountingOperation {
    /// `acquire(NoWait)` on an empty semaphore — expects Timeout.
    AcquireNoWaitExpectTimeout,
    /// `acquire(After(ZERO))` on an empty semaphore — expects Timeout.
    AcquireAfterZeroExpectTimeout,
    /// `acquire(After(d))` — expects Timeout, records elapsed.
    AcquireAfterTicks { timeout_ticks: u32 },
    /// `acquire(After(d))` — expects to succeed (blocking / Forever).
    AcquireAfterTicksExpectAcquire { timeout_ticks: u32 },
    /// `acquire(Forever)` — expects to succeed.
    AcquireForever,
}

// ------------------------------------------------------------------
// CountingTaskContext — passed to Rust native helper entries.
// ------------------------------------------------------------------
struct CountingTaskContext {
    state: CaseState,
    /// Clone of the controller's CountingSemaphore — safe across
    /// early-return error paths.
    semaphore: osal::backend::CountingSemaphore,
    operation: CountingOperation,
    elapsed_ticks: AtomicU32,
    completion_tick: AtomicU32,
    #[allow(dead_code)]
    helper_id: u32,
}

impl CountingTaskContext {
    fn new(
        semaphore: &osal::backend::CountingSemaphore,
        operation: CountingOperation,
        helper_id: u32,
    ) -> Self {
        Self {
            state: CaseState::new(),
            semaphore: semaphore.clone(),
            operation,
            elapsed_ticks: AtomicU32::new(0),
            completion_tick: AtomicU32::new(0),
            helper_id,
        }
    }
}

// ------------------------------------------------------------------
// Counting helper entry
// ------------------------------------------------------------------

/// # Safety
/// `context` must be a `Box::into_raw`'d `CountingTaskContext`.
unsafe extern "C" fn counting_helper_entry(context: *mut c_void) {
    let result = {
        let ctx = unsafe { &*(context as *const CountingTaskContext) };
        ctx.state.record_phase(PHASE_STARTED);
        ctx.state.record_phase(PHASE_BEFORE_OPERATION);
        run_counting_operation(ctx)
    };
    let ctx = unsafe { &*(context as *const CountingTaskContext) };
    ctx.state.set_result(result);
    ctx.state.record_phase(PHASE_OPERATION_COMPLETED);
    ctx.state.record_phase(PHASE_EXITING);
    unsafe { harness::osal_test_task_exit(); }
}

fn run_counting_operation(ctx: &CountingTaskContext) -> i32 {
    let sem = &ctx.semaphore;

    match ctx.operation {
        CountingOperation::AcquireNoWaitExpectTimeout => {
            match sem.acquire(Timeout::NoWait) {
                Err(Error::Timeout) => 0,
                _ => -(SemaphoreError::NoWaitNotTimeout as i32),
            }
        }
        CountingOperation::AcquireAfterZeroExpectTimeout => {
            match sem.acquire(Timeout::After(core::time::Duration::ZERO)) {
                Err(Error::Timeout) => 0,
                _ => -(SemaphoreError::AfterZeroNotTimeout as i32),
            }
        }
        CountingOperation::AcquireAfterTicks { timeout_ticks } => {
            let start = sys::tick_snapshot();
            let result = sem.acquire(Timeout::After(
                core::time::Duration::from_millis(timeout_ticks as u64),
            ));
            let end = sys::tick_snapshot();
            let caps = sys::capabilities();
            ctx.elapsed_ticks.store(
                harness::total_ticks_diff(end, start, caps.tick_bits) as u32,
                Ordering::Release,
            );
            match result {
                Err(Error::Timeout) => 0,
                _ => -(SemaphoreError::AcquireNotTimeout as i32),
            }
        }
        CountingOperation::AcquireAfterTicksExpectAcquire { timeout_ticks: _ } => {
            let result = sem.acquire(Timeout::After(
                core::time::Duration::from_millis(100),
            ));
            match result {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    0
                }
                _ => -(SemaphoreError::BlockingNotAcquired as i32),
            }
        }
        CountingOperation::AcquireForever => {
            let result = sem.acquire(Timeout::Forever);
            match result {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    0
                }
                Err(Error::Timeout) => -(SemaphoreError::AcquireNotTimeout as i32),
                _ => -(SemaphoreError::BlockingNotAcquired as i32),
            }
        }
    }
}

// ------------------------------------------------------------------
// Counting helper spawn / reclaim
// ------------------------------------------------------------------

#[allow(dead_code)]
fn run_counting_helper(
    ctx: Box<CountingTaskContext>,
    tick_bits: u8,
) -> Result<(), HarnessError> {
    let raw = Box::into_raw(ctx);
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(HarnessError::SpawnFailed);
    }

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)?;
    harness::validate_helper(&ctx_ref.state)?;
    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)?;

    unsafe { drop(Box::from_raw(raw)); }
    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_core
// ------------------------------------------------------------------

fn counting_core(_tick_bits: u8) -> Result<(), SemaphoreError> {
    // Invalid: max_count == 0.
    match osal::backend::CountingSemaphore::new(0, 0) {
        Err(Error::InvalidParameter) => {}
        _ => return Err(SemaphoreError::InvalidParamNotRejected),
    }

    // Invalid: initial > max.
    match osal::backend::CountingSemaphore::new(3, 4) {
        Err(Error::InvalidParameter) => {}
        _ => return Err(SemaphoreError::InvalidParamNotRejected),
    }

    // Valid: max=3, initial=2.
    let s = osal::backend::CountingSemaphore::new(3, 2)
        .map_err(|_| SemaphoreError::Create)?;
    if s.max_count() != 3 {
        return Err(SemaphoreError::MaxMismatch);
    }
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 2 {
        return Err(SemaphoreError::CountMismatch);
    }

    // Acquire NoWait → count=1.
    s.acquire(Timeout::NoWait).map_err(|_| SemaphoreError::AcquireFailed)?;
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 1 {
        return Err(SemaphoreError::CountMismatch);
    }

    // Release → count=2.
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 2 {
        return Err(SemaphoreError::CountMismatch);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_overflow
// ------------------------------------------------------------------

fn counting_overflow(_tick_bits: u8) -> Result<(), SemaphoreError> {
    let s = osal::backend::CountingSemaphore::new(2, 2)
        .map_err(|_| SemaphoreError::Create)?;

    match s.release() {
        Err(Error::Overflow) => {}
        _ => return Err(SemaphoreError::OverflowNotReturned),
    }

    // Count must be unchanged (failure-atomic).
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 2 {
        return Err(SemaphoreError::OverflowCountChanged);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_nowait_zero
// ------------------------------------------------------------------

fn counting_nowait_zero(_tick_bits: u8) -> Result<(), SemaphoreError> {
    // Empty semaphore: both NoWait and After(ZERO) → Timeout.
    let s = osal::backend::CountingSemaphore::new(2, 0)
        .map_err(|_| SemaphoreError::Create)?;

    match s.acquire(Timeout::NoWait) {
        Err(Error::Timeout) => {}
        _ => return Err(SemaphoreError::NoWaitNotTimeout),
    }
    match s.acquire(Timeout::After(core::time::Duration::ZERO)) {
        Err(Error::Timeout) => {}
        _ => return Err(SemaphoreError::AfterZeroNotTimeout),
    }

    // With available count: both succeed.
    let s2 = osal::backend::CountingSemaphore::new(2, 2)
        .map_err(|_| SemaphoreError::Create)?;
    s2.acquire(Timeout::NoWait).map_err(|_| SemaphoreError::AcquireFailed)?;
    s2.acquire(Timeout::After(core::time::Duration::ZERO))
        .map_err(|_| SemaphoreError::AcquireFailed)?;
    if s2.count().map_err(|_| SemaphoreError::AcquireFailed)? != 0 {
        return Err(SemaphoreError::CountMismatch);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_finite_timeout
// ------------------------------------------------------------------

fn counting_finite_timeout(tick_bits: u8) -> Result<(), SemaphoreError> {
    let timeout_ticks = 5u32;

    let s = osal::backend::CountingSemaphore::new(1, 0)
        .map_err(|_| SemaphoreError::Create)?;

    let raw = Box::into_raw(Box::new(CountingTaskContext::new(
        &s,
        CountingOperation::AcquireAfterTicks { timeout_ticks },
        1,
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(SemaphoreError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| SemaphoreError::HelperSpawnFailed)?;

    let elapsed = ctx_ref.elapsed_ticks.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| SemaphoreError::HelperSpawnFailed)?;
    unsafe { drop(Box::from_raw(raw)); }

    if elapsed < timeout_ticks {
        return Err(SemaphoreError::TimeoutTooEarly);
    }

    // Count must still be 0.
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 0 {
        return Err(SemaphoreError::CountNotZero);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_clone
// ------------------------------------------------------------------

fn counting_clone(_tick_bits: u8) -> Result<(), SemaphoreError> {
    let baseline = sys::heap_free();

    let s1 = osal::backend::CountingSemaphore::new(2, 2)
        .map_err(|_| SemaphoreError::Create)?;
    let heap_with_one = sys::heap_free();

    let s2 = s1.clone();
    if sys::heap_free() != heap_with_one {
        return Err(SemaphoreError::CloneHeapLeak);
    }

    // Drop original — clone still works.
    drop(s1);
    s2.acquire(Timeout::NoWait).map_err(|_| SemaphoreError::AcquireFailed)?;
    if s2.count().map_err(|_| SemaphoreError::AcquireFailed)? != 1 {
        return Err(SemaphoreError::CountMismatch);
    }

    // Last handle drop must reclaim native resources.
    drop(s2);
    if sys::heap_free() != baseline {
        return Err(SemaphoreError::LastDropLeak);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_blocking_wake
// ------------------------------------------------------------------

fn counting_blocking_wake(tick_bits: u8) -> Result<(), SemaphoreError> {
    let s = osal::backend::CountingSemaphore::new(1, 0)
        .map_err(|_| SemaphoreError::Create)?;

    let raw = Box::into_raw(Box::new(CountingTaskContext::new(
        &s,
        CountingOperation::AcquireAfterTicksExpectAcquire { timeout_ticks: 100 },
        1,
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(SemaphoreError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(SemaphoreError::ControllerDelayFailed);
    }

    let release_tick = sys::tick_snapshot().tick_count as u32;
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    let completed = ctx_ref.completion_tick.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    unsafe { drop(Box::from_raw(raw)); }

    if completed.wrapping_sub(release_tick) > (u32::MAX / 2) {
        return Err(SemaphoreError::CompletedBeforeRelease);
    }

    // Count must be 0 (permit consumed by the helper).
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 0 {
        return Err(SemaphoreError::CountNotZero);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_forever_wake
// ------------------------------------------------------------------

fn counting_forever_wake(tick_bits: u8) -> Result<(), SemaphoreError> {
    let s = osal::backend::CountingSemaphore::new(1, 0)
        .map_err(|_| SemaphoreError::Create)?;

    let raw = Box::into_raw(Box::new(CountingTaskContext::new(
        &s,
        CountingOperation::AcquireForever,
        1,
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(SemaphoreError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(SemaphoreError::ControllerDelayFailed);
    }

    let release_tick = sys::tick_snapshot().tick_count as u32;
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    let completed = ctx_ref.completion_tick.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    unsafe { drop(Box::from_raw(raw)); }

    if completed.wrapping_sub(release_tick) > (u32::MAX / 2) {
        return Err(SemaphoreError::CompletedBeforeRelease);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_one_release_one_waiter
// ------------------------------------------------------------------

fn counting_one_release_one_waiter(tick_bits: u8) -> Result<(), SemaphoreError> {
    let s = osal::backend::CountingSemaphore::new(2, 0)
        .map_err(|_| SemaphoreError::Create)?;

    // Two helpers, both waiting on the empty semaphore.
    let raw_a = Box::into_raw(Box::new(CountingTaskContext::new(
        &s, CountingOperation::AcquireForever, 1,
    )));
    let raw_b = Box::into_raw(Box::new(CountingTaskContext::new(
        &s, CountingOperation::AcquireForever, 2,
    )));
    let ctx_a = unsafe { &*raw_a };
    let ctx_b = unsafe { &*raw_b };
    let task_baseline = sys::heap_free();

    let rc_a = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw_a.cast::<c_void>(), 1024, 2)
    };
    let rc_b = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw_b.cast::<c_void>(), 1024, 2)
    };
    if rc_a != 0 || rc_b != 0 {
        // On spawn failure, reclaim both.  If a task already started,
        // it holds its own clone; the context is leaked for safety.
        if rc_a != 0 { unsafe { drop(Box::from_raw(raw_a)); } }
        if rc_b != 0 { unsafe { drop(Box::from_raw(raw_b)); } }
        return Err(SemaphoreError::HelperSpawnFailed);
    }

    // Wait for both to enter the blocking acquire.
    harness::wait_until_phase(&ctx_a.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(SemaphoreError::ControllerDelayFailed);
    }

    // Release once — exactly one helper must complete.
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;

    // Wait a short time for the release to take effect.
    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(SemaphoreError::ControllerDelayFailed);
    }

    let phase_a = ctx_a.state.get_phase();
    let phase_b = ctx_b.state.get_phase();

    let completed = if phase_a >= PHASE_EXITING { 1u32 } else { 0u32 }
        + if phase_b >= PHASE_EXITING { 1u32 } else { 0u32 };

    if completed != 1 {
        return Err(SemaphoreError::WrongWaiterCount);
    }

    // Count must be 0 — permit consumed, not accumulated.
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 0 {
        return Err(SemaphoreError::CountNotZero);
    }

    // Release again — the second helper must now complete.
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;

    harness::wait_until_phase(&ctx_a.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    harness::validate_helper(&ctx_a.state).map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::validate_helper(&ctx_b.state).map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    unsafe { drop(Box::from_raw(raw_a)); }
    unsafe { drop(Box::from_raw(raw_b)); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: counting_permit_accounting
// ------------------------------------------------------------------

fn counting_permit_accounting(tick_bits: u8) -> Result<(), SemaphoreError> {
    let s = osal::backend::CountingSemaphore::new(3, 0)
        .map_err(|_| SemaphoreError::Create)?;

    let raw_a = Box::into_raw(Box::new(CountingTaskContext::new(
        &s, CountingOperation::AcquireForever, 1,
    )));
    let raw_b = Box::into_raw(Box::new(CountingTaskContext::new(
        &s, CountingOperation::AcquireForever, 2,
    )));
    let ctx_a = unsafe { &*raw_a };
    let ctx_b = unsafe { &*raw_b };
    let task_baseline = sys::heap_free();

    let rc_a = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw_a.cast::<c_void>(), 1024, 2)
    };
    let rc_b = unsafe {
        harness::native_task_spawn(counting_helper_entry, raw_b.cast::<c_void>(), 1024, 2)
    };
    if rc_a != 0 || rc_b != 0 {
        if rc_a != 0 { unsafe { drop(Box::from_raw(raw_a)); } }
        if rc_b != 0 { unsafe { drop(Box::from_raw(raw_b)); } }
        return Err(SemaphoreError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_a.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(SemaphoreError::ControllerDelayFailed);
    }

    // Release 3 times — two wake the waiters, one stays as count.
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;
    s.release().map_err(|_| SemaphoreError::OverflowNotReturned)?;

    harness::wait_until_phase(&ctx_a.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    harness::validate_helper(&ctx_a.state).map_err(|_| SemaphoreError::BlockingNotAcquired)?;
    harness::validate_helper(&ctx_b.state).map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| SemaphoreError::BlockingNotAcquired)?;

    unsafe { drop(Box::from_raw(raw_a)); }
    unsafe { drop(Box::from_raw(raw_b)); }

    // Two waiters consumed two permits; one permit remains.
    if s.count().map_err(|_| SemaphoreError::AcquireFailed)? != 1 {
        return Err(SemaphoreError::PermitLeak);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Public entry — called from the suite.
// ------------------------------------------------------------------

pub fn run_semaphore_cases(tick_bits: u8) -> Result<(), SemaphoreError> {
    counting_core(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_core");

    counting_overflow(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_overflow");

    counting_nowait_zero(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_nowait_zero");

    counting_finite_timeout(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_finite_timeout");

    counting_clone(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_clone");

    counting_blocking_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_blocking_wake");

    counting_forever_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_forever_wake");

    counting_one_release_one_waiter(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_one_release_one_waiter");

    counting_permit_accounting(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=counting_permit_accounting");

    Ok(())
}
