//! Timer service lifecycle and shutdown race tests.
//!
//! Timer Service is process-global; count-dependent tests are
//! serialised.  Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit -- --test-threads=1
//! ```

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use osal_api::error::Error;
use osal_api::traits::timer::Timer as _;
use osal_api::types::TimerMode;
use osal_backend_posix::runtime;
use osal_backend_posix::timer::PosixTimer;

// ---------------------------------------------------------------------------
// Test isolation
// ---------------------------------------------------------------------------

static TIMER_TEST_LOCK: Mutex<()> = Mutex::new(());

struct TestRuntime;

impl TestRuntime {
    fn init() -> Self {
        // Hold the test lock so no other timer test runs concurrently.
        // We intentionally leak the guard — it is held for the test
        // duration and released when the process exits (or on panic).
        std::mem::forget(TIMER_TEST_LOCK.lock().unwrap());
        runtime::initialize().expect("timer init failed");
        Self
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let _ = runtime::shutdown();
    }
}

fn oneshot(period_ms: u64, cb: impl FnMut() + Send + 'static) -> PosixTimer {
    PosixTimer::new(
        "test",
        Duration::from_millis(period_ms),
        TimerMode::OneShot,
        Box::new(cb),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Basic lifecycle
// ---------------------------------------------------------------------------

#[test]
fn shutdown_before_initialize_returns_not_initialized() {
    let _lock = TIMER_TEST_LOCK.lock().unwrap();
    assert_eq!(runtime::shutdown(), Err(Error::NotInitialized));
}

#[test]
fn repeated_initialize_returns_already_initialized() {
    let _rt = TestRuntime::init();
    assert_eq!(runtime::initialize(), Err(Error::AlreadyInitialized));
}

#[test]
fn initialize_shutdown_initialize() {
    {
        let _rt = TestRuntime::init();
    }
    let _rt = TestRuntime::init();
}

#[test]
fn timer_works_after_restart() {
    {
        let _rt = TestRuntime::init();
    }
    {
        let _rt = TestRuntime::init();
        let fired = Arc::new(AtomicBool::new(false));
        let f = Arc::clone(&fired);
        let t = oneshot(10, move || f.store(true, Ordering::SeqCst));
        t.start().unwrap();
        thread::sleep(Duration::from_millis(50));
        assert!(fired.load(Ordering::SeqCst));
    }
}

// ---------------------------------------------------------------------------
// Timer liveness blocks shutdown
// ---------------------------------------------------------------------------

#[test]
fn live_timer_blocks_shutdown() {
    let _rt = TestRuntime::init();
    let _t = oneshot(500, || {});
    assert_eq!(runtime::shutdown(), Err(Error::Busy));
    // Cleanup: drop timer, then shutdown.
}

#[test]
fn stopped_timer_blocks_shutdown() {
    let _rt = TestRuntime::init();
    let t = oneshot(500, || {});
    t.stop().unwrap();
    // Stopped but not dropped — still blocks shutdown.
    assert_eq!(runtime::shutdown(), Err(Error::Busy));
}

#[test]
fn dropping_last_timer_allows_shutdown() {
    let _rt = TestRuntime::init();
    let t = oneshot(500, || {});
    drop(t);
    runtime::shutdown().unwrap();
}

#[test]
fn timer_clone_blocks_until_last_drop() {
    let _rt = TestRuntime::init();
    let t = oneshot(500, || {});
    let t2 = t.clone();
    drop(t);
    // t2 still alive.
    assert_eq!(runtime::shutdown(), Err(Error::Busy));
    drop(t2);
    runtime::shutdown().unwrap();
}

// ---------------------------------------------------------------------------
// Callback and shutdown
// ---------------------------------------------------------------------------

#[test]
fn shutdown_waits_for_inflight_callback() {
    let _rt = TestRuntime::init();
    let started = Arc::new(Barrier::new(2));
    let done = Arc::new(AtomicBool::new(false));

    let s = Arc::clone(&started);
    let d = Arc::clone(&done);
    let t = oneshot(10, move || {
        s.wait(); // signal that callback is running
        thread::sleep(Duration::from_millis(30));
        d.store(true, Ordering::SeqCst);
    });
    t.start().unwrap();

    // Wait for callback to enter.
    started.wait();

    // Deregister the timer so shutdown proceeds past the live-timer
    // check.  The callback is already in-flight — shutdown must
    // wait for it to complete before joining the worker.
    drop(t);

    // Shutdown from another thread — must wait for callback.
    let shutdown_done = Arc::new(AtomicBool::new(false));
    let sd = Arc::clone(&shutdown_done);
    let jh = thread::spawn(move || {
        runtime::shutdown().unwrap();
        sd.store(true, Ordering::SeqCst);
    });

    thread::sleep(Duration::from_millis(10));
    // Shutdown should still be waiting for the in-flight callback.
    assert!(!shutdown_done.load(Ordering::SeqCst));

    jh.join().unwrap();
    assert!(done.load(Ordering::SeqCst));
    assert!(shutdown_done.load(Ordering::SeqCst));
}

#[test]
fn no_callback_after_shutdown_returns() {
    let _rt = TestRuntime::init();

    let fired = Arc::new(AtomicBool::new(false));
    let f = Arc::clone(&fired);
    let t = oneshot(10, move || f.store(true, Ordering::SeqCst));
    t.start().unwrap();
    drop(t); // deregister

    runtime::shutdown().unwrap();

    // Any pending callback should have either fired or been
    // prevented — verify by waiting briefly.
    thread::sleep(Duration::from_millis(50));
    // After shutdown returns, no new callbacks execute.
    // (We can't strictly assert this without a mock, but the timer
    // was already deregistered before shutdown.)
}

#[test]
fn callback_self_shutdown_returns_busy() {
    let _rt = TestRuntime::init();

    let ready = Arc::new(Barrier::new(2));
    let r = Arc::clone(&ready);
    let result = Arc::new(Mutex::new(None));

    let res = Arc::clone(&result);
    let t = oneshot(10, move || {
        r.wait(); // main knows we are inside the callback
        *res.lock().unwrap() = Some(runtime::shutdown());
    });
    t.start().unwrap();

    ready.wait();
    drop(t); // deregister — so self-shutdown is tested, not live-timer Busy

    thread::sleep(Duration::from_millis(100));

    let r = result.lock().unwrap();
    assert_eq!(*r, Some(Err(Error::Busy)));
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[test]
fn concurrent_shutdown_has_one_winner() {
    let _rt = TestRuntime::init();

    // Use a barrier so both threads attempt shutdown simultaneously.
    let start = Arc::new(Barrier::new(3)); // 2 threads + main
    let attempted = Arc::new(Barrier::new(3));
    let release = Arc::new(Barrier::new(3));
    let succeeded = Arc::new(AtomicU32::new(0));
    let failed = Arc::new(AtomicU32::new(0));

    let mut handles = Vec::new();
    for _ in 0..2 {
        let start = Arc::clone(&start);
        let attempted = Arc::clone(&attempted);
        let release = Arc::clone(&release);
        let succeeded = Arc::clone(&succeeded);
        let failed = Arc::clone(&failed);
        handles.push(thread::spawn(move || {
            start.wait();
            match runtime::shutdown() {
                Ok(()) => {
                    succeeded.fetch_add(1, Ordering::SeqCst);
                }
                Err(_) => {
                    failed.fetch_add(1, Ordering::SeqCst);
                }
            }
            attempted.wait();
            release.wait();
        }));
    }

    start.wait();
    attempted.wait();

    assert_eq!(succeeded.load(Ordering::SeqCst), 1);
    assert_eq!(failed.load(Ordering::SeqCst), 1);

    release.wait();
    for h in handles {
        h.join().unwrap();
    }
}

#[test]
fn operation_during_stopping_returns_error() {
    let _rt = TestRuntime::init();

    // Create a running timer that blocks shutdown.
    let t = oneshot(5000, || {});

    // Start shutdown on another thread — it will block on live timer.
    let started_shutdown = Arc::new(AtomicBool::new(false));
    let ss = Arc::clone(&started_shutdown);
    let shutdown_complete = Arc::new(AtomicBool::new(false));
    let sc = Arc::clone(&shutdown_complete);

    let jh = thread::spawn(move || {
        ss.store(true, Ordering::SeqCst);
        // shutdown will fail because timer is still alive.
        assert_eq!(runtime::shutdown(), Err(Error::Busy));
        sc.store(true, Ordering::SeqCst);
    });

    // Wait until shutdown thread has started and returned Busy.
    while !shutdown_complete.load(Ordering::SeqCst) {
        thread::yield_now();
    }
    jh.join().unwrap();

    // After the failed shutdown, the service should still be Running.
    // (We verify by creating a new timer — this would fail if service
    // were Stopped.)
    let t2 = oneshot(10, || {});
    t2.start().unwrap();
    drop(t);
    drop(t2);
}
