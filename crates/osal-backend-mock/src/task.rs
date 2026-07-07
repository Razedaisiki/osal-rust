//! Mock task implementation.
//!
//! Tasks execute synchronously in `spawn()` — the entry function runs
//! to completion before `spawn()` returns. This is sufficient for
//! deterministic contract smoke tests. A cooperative mock scheduler
//! is deferred to a later phase.

use alloc::string::String;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicUsize, Ordering};

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::types::{ExitCode, Handle, Priority};

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);
static TASK_COUNT: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Inner — shared via Arc
// ---------------------------------------------------------------------------

struct MockTaskInner {
    handle: Handle,
    priority: Priority,
    exit_code: ExitCode,
}

impl Drop for MockTaskInner {
    fn drop(&mut self) {
        TASK_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// MockTask
// ---------------------------------------------------------------------------

/// A mock task handle.
///
/// The entry function is executed synchronously in
/// [`MockTaskBuilder::spawn`]. `join()` returns the cached
/// `ExitCode::SUCCESS` immediately.
#[derive(Clone)]
pub struct MockTask {
    inner: Arc<MockTaskInner>,
}

impl Task for MockTask {
    fn join(&self, _timeout: Timeout) -> Result<ExitCode> {
        Ok(self.inner.exit_code)
    }

    fn handle(&self) -> Handle {
        self.inner.handle
    }

    fn priority(&self) -> Priority {
        self.inner.priority
    }

    fn current() -> Handle {
        0
    }

    fn count() -> usize {
        TASK_COUNT.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// MockTaskBuilder
// ---------------------------------------------------------------------------

/// Builder for configuring and spawning a [`MockTask`].
pub struct MockTaskBuilder {
    name: String,
    stack_size: usize,
    priority: Priority,
}

impl TaskBuilder for MockTaskBuilder {
    type Task = MockTask;

    fn new() -> Self {
        Self {
            name: String::new(),
            stack_size: 4096,
            priority: 1,
        }
    }

    fn name(mut self, name: &str) -> Self {
        self.name.clear();
        self.name.push_str(name);
        self
    }

    fn stack_size(mut self, bytes: usize) -> Self {
        self.stack_size = bytes.max(1);
        self
    }

    fn priority(mut self, prio: Priority) -> Self {
        self.priority = prio;
        self
    }

    fn spawn<F>(self, entry: F) -> Result<Self::Task>
    where
        F: FnOnce() + Send + 'static,
    {
        if self.name.as_bytes().contains(&0) {
            return Err(Error::InvalidParameter);
        }

        let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
        TASK_COUNT.fetch_add(1, Ordering::SeqCst);

        // Synchronous execution — runs to completion immediately.
        entry();

        Ok(MockTask {
            inner: Arc::new(MockTaskInner {
                handle,
                priority: self.priority,
                exit_code: ExitCode::SUCCESS,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

#[cfg(feature = "testkit")]
pub struct MockTaskFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::TaskFactory for MockTaskFactory {
    type Task = MockTask;
    type TaskBuilder = MockTaskBuilder;

    fn task_builder(&self) -> Self::TaskBuilder {
        MockTaskBuilder::new()
    }
}
