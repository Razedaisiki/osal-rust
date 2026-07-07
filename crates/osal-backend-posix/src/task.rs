//! POSIX task implementation.
//!
//! Tasks are launched via [`PosixTaskBuilder::spawn`] using
//! `pthread_create`. Join semantics are implemented through a backend
//! completion-state machine because `pthread_join` can only be called
//! once, but the OSAL [`Task`] trait requires repeated join to return
//! the cached exit code.
//!
//! # Completion state machine
//!
//! ```text
//! Running ──(task thread exits)──→ Finished(code)
//!                                      │
//!                           (first joiner)
//!                                      ↓
//!                                  Joining
//!                                      │
//!                           (pthread_join done)
//!                                      ↓
//!                                  Joined(code)
//! ```
//!
//! Timeout join is implemented through `pthread_cond_timedwait` on
//! the completion state rather than non-portable `pthread_timedjoin_np`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};
use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::types::{ExitCode, Handle, Priority};

use crate::sys::condvar::{self, PosixCondvar};
use crate::sys::mutex::{PosixMutex, PosixMutexGuard};
use crate::sys::task::PosixThread;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);
static TASK_COUNT: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Completion state
// ---------------------------------------------------------------------------

/// Internal join-state machine (see module-level doc).
enum JoinState {
    /// Task thread is still executing.
    Running,
    /// Task thread exited with this code; `pthread_join` not yet called.
    Finished(ExitCode),
    /// One caller is inside `pthread_join`; others wait.
    Joining,
    /// `pthread_join` completed; exit code is cached.
    Joined(ExitCode),
}

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct PosixTaskInner {
    handle: Handle,
    priority: Priority,
    thread: UnsafeCell<Option<PosixThread>>,
    mutex: PosixMutex,
    condvar: PosixCondvar,
    state: UnsafeCell<JoinState>,
}

// Safety: all state access is guarded by the mutex.
unsafe impl Send for PosixTaskInner {}
unsafe impl Sync for PosixTaskInner {}

impl PosixTaskInner {
    fn with_state_locked<R>(
        &self,
        _guard: &PosixMutexGuard<'_>,
        f: impl FnOnce(&mut JoinState) -> R,
    ) -> R {
        // Safety: the caller holds self.mutex.
        unsafe { f(&mut *self.state.get()) }
    }
}

impl Drop for PosixTaskInner {
    fn drop(&mut self) {
        TASK_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Task trampoline
// ---------------------------------------------------------------------------

/// Boxed payload passed to the new thread.
struct TaskStart {
    inner: Arc<PosixTaskInner>,
    entry: Option<Box<dyn FnOnce() + Send + 'static>>,
}

impl TaskStart {
    fn mark_finished(&self, code: ExitCode) {
        let guard = self.inner.mutex.lock_guard().unwrap();
        self.inner.with_state_locked(&guard, |state| {
            *state = JoinState::Finished(code);
        });
        let _ = self.inner.condvar.broadcast();
    }
}

extern "C" fn task_trampoline(arg: *mut c_void) -> *mut c_void {
    let mut start: Box<TaskStart> = unsafe { Box::from_raw(arg.cast()) };

    if let Some(entry) = start.entry.take() {
        entry();
    }

    start.mark_finished(ExitCode::SUCCESS);

    core::ptr::null_mut()
}

// ---------------------------------------------------------------------------
// PosixTask
// ---------------------------------------------------------------------------

/// A POSIX task handle.
///
/// Created by [`PosixTaskBuilder::spawn`].
#[derive(Clone)]
pub struct PosixTask {
    inner: Arc<PosixTaskInner>,
}

impl PosixTask {
    /// Execute the join state machine (see module-level doc).
    fn join_inner(&self, timeout: Timeout) -> Result<ExitCode> {
        match timeout {
            Timeout::NoWait => {
                // Check state under lock.  Use a raw lock/with_state
                // approach so we can transition Finished → Joining
                // in the same critical section.
                let guard = self.inner.mutex.lock_guard().unwrap();
                let needs_join = self.inner.with_state_locked(&guard, |state| match state {
                    JoinState::Joined(code) => Some(Ok(*code)),
                    JoinState::Finished(_code) => {
                        *state = JoinState::Joining;
                        Some(Err(()))
                    }
                    _ => None,
                });

                match needs_join {
                    Some(Ok(code)) => Ok(code),
                    Some(Err(())) => {
                        drop(guard);
                        self.do_pthread_join();
                        let g2 = self.inner.mutex.lock_guard().unwrap();
                        self.inner.with_state_locked(&g2, |s| match s {
                            JoinState::Joined(c) => Ok(*c),
                            _ => Ok(ExitCode::SUCCESS),
                        })
                    }
                    None => Err(Error::Timeout),
                }
            }
            Timeout::After(d) => {
                let deadline = condvar::abs_deadline(d);
                let mut guard = self.inner.mutex.lock_guard().unwrap();

                loop {
                    let done = self.inner.with_state_locked(&guard, |s| match s {
                        JoinState::Joined(code) => Some(Ok(*code)),
                        JoinState::Finished(_code) => {
                            *s = JoinState::Joining;
                            Some(Err(()))
                        }
                        JoinState::Joining => None,
                        JoinState::Running => None,
                    });

                    match done {
                        Some(Ok(code)) => return Ok(code),
                        Some(Err(())) => {
                            drop(guard);
                            self.do_pthread_join();
                            let g2 = self.inner.mutex.lock_guard().unwrap();
                            return self.inner.with_state_locked(&g2, |s| match s {
                                JoinState::Joined(c) => Ok(*c),
                                _ => Ok(ExitCode::SUCCESS),
                            });
                        }
                        None => match self.inner.condvar.timed_wait(&mut guard, &deadline) {
                            Err(Error::Timeout) => return Err(Error::Timeout),
                            Err(e) => return Err(e),
                            Ok(()) => {}
                        },
                    }
                }
            }
            Timeout::Forever => {
                let mut guard = self.inner.mutex.lock_guard().unwrap();

                loop {
                    let done = self.inner.with_state_locked(&guard, |s| match s {
                        JoinState::Joined(code) => Some(Ok(*code)),
                        JoinState::Finished(_code) => {
                            *s = JoinState::Joining;
                            Some(Err(()))
                        }
                        JoinState::Joining => None,
                        JoinState::Running => None,
                    });

                    match done {
                        Some(Ok(code)) => return Ok(code),
                        Some(Err(())) => {
                            drop(guard);
                            self.do_pthread_join();
                            let g2 = self.inner.mutex.lock_guard().unwrap();
                            return self.inner.with_state_locked(&g2, |s| match s {
                                JoinState::Joined(c) => Ok(*c),
                                _ => Ok(ExitCode::SUCCESS),
                            });
                        }
                        None => {
                            self.inner.condvar.wait(&mut guard).unwrap();
                        }
                    }
                }
            }
        }
    }

    /// Take the thread handle and call `pthread_join`, then update
    /// the state to `Joined`.  Must be called with the mutex *unlocked*.
    fn do_pthread_join(&self) {
        let thread = unsafe { &mut *self.inner.thread.get() };
        if let Some(t) = thread.take() {
            let _ = t.join();
        }
        let guard = self.inner.mutex.lock_guard().unwrap();
        let code = self.inner.with_state_locked(&guard, |s| {
            let code = if let JoinState::Finished(c) = s {
                *c
            } else {
                ExitCode::SUCCESS
            };
            *s = JoinState::Joined(code);
            code
        });
        let _ = self.inner.condvar.broadcast();
        let _ = code;
    }
}

impl Task for PosixTask {
    fn join(&self, timeout: Timeout) -> Result<ExitCode> {
        self.join_inner(timeout)
    }

    fn handle(&self) -> Handle {
        self.inner.handle
    }

    fn priority(&self) -> Priority {
        self.inner.priority
    }

    fn current() -> Handle {
        // POSIX backend does not track OSAL-task identity per thread.
        0
    }

    fn count() -> usize {
        TASK_COUNT.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// PosixTaskBuilder
// ---------------------------------------------------------------------------

/// Builder for configuring and spawning a [`PosixTask`].
pub struct PosixTaskBuilder {
    name: String,
    stack_size: usize,
    priority: Priority,
}

impl TaskBuilder for PosixTaskBuilder {
    type Task = PosixTask;

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
        self.stack_size = bytes.max(512);
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

        let inner = Arc::new(PosixTaskInner {
            handle,
            priority: self.priority,
            thread: UnsafeCell::new(None),
            mutex: PosixMutex::new()?,
            condvar: PosixCondvar::new()?,
            state: UnsafeCell::new(JoinState::Running),
        });

        let start = Box::new(TaskStart {
            inner: Arc::clone(&inner),
            entry: Some(Box::new(entry)),
        });

        let thread = PosixThread::spawn(task_trampoline, Box::into_raw(start).cast::<c_void>())?;

        // Store the thread handle so join() can pick it up.
        unsafe {
            *inner.thread.get() = Some(thread);
        }

        Ok(PosixTask { inner })
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

#[cfg(feature = "testkit")]
pub struct PosixTaskFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::TaskFactory for PosixTaskFactory {
    type Task = PosixTask;
    type TaskBuilder = PosixTaskBuilder;

    fn task_builder(&self) -> Self::TaskBuilder {
        PosixTaskBuilder::new()
    }
}
