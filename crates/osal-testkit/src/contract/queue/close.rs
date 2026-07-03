//! Close-drain semantics contract tests.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;
use crate::factory::QueueFactory;

/// After close, send returns QueueClosed.
pub fn send_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.close().unwrap();
    let result = q.send(&[1u8, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

/// After close, recv on empty returns QueueClosed.
pub fn recv_empty_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.close().unwrap();
    let mut buf = [0u8; 2];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

/// After close, buffered messages can still be drained.
pub fn recv_drains_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.send(&[1u8, 2], Timeout::NoWait).unwrap();
    q.send(&[3u8, 4], Timeout::NoWait).unwrap();
    q.close().unwrap();

    let mut buf = [0u8; 2];
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [1, 2]);
    q.recv(&mut buf, Timeout::NoWait).unwrap();
    assert_eq!(buf, [3, 4]);
    // Now closed and empty.
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::QueueClosed)));
}

/// Close is idempotent — calling twice must not panic or error.
pub fn close_idempotent<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.close().unwrap();
    q.close().unwrap();
}

/// After close, metadata queries still work.
pub fn metadata_after_close<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.send(&[1u8, 2], Timeout::NoWait).unwrap();
    q.close().unwrap();
    // capacity and msg_size never fail; len reflects remaining messages.
    assert_eq!(q.capacity(), 4);
    assert_eq!(q.msg_size(), 2);
    assert_eq!(q.len().unwrap(), 1);
    assert!(!q.is_empty().unwrap());
    assert!(!q.is_full().unwrap());
}

/// Run all close-drain tests.
pub fn run<F: QueueFactory>(factory: &F) {
    send_after_close::<F>(factory);
    recv_empty_after_close::<F>(factory);
    recv_drains_after_close::<F>(factory);
    close_idempotent::<F>(factory);
    metadata_after_close::<F>(factory);
}
