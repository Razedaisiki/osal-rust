//! Timer service lifecycle and shutdown race tests.
//!
//! Timer Service is process-global; count-dependent tests are
//! serialised.  Requires `--features testkit`:
//! ```bash
//! cargo test -p osal-backend-posix --features testkit -- --test-threads=1
//! ```

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
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

struct TestRuntime {
    _serial: MutexGuard<'static, ()>,
}

impl TestRuntime {
    fn init() -> Self {
        let serial = TIMER_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        runtime::initialize().expect("timer init failed");
        Self { _serial: serial }
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let result = runtime::shutdown();
        match result {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) if thread::panicking() => {
                eprintln!("timer runtime cleanup failed during unwind: {e:?}");
            }
            Err(e) => {
                panic!("timer runtime cleanup failed: {e:?}");
            }
        }
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
    let _rt = TestRuntime::init();
    runtime::shutdown().unwrap();
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
}

#[test]
fn stopped_timer_blocks_shutdown() {
    let _rt = TestRuntime::init();
    let t = oneshot(500, || {});
    t.stop().unwrap();
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
        s.wait();
        thread::sleep(Duration::from_millis(30));
        d.store(true, Ordering::SeqCst);
    });
    t.start().unwrap();

    started.wait();
    drop(t); // deregister so shutdown proceeds past live-timer check

    let shutdown_done = Arc::new(AtomicBool::new(false));
    let sd = Arc::clone(&shutdown_done);
    let jh = thread::spawn(move || {
        runtime::shutdown().unwrap();
        sd.store(true, Ordering::SeqCst);
    });

    thread::sleep(Duration::from_millis(10));
    assert!(!shutdown_done.load(Ordering::SeqCst));

    jh.join().unwrap();
    assert!(done.load(Ordering::SeqCst));
    assert!(shutdown_done.load(Ordering::SeqCst));
}

#[test]
fn no_callback_after_shutdown_returns() {
    let _rt = TestRuntime::init();

    let count = Arc::new(AtomicU32::new(0));
    let c = Arc::clone(&count);
    let t = oneshot(10, move || {
        c.fetch_add(1, Ordering::SeqCst);
    });
    t.start().unwrap();
    drop(t);

    runtime::shutdown().unwrap();

    let count_at_shutdown = count.load(Ordering::SeqCst);
    thread::sleep(Duration::from_millis(100));

    assert_eq!(
        count.load(Ordering::SeqCst),
        count_at_shutdown,
        "callback executed after shutdown returned",
    );
}

#[test]
fn callback_self_shutdown_returns_busy() {
    let _rt = TestRuntime::init();

    let entered = Arc::new(Barrier::new(2));
    let continue_cb = Arc::new(Barrier::new(2));
    let completed = Arc::new(Barrier::new(2));
    let result = Arc::new(Mutex::new(None));

    let ent = Arc::clone(&entered);
    let cont = Arc::clone(&continue_cb);
    let comp = Arc::clone(&completed);
    let res = Arc::clone(&result);

    let t = oneshot(10, move || {
        ent.wait();
        cont.wait(); // main drops timer, then releases us
        *res.lock().unwrap() = Some(runtime::shutdown());
        comp.wait(); // signal that we stored the result
    });
    t.start().unwrap();

    // Wait until callback is inside.
    entered.wait();
    // Now drop the timer so self-shutdown check is reached, not live-timer Busy.
    drop(t);
    // Release the callback to call shutdown.
    continue_cb.wait();

    // Wait for callback to store its result.
    completed.wait();

    let r = result.lock().unwrap();
    assert_eq!(*r, Some(Err(Error::Busy)));
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[test]
fn concurrent_shutdown_has_one_winner() {
    let _rt = TestRuntime::init();

    let start = Arc::new(Barrier::new(3));
    let mut handles = Vec::new();

    for _ in 0..2 {
        let start = Arc::clone(&start);
        handles.push(thread::spawn(move || {
            start.wait();
            runtime::shutdown()
        }));
    }

    start.wait();

    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("shutdown thread panicked"))
        .collect();

    let success_count = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(
        success_count, 1,
        "exactly one concurrent shutdown must succeed: {results:?}",
    );

    let failure = results.iter().find(|r| r.is_err()).unwrap();
    assert!(
        matches!(failure, Err(Error::Busy) | Err(Error::NotInitialized)),
        "unexpected losing shutdown result: {failure:?}",
    );
}

#[test]
fn busy_shutdown_leaves_service_running() {
    let _rt = TestRuntime::init();

    // Live timer causes shutdown to return Busy without
    // transitioning to Stopping.
    let t = oneshot(5000, || {});

    let shutdown_result = Arc::new(Mutex::new(None));
    let sr = Arc::clone(&shutdown_result);
    let jh = thread::spawn(move || {
        *sr.lock().unwrap() = Some(runtime::shutdown());
    });

    // Wait for shutdown to return Busy.
    loop {
        if shutdown_result.lock().unwrap().is_some() {
            break;
        }
        thread::yield_now();
    }
    jh.join().unwrap();

    assert_eq!(*shutdown_result.lock().unwrap(), Some(Err(Error::Busy)));

    // Service must still be Running after a failed shutdown.
    let t2 = oneshot(10, || {});
    t2.start().unwrap();
    drop(t);
    drop(t2);
}
