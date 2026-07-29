//! Cross-thread blocking and wakeup tests.
//!
//! These verify that a blocked waiter is actually woken by a release
//! on another thread — not just "release before acquire" single-thread
//! ordering.
//!
//! All tests use `std::sync::Barrier` for deterministic synchronization
//! and poll `fixture::blocked_count()` (not sleep) to confirm the worker
//! has entered `cvar.wait_timeout` before the main thread releases.
//!
//! Forever tests use `mpsc::channel` + `recv_timeout` as a watchdog.
//!
//! ```bash
//! cargo test -p osal-backend-freertos --features testkit sync_concurrent -- --test-threads=1
//! ```

#![cfg(feature = "testkit")]

use core::time::Duration;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::Instant;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;
use osal_api::traits::semaphore::{BinarySemaphore, CountingSemaphore};
use osal_backend_freertos::mutex::FreeRtosMutex;
use osal_backend_freertos::runtime;
use osal_backend_freertos::semaphore::{FreeRtosBinarySemaphore, FreeRtosCountingSemaphore};
use osal_backend_freertos_sys::fixture;

fn setup() {
    let _ = runtime::shutdown();
    fixture::reset();
    runtime::initialize().expect("initialize");
}

/// Tear down the runtime.  Panics if any leases are still held,
/// so test leaks are caught immediately.
/// Also calls `fixture::reset()` so the next test starts from a
/// pristine global state regardless of what this test did to
/// virtual ticks, scheduler state, etc.
fn teardown() {
    match runtime::shutdown() {
        Ok(()) | Err(Error::NotInitialized) => {}
        Err(e) => panic!("test leaked runtime lease or object: {e:?}"),
    }
    fixture::reset();
}

/// Poll `fixture::blocked_count()` until it reaches `expected` or
/// `timeout` expires.  `blocked_count` is incremented atomically
/// immediately before `cvar.wait_timeout`, so when it returns non-zero
/// the worker is guaranteed to be inside the Condvar — no sleep needed.
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
// Mutex — cross-thread Forever wake
// ---------------------------------------------------------------------------

#[test]
fn mutex_forever_woken_by_cross_thread_guard_drop() {
    setup();
    let value = {
        let m = FreeRtosMutex::new(0u32).expect("create");
        let guard = m.lock(Timeout::NoWait).expect("lock");
        let m_clone = m.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = m_clone.lock(Timeout::Forever);
            tx.send(r.map(|mut g| {
                *g = 42;
            }))
            .ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));
        drop(guard);

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok());

        handle.join().expect("worker panicked");
        let g = m.lock(Timeout::NoWait).expect("lock");
        *g
    };
    teardown();
    assert_eq!(value, 42);
}

// ---------------------------------------------------------------------------
// CountingSemaphore — cross-thread Forever wake
// ---------------------------------------------------------------------------

#[test]
fn counting_semaphore_forever_woken_by_cross_thread_release() {
    setup();
    {
        let s = FreeRtosCountingSemaphore::new(1, 0).expect("create");
        let s_clone = s.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = s_clone.acquire(Timeout::Forever);
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));
        s.release().expect("release");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok());

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// BinarySemaphore — cross-thread finite-wait early release
// ---------------------------------------------------------------------------

#[test]
fn binary_semaphore_after_early_release_wakes() {
    setup();
    {
        let s = FreeRtosBinarySemaphore::new().expect("create");
        let s_clone = s.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = s_clone.acquire(Timeout::After(Duration::from_millis(500)));
            tx.send(r).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));
        s.release().expect("release");

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert!(result.is_ok(), "expected acquire, got {result:?}");

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Mutex — cross-thread After timeout
// ---------------------------------------------------------------------------

#[test]
fn mutex_held_cross_thread_after_returns_timeout() {
    setup();
    {
        let m = FreeRtosMutex::new(0u32).expect("create");
        let _guard = m.lock(Timeout::NoWait).expect("lock");
        let m_clone = m.clone();
        let (tx, rx) = mpsc::channel();
        let barrier = Arc::new(Barrier::new(2));
        let b = Arc::clone(&barrier);

        let handle = thread::spawn(move || {
            b.wait();
            let r = m_clone.lock(Timeout::After(Duration::from_millis(10)));
            tx.send(r.map(drop)).ok();
        });

        barrier.wait();
        wait_until_blocked_count(1, Duration::from_secs(2));

        let result = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("worker did not complete");
        assert_eq!(result, Err(Error::Timeout));

        handle.join().expect("worker panicked");
    }
    teardown();
}

// ---------------------------------------------------------------------------
// Mutex — NoWait from different thread fails
// ---------------------------------------------------------------------------

#[test]
fn mutex_held_cross_thread_nowait_returns_lock_failed() {
    setup();
    {
        let m = FreeRtosMutex::new(0u32).expect("create");
        let _guard = m.lock(Timeout::NoWait).expect("lock");
        let m_clone = m.clone();

        let handle = thread::spawn(move || {
            let r = m_clone.lock(Timeout::NoWait);
            r.map(drop)
        });

        let result = handle.join().expect("worker panicked");
        assert_eq!(result, Err(Error::LockFailed));
    }
    teardown();
}

// ---------------------------------------------------------------------------
// CountingSemaphore — one release wakes at most one waiter
// ---------------------------------------------------------------------------

#[test]
fn counting_semaphore_one_release_wakes_only_one_waiter() {
    setup();
    {
        let s = FreeRtosCountingSemaphore::new(2, 0).expect("create");
        // Shared channel — either waiter can wake first.  The contract
        // only requires that ONE waiter wakes per release, not which one.
        let (tx, rx) = mpsc::channel();
        let b = Arc::new(Barrier::new(3));
        let done = Arc::new(AtomicUsize::new(0));

        let tx1 = tx.clone();
        let s1 = s.clone();
        let b1 = Arc::clone(&b);
        let d1 = Arc::clone(&done);
        let h1 = thread::spawn(move || {
            b1.wait();
            let r = s1.acquire(Timeout::Forever);
            if r.is_ok() {
                d1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            tx1.send(r).ok();
        });

        let tx2 = tx.clone();
        let s2 = s.clone();
        let b2 = Arc::clone(&b);
        let d2 = Arc::clone(&done);
        let h2 = thread::spawn(move || {
            b2.wait();
            let r = s2.acquire(Timeout::Forever);
            if r.is_ok() {
                d2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            tx2.send(r).ok();
        });
        // Drop the original tx so the receiver will see a
        // disconnected sender after both workers complete.
        drop(tx);

        // Wait for both workers to block.
        b.wait();
        wait_until_blocked_count(2, Duration::from_secs(2));

        // One release → exactly one waiter wakes (whichever).
        s.release().expect("release");

        let first = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("first waiter did not complete");
        assert!(first.is_ok());
        assert_eq!(
            done.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "exactly one waiter should have acquired"
        );

        // No second result yet.
        assert!(rx.try_recv().is_err(), "second waiter woke too early");

        // Second release wakes the remaining waiter.
        s.release().expect("release 2");

        let second = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("second waiter did not complete");
        assert!(second.is_ok());
        assert_eq!(done.load(std::sync::atomic::Ordering::SeqCst), 2);

        h1.join().expect("h1");
        h2.join().expect("h2");
    }
    teardown();
}
