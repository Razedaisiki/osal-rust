//! Error precedence and message size contract tests.
//!
//! Verifies the precedence order: InvalidMessageSize > QueueClosed.

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;
use crate::factory::QueueFactory;

/// Wrong send size returns InvalidMessageSize.
pub fn send_wrong_size<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 4).unwrap();
    let result = q.send(&[1u8, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::InvalidMessageSize)));
}

/// Wrong recv buffer size returns InvalidMessageSize.
pub fn recv_wrong_size<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    q.send(&[1u8, 2], Timeout::NoWait).unwrap();
    let mut buf = [0u8; 4];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::InvalidMessageSize)));
}

/// Closed queue + wrong send size → InvalidMessageSize (not QueueClosed).
/// Parameter validation takes priority over object state.
pub fn closed_queue_wrong_send_size<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 4).unwrap();
    q.close().unwrap();
    let result = q.send(&[1u8, 2], Timeout::NoWait);
    assert!(matches!(result, Err(Error::InvalidMessageSize)));
}

/// Closed queue + wrong recv buffer → InvalidMessageSize (not QueueClosed).
pub fn closed_queue_wrong_recv_size<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 4).unwrap();
    q.close().unwrap();
    let mut buf = [0u8; 2];
    let result = q.recv(&mut buf, Timeout::NoWait);
    assert!(matches!(result, Err(Error::InvalidMessageSize)));
}

/// Run all error precedence tests.
pub fn run<F: QueueFactory>(factory: &F) {
    send_wrong_size::<F>(factory);
    recv_wrong_size::<F>(factory);
    closed_queue_wrong_send_size::<F>(factory);
    closed_queue_wrong_recv_size::<F>(factory);
}
