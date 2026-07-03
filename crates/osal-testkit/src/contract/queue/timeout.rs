//! Timeout contract tests (After on full/empty).

use core::time::Duration;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;
use crate::factory::QueueFactory;

/// Send with After on full queue returns Timeout.
pub fn send_timeout_when_full<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(1, 1).unwrap();
    q.send(&[42], Timeout::NoWait).unwrap();
    let result = q.send(&[99], Timeout::After(Duration::from_millis(10)));
    assert!(matches!(result, Err(Error::Timeout)));
}

/// Recv with After on empty queue returns Timeout.
pub fn recv_timeout<F: QueueFactory>(factory: &F) {
    let q = factory.create_queue(4, 2).unwrap();
    let mut buf = [0u8; 2];
    let result = q.recv(&mut buf, Timeout::After(Duration::from_millis(10)));
    assert!(matches!(result, Err(Error::Timeout)));
}

/// Run all timeout tests.
pub fn run<F: QueueFactory>(factory: &F) {
    send_timeout_when_full::<F>(factory);
    recv_timeout::<F>(factory);
}
