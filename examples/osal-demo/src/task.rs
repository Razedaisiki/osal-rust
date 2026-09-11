//! Task demo — spawn a worker, observe its identity, join it.

use alloc::sync::Arc;
use core::fmt;
use core::sync::atomic::{AtomicBool, Ordering};

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};
use crate::runtime::with_runtime;

/// Outcome of the task demo.
pub struct TaskReport {
    /// Exit code returned by the join.
    pub exit_code: u32,
    /// Whether the worker body executed.
    pub worker_ran: bool,
    /// Whether `Task::current()` resolved inside the worker.
    pub current_identity_seen: bool,
}

impl fmt::Display for TaskReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[task] exit_code={}\n[task] worker_ran={}\n[task] current_identity_seen={}",
            self.exit_code, self.worker_ran, self.current_identity_seen
        )
    }
}

/// Run the task demo against the active backend.
pub fn run() -> DemoResult<TaskReport> {
    with_runtime(|| {
        let worker_ran = Arc::new(AtomicBool::new(false));
        let identity_seen = Arc::new(AtomicBool::new(false));

        let ran = Arc::clone(&worker_ran);
        let seen = Arc::clone(&identity_seen);

        let task = TaskBuilder::new()
            .name("worker")
            .priority(1)
            .spawn(move || {
                if Task::current().is_some() {
                    seen.store(true, Ordering::Release);
                }
                ran.store(true, Ordering::Release);
            })
            .demo_context("task.spawn")?;

        let exit = task.join(Timeout::Forever).demo_context("task.join")?;

        if exit != ExitCode::SUCCESS {
            return Err(DemoError::Check("task exit code was not SUCCESS"));
        }
        if !worker_ran.load(Ordering::Acquire) {
            return Err(DemoError::Check("task entry did not run"));
        }
        if !identity_seen.load(Ordering::Acquire) {
            return Err(DemoError::Check(
                "Task::current() returned None inside task",
            ));
        }

        drop(task);

        Ok(TaskReport {
            exit_code: exit.code(),
            worker_ran: true,
            current_identity_seen: true,
        })
    })
}
