//! Backend runtime lifecycle integration tests.
//!
//! Verifies that the POSIX backend correctly owns a `RuntimeLifecycle`
//! instance (ADR 0019) and orchestrates timer-service start/stop.
//! Count-dependent tests are serialised via `TIMER_TEST_LOCK`.
//!
//! ```bash
//! cargo test -p osal-backend-posix --features testkit -- --test-threads=1
//! ```

use core::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier, Mutex, MutexGuard};
use std::thread;

use osal_api::error::Error;
use osal_api::runtime::RuntimeState;
use osal_backend_posix::runtime;

// ---------------------------------------------------------------------------
// Test isolation (shared with timer_lifecycle.rs)
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
        runtime::initialize().expect("runtime init failed");
        Self { _serial: serial }
    }
}

impl Drop for TestRuntime {
    fn drop(&mut self) {
        let result = runtime::shutdown();
        match result {
            Ok(()) | Err(Error::NotInitialized) => {}
            Err(e) if thread::panicking() => {
                eprintln!("runtime cleanup failed during unwind: {e:?}");
            }
            Err(e) => {
                panic!("runtime cleanup failed: {e:?}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Basic state transitions
// ---------------------------------------------------------------------------

#[test]
fn initial_state_is_uninitialized() {
    let _rt = TestRuntime::init();
    runtime::shutdown().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Uninitialized);
}

#[test]
fn initialize_enters_running() {
    let _rt = TestRuntime::init();
    assert_eq!(runtime::state(), RuntimeState::Running);
}

#[test]
fn repeated_initialize_returns_already_initialized() {
    let _rt = TestRuntime::init();
    assert_eq!(runtime::initialize(), Err(Error::AlreadyInitialized));
    // State must still be Running — the failed attempt did not
    // corrupt it.
    assert_eq!(runtime::state(), RuntimeState::Running);
}

#[test]
fn shutdown_returns_to_uninitialized() {
    let _rt = TestRuntime::init();
    runtime::shutdown().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Uninitialized);
}

#[test]
fn shutdown_before_initialize_returns_not_initialized() {
    let _rt = TestRuntime::init();
    runtime::shutdown().unwrap();
    // Second shutdown on an already-uninitialized runtime.
    assert_eq!(runtime::shutdown(), Err(Error::NotInitialized));
}

#[test]
fn runtime_can_reinitialize() {
    {
        let _rt = TestRuntime::init();
    } // shutdown
    {
        let _rt = TestRuntime::init();
        assert_eq!(runtime::state(), RuntimeState::Running);
    }
}

// ---------------------------------------------------------------------------
// active_objects (testkit)
// ---------------------------------------------------------------------------

#[test]
fn no_active_objects_when_idle() {
    let _rt = TestRuntime::init();
    assert_eq!(runtime::active_objects(), 0);
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

#[test]
fn concurrent_initialize_has_one_winner() {
    // Start from a clean Uninitialized state.
    let _rt = TestRuntime::init();
    runtime::shutdown().unwrap();

    let n: usize = 4;
    let start = Arc::new(Barrier::new(n + 1));
    let attempted = Arc::new(Barrier::new(n + 1));
    let release = Arc::new(Barrier::new(n + 1));
    let successes = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..n {
        let start = Arc::clone(&start);
        let attempted = Arc::clone(&attempted);
        let release = Arc::clone(&release);
        let successes = Arc::clone(&successes);

        handles.push(thread::spawn(move || {
            start.wait();
            if runtime::initialize().is_ok() {
                successes.fetch_add(1, Ordering::SeqCst);
            }
            attempted.wait();
            release.wait();
        }));
    }

    start.wait();
    attempted.wait();
    assert_eq!(
        successes.load(Ordering::SeqCst),
        1,
        "exactly one concurrent initialize must succeed"
    );

    release.wait();
    for h in handles {
        h.join().unwrap();
    }

    // Clean up: the winner left us in Running.
    runtime::shutdown().unwrap();
    assert_eq!(runtime::state(), RuntimeState::Uninitialized);
}

#[test]
fn concurrent_shutdown_has_one_winner() {
    let _rt = TestRuntime::init();

    let n: usize = 4;
    let start = Arc::new(Barrier::new(n + 1));
    let attempted = Arc::new(Barrier::new(n + 1));
    let release = Arc::new(Barrier::new(n + 1));
    let successes = Arc::new(AtomicUsize::new(0));

    let mut handles = Vec::new();
    for _ in 0..n {
        let start = Arc::clone(&start);
        let attempted = Arc::clone(&attempted);
        let release = Arc::clone(&release);
        let successes = Arc::clone(&successes);

        handles.push(thread::spawn(move || {
            start.wait();
            if runtime::shutdown().is_ok() {
                successes.fetch_add(1, Ordering::SeqCst);
            }
            attempted.wait();
            release.wait();
        }));
    }

    start.wait();
    attempted.wait();
    assert_eq!(
        successes.load(Ordering::SeqCst),
        1,
        "exactly one concurrent shutdown must succeed"
    );

    release.wait();
    for h in handles {
        h.join().unwrap();
    }
}
