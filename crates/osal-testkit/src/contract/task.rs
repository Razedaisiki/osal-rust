//! Contract tests for the [`Task`] and [`TaskBuilder`] traits.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md#8-task-contract`.
//!
//! # Note on concurrency
//!
//! These are smoke tests that verify basic task lifecycle. Cross-task
//! concurrency and wait/wakeup tests require a cooperative scheduler
//! and live in separate modules (`contract/mutex.rs` concurrency group,
//! `contract/queue.rs` wait group).

use core::sync::atomic::{AtomicBool, Ordering};

use osal_api::time::Timeout;
use osal_api::traits::task::{Task as _, TaskBuilder as _};

use crate::factory::TaskFactory;

// ---------------------------------------------------------------------------
// Smoke tests
// ---------------------------------------------------------------------------

/// `Task::count()` returns a valid count.
pub fn task_count_is_callable<F: TaskFactory>(_factory: &F) {
    let _count = F::Task::count();
}

/// `Task::current()` returns a valid handle.
pub fn current_returns_valid_handle<F: TaskFactory>(_factory: &F) {
    let _handle = F::Task::current();
}

/// Spawned task runs its entry function exactly once.
pub fn spawn_runs_entry_once<F: TaskFactory>(factory: &F) {
    static FLAG: AtomicBool = AtomicBool::new(false);
    FLAG.store(false, Ordering::SeqCst);

    let task = factory
        .task_builder()
        .name("smoke")
        .spawn(|| {
            FLAG.store(true, Ordering::SeqCst);
        })
        .unwrap();

    task.join(Timeout::Forever).unwrap();
    assert!(FLAG.load(Ordering::SeqCst));
}

/// `join()` returns after the task exits.
pub fn join_returns_after_task_exit<F: TaskFactory>(factory: &F) {
    let task = factory
        .task_builder()
        .name("joiner")
        .spawn(|| { /* immediate return */ })
        .unwrap();

    let result = task.join(Timeout::Forever);
    assert!(result.is_ok());
}

/// `join()` succeeds immediately for an already-exited task.
pub fn join_after_exit_returns_immediately<F: TaskFactory>(factory: &F) {
    let task = factory
        .task_builder()
        .name("quick")
        .spawn(|| {})
        .unwrap();

    // First join — wait for exit.
    task.join(Timeout::Forever).unwrap();
    // Second join — should return immediately with cached exit code.
    task.join(Timeout::NoWait).unwrap();
}

// ---------------------------------------------------------------------------
// Grouped entry point
// ---------------------------------------------------------------------------

/// Basic task lifecycle smoke tests.
pub fn run_smoke_contracts<F: TaskFactory>(factory: &F) {
    task_count_is_callable::<F>(factory);
    current_returns_valid_handle::<F>(factory);
    spawn_runs_entry_once::<F>(factory);
    join_returns_after_task_exit::<F>(factory);
    join_after_exit_returns_immediately::<F>(factory);
}
