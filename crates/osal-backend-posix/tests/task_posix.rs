//! POSIX-specific task tests: timeout join, repeated join, priority,
//! handle validity, and invalid-name rejection.

use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::clock::Clock as _;
use osal_api::traits::task::{Task as _, TaskBuilder as _};

use osal_backend_posix::clock::PosixClock;
use osal_backend_posix::task::PosixTaskBuilder;

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

    // Repeated join after completion.
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

    // Spin until the task marks itself running.
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
        .spawn(|| { /* immediate */ })
        .unwrap();

    let result = task.join(Timeout::Forever);
    assert!(result.is_ok());
}

// ---------------------------------------------------------------------------
// Repeated join
// ---------------------------------------------------------------------------

#[test]
fn repeated_join_returns_cached_exit_code() {
    let task = PosixTaskBuilder::new()
        .name("repeat")
        .spawn(|| {})
        .unwrap();

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
    let task = PosixTaskBuilder::new()
        .name("handle")
        .spawn(|| {})
        .unwrap();

    assert_ne!(task.handle(), 0);
    task.join(Timeout::Forever).unwrap();
}

#[test]
fn invalid_name_rejected() {
    let result = PosixTaskBuilder::new()
        .name("bad\0name")
        .spawn(|| {});

    assert!(matches!(result, Err(Error::InvalidParameter)));
}
