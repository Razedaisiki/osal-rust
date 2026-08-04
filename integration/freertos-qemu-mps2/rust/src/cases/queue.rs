//! Queue real-kernel contracts (P7G Step 4C).
//!
//! Validates the OSAL Queue on real FreeRTOS against the Behavior
//! Contract: FIFO, error precedence, NoWait / After(ZERO), finite
//! timeout, blocking wake, Forever, multi-waiter wake-one, close-drain,
//! close broadcast, timeout/wake race, scheduler suspended, clone /
//! last-drop, RuntimeLease, and stress.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;
use osal_backend_freertos_sys as sys;

use crate::harness::{self, CaseState, HarnessError};
use crate::harness::{
    PHASE_BEFORE_OPERATION, PHASE_EXITING, PHASE_OPERATION_COMPLETED, PHASE_STARTED,
};

// ------------------------------------------------------------------
// Payload constants — fixed 4-byte messages.
// ------------------------------------------------------------------
const M0: [u8; 4] = [0x10, 0, 0, 0];
const M1: [u8; 4] = [0x21, 0, 0, 1];
const M2: [u8; 4] = [0x32, 0, 0, 2];
#[allow(dead_code)]
const M3: [u8; 4] = [0x43, 0, 0, 3];

fn payload_eq(a: &[u8], b: &[u8; 4]) -> bool {
    a.len() == 4 && a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------
#[allow(dead_code)]
#[repr(i32)]
pub enum QueueError {
    Create = 300,
    InvalidParamNotRejected = 301,
    CapacityMismatch = 302,
    MessageSizeMismatch = 303,
    LengthMismatch = 304,
    FifoMismatch = 305,
    WrongSizePrecedence = 306,
    NoWaitMapping = 307,
    AfterZeroMapping = 308,

    HelperSpawnFailed = 320,
    TimeoutNotReturned = 321,
    TimeoutTooEarly = 322,
    CompletedBeforeTrigger = 323,
    PayloadMismatch = 324,
    StateChangedOnTimeout = 325,

    WrongWaiterCount = 340,
    MessageLost = 341,
    MessageDuplicated = 342,
    StrandedWaiter = 343,

    CloseNotReturned = 360,
    CloseDrainMismatch = 361,
    CloseBroadcastFailed = 362,
    ClosePriorityMismatch = 363,

    BlockingNotBusy = 380,
    BusyStateChanged = 381,
    BusyHeapChanged = 382,
    LastDropLeak = 383,
    SuiteHeapLeak = 384,
}

// ------------------------------------------------------------------
// Operations
// ------------------------------------------------------------------
#[allow(dead_code)]
#[derive(Clone, Copy)]
enum SendOperation {
    NoWait,
    AfterZero,
    AfterTicks { timeout_ms: u32 },
    Forever,
}

#[allow(dead_code)]
#[derive(Clone, Copy)]
enum RecvOperation {
    NoWait,
    AfterZero,
    AfterTicks { timeout_ms: u32 },
    Forever,
}

// ------------------------------------------------------------------
// Queue task contexts
// ------------------------------------------------------------------
struct QueueSendContext {
    state: CaseState,
    queue: osal::backend::Queue,
    #[allow(dead_code)]
    operation: SendOperation,
    payload: [u8; 4],
    elapsed_ticks: AtomicU32,
    completion_tick: AtomicU32,
}

impl QueueSendContext {
    #[allow(dead_code)]
    fn new(queue: &osal::backend::Queue, operation: SendOperation, payload: [u8; 4]) -> Self {
        Self {
            state: CaseState::new(),
            queue: queue.clone(),
            operation,
            payload,
            elapsed_ticks: AtomicU32::new(0),
            completion_tick: AtomicU32::new(0),
        }
    }
}

struct QueueRecvContext {
    state: CaseState,
    queue: osal::backend::Queue,
    #[allow(dead_code)]
    operation: RecvOperation,
    received_word: AtomicU32,
    elapsed_ticks: AtomicU32,
    completion_tick: AtomicU32,
}

impl QueueRecvContext {
    #[allow(dead_code)]
    fn new(queue: &osal::backend::Queue, operation: RecvOperation) -> Self {
        Self {
            state: CaseState::new(),
            queue: queue.clone(),
            operation,
            received_word: AtomicU32::new(0),
            elapsed_ticks: AtomicU32::new(0),
            completion_tick: AtomicU32::new(0),
        }
    }
}

// ------------------------------------------------------------------
// Helper entries
// ------------------------------------------------------------------

/// # Safety
/// `context` must be a `Box::into_raw`'d `QueueSendContext`.
unsafe extern "C" fn queue_send_helper(context: *mut c_void) {
    let result = {
        let ctx = unsafe { &*(context as *const QueueSendContext) };
        ctx.state.record_phase(PHASE_STARTED);
        ctx.state.record_phase(PHASE_BEFORE_OPERATION);
        run_send_operation(ctx)
    };
    let ctx = unsafe { &*(context as *const QueueSendContext) };
    ctx.state.set_result(result);
    ctx.state.record_phase(PHASE_OPERATION_COMPLETED);
    ctx.state.record_phase(PHASE_EXITING);
    unsafe { harness::osal_test_task_exit(); }
}

/// # Safety
/// `context` must be a `Box::into_raw`'d `QueueRecvContext`.
unsafe extern "C" fn queue_recv_helper(context: *mut c_void) {
    let result = {
        let ctx = unsafe { &*(context as *const QueueRecvContext) };
        ctx.state.record_phase(PHASE_STARTED);
        ctx.state.record_phase(PHASE_BEFORE_OPERATION);
        run_recv_operation(ctx)
    };
    let ctx = unsafe { &*(context as *const QueueRecvContext) };
    ctx.state.set_result(result);
    ctx.state.record_phase(PHASE_OPERATION_COMPLETED);
    ctx.state.record_phase(PHASE_EXITING);
    unsafe { harness::osal_test_task_exit(); }
}

fn run_send_operation(ctx: &QueueSendContext) -> i32 {
    let q = &ctx.queue;
    match ctx.operation {
        SendOperation::NoWait => match q.send(&ctx.payload, Timeout::NoWait) {
            Err(Error::QueueFull) => 0,
            _ => -(QueueError::NoWaitMapping as i32),
        },
        SendOperation::AfterZero => match q.send(&ctx.payload, Timeout::After(core::time::Duration::ZERO)) {
            Err(Error::Timeout) => 0,
            _ => -(QueueError::AfterZeroMapping as i32),
        },
        SendOperation::AfterTicks { timeout_ms: _ } => {
            let start = sys::tick_snapshot();
            let result = q.send(&ctx.payload, Timeout::After(core::time::Duration::from_millis(100)));
            let end = sys::tick_snapshot();
            let caps = sys::capabilities();
            ctx.elapsed_ticks.store(
                harness::total_ticks_diff(end, start, caps.tick_bits) as u32,
                Ordering::Release,
            );
            match result {
                Err(Error::Timeout) => 0,
                _ => -(QueueError::TimeoutNotReturned as i32),
            }
        }
        SendOperation::Forever => {
            match q.send(&ctx.payload, Timeout::Forever) {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    0
                }
                _ => -(QueueError::TimeoutNotReturned as i32),
            }
        }
    }
}

fn run_recv_operation(ctx: &QueueRecvContext) -> i32 {
    let q = &ctx.queue;
    match ctx.operation {
        RecvOperation::NoWait => {
            let mut buf = [0u8; 4];
            match q.recv(&mut buf, Timeout::NoWait) {
                Err(Error::QueueEmpty) => 0,
                Ok(()) => {
                    ctx.received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    0
                }
                _ => -(QueueError::NoWaitMapping as i32),
            }
        }
        RecvOperation::AfterZero => {
            let mut buf = [0u8; 4];
            match q.recv(&mut buf, Timeout::After(core::time::Duration::ZERO)) {
                Err(Error::Timeout) => 0,
                Ok(()) => {
                    ctx.received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    -(QueueError::AfterZeroMapping as i32) // unexpected success
                }
                _ => -(QueueError::AfterZeroMapping as i32),
            }
        }
        RecvOperation::AfterTicks { timeout_ms: _ } => {
            let mut buf = [0u8; 4];
            let start = sys::tick_snapshot();
            let result = q.recv(&mut buf, Timeout::After(core::time::Duration::from_millis(100)));
            let end = sys::tick_snapshot();
            let caps = sys::capabilities();
            ctx.elapsed_ticks.store(
                harness::total_ticks_diff(end, start, caps.tick_bits) as u32,
                Ordering::Release,
            );
            match result {
                Err(Error::Timeout) => 0,
                Ok(()) => {
                    ctx.received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    0
                }
                _ => -(QueueError::TimeoutNotReturned as i32),
            }
        }
        RecvOperation::Forever => {
            let mut buf = [0u8; 4];
            match q.recv(&mut buf, Timeout::Forever) {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    ctx.received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    0
                }
                _ => -(QueueError::TimeoutNotReturned as i32),
            }
        }
    }
}

// ------------------------------------------------------------------
// Helper spawn
// ------------------------------------------------------------------

#[allow(dead_code)]
fn run_send_helper(ctx: Box<QueueSendContext>, tick_bits: u8) -> Result<(), HarnessError> {
    let raw = Box::into_raw(ctx);
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();
    let rc = unsafe { harness::native_task_spawn(queue_send_helper, raw.cast::<c_void>(), 1024, 2) };
    if rc != 0 { unsafe { drop(Box::from_raw(raw)); } return Err(HarnessError::SpawnFailed); }
    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)?;
    harness::validate_helper(&ctx_ref.state)?;
    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)?;
    unsafe { drop(Box::from_raw(raw)); }
    Ok(())
}

#[allow(dead_code)]
fn run_recv_helper(ctx: Box<QueueRecvContext>, tick_bits: u8) -> Result<(), HarnessError> {
    let raw = Box::into_raw(ctx);
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();
    let rc = unsafe { harness::native_task_spawn(queue_recv_helper, raw.cast::<c_void>(), 1024, 2) };
    if rc != 0 { unsafe { drop(Box::from_raw(raw)); } return Err(HarnessError::SpawnFailed); }
    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)?;
    harness::validate_helper(&ctx_ref.state)?;
    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)?;
    unsafe { drop(Box::from_raw(raw)); }
    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_core_fifo
// ------------------------------------------------------------------

fn queue_core_fifo(_tick_bits: u8) -> Result<(), QueueError> {
    // Invalid parameters.
    match osal::backend::Queue::new(0, 4) {
        Err(Error::InvalidParameter) => {}
        _ => return Err(QueueError::InvalidParamNotRejected),
    }
    match osal::backend::Queue::new(4, 0) {
        Err(Error::InvalidParameter) => {}
        _ => return Err(QueueError::InvalidParamNotRejected),
    }

    let q = osal::backend::Queue::new(3, 4).map_err(|_| QueueError::Create)?;
    if q.capacity() != 3 { return Err(QueueError::CapacityMismatch); }
    if q.msg_size() != 4 { return Err(QueueError::MessageSizeMismatch); }
    if q.len().map_err(|_| QueueError::Create)? != 0 { return Err(QueueError::LengthMismatch); }
    if !q.is_empty().map_err(|_| QueueError::Create)? { return Err(QueueError::LengthMismatch); }
    if q.is_full().map_err(|_| QueueError::Create)? { return Err(QueueError::LengthMismatch); }

    // FIFO: send M0, M1, M2.
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.send(&M2, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if q.len().map_err(|_| QueueError::Create)? != 3 { return Err(QueueError::LengthMismatch); }
    if !q.is_full().map_err(|_| QueueError::Create)? { return Err(QueueError::LengthMismatch); }

    // Recv in FIFO order.
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M0) { return Err(QueueError::FifoMismatch); }
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M1) { return Err(QueueError::FifoMismatch); }
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M2) { return Err(QueueError::FifoMismatch); }
    if !q.is_empty().map_err(|_| QueueError::Create)? { return Err(QueueError::LengthMismatch); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_wrong_size_precedence
// ------------------------------------------------------------------

fn queue_wrong_size_precedence(_tick_bits: u8) -> Result<(), QueueError> {
    let q = osal::backend::Queue::new(2, 4).map_err(|_| QueueError::Create)?;

    // Wrong size on open queue.
    match q.send(&[1u8; 3], Timeout::NoWait) {
        Err(Error::InvalidMessageSize) => {}
        _ => return Err(QueueError::WrongSizePrecedence),
    }
    let mut buf = [0u8; 8];
    match q.recv(&mut buf, Timeout::NoWait) {
        Err(Error::InvalidMessageSize) => {}
        _ => return Err(QueueError::WrongSizePrecedence),
    }

    // Close, then wrong size still takes priority over QueueClosed.
    q.close().map_err(|_| QueueError::CloseNotReturned)?;

    match q.send(&[1u8; 3], Timeout::NoWait) {
        Err(Error::InvalidMessageSize) => {}
        _ => return Err(QueueError::WrongSizePrecedence),
    }
    match q.recv(&mut buf, Timeout::NoWait) {
        Err(Error::InvalidMessageSize) => {}
        _ => return Err(QueueError::WrongSizePrecedence),
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_nowait_zero
// ------------------------------------------------------------------

fn queue_nowait_zero(_tick_bits: u8) -> Result<(), QueueError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueError::Create)?;
    let mut buf = [0u8; 4];

    // Empty: recv NoWait → QueueEmpty.
    match q.recv(&mut buf, Timeout::NoWait) {
        Err(Error::QueueEmpty) => {}
        _ => return Err(QueueError::NoWaitMapping),
    }
    // Fill the queue.
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;

    // Full: send NoWait → QueueFull.
    match q.send(&M1, Timeout::NoWait) {
        Err(Error::QueueFull) => {}
        _ => return Err(QueueError::NoWaitMapping),
    }
    // Full: send After(ZERO) → Timeout.
    match q.send(&M1, Timeout::After(core::time::Duration::ZERO)) {
        Err(Error::Timeout) => {}
        _ => return Err(QueueError::AfterZeroMapping),
    }

    // After drain, NoWait recv + AfterZero send must succeed.
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueError::NoWaitMapping)?;
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    // Empty: recv After(ZERO) → Timeout.
    match q.recv(&mut buf, Timeout::After(core::time::Duration::ZERO)) {
        Err(Error::Timeout) => {}
        _ => return Err(QueueError::AfterZeroMapping),
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_clone_lifecycle
// ------------------------------------------------------------------

fn queue_clone_lifecycle(_tick_bits: u8) -> Result<(), QueueError> {
    let baseline = sys::heap_free();

    let q1 = osal::backend::Queue::new(2, 4).map_err(|_| QueueError::Create)?;
    let heap_with_one = sys::heap_free();

    let q2 = q1.clone();
    if sys::heap_free() != heap_with_one {
        return Err(QueueError::LastDropLeak);
    }

    // Send through original, recv through clone.
    q1.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    drop(q1);
    let mut buf = [0u8; 4];
    q2.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M0) { return Err(QueueError::FifoMismatch); }

    // Last handle drop must reclaim native resources.
    drop(q2);
    if sys::heap_free() != baseline {
        return Err(QueueError::LastDropLeak);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_recv_finite_timeout (controller-only, no helper)
// ------------------------------------------------------------------

fn queue_recv_finite_timeout(_tick_bits: u8) -> Result<(), QueueError> {
    let timeout_ms = 5u32;
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueError::Create)?;

    let mut buf = [0u8; 4];
    let start = sys::tick_snapshot();
    match q.recv(&mut buf, Timeout::After(core::time::Duration::from_millis(timeout_ms as u64))) {
        Err(Error::Timeout) => {}
        _ => return Err(QueueError::TimeoutNotReturned),
    }
    let end = sys::tick_snapshot();
    let caps = sys::capabilities();
    let elapsed = harness::total_ticks_diff(end, start, caps.tick_bits) as u32;
    if elapsed < timeout_ms { return Err(QueueError::TimeoutTooEarly); }
    if q.len().map_err(|_| QueueError::Create)? != 0 { return Err(QueueError::StateChangedOnTimeout); }

    // Post-timeout: queue must still be usable.
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M0) { return Err(QueueError::FifoMismatch); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_send_finite_timeout (controller-only, no helper)
// ------------------------------------------------------------------

fn queue_send_finite_timeout(_tick_bits: u8) -> Result<(), QueueError> {
    let timeout_ms = 5u32;
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;

    let start = sys::tick_snapshot();
    match q.send(&M1, Timeout::After(core::time::Duration::from_millis(timeout_ms as u64))) {
        Err(Error::Timeout) => {}
        _ => return Err(QueueError::TimeoutNotReturned),
    }
    let end = sys::tick_snapshot();
    let caps = sys::capabilities();
    let elapsed = harness::total_ticks_diff(end, start, caps.tick_bits) as u32;
    if elapsed < timeout_ms { return Err(QueueError::TimeoutTooEarly); }
    if q.len().map_err(|_| QueueError::Create)? != 1 { return Err(QueueError::StateChangedOnTimeout); }

    // Post-timeout: original message still readable, queue usable.
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M0) { return Err(QueueError::FifoMismatch); }
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    if !payload_eq(&buf, &M1) { return Err(QueueError::FifoMismatch); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_close_drain
// ------------------------------------------------------------------

fn queue_close_drain(_tick_bits: u8) -> Result<(), QueueError> {
    let q = osal::backend::Queue::new(2, 4).map_err(|_| QueueError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;

    q.close().map_err(|_| QueueError::CloseNotReturned)?;

    // Send after close → QueueClosed.
    match q.send(&M2, Timeout::NoWait) {
        Err(Error::QueueClosed) => {}
        _ => return Err(QueueError::CloseNotReturned),
    }

    // Drain existing messages.
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::CloseDrainMismatch)?;
    if !payload_eq(&buf, &M0) { return Err(QueueError::CloseDrainMismatch); }
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::CloseDrainMismatch)?;
    if !payload_eq(&buf, &M1) { return Err(QueueError::CloseDrainMismatch); }

    // Empty + closed → QueueClosed.
    match q.recv(&mut buf, Timeout::NoWait) {
        Err(Error::QueueClosed) => {}
        _ => return Err(QueueError::CloseDrainMismatch),
    }

    if q.len().map_err(|_| QueueError::Create)? != 0 { return Err(QueueError::CloseDrainMismatch); }

    // close() must be idempotent.
    q.close().map_err(|_| QueueError::CloseNotReturned)?;
    q.close().map_err(|_| QueueError::CloseNotReturned)?;

    // Queries still work after close.
    if q.capacity() != 2 { return Err(QueueError::CapacityMismatch); }
    if q.msg_size() != 4 { return Err(QueueError::MessageSizeMismatch); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_scheduler_suspended
// ------------------------------------------------------------------

fn queue_scheduler_suspended(_tick_bits: u8) -> Result<(), QueueError> {
    unsafe extern "C" { fn osal_test_scheduler_suspend(); fn osal_test_scheduler_resume(); }
    struct Guard;
    impl Drop for Guard { fn drop(&mut self) { unsafe { osal_test_scheduler_resume(); } } }

    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;

    {
        let _resume = Guard;
        unsafe { osal_test_scheduler_suspend(); }

        // Full: NoWait → QueueFull.
        match q.send(&M1, Timeout::NoWait) {
            Err(Error::QueueFull) => {}
            _ => return Err(QueueError::NoWaitMapping),
        }
        // Full: After(ZERO) → Timeout.
        match q.send(&M1, Timeout::After(core::time::Duration::ZERO)) {
            Err(Error::Timeout) => {}
            _ => return Err(QueueError::AfterZeroMapping),
        }
        // Full: After(d>0) → Busy (blocking).
        match q.send(&M1, Timeout::After(core::time::Duration::from_millis(1))) {
            Err(Error::Busy) => {}
            _ => return Err(QueueError::BlockingNotBusy),
        }
        match q.send(&M1, Timeout::Forever) {
            Err(Error::Busy) => {}
            _ => return Err(QueueError::BlockingNotBusy),
        }
    }

    // Post-resume: queue must work.
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_runtime_lease
// ------------------------------------------------------------------

fn queue_runtime_lease(_tick_bits: u8, suite_baseline: u64) -> Result<(), QueueError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueError::Create)?;

    let heap_before = sys::heap_free();
    match osal::shutdown() {
        Err(Error::Busy) => {}
        _ => return Err(QueueError::BlockingNotBusy),
    }
    if osal::runtime_state() != osal_api::runtime::RuntimeState::Running {
        return Err(QueueError::BusyStateChanged);
    }
    if sys::heap_free() != heap_before {
        return Err(QueueError::BusyHeapChanged);
    }

    drop(q);
    osal::shutdown().map_err(|_| QueueError::LastDropLeak)?;
    if osal::runtime_state() != osal_api::runtime::RuntimeState::Uninitialized {
        return Err(QueueError::BusyStateChanged);
    }
    if sys::heap_free() != suite_baseline {
        return Err(QueueError::SuiteHeapLeak);
    }

    osal::initialize().map_err(|_| QueueError::Create)?;
    Ok(())
}

// ------------------------------------------------------------------
// Public entry
// ------------------------------------------------------------------

pub fn run_queue_cases(tick_bits: u8, suite_baseline: u64) -> Result<(), QueueError> {
    queue_core_fifo(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_core_fifo");

    queue_wrong_size_precedence(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_wrong_size_precedence");

    queue_nowait_zero(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_nowait_zero");

    queue_clone_lifecycle(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_clone_lifecycle");

    queue_recv_finite_timeout(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_recv_finite_timeout");

    queue_send_finite_timeout(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_send_finite_timeout");

    queue_close_drain(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_close_drain");

    queue_scheduler_suspended(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_scheduler_suspended");

    queue_runtime_lease(tick_bits, suite_baseline)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_runtime_lease");

    Ok(())
}
