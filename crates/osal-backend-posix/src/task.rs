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
//!
//! # P6A: TLS, live count, stack size
//!
//! A `pthread_key_t` slot (ADR 0017) provides `current()` via
//! `CurrentGuard` set in the trampoline. A `LiveTaskToken` ensures
//! `count()` reflects running entries, not handle lifecycle.
//! Stack size is passed through `pthread_attr_setstacksize`.

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::sync::atomic::{AtomicUsize, Ordering};

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::types::{ExitCode, Priority, TaskHandle};

use crate::sys::condvar::{self, PosixCondvar};
use crate::sys::mutex::{PosixMutex, PosixMutexGuard};
use crate::sys::task::PosixThread;

use osal_shared::validation;

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);

/// Number of tasks whose entry function has not yet completed.
static LIVE_COUNT: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Per-thread current-task identity via pthread TLS (ADR 0017)
// ---------------------------------------------------------------------------

use crate::sys::tls::{CurrentGuard as PthreadTlsGuard, TaskTlsSlot};

static TASK_TLS: TaskTlsSlot = TaskTlsSlot::new();

/// RAII guard that installs the current task identity into the
/// per-thread pthread TLS slot and clears it on drop.
///
/// Holds an `Arc<PosixTaskInner>` so the pointee remains alive as
/// long as the TLS slot points to it.
///
/// # Safety: field order
///
/// Rust drops struct fields in declaration order.  `_tls` must
/// appear before `_inner` so the pthread TLS slot is cleared
/// before the `Arc` keeping the pointee alive is released.
struct CurrentGuard {
    _tls: PthreadTlsGuard,
    _inner: Arc<PosixTaskInner>,
}

impl CurrentGuard {
    /// Install `inner` as the current task identity.
    ///
    /// The `Arc` pointer is stored in pthread TLS.  The guard keeps
    /// the `Arc` alive; `Drop` clears the TLS before releasing it.
    fn enter(inner: Arc<PosixTaskInner>) -> Result<Self> {
        let key = TASK_TLS.get_or_init()?;
        let value = Arc::as_ptr(&inner).cast_mut().cast::<c_void>();
        let tls = PthreadTlsGuard::enter(key, value)?;
        Ok(Self {
            _tls: tls,
            _inner: inner,
        })
    }
}

// ---------------------------------------------------------------------------
// Live task token
// ---------------------------------------------------------------------------

/// RAII guard: increments `LIVE_COUNT` on creation, decrements on drop.
struct LiveTaskToken;

impl LiveTaskToken {
    fn register() -> Self {
        LIVE_COUNT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for LiveTaskToken {
    fn drop(&mut self) {
        LIVE_COUNT.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Handle allocation
// ---------------------------------------------------------------------------

fn allocate_task_handle() -> Result<TaskHandle> {
    let raw = NEXT_HANDLE
        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
            current.checked_add(1)
        })
        .map_err(|_| Error::Overflow)?;
    TaskHandle::from_raw(raw).ok_or(Error::Overflow)
}

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
// Startup handshake
// ---------------------------------------------------------------------------

enum StartupState {
    Pending,
    Ready,
    Failed(Error),
}

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct PosixTaskInner {
    handle: TaskHandle,
    priority: Priority,
    thread: UnsafeCell<Option<PosixThread>>,
    mutex: PosixMutex,
    condvar: PosixCondvar,
    join_state: UnsafeCell<JoinState>,
    startup: UnsafeCell<StartupState>,
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
        unsafe { f(&mut *self.join_state.get()) }
    }

    fn set_finished(&self, code: ExitCode) {
        let guard = self.mutex.lock_guard().unwrap();
        self.with_state_locked(&guard, |state| {
            *state = JoinState::Finished(code);
        });
    }

    fn publish_startup(&self, result: Result<()>) {
        let _guard = self.mutex.lock_guard().unwrap();
        let startup = unsafe { &mut *self.startup.get() };
        *startup = match result {
            Ok(()) => StartupState::Ready,
            Err(e) => StartupState::Failed(e),
        };
        self.condvar
            .broadcast()
            .expect("task startup condvar broadcast failed");
    }

    fn wait_startup(inner: &Arc<PosixTaskInner>) -> Result<()> {
        let mut guard = inner.mutex.lock_guard().unwrap();
        loop {
            let startup = unsafe { &*inner.startup.get() };
            match startup {
                StartupState::Pending => {
                    inner.condvar.wait(&mut guard).unwrap();
                }
                StartupState::Ready => return Ok(()),
                StartupState::Failed(e) => return Err(e.clone()),
            }
        }
    }
}

impl Drop for PosixTaskInner {
    fn drop(&mut self) {
        // Detach the thread if it was never joined.  This satisfies the
        // contract that drop does not cancel the task, while preventing
        // a joinable pthread resource leak.
        // Note: LIVE_COUNT is NOT decremented here — that is handled
        // by LiveTaskToken in the trampoline.
        // PosixThread::Drop will detach if needed.
        unsafe {
            drop((*self.thread.get()).take());
        }
    }
}

// ---------------------------------------------------------------------------
// Task trampoline
// ---------------------------------------------------------------------------

/// Boxed payload passed to the new thread.
struct TaskStart {
    inner: Arc<PosixTaskInner>,
    entry: Option<Box<dyn FnOnce() + Send + 'static>>,
    live_token: Option<LiveTaskToken>,
}

extern "C" fn task_trampoline(arg: *mut c_void) -> *mut c_void {
    let mut start: Box<TaskStart> = unsafe { Box::from_raw(arg.cast()) };

    // Install TLS context.  If this fails, report the error via the
    // startup handshake and exit without executing the entry.
    let _context = match CurrentGuard::enter(Arc::clone(&start.inner)) {
        Ok(guard) => {
            start.inner.publish_startup(Ok(()));
            guard
        }
        Err(e) => {
            start.inner.publish_startup(Err(e));
            start.live_token.take();
            return core::ptr::null_mut();
        }
    };

    if let Some(entry) = start.entry.take() {
        entry();
    }

    // Drop live token BEFORE publishing Finished.  This guarantees
    // that any observer who sees Finished (including NoWait pollers)
    // also sees the decremented count.
    start.live_token.take();

    // Now publish completion.
    start.inner.set_finished(ExitCode::SUCCESS);

    // Wake any blocked joiners.
    let _ = start.inner.condvar.broadcast();

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
                // NoWait must never block.  Joined → cached code;
                // Finished but unreaped → return code without joining
                // (the thread will be reaped by a subsequent Forever
                // join or by Drop detach).
                let guard = self.inner.mutex.lock_guard().unwrap();
                match self.inner.with_state_locked(&guard, |state| match state {
                    JoinState::Joined(code) => Some(*code),
                    JoinState::Finished(code) => Some(*code),
                    _ => None,
                }) {
                    Some(code) => Ok(code),
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
                            return self.do_pthread_join();
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
                            return self.do_pthread_join();
                        }
                        None => {
                            self.inner.condvar.wait(&mut guard).unwrap();
                        }
                    }
                }
            }
        }
    }

    /// Call `pthread_join` on the stored thread handle, then update
    /// the state to `Joined`.  Must be called with the mutex *unlocked*.
    ///
    /// The thread handle is only removed from storage on a successful
    /// join.  On failure the handle is preserved so a subsequent joiner
    /// can retry.
    fn do_pthread_join(&self) -> Result<ExitCode> {
        let join_result = {
            let thread = unsafe { &mut *self.inner.thread.get() };
            match thread.as_mut() {
                Some(t) => t.try_join(),
                None => Ok(()),
            }
        };

        let guard = self.inner.mutex.lock_guard().unwrap();
        match join_result {
            Ok(()) => {
                // Remove the handle now that pthread_join succeeded.
                unsafe {
                    (*self.inner.thread.get()).take();
                }
                let code = self.inner.with_state_locked(&guard, |s| {
                    let code = match s {
                        JoinState::Finished(c) => *c,
                        JoinState::Joining => ExitCode::SUCCESS,
                        JoinState::Joined(c) => *c,
                        JoinState::Running => ExitCode::SUCCESS,
                    };
                    *s = JoinState::Joined(code);
                    code
                });
                let _ = self.inner.condvar.broadcast();
                Ok(code)
            }
            Err(e) => {
                // pthread_join failed — handle is still stored.
                // Restore Finished so waiters can retry.
                let _code = self.inner.with_state_locked(&guard, |s| {
                    let code = match s {
                        JoinState::Finished(c) => *c,
                        JoinState::Joining => ExitCode::SUCCESS,
                        JoinState::Joined(c) => *c,
                        _ => ExitCode::SUCCESS,
                    };
                    *s = JoinState::Finished(code);
                    code
                });
                let _ = self.inner.condvar.broadcast();
                Err(e)
            }
        }
    }
}

impl Task for PosixTask {
    fn join(&self, timeout: Timeout) -> Result<ExitCode> {
        self.join_inner(timeout)
    }

    fn handle(&self) -> TaskHandle {
        self.inner.handle
    }

    fn priority(&self) -> Priority {
        self.inner.priority
    }

    fn current() -> Option<TaskHandle> {
        let key = TASK_TLS.get()?;
        let ptr = unsafe { libc::pthread_getspecific(key) };
        if ptr.is_null() {
            return None;
        }
        // Safety: ptr was set by CurrentGuard::enter, which stores
        // Arc::as_ptr(&PosixTaskInner).  CurrentGuard holds the Arc,
        // so the pointee is alive as long as the guard is alive.
        // The guard is held in the trampoline's stack, so it is alive
        // for the duration of entry execution.
        let inner = unsafe { &*ptr.cast::<PosixTaskInner>() };
        Some(inner.handle)
    }

    fn count() -> usize {
        LIVE_COUNT.load(Ordering::SeqCst)
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
        self.stack_size = bytes;
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
        validation::validate_task_config(&self.name, self.stack_size)?;

        // Pre-initialise the TLS key on the parent thread so the
        // child only needs to call pthread_setspecific.
        let _tls_key = TASK_TLS.get_or_init()?;

        // All fallible resources first.
        let mutex = PosixMutex::new()?;
        let condvar = PosixCondvar::new()?;
        let handle = allocate_task_handle()?;

        let inner = Arc::new(PosixTaskInner {
            handle,
            priority: self.priority,
            thread: UnsafeCell::new(None),
            mutex,
            condvar,
            join_state: UnsafeCell::new(JoinState::Running),
            startup: UnsafeCell::new(StartupState::Pending),
        });

        let start = Box::new(TaskStart {
            inner: Arc::clone(&inner),
            entry: Some(Box::new(entry)),
            live_token: Some(LiveTaskToken::register()),
        });

        let raw_start = Box::into_raw(start).cast::<c_void>();

        let cfg = crate::sys::task::PosixThreadConfig {
            stack_size: self.stack_size,
        };
        let thread = match PosixThread::spawn(&cfg, task_trampoline, raw_start) {
            Ok(t) => t,
            Err(e) => {
                // pthread_create failed — reclaim the Box.
                // LiveTaskToken drop will roll back the count.
                unsafe {
                    drop(Box::from_raw(raw_start.cast::<TaskStart>()));
                }
                return Err(e);
            }
        };

        // Store the thread handle so join() can pick it up.
        unsafe {
            *inner.thread.get() = Some(thread);
        }

        // Wait for the child to install TLS and publish Ready.
        // If the child reports a startup failure, join it and
        // return the error.
        match PosixTaskInner::wait_startup(&inner) {
            Ok(()) => Ok(PosixTask { inner }),
            Err(e) => {
                // Join the failed child thread.  The child has
                // already published Failed and is about to exit;
                // join must succeed under normal operation.
                let thread = unsafe { &mut *inner.thread.get() };
                let mut t = thread
                    .take()
                    .expect("failed task startup must retain its pthread handle");
                t.try_join()
                    .expect("failed task startup pthread must be joinable");
                Err(e)
            }
        }
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
