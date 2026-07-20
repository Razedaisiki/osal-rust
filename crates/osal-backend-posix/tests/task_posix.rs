//! POSIX-specific task tests: timeout join, concurrency, drop-without-join.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use core::time::Duration;
use std::sync::Arc;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::types::ExitCode;
use osal_api::traits::clock::Clock as _;
use osal_api::traits::task::{Task as _, TaskBuilder as _};

use osal_backend_posix::clock::PosixClock;
use osal_backend_posix::task::{PosixTask, PosixTaskBuilder};

// ---------------------------------------------------------------------------
// Count-test spinlock — prevents parallel count-dependent tests from
// interfering with each other's baseline assertions.
// ---------------------------------------------------------------------------

static COUNT_LOCK: AtomicUsize = AtomicUsize::new(0);

struct CountTestLock;

fn count_lock() -> CountTestLock {
    while COUNT_LOCK.swap(1, Ordering::Acquire) != 0 {
        std::hint::spin_loop();
    }
    CountTestLock
}
impl Drop for CountTestLock {
    fn drop(&mut self) {
        COUNT_LOCK.store(0, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// Timeout join
// ---------------------------------------------------------------------------

#[test]
fn join_after_times_out_then_can_retry() {
    static DONE: AtomicBool = AtomicBool::new(false);
    DONE.store(false, Ordering::SeqCst);

    let task = PosixTaskBuilder::new()
        .name("timeout")
        .spawn(|| {
            PosixClock::delay(Duration::from_millis(30));
            DONE.store(true, Ordering::SeqCst);
        })
        .unwrap();

    let result = task.join(Timeout::After(Duration::from_millis(1)));
    assert_eq!(result, Err(Error::Timeout));

    let result = task.join(Timeout::Forever);
    assert!(result.is_ok());
    assert!(DONE.load(Ordering::SeqCst));

    let result = task.join(Timeout::NoWait);
    assert!(result.is_ok());
}

#[test]
fn join_no_wait_times_out_while_running() {
    static RUNNING: AtomicBool = AtomicBool::new(false);
    RUNNING.store(false, Ordering::SeqCst);

    let task = PosixTaskBuilder::new()
        .name("nowait")
        .spawn(|| {
            RUNNING.store(true, Ordering::SeqCst);
            PosixClock::delay(Duration::from_millis(50));
        })
        .unwrap();

    while !RUNNING.load(Ordering::SeqCst) {
        PosixClock::delay(Duration::from_millis(1));
    }

    let result = task.join(Timeout::NoWait);
    assert_eq!(result, Err(Error::Timeout));

    task.join(Timeout::Forever).unwrap();
}

#[test]
fn join_forever_returns_after_completion() {
    let task = PosixTaskBuilder::new()
        .name("forever")
        .spawn(|| {})
        .unwrap();

    let result = task.join(Timeout::Forever);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Repeated join
// ---------------------------------------------------------------------------

#[test]
fn repeated_join_returns_cached_exit_code() {
    let task = PosixTaskBuilder::new().name("repeat").spawn(|| {}).unwrap();

    let r1 = task.join(Timeout::Forever).unwrap();
    let r2 = task.join(Timeout::NoWait).unwrap();
    let r3 = task.join(Timeout::Forever).unwrap();
    let r4 = task.join(Timeout::After(Duration::from_millis(1))).unwrap();

    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
    assert_eq!(r3, r4);
}

// ---------------------------------------------------------------------------
// Basic properties
// ---------------------------------------------------------------------------

#[test]
fn priority_is_preserved() {
    let task = PosixTaskBuilder::new()
        .name("prio")
        .priority(7)
        .spawn(|| {})
        .unwrap();

    assert_eq!(task.priority(), 7);
    task.join(Timeout::Forever).unwrap();
    assert_eq!(task.priority(), 7);
}

#[test]
fn handle_is_nonzero() {
    let task = PosixTaskBuilder::new().name("handle").spawn(|| {}).unwrap();

    assert_ne!(task.handle().get(), 0);
    task.join(Timeout::Forever).unwrap();
}

#[test]
fn invalid_name_rejected() {
    let result = PosixTaskBuilder::new().name("bad\0name").spawn(|| {});
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

// ---------------------------------------------------------------------------
// Drop-without-join does not cancel
// ---------------------------------------------------------------------------

#[test]
fn drop_without_join_does_not_cancel_task() {
    static DONE: AtomicBool = AtomicBool::new(false);
    DONE.store(false, Ordering::SeqCst);

    {
        let _task = PosixTaskBuilder::new()
            .name("drop-no-join")
            .spawn(|| {
                PosixClock::delay(Duration::from_millis(10));
                DONE.store(true, Ordering::SeqCst);
            })
            .unwrap();
    }

    PosixClock::delay(Duration::from_millis(50));
    assert!(DONE.load(Ordering::SeqCst));
}

#[test]
fn many_tasks_can_be_dropped_without_join() {
    let counter = Arc::new(AtomicU32::new(0));

    for _ in 0..100 {
        let c = Arc::clone(&counter);
        let _task = PosixTaskBuilder::new()
            .name("drop-stress")
            .spawn(move || {
                c.fetch_add(1, Ordering::Relaxed);
            })
            .unwrap();
    }

    PosixClock::delay(Duration::from_millis(50));
    assert_eq!(counter.load(Ordering::Relaxed), 100);
}

// ---------------------------------------------------------------------------
// Concurrency (POSIX only)
// ---------------------------------------------------------------------------

#[test]
fn three_tasks_run_concurrently() {
    use osal_api::traits::task::Task as _;
    use std::thread;

    static ENTERED: AtomicU32 = AtomicU32::new(0);
    static RELEASE: AtomicBool = AtomicBool::new(false);
    ENTERED.store(0, Ordering::SeqCst);
    RELEASE.store(false, Ordering::SeqCst);

    let t1 = PosixTaskBuilder::new()
        .name("c1")
        .spawn(|| {
            ENTERED.fetch_add(1, Ordering::SeqCst);
            while !RELEASE.load(Ordering::SeqCst) {
                thread::yield_now();
            }
        })
        .unwrap();

    let t2 = PosixTaskBuilder::new()
        .name("c2")
        .spawn(|| {
            ENTERED.fetch_add(1, Ordering::SeqCst);
            while !RELEASE.load(Ordering::SeqCst) {
                thread::yield_now();
            }
        })
        .unwrap();

    let t3 = PosixTaskBuilder::new()
        .name("c3")
        .spawn(|| {
            ENTERED.fetch_add(1, Ordering::SeqCst);
            while !RELEASE.load(Ordering::SeqCst) {
                thread::yield_now();
            }
        })
        .unwrap();

    // Wait until all three have entered their barrier.
    while ENTERED.load(Ordering::SeqCst) < 3 {
        thread::yield_now();
    }

    RELEASE.store(true, Ordering::SeqCst);
    t1.join(Timeout::Forever).unwrap();
    t2.join(Timeout::Forever).unwrap();
    t3.join(Timeout::Forever).unwrap();
}

#[test]
fn count_decremented_before_join_returns() {
    use osal_api::traits::task::Task as _;
    let _lock = count_lock();

    let baseline = PosixTask::count();

    let task = PosixTaskBuilder::new()
        .name("count-join")
        .spawn(|| {})
        .unwrap();

    // Block until done — after Forever returns, count must reflect
    // completion (the trampoline drops live_token before Finished).
    task.join(Timeout::Forever).unwrap();
    assert_eq!(PosixTask::count(), baseline);

    // NoWait on an already-completed task succeeds immediately.
    assert!(task.join(Timeout::NoWait).is_ok());
}

// ---------------------------------------------------------------------------
// Regression: concurrent join
// ---------------------------------------------------------------------------

#[test]
fn two_threads_can_join_same_task() {
    use std::thread;

    let task = PosixTaskBuilder::new()
        .name("concurrent-join")
        .spawn(|| {})
        .unwrap();

    let t1_handle = task.clone();
    let t2_handle = task.clone();

    let j1 = thread::spawn(move || t1_handle.join(Timeout::Forever).unwrap());
    let j2 = thread::spawn(move || t2_handle.join(Timeout::Forever).unwrap());

    let r1 = j1.join().unwrap();
    let r2 = j2.join().unwrap();

    // Both joiners get the same cached exit code.
    assert_eq!(r1, r2);
    assert_eq!(r1, ExitCode::SUCCESS);
}

// ---------------------------------------------------------------------------
// Regression: spawn failure does not affect count
// ---------------------------------------------------------------------------

#[test]
fn spawn_failure_does_not_pollute_count() {
    let _lock = count_lock();

    let baseline = PosixTask::count();

    // Overlong name causes spawn failure.
    let long = "a".repeat(32);
    let result = PosixTaskBuilder::new().name(&long).spawn(|| {});
    assert!(result.is_err());

    // Count must be unchanged.
    assert_eq!(PosixTask::count(), baseline);

    // Zero stack also fails without affecting count.
    let result = PosixTaskBuilder::new().stack_size(0).spawn(|| {});
    assert!(result.is_err());
    assert_eq!(PosixTask::count(), baseline);
}
