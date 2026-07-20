//! Mock task stabilisation tests — panic rollback, count invariance.

use core::sync::atomic::{AtomicUsize, Ordering};
use std::panic::catch_unwind;

use osal_api::traits::task::{Task as _, TaskBuilder as _};
use osal_backend_mock::task::{MockTask, MockTaskBuilder};

// Simple spinlock — serialises count-dependent tests to avoid
// interference from other tests manipulating LIVE_COUNT.
static LOCK: AtomicUsize = AtomicUsize::new(0);

struct CountLock;

fn acquire() -> CountLock {
    while LOCK.swap(1, Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
    CountLock
}
impl Drop for CountLock {
    fn drop(&mut self) {
        LOCK.store(0, Ordering::Release);
    }
}

#[test]
fn panic_rollback_current_returns_none() {
    let _lock = acquire();

    let result = catch_unwind(|| {
        let _ = MockTaskBuilder::new()
            .name("panic")
            .spawn(|| panic!("test unwind"));
    });

    assert!(result.is_err());
    // After panic, TLS must be restored.
    assert_eq!(MockTask::current(), None);
}

#[test]
fn panic_rollback_count_unchanged() {
    let _lock = acquire();

    let baseline = MockTask::count();

    let _ = catch_unwind(|| {
        let _ = MockTaskBuilder::new()
            .name("panic-count")
            .spawn(|| panic!("test unwind"));
    });

    // After panic, live count must be unchanged.
    assert_eq!(MockTask::count(), baseline);
}
