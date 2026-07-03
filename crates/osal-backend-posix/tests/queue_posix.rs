//! POSIX QueueBlockingContract — tests requiring real concurrent blocking.
//!
//! These use `std::thread` and are only meaningful on the POSIX backend.
//! The Mock backend does not run these; its blocking scheduler is deferred.
//!
//! Run via:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit --test queue_posix
//! ```

use std::thread;
use std::time::Duration;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue as _;

use osal_backend_posix::queue::PosixQueue;

// ---------------------------------------------------------------------------
// Blocking send/recv — Forever
// ---------------------------------------------------------------------------

/// recv(Forever) is woken by a send from another thread.
#[test]
fn recv_forever_woken_by_send() {
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

/// send(Forever) is woken by a recv from another thread.
#[test]
fn send_forever_woken_by_recv() {
    let q = PosixQueue::new(1, 4).unwrap();
    q.send(&[1, 2, 3, 4], Timeout::NoWait).unwrap();

    let q2 = q.clone();
    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        let mut buf = [0u8; 4];
        q2.recv(&mut buf, Timeout::NoWait).unwrap();
    });

    q.send(&[5, 6, 7, 8], Timeout::Forever).unwrap();
    handle.join().unwrap();
}

// ---------------------------------------------------------------------------
// Timeout — After
// ---------------------------------------------------------------------------

/// recv(After) returns Timeout when no message arrives within the duration.
#[test]
fn recv_after_returns_timeout() {
    let q = PosixQueue::new(4, 4).unwrap();
    let mut buf = [0u8; 4];
    assert_eq!(
        q.recv(&mut buf, Timeout::After(Duration::from_millis(1))).unwrap_err(),
        Error::Timeout
    );
}

/// send(After) returns Timeout when the queue stays full.
#[test]
fn send_after_returns_timeout_when_full() {
    let q = PosixQueue::new(1, 4).unwrap();
    q.send(&[1, 2, 3, 4], Timeout::NoWait).unwrap();
    assert_eq!(
        q.send(&[5, 6, 7, 8], Timeout::After(Duration::from_millis(1))).unwrap_err(),
        Error::Timeout
    );
}

// ---------------------------------------------------------------------------
// Close wakes blocked operations
// ---------------------------------------------------------------------------

/// close() wakes a thread blocked on recv(Forever) with QueueClosed.
#[test]
fn close_wakes_blocked_recv() {
    let q = PosixQueue::new(4, 4).unwrap();
    let q2 = q.clone();

    let handle = thread::spawn(move || {
        thread::sleep(Duration::from_millis(10));
        let _ = q2.close();
    });

    let mut buf = [0u8; 4];
    assert_eq!(
        q.recv(&mut buf, Timeout::Forever).unwrap_err(),
        Error::QueueClosed
    );
    handle.join().unwrap();
}

/// close() wakes a thread blocked on send(Forever) with QueueClosed.
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

    assert_eq!(
        q.send(&[5, 6, 7, 8], Timeout::Forever).unwrap_err(),
        Error::QueueClosed
    );
    handle.join().unwrap();
}
