//! Contract tests for the [`Queue`] trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#11-queue-contract`.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;

use crate::factory::QueueFactory;

/// Create with valid parameters.
pub fn create<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(8, 4).unwrap();
    assert_eq!(q.capacity(), 8);
    assert_eq!(q.msg_size(), 4);
    assert_eq!(q.len(), 0);
    assert!(q.is_empty());
    assert!(!q.is_full());
}

/// Reject zero capacity.
pub fn reject_zero_capacity<F: QueueFactory>(factory: &F) {
    let result = factory.create_queue(0, 4);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Reject zero message size.
pub fn reject_zero_msg_size<F: QueueFactory>(factory: &F) {
    let result = factory.create_queue(8, 0);
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Send and receive a single message; bytes preserved.
pub fn send_recv_roundtrip<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 4).unwrap();
    let data = [1u8, 2, 3, 4];
    q.send(&data, Timeout::NoWait).unwrap();

    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, data);
}

/// Messages are received in FIFO order.
pub fn fifo_order<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.send(&[1u8, 2], Timeout::NoWait).unwrap();
    q.send(&[3u8, 4], Timeout::NoWait).unwrap();

    let mut buf = [0u8; 2];
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [1, 2]);
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [3, 4]);
}

/// Non-blocking send on full queue returns QueueFull.
pub fn send_full_no_wait<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(1, 1).unwrap();
    q.send(&[42], Timeout::NoWait).unwrap();
    let result = q.send(&[99], Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueFull)));
}

/// Non-blocking recv on empty queue returns QueueEmpty.
pub fn recv_empty_no_wait<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    let mut buf = [0u8; 2];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueEmpty)));
}

/// Send with wrong message size returns InvalidMessageSize.
pub fn send_wrong_size<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 4).unwrap();
    let result = q.send(&[1u8, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::InvalidMessageSize)));
}

/// Recv with wrong buffer size returns InvalidMessageSize.
pub fn recv_wrong_size<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.send(&[1u8, 2], Timeout::NoWait).unwrap();
    let mut buf = [0u8; 4];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::InvalidMessageSize)));
}

/// After close, send returns QueueClosed.
pub fn send_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.close();
    let result = q.send(&[1u8, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

/// After close, recv on empty returns QueueClosed.
pub fn recv_empty_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.close();
    let mut buf = [0u8; 2];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

/// After close, recv drains remaining messages before returning
/// QueueClosed.
pub fn recv_drains_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.send(&[1u8, 2], Timeout::NoWait).unwrap();
    q.send(&[3u8, 4], Timeout::NoWait).unwrap();
    q.close();

    let mut buf = [0u8; 2];
    // Drain first message.
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [1, 2]);
    // Drain second message.
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [3, 4]);
    // Now closed and empty.
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

/// Close is idempotent.
pub fn close_idempotent<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.close();
    q.close(); // Must not panic or double-free.
}

/// ISR send succeeds when not full.
pub fn isr_send<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.isr_send(&[1u8, 2]).unwrap();
    assert_eq!(q.len(), 1);
}

/// ISR recv succeeds when not empty.
pub fn isr_recv<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.isr_send(&[1u8, 2]).unwrap();
    let mut buf = [0u8; 2];
    q.isr_recv(&mut buf).unwrap();
    assert_eq!(buf, [1, 2]);
}

/// Recv timeout on empty queue.
pub fn recv_timeout<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    let mut buf = [0u8; 2];
    let result = q.recv(
        &mut buf,
        Timeout::After(core::time::Duration::from_millis(10)),
    );
    assert!(matches!(result, Err(Error::Timeout)));
}

// ---------------------------------------------------------------------------
// Aggregator
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Immediate-behavior tests (no blocking, no close, no ISR).
pub fn run_immediate_contracts<F: QueueFactory>(factory: &F) {
    create::<F>(factory);
    reject_zero_capacity::<F>(factory);
    reject_zero_msg_size::<F>(factory);
    send_recv_roundtrip::<F>(factory);
    fifo_order::<F>(factory);
    send_full_no_wait::<F>(factory);
    recv_empty_no_wait::<F>(factory);
    send_wrong_size::<F>(factory);
    recv_wrong_size::<F>(factory);
}

/// Lifetime / close-drain tests.
pub fn run_lifetime_contracts<F: QueueFactory>(factory: &F) {
    send_after_close::<F>(factory);
    recv_empty_after_close::<F>(factory);
    recv_drains_after_close::<F>(factory);
    close_idempotent::<F>(factory);
}

/// ISR-variant tests.
pub fn run_isr_contracts<F: QueueFactory>(factory: &F) {
    isr_send::<F>(factory);
    isr_recv::<F>(factory);
}

/// Wait / timeout tests (requires [`ClockFactory`]).
pub fn run_wait_contracts<F: QueueFactory>(factory: &F) {
    recv_timeout::<F>(factory);
}

/// All current contract tests.
pub fn run_all<F: QueueFactory>(factory: &F) {
    run_immediate_contracts(factory);
    run_lifetime_contracts(factory);
    run_isr_contracts(factory);
    run_wait_contracts(factory);
}
