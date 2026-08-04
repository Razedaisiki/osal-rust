//! Mutex real-kernel contracts (P7G Step 4A).
//!
//! Tests the OSAL Mutex on real FreeRTOS against the Behavior Contract:
//! clone, non-recursive, NoWait / After(ZERO) distinction, and
//! finite-timeout guards.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;
use osal_backend_freertos_sys as sys;

use crate::harness::{self, CaseState, HarnessError};

unsafe extern "C" {
    fn osal_test_scheduler_suspend();
    fn osal_test_scheduler_resume();
}
use crate::harness::{
    PHASE_BEFORE_OPERATION, PHASE_EXITING, PHASE_OPERATION_COMPLETED, PHASE_STARTED,
};

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------
#[repr(i32)]
pub enum MutexError {
    Create = 120,
    CloneHeapLeak = 121,
    FirstLock = 122,
    SecondLock = 123,
    ThirdLock = 124,
    ValueMismatch = 125,
    LastDropLeak = 126,
    RelockNotFailed = 127,
    RelockAfterDrop = 128,
    NoWaitNotFailed = 130,
    AfterZeroNotTimeout = 131,
    TimeoutNotFailed = 132,
    ElapsedTooShort = 133,
    BlockingNotAcquired = 134,
    ForeverTimeout = 135,
    AcquiredBeforeRelease = 136,
    BlockingValueUnchanged = 137,
    ShutdownBusyNotReturned = 138,
    BusyStateChanged = 139,
    BusyHeapChanged = 140,
    ShutdownFailed = 141,
    ShutdownStateInvalid = 142,
    ControllerDelayFailed = 143,
}

// ------------------------------------------------------------------
// MutexOperation — what a native helper should attempt.
// ------------------------------------------------------------------
#[derive(Clone, Copy)]
enum MutexOperation {
    /// `lock(NoWait)` — expects LockFailed (mutex held by controller).
    NoWait,
    /// `lock(After(ZERO))` — expects Timeout (mutex held by controller).
    AfterZero,
    /// `lock(After(d))` — expects Timeout, records elapsed ticks.
    AfterTicks { timeout_ticks: u32 },
    /// `lock(After(d))` — expects to acquire, records acquired tick.
    AfterTicksExpectAcquire { timeout_ticks: u32 },
    /// `lock(Forever)` — expects to acquire (watchdog in controller).
    Forever,
}

// ------------------------------------------------------------------
// MutexTaskContext — passed to Rust native helper entries.
// ------------------------------------------------------------------
struct MutexTaskContext {
    state: CaseState,
    /// Clone of the controller's Mutex — the context owns this handle
    /// so the helper's access remains valid even if the controller
    /// returns early on an error path.
    mutex: osal::backend::Mutex<u32>,
    operation: MutexOperation,
    /// Ticks elapsed between BEFORE_OPERATION and the lock attempt result.
    elapsed_ticks: AtomicU32,
    /// Raw tick_count at the moment the lock was acquired.
    acquired_tick: AtomicU32,
}

impl MutexTaskContext {
    fn new(
        mutex: &osal::backend::Mutex<u32>,
        operation: MutexOperation,
    ) -> Self {
        Self {
            state: CaseState::new(),
            mutex: mutex.clone(),
            operation,
            elapsed_ticks: AtomicU32::new(0),
            acquired_tick: AtomicU32::new(0),
        }
    }

}

// ------------------------------------------------------------------
// Rust extern "C" helper entries
// ------------------------------------------------------------------

/// # Safety
/// `context` must be a `Box::leak`'d `MutexTaskContext`.
unsafe extern "C" fn mutex_helper_entry(context: *mut c_void) {
    // All MutexGuard and other drop work must complete inside this block.
    let result = {
        let ctx = unsafe { &*(context as *const MutexTaskContext) };

        ctx.state.record_phase(PHASE_STARTED);
        ctx.state.record_phase(PHASE_BEFORE_OPERATION);

        run_mutex_operation(ctx)
    };

    // Re-acquire context after the inner block — the reference above is gone.
    let ctx = unsafe { &*(context as *const MutexTaskContext) };
    ctx.state.set_result(result);
    ctx.state.record_phase(PHASE_OPERATION_COMPLETED);
    ctx.state.record_phase(PHASE_EXITING);

    // No active MutexGuard or other borrows at this point.
    unsafe { harness::osal_test_task_exit(); }
}

fn run_mutex_operation(ctx: &MutexTaskContext) -> i32 {
    let mutex = &ctx.mutex;

    match ctx.operation {
        MutexOperation::NoWait => match mutex.lock(Timeout::NoWait) {
            Err(Error::LockFailed) => 0,
            _ => -(MutexError::NoWaitNotFailed as i32),
        },
        MutexOperation::AfterZero => match mutex.lock(Timeout::After(core::time::Duration::ZERO)) {
            Err(Error::Timeout) => 0,
            _ => -(MutexError::AfterZeroNotTimeout as i32),
        },
        MutexOperation::AfterTicks { timeout_ticks } => {
            let start = sys::tick_snapshot();
            let result = mutex.lock(Timeout::After(
                core::time::Duration::from_millis(timeout_ticks as u64),
            ));
            let end = sys::tick_snapshot();

            // Record elapsed regardless of outcome.
            let caps = sys::capabilities();
            ctx.elapsed_ticks.store(
                harness::total_ticks_diff(end, start, caps.tick_bits) as u32,
                Ordering::Release,
            );

            match result {
                Err(Error::Timeout) => 0,
                _ => -(MutexError::TimeoutNotFailed as i32),
            }
        }
        MutexOperation::AfterTicksExpectAcquire { timeout_ticks } => {
            let result = mutex.lock(Timeout::After(
                core::time::Duration::from_millis(timeout_ticks as u64),
            ));
            match result {
                Ok(mut guard) => {
                    let snap = sys::tick_snapshot();
                    ctx.acquired_tick.store(snap.tick_count as u32, Ordering::Release);
                    *guard = guard.wrapping_add(1);
                    drop(guard);
                    0
                }
                _ => -(MutexError::BlockingNotAcquired as i32),
            }
        }
        MutexOperation::Forever => {
            let result = mutex.lock(Timeout::Forever);
            match result {
                Ok(mut guard) => {
                    let snap = sys::tick_snapshot();
                    ctx.acquired_tick.store(snap.tick_count as u32, Ordering::Release);
                    *guard = guard.wrapping_add(1);
                    drop(guard);
                    0
                }
                Err(Error::Timeout) => -(MutexError::ForeverTimeout as i32),
                _ => -(MutexError::BlockingNotAcquired as i32),
            }
        }
    }
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

/// Spawn a Rust native helper, wait for it to self-delete, verify
/// phases, wait for Idle-task TCB/stack reclamation, then reclaim
/// the context Box.
///
/// On spawn failure the Box is immediately reclaimed.  On timeout or
/// helper-error paths the context is intentionally leaked — the
/// native task may still be running and its Mutex clone must remain
/// valid.  The test then fails via QEMU non-zero exit.
fn run_mutex_helper(
    ctx: Box<MutexTaskContext>,
    tick_bits: u8,
) -> Result<(), HarnessError> {
    let raw = Box::into_raw(ctx);
    let ctx_ref = unsafe { &*raw };

    // Take a task-spawn baseline so we can wait for TCB/stack reclaim.
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(mutex_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };

    if rc != 0 {
        // Spawn failed — helper never ran, safe to reclaim.
        unsafe { drop(Box::from_raw(raw)); }
        return Err(HarnessError::SpawnFailed);
    }

    // Wait for the helper to self-delete.
    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)?;
    harness::validate_helper(&ctx_ref.state)?;

    // The helper has called vTaskDelete(NULL).  Wait for the Idle
    // task to reclaim the TCB and stack before we reclaim the
    // context (and its Mutex clone).
    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)?;

    // Safe: helper is gone; context and its Mutex clone can be dropped.
    unsafe { drop(Box::from_raw(raw)); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_basic_clone
// ------------------------------------------------------------------

fn mutex_basic_clone(_tick_bits: u8) -> Result<(), MutexError> {
    let baseline = sys::heap_free();

    let m1 = osal::backend::Mutex::new(42u32).map_err(|_| MutexError::Create)?;
    let heap_with_one = sys::heap_free();

    // Clone must not allocate a second native mutex.
    let m2 = m1.clone();
    if sys::heap_free() != heap_with_one {
        return Err(MutexError::CloneHeapLeak);
    }

    // Drop the original — clone must still work.
    drop(m1);
    {
        let guard = m2.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;
        if *guard != 42 {
            return Err(MutexError::ValueMismatch);
        }
    }

    // Modify through the clone.
    {
        let mut guard = m2.lock(Timeout::NoWait).map_err(|_| MutexError::SecondLock)?;
        *guard = 99;
    }

    // Read back.
    {
        let guard = m2.lock(Timeout::NoWait).map_err(|_| MutexError::ThirdLock)?;
        if *guard != 99 {
            return Err(MutexError::ValueMismatch);
        }
    }

    // Last handle drop must reclaim native resources.
    drop(m2);
    if sys::heap_free() != baseline {
        return Err(MutexError::LastDropLeak);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_non_recursive
// ------------------------------------------------------------------

fn mutex_non_recursive(_tick_bits: u8) -> Result<(), MutexError> {
    let m = osal::backend::Mutex::new(1u32).map_err(|_| MutexError::Create)?;

    // Hold a guard, then try to re-lock from the same task.
    let guard = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    match m.lock(Timeout::NoWait) {
        Err(Error::LockFailed) => {} // expected
        _ => return Err(MutexError::RelockNotFailed),
    }

    // Drop the guard; now re-lock must succeed.
    drop(guard);
    let _g2 = m.lock(Timeout::NoWait).map_err(|_| MutexError::RelockAfterDrop)?;

    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_nowait_zero
// ------------------------------------------------------------------

/// Proves NoWait → LockFailed and After(ZERO) → Timeout from a second
/// task while the controller holds the mutex.
fn mutex_nowait_zero(tick_bits: u8) -> Result<(), MutexError> {
    let m = osal::backend::Mutex::new(1u32).map_err(|_| MutexError::Create)?;
    let guard = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    // Helper A — NoWait must return LockFailed.
    {
        let ctx = Box::new(MutexTaskContext::new(&m, MutexOperation::NoWait));
        run_mutex_helper(ctx, tick_bits).map_err(|_| MutexError::NoWaitNotFailed)?;
    }

    // Helper B — After(ZERO) must return Timeout.
    {
        let ctx = Box::new(MutexTaskContext::new(&m, MutexOperation::AfterZero));
        run_mutex_helper(ctx, tick_bits).map_err(|_| MutexError::AfterZeroNotTimeout)?;
    }

    drop(guard);

    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_finite_timeout
// ------------------------------------------------------------------

/// Controller holds the mutex; helper calls After(5ms) and must get
/// Timeout with elapsed_ticks >= 5.
fn mutex_finite_timeout(tick_bits: u8) -> Result<(), MutexError> {
    let timeout_ticks = 5u32;

    let m = osal::backend::Mutex::new(1u32).map_err(|_| MutexError::Create)?;
    let guard = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    let raw = Box::into_raw(Box::new(MutexTaskContext::new(
        &m,
        MutexOperation::AfterTicks { timeout_ticks },
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(mutex_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };

    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(MutexError::NoWaitNotFailed);
    }

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| MutexError::TimeoutNotFailed)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| MutexError::TimeoutNotFailed)?;

    // Read elapsed before reclaiming.
    let elapsed = ctx_ref.elapsed_ticks.load(Ordering::Acquire);

    // Wait for Idle task TCB/stack reclamation.
    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| MutexError::TimeoutNotFailed)?;

    unsafe { drop(Box::from_raw(raw)); }
    drop(guard);

    if elapsed < timeout_ticks {
        return Err(MutexError::ElapsedTooShort);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_blocking_wake
// ------------------------------------------------------------------

/// Controller holds the mutex; helper blocks on After(100ms).
/// Controller drops the guard → helper acquires.
/// Verifies acquired_tick >= release_tick (acquire happens after release).
fn mutex_blocking_wake(tick_bits: u8) -> Result<(), MutexError> {
    let m = osal::backend::Mutex::new(0u32).map_err(|_| MutexError::Create)?;
    let guard = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    let raw = Box::into_raw(Box::new(MutexTaskContext::new(
        &m,
        MutexOperation::AfterTicksExpectAcquire { timeout_ticks: 100 },
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(mutex_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(MutexError::BlockingNotAcquired);
    }

    // Wait for the helper to enter the blocking lock call.
    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| MutexError::BlockingNotAcquired)?;

    // Hold the lock for at least 2 ticks to ensure the helper is truly
    // blocked in the native mutex take, not just en route.
    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(MutexError::ControllerDelayFailed);
    }

    // Record release tick before dropping the guard.
    let release_tick = sys::tick_snapshot().tick_count as u32;
    drop(guard);

    // Wait for the helper to acquire and complete.
    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| MutexError::BlockingNotAcquired)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| MutexError::BlockingNotAcquired)?;

    let acquired = ctx_ref.acquired_tick.load(Ordering::Acquire);

    // Wait for Idle task TCB/stack reclamation.
    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| MutexError::BlockingNotAcquired)?;

    unsafe { drop(Box::from_raw(raw)); }

    if acquired < release_tick {
        return Err(MutexError::AcquiredBeforeRelease);
    }

    let final_val = {
        let g = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;
        *g
    };
    if final_val != 1 {
        return Err(MutexError::BlockingValueUnchanged);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_forever_wake
// ------------------------------------------------------------------

/// Controller holds the mutex; helper calls Forever.
/// Controller releases within a finite watchdog; helper must acquire
/// and must NOT return Timeout.
fn mutex_forever_wake(tick_bits: u8) -> Result<(), MutexError> {
    let m = osal::backend::Mutex::new(0u32).map_err(|_| MutexError::Create)?;
    let guard = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    let raw = Box::into_raw(Box::new(MutexTaskContext::new(&m, MutexOperation::Forever)));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(mutex_helper_entry, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != 0 {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(MutexError::BlockingNotAcquired);
    }

    // Wait for the helper to enter the blocking call.
    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| MutexError::BlockingNotAcquired)?;

    if sys::delay_ticks(2) != sys::DelayStatus::Ok {
        return Err(MutexError::ControllerDelayFailed);
    }

    let release_tick = sys::tick_snapshot().tick_count as u32;
    drop(guard);

    // Watchdog: the helper must finish within 100 ticks.
    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| MutexError::BlockingNotAcquired)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| MutexError::BlockingNotAcquired)?;

    let acquired = ctx_ref.acquired_tick.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| MutexError::BlockingNotAcquired)?;

    unsafe { drop(Box::from_raw(raw)); }

    if acquired < release_tick {
        return Err(MutexError::AcquiredBeforeRelease);
    }

    let final_val = {
        let g = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;
        *g
    };
    if final_val != 1 {
        return Err(MutexError::BlockingValueUnchanged);
    }

    Ok(())
}

// ------------------------------------------------------------------
// SchedulerResumeGuard — RAII resume on drop.
// ------------------------------------------------------------------

struct SchedulerResumeGuard;

impl SchedulerResumeGuard {
    fn new() -> Self {
        unsafe { osal_test_scheduler_suspend(); }
        SchedulerResumeGuard
    }
}

impl Drop for SchedulerResumeGuard {
    fn drop(&mut self) {
        unsafe { osal_test_scheduler_resume(); }
    }
}

// ------------------------------------------------------------------
// Case: mutex_scheduler_suspended
// ------------------------------------------------------------------

/// When the scheduler is suspended, After(d>0) and Forever must
/// return Busy; NoWait and After(ZERO) remain non-blocking.
fn mutex_scheduler_suspended(tick_bits: u8) -> Result<(), MutexError> {
    let m = osal::backend::Mutex::new(1u32).map_err(|_| MutexError::Create)?;
    let guard = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    {
        // RAII: resume on any early return.
        let _resume = SchedulerResumeGuard::new();

        // NoWait — still non-blocking, must return LockFailed.
        match m.lock(Timeout::NoWait) {
            Err(Error::LockFailed) => {}
            _ => return Err(MutexError::NoWaitNotFailed),
        }

        // After(ZERO) — still non-blocking, must return Timeout.
        match m.lock(Timeout::After(core::time::Duration::ZERO)) {
            Err(Error::Timeout) => {}
            _ => return Err(MutexError::AfterZeroNotTimeout),
        }

        // After(d>0) — blocking, must return Busy when suspended.
        match m.lock(Timeout::After(core::time::Duration::from_millis(1))) {
            Err(Error::Busy) => {}
            _ => return Err(MutexError::TimeoutNotFailed),
        }

        // Forever — blocking, must return Busy when suspended.
        match m.lock(Timeout::Forever) {
            Err(Error::Busy) => {}
            _ => return Err(MutexError::BlockingNotAcquired),
        }

        // SchedulerResumeGuard drops here → scheduler resumed.
    }

    // Post-resume: NoWait must still work correctly.
    drop(guard);
    let _g = m.lock(Timeout::NoWait).map_err(|_| MutexError::FirstLock)?;

    let _ = tick_bits;
    Ok(())
}

// ------------------------------------------------------------------
// Case: mutex_runtime_lease
// ------------------------------------------------------------------

/// Active Mutex handle blocks shutdown.  After last handle drop,
/// shutdown succeeds and heap returns to suite baseline.
fn mutex_runtime_lease(_tick_bits: u8, suite_baseline: u64) -> Result<(), MutexError> {
    let m = osal::backend::Mutex::new(99u32).map_err(|_| MutexError::Create)?;

    // Active handle: shutdown must be Busy and failure-atomic.
    let heap_before = sys::heap_free();
    match osal::shutdown() {
        Err(Error::Busy) => {}
        _ => return Err(MutexError::ShutdownBusyNotReturned),
    }
    if osal::runtime_state() != osal_api::runtime::RuntimeState::Running {
        return Err(MutexError::BusyStateChanged);
    }
    if sys::heap_free() != heap_before {
        return Err(MutexError::BusyHeapChanged);
    }

    // Drop the last handle and shut down.
    drop(m);
    osal::shutdown().map_err(|_| MutexError::ShutdownFailed)?;

    if osal::runtime_state() != osal_api::runtime::RuntimeState::Uninitialized {
        return Err(MutexError::ShutdownStateInvalid);
    }

    if sys::heap_free() != suite_baseline {
        return Err(MutexError::LastDropLeak);
    }

    // Re-initialize so the suite can finish its protocol.
    osal::initialize().map_err(|_| MutexError::Create)?;

    Ok(())
}

// ------------------------------------------------------------------
// Public entry — called from the suite.
// ------------------------------------------------------------------

pub fn run_mutex_cases(tick_bits: u8, suite_baseline: u64) -> Result<(), MutexError> {
    mutex_basic_clone(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_basic_clone");

    mutex_non_recursive(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_non_recursive");

    mutex_nowait_zero(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_nowait_zero");

    mutex_finite_timeout(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_finite_timeout");

    mutex_blocking_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_blocking_wake");

    mutex_forever_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_forever_wake");

    mutex_scheduler_suspended(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_scheduler_suspended");

    mutex_runtime_lease(tick_bits, suite_baseline)?;
    harness::console_line(c"OSAL_CASE_PASS name=mutex_runtime_lease");

    Ok(())
}
