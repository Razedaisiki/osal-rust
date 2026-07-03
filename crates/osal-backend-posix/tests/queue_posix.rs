//! POSIX-specific queue tests — blocking, wakeup, and race behavior.
//!
//! These use `std::thread` and are only meaningful on the POSIX backend.
//! They are NOT part of the no_std osal-testkit contract suite.

use std::thread;
use std::time::Duration;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;

use osal_backend_posix::queue::PosixQueue;

// ---------------------------------------------------------------------------
// Blocking send/recv
// ---------------------------------------------------------------------------

#[test]
fn send_forever_succeeds_when_receiver_makes_space() {
    let q = PosixQueue::new(1, 4).unwrap();
    q.send(&[1, 2, 3, 4], Timeout::NoWait).unwrap();

    let q2 = q.clone();
    let handle = thread::spawn(move || {
        // Drain the queue after a short delay.
        thread::sleep(Duration::from_millis(10));
        let mut buf = [0u8; 4];
        q2.recv(&mut buf, Timeout::NoWait).unwrap();
    });

    // This blocks until the receiver drains.
    q.send(&[5, 6, 7, 8], Timeout::Forever).unwrap();
    handle.join().unwrap();
}

#[test]
fn recv_forever_succeeds_when_sender_provides_message() {
    let q = PosixQueue::new(1, 4).unwrap();

    let q2 = q.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        q2.send(&[1, 2, 3, 4], Timeout::NoWait).unwrap();
    });

    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::Forever).unwrap();
    assert_eq!(buf, [1, 2, 3, 4]);
    handle.join().unwrap();
}

// ---------------------------------------------------------------------------
// Close wakeup
// ---------------------------------------------------------------------------

#[test]
fn close_wakes_blocked_recv() {
    let q = PosixQueue::new(4, 4).unwrap();
    let q2 = q.clone();

    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        let _ = q2.close();
    });

    let mut buf = [0u8; 4];
    let result = q.recv(&mut buf, Timeout::Forever);
    assert!(matches!(result, Err(Error::QueueClosed)));
    handle.join().unwrap();
}

#[test]
fn close_wakes_blocked_send() {
    let q = PosixQueue::new(1, 4).unwrap();
    // Fill the queue.
    q.send(&[1, 2, 3, 4], Timeout::NoWait).unwrap();

    let q2 = q.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        let _ = q2.close();
    });

    let result = q.send(&[5, 6, 7, 8], Timeout::Forever);
    assert!(matches!(result, Err(Error::QueueClosed)));
    handle.join().unwrap();
}

// ---------------------------------------------------------------------------
// Timeout on full/empty
// ---------------------------------------------------------------------------

#[test]
fn timed_recv_returns_timeout() {
    let q = PosixQueue::new(4, 4).unwrap();
    let mut buf = [0u8; 4];
    let result = q.recv(&mut buf, Timeout::After(Duration::from_millis(1)));
    assert!(matches!(result, Err(Error::Timeout)));
}

#[test]
fn timed_send_returns_timeout_when_full() {
    let q = PosixQueue::new(1, 4).unwrap();
    q.send(&[1, 2, 3, 4], Timeout::NoWait).unwrap();
    let result = q.send(&[5, 6, 7, 8], Timeout::After(Duration::from_millis(1)));
    assert!(matches!(result, Err(Error::Timeout)));
}
