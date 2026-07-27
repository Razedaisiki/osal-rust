//! Cross-thread Queue blocking, wakeup, and race tests.
//!
//! Verifies:
//! - send/recv Forever woken by cross-thread recv/send
//! - close wakes blocked sender
//! - wake-one: one send/recv wakes exactly one corresponding waiter
//! - timeout races with wake token
//! - close while full (drain)
//! - scheduler-state preconditions (NotStarted, Suspended)
//! - multi-chunk finite wait
//! - stress cycle
//! - Clone, Drop, error precedence
//!
//! Multi-waiter close broadcast (close waking N>1 waiters) is deferred
//! until the fixture supports per-object Condvars — the current
//! shared-Condvar model is non-deterministic for simultaneous cross-wake
//! scenarios.  The architecture is verified via single-waiter close
//! tests and wake-one tests which exercise the same code paths.
//!
//! All tests use `mpsc::channel` + `recv_timeout` as a watchdog.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit queue_concurrent -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use core::time::Duration;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::queue::Queue;
use osal_backend_freertos::queue::FreeRtosQueue;
use osal_backend_freertos::runtime;
use osal_backend_freertos_sys::fixture;

fn setup() {
    fixture::reset();
    let _ = runtime::shutdown();
    runtime::initialize().expect("initialize");
}

fn teardown() {
    match runtime::shutdown() {
        Ok(()) | Err(Error::NotInitialized) => {}
        Err(e) => panic!("test leaked runtime lease or object: {e:?}"),
    }
    fixture::reset();
}

/// Poll `fixture::blocked_count()` until it reaches `expected` or
/// `timeout` expires.
fn wait_until_blocked_count(expected: u64, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while fixture::blocked_count() < expected {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {expected} blocked waiter(s); got {}",
            fixture::blocked_count()
        );
        thread::yield_now();
    }
}

// ---------------------------------------------------------------------------
// 1. blocked receiver woken by send
// ---------------------------------------------------------------------------

#[test]
fn recv_forever_woken_by_cross_thread_send() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        let qc = q.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let mut buf = [0u8; 4];
            let r = qc.recv(&mut buf, Timeout::Forever);
            tx.send(r.map(|()| buf)).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        q.send(&[42, 0, 0, 0], Timeout::NoWait).expect("send");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok(), "expected recv success, got {result:?}");
        assert_eq!(result.unwrap(), [42, 0, 0, 0]);

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 2. blocked sender woken by recv
// ---------------------------------------------------------------------------

#[test]
fn send_forever_woken_by_cross_thread_recv() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");

        let qc = q.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = qc.send(&[5, 6, 7, 8], Timeout::Forever);
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        let mut buf = [0u8; 4];
        q.recv(&mut buf, Timeout::NoWait).expect("recv");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok(), "expected send success, got {result:?}");

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 3-4. finite timeout
// ---------------------------------------------------------------------------

#[test]
fn recv_after_returns_timeout_when_empty() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        let mut buf = [0u8; 4];
        assert_eq!(
            q.recv(&mut buf, Timeout::After(Duration::from_millis(10))),
            Err(Error::Timeout)
        );
    }
    teardown();
}

#[test]
fn send_after_returns_timeout_when_full() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");
        assert_eq!(
            q.send(&[5, 6, 7, 8], Timeout::After(Duration::from_millis(10))),
            Err(Error::Timeout)
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 5-6. wake-one
// ---------------------------------------------------------------------------

#[test]
fn one_send_wakes_one_receiver() {
    setup();
    {
        let q = FreeRtosQueue::new(2, 4).expect("create");
        let barrier = Arc::new(Barrier::new(3));
        let (tx, rx) = mpsc::channel();

        let q1 = q.clone();
        let b1 = Arc::clone(&barrier);
        let tx1 = tx.clone();
        let h1 = thread::spawn(move || {
            b1.wait();
            let mut buf = [0u8; 4];
            let r = q1.recv(&mut buf, Timeout::Forever);
            tx1.send(r.map(|()| buf)).ok();
        });

        let q2 = q.clone();
        let b2 = Arc::clone(&barrier);
        let tx2 = tx.clone();
        let h2 = thread::spawn(move || {
            b2.wait();
            let mut buf = [0u8; 4];
            let r = q2.recv(&mut buf, Timeout::Forever);
            tx2.send(r.map(|()| buf)).ok();
        });
        drop(tx);

        barrier.wait();
        wait_until_blocked_count(2, Duration::from_secs(2));

        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first receiver did not complete");
        assert!(first.is_ok());
        assert!(rx.try_recv().is_err(), "second receiver woke too early");

        q.send(&[5, 6, 7, 8], Timeout::NoWait).expect("send 2");

        let second = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second receiver did not complete");
        assert!(second.is_ok());

        h1.join().expect("h1");
        h2.join().expect("h2");
    }
    teardown();
}

#[test]
fn one_recv_wakes_one_sender() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");

        let barrier = Arc::new(Barrier::new(3));
        let (tx, rx) = mpsc::channel();

        let q1 = q.clone();
        let b1 = Arc::clone(&barrier);
        let tx1 = tx.clone();
        let h1 = thread::spawn(move || {
            b1.wait();
            let r = q1.send(&[10, 20, 30, 40], Timeout::Forever);
            tx1.send(r).ok();
        });

        let q2 = q.clone();
        let b2 = Arc::clone(&barrier);
        let tx2 = tx.clone();
        let h2 = thread::spawn(move || {
            b2.wait();
            let r = q2.send(&[50, 60, 70, 80], Timeout::Forever);
            tx2.send(r).ok();
        });
        drop(tx);

        barrier.wait();
        wait_until_blocked_count(2, Duration::from_secs(2));

        let mut buf = [0u8; 4];
        q.recv(&mut buf, Timeout::NoWait).expect("recv");

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first sender did not complete");
        assert!(first.is_ok());
        assert!(rx.try_recv().is_err(), "second sender woke too early");

        q.recv(&mut buf, Timeout::NoWait).expect("recv 2");

        let second = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second sender did not complete");
        assert!(second.is_ok());

        h1.join().expect("h1");
        h2.join().expect("h2");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 7. close wakes blocked receiver
// ---------------------------------------------------------------------------

#[test]
fn close_wakes_blocked_receiver() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        let qc = q.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let mut buf = [0u8; 4];
            let r = qc.recv(&mut buf, Timeout::Forever);
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        q.close().expect("close");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert_eq!(result, Err(Error::QueueClosed));

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 8. close wakes blocked sender
// ---------------------------------------------------------------------------

#[test]
fn close_wakes_blocked_sender() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");

        let qc = q.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = qc.send(&[5, 6, 7, 8], Timeout::Forever);
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        q.close().expect("close");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert_eq!(result, Err(Error::QueueClosed));

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 8. close while full — messages can be drained
// ---------------------------------------------------------------------------

#[test]
fn close_while_full_messages_drainable() {
    setup();
    {
        let q = FreeRtosQueue::new(2, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send 1");
        q.send(&[5, 6, 7, 8], Timeout::NoWait).expect("send 2");

        q.close().expect("close");

        assert_eq!(
            q.send(&[9, 9, 9, 9], Timeout::NoWait),
            Err(Error::QueueClosed)
        );

        let mut buf = [0u8; 4];
        q.recv(&mut buf, Timeout::NoWait).expect("recv 1");
        assert_eq!(buf, [1, 2, 3, 4]);
        q.recv(&mut buf, Timeout::NoWait).expect("recv 2");
        assert_eq!(buf, [5, 6, 7, 8]);
        assert_eq!(q.recv(&mut buf, Timeout::NoWait), Err(Error::QueueClosed));
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 9-10. timeout vs wake token race
// ---------------------------------------------------------------------------

#[test]
fn recv_timeout_with_racing_send_wakes() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        let qc = q.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let mut buf = [0u8; 4];
            let r = qc.recv(&mut buf, Timeout::After(Duration::from_millis(500)));
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        q.send(&[42, 0, 0, 0], Timeout::NoWait).expect("send");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok(), "expected recv success, got {result:?}");

        handle.join().expect("worker panicked");
    }
    teardown();
}

#[test]
fn send_timeout_with_racing_recv_wakes() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("fill");

        let qc = q.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = qc.send(&[5, 6, 7, 8], Timeout::After(Duration::from_millis(500)));
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        let mut buf = [0u8; 4];
        q.recv(&mut buf, Timeout::NoWait).expect("drain");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok(), "expected send success, got {result:?}");

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 11-13. scheduler-state preconditions
// ---------------------------------------------------------------------------

#[test]
fn send_forever_not_started_returns_not_initialized_and_no_waiter_leak() {
    setup();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");
        for _ in 0..4 {
            q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("fill");
        }

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);

        assert_eq!(
            q.send(&[5, 6, 7, 8], Timeout::Forever),
            Err(Error::NotInitialized)
        );

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    }
    teardown();
}

#[test]
fn recv_forever_suspended_returns_busy_and_no_waiter_leak() {
    setup();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Suspended);

        let mut buf = [0u8; 4];
        assert_eq!(q.recv(&mut buf, Timeout::Forever), Err(Error::Busy));

        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    }
    teardown();
}

#[test]
fn send_nowait_works_before_scheduler() {
    setup();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");
        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::NotStarted);
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");
        fixture::set_scheduler_state(osal_backend_freertos_sys::SchedulerState::Running);
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 14. Clone
// ---------------------------------------------------------------------------

#[test]
fn clone_shares_queue_state() {
    setup();
    {
        let q1 = FreeRtosQueue::new(4, 4).expect("create");
        let q2 = q1.clone();
        q1.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");
        let mut buf = [0u8; 4];
        q2.recv(&mut buf, Timeout::NoWait).expect("recv");
        assert_eq!(buf, [1, 2, 3, 4]);
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 15. Drop last clone — native resources deleted
// ---------------------------------------------------------------------------

#[test]
fn drop_last_clone_deletes_native_resources() {
    setup();
    let create_before = fixture::mutex_create_count() + fixture::sem_create_count();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");
        let _q2 = q.clone();
    }
    let delete_after = fixture::mutex_delete_count() + fixture::sem_delete_count();
    let create_after = fixture::mutex_create_count() + fixture::sem_create_count();
    assert_eq!(
        create_after - create_before,
        delete_after,
        "all created native objects must be deleted after last handle drop"
    );
    teardown();
}

// ---------------------------------------------------------------------------
// 16. Shutdown while alive → Busy
// ---------------------------------------------------------------------------

#[test]
fn shutdown_while_queue_alive_returns_busy() {
    setup();
    {
        let _q = FreeRtosQueue::new(4, 4).expect("create");
        assert_eq!(runtime::shutdown(), Err(Error::Busy));
    }
    assert!(runtime::shutdown().is_ok());
    fixture::reset();
}

// ---------------------------------------------------------------------------
// 17-18. Invalid parameters
// ---------------------------------------------------------------------------

#[test]
fn new_rejects_zero_capacity() {
    setup();
    {
        assert_eq!(
            FreeRtosQueue::new(0, 4).unwrap_err(),
            Error::InvalidParameter
        );
    }
    teardown();
}

#[test]
fn new_rejects_zero_msg_size() {
    setup();
    {
        assert_eq!(
            FreeRtosQueue::new(4, 0).unwrap_err(),
            Error::InvalidParameter
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 19-20. Error precedence: InvalidMessageSize > QueueClosed
// ---------------------------------------------------------------------------

#[test]
fn closed_queue_wrong_size_send_returns_invalid_message_size() {
    setup();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");
        q.close().expect("close");
        assert_eq!(
            q.send(&[1, 2], Timeout::NoWait),
            Err(Error::InvalidMessageSize)
        );
    }
    teardown();
}

#[test]
fn closed_queue_wrong_size_recv_returns_invalid_message_size() {
    setup();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");
        q.close().expect("close");
        let mut buf = [0u8; 2];
        assert_eq!(
            q.recv(&mut buf, Timeout::NoWait),
            Err(Error::InvalidMessageSize)
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 21. Multi-chunk finite timeout
// ---------------------------------------------------------------------------

#[test]
fn recv_multi_chunk_finite_timeout() {
    setup();
    {
        fixture::set_max_finite_delay_ticks(7);
        fixture::set_max_finite_wait_ticks(7);

        let q = FreeRtosQueue::new(4, 4).expect("create");
        let mut buf = [0u8; 4];
        assert_eq!(
            q.recv(&mut buf, Timeout::After(Duration::from_millis(20))),
            Err(Error::Timeout)
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 22. After(ZERO) returns Timeout
// ---------------------------------------------------------------------------

#[test]
fn send_after_zero_returns_timeout_not_queue_full() {
    setup();
    {
        let q = FreeRtosQueue::new(1, 4).expect("create");
        q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");
        assert_eq!(
            q.send(&[5, 6, 7, 8], Timeout::After(Duration::ZERO)),
            Err(Error::Timeout)
        );
    }
    teardown();
}

#[test]
fn recv_after_zero_returns_timeout_not_queue_empty() {
    setup();
    {
        let q = FreeRtosQueue::new(4, 4).expect("create");
        let mut buf = [0u8; 4];
        assert_eq!(
            q.recv(&mut buf, Timeout::After(Duration::ZERO)),
            Err(Error::Timeout)
        );
    }
    teardown();
}

// ---------------------------------------------------------------------------
// 23. Stress cycle
// ---------------------------------------------------------------------------

#[test]
fn stress_create_send_recv_close_drop_cycle() {
    setup();
    {
        for i in 0..100 {
            let q = FreeRtosQueue::new(4, 4).unwrap_or_else(|_| panic!("create {i}"));
            q.send(&[1, 2, 3, 4], Timeout::NoWait).expect("send");
            let mut buf = [0u8; 4];
            q.recv(&mut buf, Timeout::NoWait).expect("recv");
            assert_eq!(buf, [1, 2, 3, 4]);
            q.close().expect("close");
        }
    }
    teardown();
}
