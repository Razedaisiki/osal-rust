//! POSIX SemaphoreBlockingContract — cross-thread tests.
//!
//! These use `std::thread` and are only meaningful on the POSIX backend.

use std::thread;
use std::time::Duration;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::semaphore::CountingSemaphore as _;

use osal_backend_posix::semaphore::PosixCountingSemaphore;

// ---------------------------------------------------------------------------
// CountingSemaphore blocking
// ---------------------------------------------------------------------------

/// Forever acquire is woken by release from another thread.
#[test]
fn counting_forever_wakes_after_release() {
    let sem = PosixCountingSemaphore::new(1, 0).unwrap();
    let s2 = sem.clone();

    let handle = thread::spawn(move || {
        s2.acquire(Timeout::Forever).unwrap();
    });

    thread::sleep(Duration::from_millis(10));
    sem.release().unwrap();
    handle.join().unwrap();
}

/// After succeeds before deadline when released.
#[test]
fn counting_after_succeeds_before_deadline() {
    let sem = PosixCountingSemaphore::new(1, 0).unwrap();
    let s2 = sem.clone();

    let handle = thread::spawn(move || {
        s2.acquire(Timeout::After(Duration::from_millis(200))).unwrap();
    });

    thread::sleep(Duration::from_millis(10));
    sem.release().unwrap();
    handle.join().unwrap();
}

/// After does not timeout early.
#[test]
fn counting_after_does_not_timeout_early() {
    use std::time::Instant;
    let sem = PosixCountingSemaphore::new(1, 0).unwrap();
    let s2 = sem.clone();

    let handle = thread::spawn(move || {
        let start = Instant::now();
        let result = s2.acquire(Timeout::After(Duration::from_millis(30)));
        assert!(matches!(result, Err(Error::Timeout)));
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert!(start.elapsed() < Duration::from_secs(1));
    });

    thread::sleep(Duration::from_millis(10));
    handle.join().unwrap();
}

/// After times out when no release occurs.
#[test]
fn counting_after_times_out_when_unreleased() {
    let sem = PosixCountingSemaphore::new(1, 0).unwrap();
    let result = sem.acquire(Timeout::After(Duration::from_millis(1)));
    assert!(matches!(result, Err(Error::Timeout)));
}

/// One release wakes exactly one waiter.
#[test]
fn counting_one_release_wakes_one_waiter() {
    let sem = PosixCountingSemaphore::new(2, 2).unwrap();

    // Acquire both permits first so waiters block
    sem.acquire(Timeout::NoWait).unwrap();
    sem.acquire(Timeout::NoWait).unwrap();

    let s2 = sem.clone();
    let s3 = sem.clone();
    let done = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
    let d2 = done.clone();
    let d3 = done.clone();

    let h2 = thread::spawn(move || {
        s2.acquire(Timeout::Forever).unwrap();
        d2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let h3 = thread::spawn(move || {
        s3.acquire(Timeout::Forever).unwrap();
        d3.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });

    thread::sleep(Duration::from_millis(10));
    // One release should wake exactly one waiter
    sem.release().unwrap();
    thread::sleep(Duration::from_millis(10));
    assert_eq!(done.load(std::sync::atomic::Ordering::SeqCst), 1);

    // Second release wakes the other
    sem.release().unwrap();
    thread::sleep(Duration::from_millis(10));
    assert_eq!(done.load(std::sync::atomic::Ordering::SeqCst), 2);

    h2.join().unwrap();
    h3.join().unwrap();
}

/// Count never exceeds max_count under concurrent release.
#[test]
fn counting_permit_limit_never_exceeded() {
    let sem = PosixCountingSemaphore::new(3, 0).unwrap();
    let s2 = sem.clone();
    let s3 = sem.clone();

    let h1 = thread::spawn(move || {
        for _ in 0..100 {
            let _ = s2.release(); // Overflow is fine — we just test the cap
        }
    });
    let h2 = thread::spawn(move || {
        for _ in 0..100 {
            let _ = s3.release();
        }
    });

    h1.join().unwrap();
    h2.join().unwrap();

    // count must not exceed max_count
    let count = sem.count().unwrap();
    assert!(count <= 3);
}
