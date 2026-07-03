//! FIFO ordering and round-trip contract tests.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;
use crate::factory::QueueFactory;

/// Send and receive a single message; bytes are preserved.
pub fn send_recv_roundtrip<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 4).unwrap();
    q.send(&[1u8, 2, 3, 4], Timeout::NoWait).unwrap();
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [1, 2, 3, 4]);
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

/// Send on full queue returns QueueFull.
pub fn send_full_no_wait<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(1, 1).unwrap();
    q.send(&[42], Timeout::NoWait).unwrap();
    let result = q.send(&[99], Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueFull)));
}

/// Recv on empty queue returns QueueEmpty.
pub fn recv_empty_no_wait<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    let mut buf = [0u8; 2];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueEmpty)));
}

/// Run all FIFO tests.
pub fn run<F: QueueFactory>(factory: &F) {
    send_recv_roundtrip::<F>(factory);
    fifo_order::<F>(factory);
    send_full_no_wait::<F>(factory);
    recv_empty_no_wait::<F>(factory);
}
