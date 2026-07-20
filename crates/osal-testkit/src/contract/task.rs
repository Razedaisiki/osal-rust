//! Contract tests for the `Task` and `TaskBuilder` traits.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#8-task-contract`.
//!
//! Split into:
//! - `TaskCoreContract` — both Mock and POSIX must pass
//! - `TaskConcurrencyContract` — POSIX only (defined here, called
//!   from backend-specific test files)

use core::sync::atomic::{AtomicUsize, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::traits::task::{Task as _, TaskBuilder as _};

use crate::factory::TaskFactory;

// ---------------------------------------------------------------------------
// Simple spinlock for tests that assert exact global count() values.
// Gate only count-dependent tests, not the full suite.
// ---------------------------------------------------------------------------

static COUNT_LOCK: AtomicUsize = AtomicUsize::new(0);

fn count_guard() {
    while COUNT_LOCK.swap(1, Ordering::Acquire) != 0 {
        core::hint::spin_loop();
    }
}

impl Drop for CountGuard {
    fn drop(&mut self) {
        COUNT_LOCK.store(0, Ordering::Release);
    }
}

struct CountGuard;

fn acquire_count_lock() -> CountGuard {
    count_guard();
    CountGuard
}

// ---------------------------------------------------------------------------
// TaskCoreContract — both backends
// ---------------------------------------------------------------------------

/// Default builder creates a valid task.
pub fn create_with_default_config<F: TaskFactory>(factory: &F) {
    let task = factory.task_builder().spawn(|| {}).unwrap();
    task.join(Timeout::Forever).unwrap();
}

/// Empty name is valid.
pub fn accept_empty_name<F: TaskFactory>(factory: &F) {
    let task = factory.task_builder().name("").spawn(|| {}).unwrap();
    task.join(Timeout::Forever).unwrap();
}

/// Name of exactly 31 bytes is valid.
pub fn accept_max_length_name<F: TaskFactory>(factory: &F) {
    let max_name = "a".repeat(31);
    let task = factory.task_builder().name(&max_name).spawn(|| {}).unwrap();
    task.join(Timeout::Forever).unwrap();
}

/// Name containing NUL is rejected with precise error.
pub fn reject_nul_in_name<F: TaskFactory>(factory: &F) {
    let result = factory.task_builder().name("bad\0name").spawn(|| {});
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Name > 31 bytes is rejected with precise error.
pub fn reject_overlong_name<F: TaskFactory>(factory: &F) {
    let long = "a".repeat(32);
    let result = factory.task_builder().name(&long).spawn(|| {});
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Zero stack size is rejected with precise error.
pub fn reject_zero_stack<F: TaskFactory>(factory: &F) {
    let result = factory.task_builder().stack_size(0).spawn(|| {});
    assert!(matches!(result, Err(Error::InvalidParameter)));
}

/// Positive stack size creates successfully.
pub fn positive_stack_size_succeeds<F: TaskFactory>(factory: &F) {
    let task = factory
        .task_builder()
        .stack_size(8192)
        .spawn(|| {})
        .unwrap();
    task.join(Timeout::Forever).unwrap();
}

/// Spawned task runs its entry exactly once.
pub fn spawn_runs_entry_exactly_once<F: TaskFactory>(factory: &F) {
    static COUNTER: AtomicUsize = AtomicUsize::new(0);
    COUNTER.store(0, Ordering::SeqCst);

    let task = factory
        .task_builder()
        .name("exact")
        .spawn(|| {
            COUNTER.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();

    task.join(Timeout::Forever).unwrap();
    assert_eq!(COUNTER.load(Ordering::SeqCst), 1);
}

/// join() returns after task exit.
pub fn join_returns_after_task_exit<F: TaskFactory>(factory: &F) {
    let task = factory.task_builder().name("join").spawn(|| {}).unwrap();
    assert!(task.join(Timeout::Forever).is_ok());
}

/// Repeated join after completion returns cached immediately.
pub fn repeated_join_returns_cached<F: TaskFactory>(factory: &F) {
    let task = factory.task_builder().name("repeat").spawn(|| {}).unwrap();
    let r1 = task.join(Timeout::Forever).unwrap();
    let r2 = task.join(Timeout::NoWait).unwrap();
    let r3 = task.join(Timeout::Forever).unwrap();
    assert_eq!(r1, r2);
    assert_eq!(r2, r3);
}

/// handle() is non-zero.
pub fn handle_is_nonzero<F: TaskFactory>(factory: &F) {
    let task = factory.task_builder().name("h").spawn(|| {}).unwrap();
    assert_ne!(task.handle().get(), 0);
    task.join(Timeout::Forever).unwrap();
}

/// handle() is unique across tasks.
pub fn handle_is_unique<F: TaskFactory>(factory: &F) {
    let t1 = factory.task_builder().name("a").spawn(|| {}).unwrap();
    let t2 = factory.task_builder().name("b").spawn(|| {}).unwrap();
    assert_ne!(t1.handle(), t2.handle());
}

/// current() returns Some(handle) from within the entry.
pub fn current_from_within_task<F: TaskFactory>(factory: &F) {
    static CAPTURED_RAW: AtomicUsize = AtomicUsize::new(0);

    let task = factory
        .task_builder()
        .name("curr")
        .spawn(|| {
            if let Some(h) = F::Task::current() {
                CAPTURED_RAW.store(h.get(), Ordering::SeqCst);
            }
        })
        .unwrap();

    task.join(Timeout::Forever).unwrap();
    let captured_raw = CAPTURED_RAW.load(Ordering::SeqCst);
    assert_ne!(captured_raw, 0);
    assert_eq!(captured_raw, task.handle().get());
}

/// current() returns None from main/external thread.
pub fn current_from_main_is_none<F: TaskFactory>(_factory: &F) {
    assert_eq!(F::Task::current(), None);
}

/// Priority is preserved.
pub fn priority_is_preserved<F: TaskFactory>(factory: &F) {
    let task = factory.task_builder().priority(7).spawn(|| {}).unwrap();
    assert_eq!(task.priority(), 7);
    task.join(Timeout::Forever).unwrap();
    assert_eq!(task.priority(), 7);
}

/// count() reflects a running entry and returns to baseline after.
pub fn count_reflects_live_tasks<F: TaskFactory>(factory: &F) {
    let _lock = acquire_count_lock();

    static INSIDE_COUNT: AtomicUsize = AtomicUsize::new(0);
    INSIDE_COUNT.store(0, Ordering::SeqCst);

    let baseline = F::Task::count();

    let task = factory
        .task_builder()
        .name("count")
        .spawn(|| {
            INSIDE_COUNT.store(F::Task::count(), Ordering::SeqCst);
        })
        .unwrap();

    task.join(Timeout::Forever).unwrap();

    // Inside the entry, at least baseline + 1.
    let inside = INSIDE_COUNT.load(Ordering::SeqCst);
    assert!(inside > baseline, "inside={inside} baseline={baseline}");

    // After join, count back to baseline.
    assert_eq!(F::Task::count(), baseline);
}

/// Task finished but handle still alive → count() already back to baseline.
pub fn finished_task_not_in_count<F: TaskFactory>(factory: &F) {
    let _lock = acquire_count_lock();

    let baseline = F::Task::count();

    let task = factory.task_builder().name("fin").spawn(|| {}).unwrap();
    task.join(Timeout::Forever).unwrap();

    // Task is finished, handle still alive.
    assert_eq!(F::Task::count(), baseline);
    drop(task);
}

// ---------------------------------------------------------------------------
// Grouped entry points
// ---------------------------------------------------------------------------

/// Core contract — both Mock and POSIX (14 tests).
pub fn run_core_contracts<F: TaskFactory>(factory: &F) {
    create_with_default_config::<F>(factory);
    accept_empty_name::<F>(factory);
    accept_max_length_name::<F>(factory);
    reject_nul_in_name::<F>(factory);
    reject_overlong_name::<F>(factory);
    reject_zero_stack::<F>(factory);
    positive_stack_size_succeeds::<F>(factory);
    spawn_runs_entry_exactly_once::<F>(factory);
    join_returns_after_task_exit::<F>(factory);
    repeated_join_returns_cached::<F>(factory);
    handle_is_nonzero::<F>(factory);
    handle_is_unique::<F>(factory);
    current_from_within_task::<F>(factory);
    current_from_main_is_none::<F>(factory);
    priority_is_preserved::<F>(factory);
    count_reflects_live_tasks::<F>(factory);
    finished_task_not_in_count::<F>(factory);
}
