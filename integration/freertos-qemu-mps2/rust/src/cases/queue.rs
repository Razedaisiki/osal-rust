//! Queue real-kernel contracts (P7G Step 4C-1).
//!
//! Validates the OSAL Queue on real FreeRTOS against the Behavior
//! Contract: FIFO, error precedence, NoWait / After(ZERO), finite
//! timeout (send+recv, controller-side), close-drain, scheduler
//! suspended, clone / last-drop, and RuntimeLease.
//!
//! Blocking wake, Forever, multi-waiter wake-one, close-broadcast,
//! timeout/wake race, and stress are deferred to Step 4C-2
//! (native Queue helper creation fails in the aggregate object suite
//! due to FreeRTOS heap_4 fragmentation; needs isolation or heap_5).

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;
use osal_backend_freertos_sys as sys;

use crate::harness;

// ------------------------------------------------------------------
// Payload constants — fixed 4-byte messages.
// ------------------------------------------------------------------
const M0: [u8; 4] = [0x10, 0, 0, 0];
const M1: [u8; 4] = [0x21, 0, 0, 1];
const M2: [u8; 4] = [0x32, 0, 0, 2];
// M3 reserved for future multi-waiter / blocking tests (Step 4C-2).

fn payload_eq(a: &[u8], b: &[u8; 4]) -> bool {
    a.len() == 4 && a[0] == b[0] && a[1] == b[1] && a[2] == b[2] && a[3] == b[3]
}

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------
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

    TimeoutNotReturned = 321,
    TimeoutTooEarly = 322,
    StateChangedOnTimeout = 325,

    CloseNotReturned = 360,
    CloseDrainMismatch = 361,

    BlockingNotBusy = 380,
    BusyStateChanged = 381,
    BusyHeapChanged = 382,
    LastDropLeak = 383,
    SuiteHeapLeak = 384,
}

// (SendOperation, RecvOperation, QueueSendContext, QueueRecvContext,
//  helper entries, and spawn wrappers deferred to Step 4C-2.)

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

        // --- Full queue: send direction ---
        match q.send(&M1, Timeout::NoWait) {
            Err(Error::QueueFull) => {}
            _ => return Err(QueueError::NoWaitMapping),
        }
        match q.send(&M1, Timeout::After(core::time::Duration::ZERO)) {
            Err(Error::Timeout) => {}
            _ => return Err(QueueError::AfterZeroMapping),
        }
        match q.send(&M1, Timeout::After(core::time::Duration::from_millis(1))) {
            Err(Error::Busy) => {}
            _ => return Err(QueueError::BlockingNotBusy),
        }
        match q.send(&M1, Timeout::Forever) {
            Err(Error::Busy) => {}
            _ => return Err(QueueError::BlockingNotBusy),
        }

        // --- Empty queue: recv direction (drain first) ---
        let mut buf = [0u8; 4];
        q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
        // Queue is now empty.

        match q.recv(&mut buf, Timeout::NoWait) {
            Err(Error::QueueEmpty) => {}
            _ => return Err(QueueError::NoWaitMapping),
        }
        match q.recv(&mut buf, Timeout::After(core::time::Duration::ZERO)) {
            Err(Error::Timeout) => {}
            _ => return Err(QueueError::AfterZeroMapping),
        }
        match q.recv(&mut buf, Timeout::After(core::time::Duration::from_millis(1))) {
            Err(Error::Busy) => {}
            _ => return Err(QueueError::BlockingNotBusy),
        }
        match q.recv(&mut buf, Timeout::Forever) {
            Err(Error::Busy) => {}
            _ => return Err(QueueError::BlockingNotBusy),
        }
    }

    // Post-resume: queue must work (refill and drain).
    q.send(&M1, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).map_err(|_| QueueError::FifoMismatch)?;

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
