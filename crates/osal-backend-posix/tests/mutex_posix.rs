//! POSIX MutexBlockingContract — cross-thread contention tests.
//!
//! These use `std::thread` and are only meaningful on the POSIX backend.

use std::thread;
use std::time::Duration;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex as _;

use osal_backend_posix::mutex::PosixMutexImpl;
use osal_backend_posix::runtime;

/// NoWait fails with LockFailed when mutex is held by another thread.
#[test]
fn no_wait_fails_when_held() {
    let _ = runtime::initialize();
    let m = PosixMutexImpl::new(0u32).unwrap();
    let m2 = m.clone();

    let _guard = m.lock(Timeout::NoWait).unwrap();

    let handle = thread::spawn(move || {
        let result = m2.lock(Timeout::NoWait);
        assert!(matches!(result, Err(Error::LockFailed)));
    });
    handle.join().unwrap();
}

/// After returns Timeout when mutex stays held.
#[test]
fn after_returns_timeout_when_held() {
    let _ = runtime::initialize();
    let m = PosixMutexImpl::new(0u32).unwrap();
    let m2 = m.clone();

    let _guard = m.lock(Timeout::NoWait).unwrap();

    let handle = thread::spawn(move || {
        let result = m2.lock(Timeout::After(Duration::from_millis(1)));
        assert!(matches!(result, Err(Error::Timeout)));
    });
    handle.join().unwrap();
}

/// Forever is woken when the guard is dropped by another thread.
#[test]
fn forever_woken_by_guard_drop() {
    let _ = runtime::initialize();
    let m = PosixMutexImpl::new(0u32).unwrap();
    let m2 = m.clone();

    let guard = m.lock(Timeout::NoWait).unwrap();

    let handle = thread::spawn(move || {
        let g = m2.lock(Timeout::Forever).unwrap();
        assert_eq!(*g, 0);
    });

    thread::sleep(Duration::from_millis(10));
    drop(guard);
    handle.join().unwrap();
}

/// Timed lock does not return Timeout before the requested duration.
#[test]
fn after_does_not_timeout_early() {
    use std::time::Instant;

    let _ = runtime::initialize();
    let m = PosixMutexImpl::new(0u32).unwrap();
    let m2 = m.clone();
    let _guard = m.lock(Timeout::NoWait).unwrap();

    let handle = thread::spawn(move || {
        let start = Instant::now();
        let result = m2.lock(Timeout::After(Duration::from_millis(30)));
        assert!(matches!(result, Err(Error::Timeout)));
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert!(start.elapsed() < Duration::from_secs(1));
    });

    handle.join().unwrap();
}
