//! POSIX SemaphoreBlockingContract — reusable cross-thread tests.
//!
//! These test functions are generic over `SemaphoreFactory` so that
//! future backends (e.g. FreeRTOS) can reuse them. The POSIX backend
//! binds them to `PosixSemaphoreFactory` in the `#[test]` entry points.
//!
//! Tests use `Barrier` / explicit ready-signaling to reduce reliance
//! on pure sleep-based timing, minimizing CI flakiness.

use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::semaphore::{BinarySemaphore as _, CountingSemaphore as _};
use osal_testkit::factory::SemaphoreFactory;
use osal_backend_posix::runtime;

// ===========================================================================
// CountingSemaphore blocking
// ===========================================================================

/// Forever acquire is woken by release from another thread.
pub fn counting_forever_wakes_after_release<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_counting_semaphore(1, 0).unwrap();
    let s2 = sem.clone();
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();

    let handle = thread::spawn(move || {
        b.wait(); // signal: we are about to block
        s2.acquire(Timeout::Forever).unwrap();
    });

    barrier.wait(); // worker is ready
    thread::sleep(Duration::from_millis(5));
    sem.release().unwrap();
    handle.join().unwrap();
}

/// After succeeds before deadline when released.
pub fn counting_after_succeeds_before_deadline<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_counting_semaphore(1, 0).unwrap();
    let s2 = sem.clone();
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();

    let handle = thread::spawn(move || {
        b.wait();
        s2.acquire(Timeout::After(Duration::from_millis(200)))
            .unwrap();
    });

    barrier.wait();
    thread::sleep(Duration::from_millis(5));
    sem.release().unwrap();
    handle.join().unwrap();
}

/// After does not timeout before the requested duration.
pub fn counting_after_does_not_timeout_early<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_counting_semaphore(1, 0).unwrap();
    let s2 = sem.clone();

    let handle = thread::spawn(move || {
        let start = Instant::now();
        let result = s2.acquire(Timeout::After(Duration::from_millis(30)));
        assert!(matches!(result, Err(Error::Timeout)));
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert!(start.elapsed() < Duration::from_secs(1));
    });

    handle.join().unwrap();
}

/// After times out when no release occurs.
pub fn counting_after_times_out<F: SemaphoreFactory>(factory: &F) {
    let sem = factory.create_counting_semaphore(1, 0).unwrap();
    let result = sem.acquire(Timeout::After(Duration::from_millis(1)));
    assert!(matches!(result, Err(Error::Timeout)));
}

/// One release wakes exactly one waiter.
pub fn counting_one_release_wakes_one_waiter<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_counting_semaphore(1, 0).unwrap();
    let s2 = sem.clone();
    let s3 = sem.clone();
    let barrier = Arc::new(Barrier::new(3));
    let b2 = barrier.clone();
    let b3 = barrier.clone();
    let done = Arc::new(std::sync::atomic::AtomicU32::new(0));
    let d2 = done.clone();
    let d3 = done.clone();

    let h2 = thread::spawn(move || {
        b2.wait();
        s2.acquire(Timeout::Forever).unwrap();
        d2.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });
    let h3 = thread::spawn(move || {
        b3.wait();
        s3.acquire(Timeout::Forever).unwrap();
        d3.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    });

    barrier.wait();
    thread::sleep(Duration::from_millis(5));
    sem.release().unwrap();
    thread::sleep(Duration::from_millis(10));
    assert_eq!(done.load(std::sync::atomic::Ordering::SeqCst), 1);

    sem.release().unwrap();
    thread::sleep(Duration::from_millis(10));
    assert_eq!(done.load(std::sync::atomic::Ordering::SeqCst), 2);

    h2.join().unwrap();
    h3.join().unwrap();
}

/// Count never exceeds max_count under concurrent release.
pub fn counting_limit_never_exceeded<F: SemaphoreFactory>(factory: &F)
where
    F::CountingSemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_counting_semaphore(3, 0).unwrap();
    let s2 = sem.clone();
    let s3 = sem.clone();

    let h1 = thread::spawn(move || {
        for _ in 0..100 {
            let _ = s2.release();
        }
    });
    let h2 = thread::spawn(move || {
        for _ in 0..100 {
            let _ = s3.release();
        }
    });

    h1.join().unwrap();
    h2.join().unwrap();

    let count = sem.count().unwrap();
    assert!(count <= 3);
}

// ===========================================================================
// BinarySemaphore blocking
// ===========================================================================

/// Forever acquire on binary semaphore is woken by release.
pub fn binary_forever_wakes_after_release<F: SemaphoreFactory>(factory: &F)
where
    F::BinarySemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_binary_semaphore().unwrap();
    let s2 = sem.clone();
    let barrier = Arc::new(Barrier::new(2));
    let b = barrier.clone();

    let handle = thread::spawn(move || {
        b.wait();
        s2.acquire(Timeout::Forever).unwrap();
    });

    barrier.wait();
    thread::sleep(Duration::from_millis(5));
    sem.release().unwrap();
    handle.join().unwrap();
}

/// After does not timeout early on binary semaphore.
pub fn binary_after_does_not_timeout_early<F: SemaphoreFactory>(factory: &F)
where
    F::BinarySemaphore: Clone + Send + Sync + 'static,
{
    let sem = factory.create_binary_semaphore().unwrap();
    let s2 = sem.clone();

    let handle = thread::spawn(move || {
        let start = Instant::now();
        let result = s2.acquire(Timeout::After(Duration::from_millis(30)));
        assert!(matches!(result, Err(Error::Timeout)));
        assert!(start.elapsed() >= Duration::from_millis(20));
        assert!(start.elapsed() < Duration::from_secs(1));
    });

    handle.join().unwrap();
}

// ===========================================================================
// POSIX backend binding
// ===========================================================================

use osal_backend_posix::semaphore::PosixSemaphoreFactory;

#[test]
fn posix_counting_forever_wakes() {
    let _ = runtime::initialize();
    counting_forever_wakes_after_release(&PosixSemaphoreFactory);
}

#[test]
fn posix_counting_after_succeeds() {
    let _ = runtime::initialize();
    counting_after_succeeds_before_deadline(&PosixSemaphoreFactory);
}

#[test]
fn posix_counting_after_not_early() {
    let _ = runtime::initialize();
    counting_after_does_not_timeout_early(&PosixSemaphoreFactory);
}

#[test]
fn posix_counting_after_times_out() {
    let _ = runtime::initialize();
    counting_after_times_out(&PosixSemaphoreFactory);
}

#[test]
fn posix_counting_one_wakes_one() {
    let _ = runtime::initialize();
    counting_one_release_wakes_one_waiter(&PosixSemaphoreFactory);
}

#[test]
fn posix_counting_limit_ok() {
    let _ = runtime::initialize();
    counting_limit_never_exceeded(&PosixSemaphoreFactory);
}

#[test]
fn posix_binary_forever_wakes() {
    let _ = runtime::initialize();
    binary_forever_wakes_after_release(&PosixSemaphoreFactory);
}

#[test]
fn posix_binary_after_not_early() {
    let _ = runtime::initialize();
    binary_after_does_not_timeout_early(&PosixSemaphoreFactory);
}
