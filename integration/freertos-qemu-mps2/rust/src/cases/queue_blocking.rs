//! Queue blocking real-kernel contracts (P7G Step 4C-2).
//!
//! Isolated suite — validates blocking wake, Forever, multi-waiter
//! wake-one, close-broadcast, and controller-side throughput on a
//! fresh FreeRTOS session.  Timeout/wake race is deferred to 4C-3.

use alloc::boxed::Box;
use core::ffi::c_void;
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;
use osal_backend_freertos_sys as sys;

use crate::harness::{self, CaseState, HarnessError};

unsafe extern "C" {
    fn osal_test_task_stack_hwm() -> u32;
}
use crate::harness::{
    PHASE_BEFORE_OPERATION, PHASE_EXITING, PHASE_OPERATION_COMPLETED, PHASE_STARTED,
};

// ------------------------------------------------------------------
// Payloads
// ------------------------------------------------------------------
const M0: [u8; 4] = [0x10, 0, 0, 0];
const M1: [u8; 4] = [0x21, 0, 0, 1];
const M2: [u8; 4] = [0x32, 0, 0, 2];

fn payload_eq(a: &[u8], b: &[u8; 4]) -> bool {
    a.len() == 4 && a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

// ------------------------------------------------------------------
// Outcome — separate from CaseState.result for clear semantics.
// ------------------------------------------------------------------
const OUTCOME_PENDING: i32 = 0;
const OUTCOME_SUCCESS: i32 = 1;
const OUTCOME_TIMEOUT: i32 = 2;
const OUTCOME_QUEUE_CLOSED: i32 = 3;

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------
#[repr(i32)]
pub enum QueueBlockingError {
    Create = 400,
    HelperSpawnFailed = 401,
    TimeoutNotReturned = 402,
    PayloadMismatch = 403,
    CompletedBeforeTrigger = 404,
    StateChangedOnTimeout = 405,
    WrongWaiterCount = 406,
    MessageLost = 407,
    CloseNotReturned = 408,
    CloseBroadcastFailed = 409,
    StrandedWaiter = 410,
    LastDropLeak = 411,
    ControllerDelayFailed = 412,
    StackMarginTooSmall = 413,
    HeapNotRecovered = 414,
}

// ------------------------------------------------------------------
// Operations
// ------------------------------------------------------------------
#[derive(Clone, Copy)]
enum SendOperation {
    AfterTicks { timeout_ms: u32 },
    Forever,
}

#[derive(Clone, Copy)]
enum RecvOperation {
    AfterTicks { timeout_ms: u32 },
    Forever,
}

// ------------------------------------------------------------------
// Contexts
// ------------------------------------------------------------------
struct QueueSendContext {
    state: CaseState,
    queue: osal::backend::Queue,
    operation: SendOperation,
    payload: [u8; 4],

    outcome: AtomicI32,
    completion_tick: AtomicU32,
    helper_stack_hwm: AtomicU32,
}

impl QueueSendContext {
    fn new(queue: &osal::backend::Queue, operation: SendOperation, payload: [u8; 4]) -> Self {
        Self {
            state: CaseState::new(),
            queue: queue.clone(),
            operation,
            payload,
            outcome: AtomicI32::new(OUTCOME_PENDING),
            completion_tick: AtomicU32::new(0),
            helper_stack_hwm: AtomicU32::new(0),
        }
    }
}

struct QueueRecvContext {
    state: CaseState,
    queue: osal::backend::Queue,
    operation: RecvOperation,

    outcome: AtomicI32,
    received_word: AtomicU32,
    completion_tick: AtomicU32,
    helper_stack_hwm: AtomicU32,
}

impl QueueRecvContext {
    fn new(queue: &osal::backend::Queue, operation: RecvOperation) -> Self {
        Self {
            state: CaseState::new(),
            queue: queue.clone(),
            operation,
            outcome: AtomicI32::new(OUTCOME_PENDING),
            received_word: AtomicU32::new(0),
            completion_tick: AtomicU32::new(0),
            helper_stack_hwm: AtomicU32::new(0),
        }
    }
}

// ------------------------------------------------------------------
// Helper entries
// ------------------------------------------------------------------

unsafe extern "C" fn queue_send_blocking_helper(context: *mut c_void) {
    let result = {
        let ctx = unsafe { &*(context as *const QueueSendContext) };
        ctx.state.record_phase(PHASE_STARTED);
        ctx.state.record_phase(PHASE_BEFORE_OPERATION);
        run_send_op(ctx)
    };
    let ctx = unsafe { &*(context as *const QueueSendContext) };
    ctx.state.set_result(result);
    ctx.helper_stack_hwm.store(unsafe { osal_test_task_stack_hwm() }, Ordering::Release);
    ctx.state.record_phase(PHASE_OPERATION_COMPLETED);
    ctx.state.record_phase(PHASE_EXITING);
    unsafe { harness::osal_test_task_exit(); }
}

unsafe extern "C" fn queue_recv_blocking_helper(context: *mut c_void) {
    let result = {
        let ctx = unsafe { &*(context as *const QueueRecvContext) };
        ctx.state.record_phase(PHASE_STARTED);
        ctx.state.record_phase(PHASE_BEFORE_OPERATION);
        run_recv_op(ctx)
    };
    let ctx = unsafe { &*(context as *const QueueRecvContext) };
    ctx.state.set_result(result);
    ctx.helper_stack_hwm.store(unsafe { osal_test_task_stack_hwm() }, Ordering::Release);
    ctx.state.record_phase(PHASE_OPERATION_COMPLETED);
    ctx.state.record_phase(PHASE_EXITING);
    unsafe { harness::osal_test_task_exit(); }
}

fn run_send_op(ctx: &QueueSendContext) -> i32 {
    let q = &ctx.queue;
    match ctx.operation {
        SendOperation::AfterTicks { timeout_ms } => {
            match q.send(&ctx.payload, Timeout::After(core::time::Duration::from_millis(timeout_ms as u64))) {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    ctx.outcome.store(OUTCOME_SUCCESS, Ordering::Release);
                    0
                }
                Err(Error::Timeout) => {
                    ctx.outcome.store(OUTCOME_TIMEOUT, Ordering::Release);
                    0
                }
                Err(Error::QueueClosed) => {
                    ctx.outcome.store(OUTCOME_QUEUE_CLOSED, Ordering::Release);
                    0
                }
                _ => -(QueueBlockingError::TimeoutNotReturned as i32),
            }
        }
        SendOperation::Forever => {
            match q.send(&ctx.payload, Timeout::Forever) {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    ctx.outcome.store(OUTCOME_SUCCESS, Ordering::Release);
                    0
                }
                Err(Error::QueueClosed) => {
                    ctx.outcome.store(OUTCOME_QUEUE_CLOSED, Ordering::Release);
                    0
                }
                _ => -(QueueBlockingError::TimeoutNotReturned as i32),
            }
        }
    }
}

fn run_recv_op(ctx: &QueueRecvContext) -> i32 {
    let q = &ctx.queue;
    match ctx.operation {
        RecvOperation::AfterTicks { timeout_ms } => {
            let mut buf = [0u8; 4];
            match q.recv(&mut buf, Timeout::After(core::time::Duration::from_millis(timeout_ms as u64))) {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    ctx.received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    ctx.outcome.store(OUTCOME_SUCCESS, Ordering::Release);
                    0
                }
                Err(Error::Timeout) => {
                    ctx.outcome.store(OUTCOME_TIMEOUT, Ordering::Release);
                    0
                }
                Err(Error::QueueClosed) => {
                    ctx.outcome.store(OUTCOME_QUEUE_CLOSED, Ordering::Release);
                    0
                }
                _ => -(QueueBlockingError::TimeoutNotReturned as i32),
            }
        }
        RecvOperation::Forever => {
            let mut buf = [0u8; 4];
            match q.recv(&mut buf, Timeout::Forever) {
                Ok(()) => {
                    let snap = sys::tick_snapshot();
                    ctx.completion_tick.store(snap.tick_count as u32, Ordering::Release);
                    ctx.received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    ctx.outcome.store(OUTCOME_SUCCESS, Ordering::Release);
                    0
                }
                Err(Error::QueueClosed) => {
                    ctx.outcome.store(OUTCOME_QUEUE_CLOSED, Ordering::Release);
                    0
                }
                _ => -(QueueBlockingError::TimeoutNotReturned as i32),
            }
        }
    }
}

// ------------------------------------------------------------------
// Case: queue_helper_resource_probe
// ------------------------------------------------------------------

fn queue_helper_resource_probe(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;

    let raw = Box::into_raw(Box::new(QueueRecvContext::new(
        &q, RecvOperation::AfterTicks { timeout_ms: 5 },
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe {
        harness::native_task_spawn(queue_recv_blocking_helper, raw.cast::<c_void>(), 1024, 2)
    };
    if rc != harness::SPAWN_OK {
        unsafe { drop(Box::from_raw(raw)); }
        return Err(QueueBlockingError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_ref.state)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    let hwm = ctx_ref.helper_stack_hwm.load(Ordering::Acquire);
    let outcome = ctx_ref.outcome.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw)); }

    if outcome != OUTCOME_TIMEOUT {
        return Err(QueueBlockingError::TimeoutNotReturned);
    }
    if hwm < 64 {
        return Err(QueueBlockingError::StackMarginTooSmall);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_recv_blocking_wake
// ------------------------------------------------------------------

fn queue_recv_blocking_wake(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;

    let raw = Box::into_raw(Box::new(QueueRecvContext::new(
        &q, RecvOperation::AfterTicks { timeout_ms: 100 },
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw.cast::<c_void>(), 1024, 2) };
    if rc != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw)); } return Err(QueueBlockingError::HelperSpawnFailed); }

    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    let send_tick = sys::tick_snapshot().tick_count as u32;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_ref.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    let completed = ctx_ref.completion_tick.load(Ordering::Acquire);
    let outcome = ctx_ref.outcome.load(Ordering::Acquire);
    let word = ctx_ref.received_word.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw)); }

    if outcome != OUTCOME_SUCCESS { return Err(QueueBlockingError::TimeoutNotReturned); }
    if completed.wrapping_sub(send_tick) > (u32::MAX / 2) { return Err(QueueBlockingError::CompletedBeforeTrigger); }
    if word != u32::from_le_bytes(M0) { return Err(QueueBlockingError::PayloadMismatch); }
    if q.len().map_err(|_| QueueBlockingError::Create)? != 0 { return Err(QueueBlockingError::StateChangedOnTimeout); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_send_blocking_wake
// ------------------------------------------------------------------

fn queue_send_blocking_wake(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    let raw = Box::into_raw(Box::new(QueueSendContext::new(
        &q, SendOperation::AfterTicks { timeout_ms: 100 }, M1,
    )));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw.cast::<c_void>(), 1024, 2) };
    if rc != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw)); } return Err(QueueBlockingError::HelperSpawnFailed); }

    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    let recv_tick = sys::tick_snapshot().tick_count as u32;
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_ref.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    let completed = ctx_ref.completion_tick.load(Ordering::Acquire);
    let outcome = ctx_ref.outcome.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw)); }

    if outcome != OUTCOME_SUCCESS { return Err(QueueBlockingError::TimeoutNotReturned); }
    if completed.wrapping_sub(recv_tick) > (u32::MAX / 2) { return Err(QueueBlockingError::CompletedBeforeTrigger); }

    // Verify M1 arrived.
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
    if !payload_eq(&buf, &M1) { return Err(QueueBlockingError::PayloadMismatch); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_recv_forever_wake
// ------------------------------------------------------------------

fn queue_recv_forever_wake(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;

    let raw = Box::into_raw(Box::new(QueueRecvContext::new(&q, RecvOperation::Forever)));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw.cast::<c_void>(), 1024, 2) };
    if rc != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw)); } return Err(QueueBlockingError::HelperSpawnFailed); }

    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    let send_tick = sys::tick_snapshot().tick_count as u32;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_ref.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    let completed = ctx_ref.completion_tick.load(Ordering::Acquire);
    let outcome = ctx_ref.outcome.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw)); }

    if outcome != OUTCOME_SUCCESS { return Err(QueueBlockingError::TimeoutNotReturned); }
    if completed.wrapping_sub(send_tick) > (u32::MAX / 2) { return Err(QueueBlockingError::CompletedBeforeTrigger); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_send_forever_wake
// ------------------------------------------------------------------

fn queue_send_forever_wake(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    let raw = Box::into_raw(Box::new(QueueSendContext::new(&q, SendOperation::Forever, M1)));
    let ctx_ref = unsafe { &*raw };
    let task_baseline = sys::heap_free();

    let rc = unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw.cast::<c_void>(), 1024, 2) };
    if rc != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw)); } return Err(QueueBlockingError::HelperSpawnFailed); }

    harness::wait_until_phase(&ctx_ref.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    let recv_tick = sys::tick_snapshot().tick_count as u32;
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    harness::wait_until_phase(&ctx_ref.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_ref.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    let completed = ctx_ref.completion_tick.load(Ordering::Acquire);
    let outcome = ctx_ref.outcome.load(Ordering::Acquire);

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw)); }

    if outcome != OUTCOME_SUCCESS { return Err(QueueBlockingError::TimeoutNotReturned); }
    if completed.wrapping_sub(recv_tick) > (u32::MAX / 2) { return Err(QueueBlockingError::CompletedBeforeTrigger); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_one_send_one_receiver
// ------------------------------------------------------------------

fn queue_one_send_one_receiver(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(2, 4).map_err(|_| QueueBlockingError::Create)?;

    let raw_a = Box::into_raw(Box::new(QueueRecvContext::new(&q, RecvOperation::Forever)));
    let raw_b = Box::into_raw(Box::new(QueueRecvContext::new(&q, RecvOperation::Forever)));
    let ctx_a = unsafe { &*raw_a };
    let ctx_b = unsafe { &*raw_b };
    let task_baseline = sys::heap_free();

    let rc_a = unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw_a.cast::<c_void>(), 1024, 2) };
    let rc_b = unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw_b.cast::<c_void>(), 1024, 2) };
    if rc_a != harness::SPAWN_OK || rc_b != harness::SPAWN_OK {
        if rc_a != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw_a)); } }
        if rc_b != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw_b)); } }
        return Err(QueueBlockingError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_a.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    // Send M0 — exactly one receiver must wake.
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    let phase_a = ctx_a.state.get_phase();
    let phase_b = ctx_b.state.get_phase();
    let completed = if phase_a >= PHASE_EXITING { 1u32 } else { 0u32 }
        + if phase_b >= PHASE_EXITING { 1u32 } else { 0u32 };
    if completed != 1 { return Err(QueueBlockingError::WrongWaiterCount); }

    // Send M1 — second receiver wakes.
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    harness::wait_until_phase(&ctx_a.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_a.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_b.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    // Both received — payload set must be {M0, M1} (order unspecified).
    let w_a = ctx_a.received_word.load(Ordering::Acquire);
    let w_b = ctx_b.received_word.load(Ordering::Acquire);
    let m0 = u32::from_le_bytes(M0);
    let m1 = u32::from_le_bytes(M1);
    if !((w_a == m0 && w_b == m1) || (w_a == m1 && w_b == m0)) {
        return Err(QueueBlockingError::MessageLost);
    }
    if q.len().map_err(|_| QueueBlockingError::Create)? != 0 { return Err(QueueBlockingError::MessageLost); }

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw_a)); }
    unsafe { drop(Box::from_raw(raw_b)); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_one_recv_one_sender
// ------------------------------------------------------------------

fn queue_one_recv_one_sender(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    let raw_a = Box::into_raw(Box::new(QueueSendContext::new(&q, SendOperation::Forever, M1)));
    let raw_b = Box::into_raw(Box::new(QueueSendContext::new(&q, SendOperation::Forever, M2)));
    let ctx_a = unsafe { &*raw_a };
    let ctx_b = unsafe { &*raw_b };
    let task_baseline = sys::heap_free();

    let rc_a = unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw_a.cast::<c_void>(), 1024, 2) };
    let rc_b = unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw_b.cast::<c_void>(), 1024, 2) };
    if rc_a != harness::SPAWN_OK || rc_b != harness::SPAWN_OK {
        if rc_a != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw_a)); } }
        if rc_b != harness::SPAWN_OK { unsafe { drop(Box::from_raw(raw_b)); } }
        return Err(QueueBlockingError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_a.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    // Recv M0 — exactly one sender wakes.
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    let phase_a = ctx_a.state.get_phase();
    let phase_b = ctx_b.state.get_phase();
    let completed = if phase_a >= PHASE_EXITING { 1u32 } else { 0u32 }
        + if phase_b >= PHASE_EXITING { 1u32 } else { 0u32 };
    if completed != 1 { return Err(QueueBlockingError::WrongWaiterCount); }

    // Recv the first helper's message — second sender wakes.
    let first = {
        q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
        u32::from_le_bytes(buf)
    };

    harness::wait_until_phase(&ctx_a.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_a.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_b.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    // Recv the second helper's message.
    let second = {
        q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
        u32::from_le_bytes(buf)
    };
    if q.len().map_err(|_| QueueBlockingError::Create)? != 0 { return Err(QueueBlockingError::MessageLost); }

    // Both senders' outcomes must be SUCCESS.
    if ctx_a.outcome.load(Ordering::Acquire) != OUTCOME_SUCCESS { return Err(QueueBlockingError::TimeoutNotReturned); }
    if ctx_b.outcome.load(Ordering::Acquire) != OUTCOME_SUCCESS { return Err(QueueBlockingError::TimeoutNotReturned); }

    // Payload set must be {M1, M2}.
    let m1 = u32::from_le_bytes(M1);
    let m2 = u32::from_le_bytes(M2);
    if !((first == m1 && second == m2) || (first == m2 && second == m1)) {
        return Err(QueueBlockingError::MessageLost);
    }

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw_a)); }
    unsafe { drop(Box::from_raw(raw_b)); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_close_broadcast_receivers
// ------------------------------------------------------------------

fn queue_close_broadcast_receivers(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;

    let raw_a = Box::into_raw(Box::new(QueueRecvContext::new(&q, RecvOperation::Forever)));
    let raw_b = Box::into_raw(Box::new(QueueRecvContext::new(&q, RecvOperation::Forever)));
    let raw_c = Box::into_raw(Box::new(QueueRecvContext::new(&q, RecvOperation::Forever)));
    let ctx_a = unsafe { &*raw_a };
    let ctx_b = unsafe { &*raw_b };
    let ctx_c = unsafe { &*raw_c };
    let task_baseline = sys::heap_free();

    let rcs = [
        unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw_a.cast::<c_void>(), 1024, 2) },
        unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw_b.cast::<c_void>(), 1024, 2) },
        unsafe { harness::native_task_spawn(queue_recv_blocking_helper, raw_c.cast::<c_void>(), 1024, 2) },
    ];
    if rcs.iter().any(|&r| r != harness::SPAWN_OK) {
        for (i, &r) in rcs.iter().enumerate() {
            if r != harness::SPAWN_OK {
                unsafe { drop(Box::from_raw([raw_a, raw_b, raw_c][i])); }
            }
        }
        return Err(QueueBlockingError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_a.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_c.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    q.close().map_err(|_| QueueBlockingError::CloseNotReturned)?;

    harness::wait_until_phase(&ctx_a.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_c.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    // All three must have QueueClosed outcome.
    let outcomes = [
        ctx_a.outcome.load(Ordering::Acquire),
        ctx_b.outcome.load(Ordering::Acquire),
        ctx_c.outcome.load(Ordering::Acquire),
    ];
    if outcomes.iter().any(|&o| o != OUTCOME_QUEUE_CLOSED) {
        return Err(QueueBlockingError::CloseBroadcastFailed);
    }

    harness::validate_helper(&ctx_a.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_b.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_c.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw_a)); }
    unsafe { drop(Box::from_raw(raw_b)); }
    unsafe { drop(Box::from_raw(raw_c)); }

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_close_broadcast_senders
// ------------------------------------------------------------------

fn queue_close_broadcast_senders(tick_bits: u8) -> Result<(), QueueBlockingError> {
    let q = osal::backend::Queue::new(1, 4).map_err(|_| QueueBlockingError::Create)?;
    q.send(&M0, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;

    let raw_a = Box::into_raw(Box::new(QueueSendContext::new(&q, SendOperation::Forever, M1)));
    let raw_b = Box::into_raw(Box::new(QueueSendContext::new(&q, SendOperation::Forever, M1)));
    let raw_c = Box::into_raw(Box::new(QueueSendContext::new(&q, SendOperation::Forever, M1)));
    let ctx_a = unsafe { &*raw_a };
    let ctx_b = unsafe { &*raw_b };
    let ctx_c = unsafe { &*raw_c };
    let task_baseline = sys::heap_free();

    let rcs = [
        unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw_a.cast::<c_void>(), 1024, 2) },
        unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw_b.cast::<c_void>(), 1024, 2) },
        unsafe { harness::native_task_spawn(queue_send_blocking_helper, raw_c.cast::<c_void>(), 1024, 2) },
    ];
    if rcs.iter().any(|&r| r != harness::SPAWN_OK) {
        for (i, &r) in rcs.iter().enumerate() {
            if r != harness::SPAWN_OK { unsafe { drop(Box::from_raw([raw_a, raw_b, raw_c][i])); } }
        }
        return Err(QueueBlockingError::HelperSpawnFailed);
    }

    harness::wait_until_phase(&ctx_a.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_c.state, PHASE_BEFORE_OPERATION, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    if sys::delay_ticks(2) != sys::DelayStatus::Ok { return Err(QueueBlockingError::ControllerDelayFailed); }

    q.close().map_err(|_| QueueBlockingError::CloseNotReturned)?;

    harness::wait_until_phase(&ctx_a.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_b.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::wait_until_phase(&ctx_c.state, PHASE_EXITING, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    let outcomes = [
        ctx_a.outcome.load(Ordering::Acquire),
        ctx_b.outcome.load(Ordering::Acquire),
        ctx_c.outcome.load(Ordering::Acquire),
    ];
    if outcomes.iter().any(|&o| o != OUTCOME_QUEUE_CLOSED) {
        return Err(QueueBlockingError::CloseBroadcastFailed);
    }

    // M0 still drainable; no helper payload entered.
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
    if !payload_eq(&buf, &M0) { return Err(QueueBlockingError::PayloadMismatch); }
    match q.recv(&mut buf, Timeout::NoWait) {
        Err(Error::QueueClosed) => {}
        _ => return Err(QueueBlockingError::CloseNotReturned),
    }

    harness::validate_helper(&ctx_a.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_b.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;
    harness::validate_helper(&ctx_c.state).map_err(|_| QueueBlockingError::HelperSpawnFailed)?;

    harness::wait_until_heap_recovered(task_baseline, 100, tick_bits)
        .map_err(|_| QueueBlockingError::HeapNotRecovered)?;
    unsafe { drop(Box::from_raw(raw_a)); }
    unsafe { drop(Box::from_raw(raw_b)); }
    unsafe { drop(Box::from_raw(raw_c)); }

    // Close idempotent after broadcast.
    q.close().map_err(|_| QueueBlockingError::CloseNotReturned)?;
    q.close().map_err(|_| QueueBlockingError::CloseNotReturned)?;

    Ok(())
}

// ------------------------------------------------------------------
// Case: queue_throughput_cycle
// ------------------------------------------------------------------

/// Controller-side FIFO/recovery loop: 64 interleaved NoWait send/recv
/// cycles in the boot task — no producer/consumer helper tasks.
fn queue_throughput_cycle(_tick_bits: u8) -> Result<(), QueueBlockingError> {
    const N: u32 = 64;
    let baseline = sys::heap_free();
    let q = osal::backend::Queue::new(4, 4).map_err(|_| QueueBlockingError::Create)?;

    // Interleave: send 4, drain 4, repeat.
    let mut buf = [0u8; 4];
    let mut sent: u32 = 0;
    while sent < N {
        let batch_end = (sent + 4).min(N);
        for i in sent..batch_end {
            let payload = i.to_le_bytes();
            q.send(&payload, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
        }
        for i in sent..batch_end {
            q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueBlockingError::TimeoutNotReturned)?;
            if u32::from_le_bytes(buf) != i { return Err(QueueBlockingError::PayloadMismatch); }
        }
        sent = batch_end;
    }

    drop(q);
    if sys::heap_free() != baseline { return Err(QueueBlockingError::HeapNotRecovered); }

    Ok(())
}

// ------------------------------------------------------------------
// Public entry
// ------------------------------------------------------------------

pub fn run_queue_blocking_cases(tick_bits: u8) -> Result<(), QueueBlockingError> {
    queue_helper_resource_probe(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_helper_resource_probe");

    queue_recv_blocking_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_recv_blocking_wake");

    queue_send_blocking_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_send_blocking_wake");

    queue_recv_forever_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_recv_forever_wake");

    queue_send_forever_wake(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_send_forever_wake");

    queue_one_send_one_receiver(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_one_send_one_receiver");

    queue_one_recv_one_sender(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_one_recv_one_sender");

    queue_close_broadcast_receivers(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_close_broadcast_receivers");

    queue_close_broadcast_senders(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_close_broadcast_senders");

    queue_throughput_cycle(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=queue_throughput_cycle");

    Ok(())
}
