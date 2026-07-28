//! FreeRTOS task implementation.
//!
//! Tasks are launched via `FreeRtosTaskBuilder::spawn` using `xTaskCreate`.
//! Completion is signalled via a native EventGroup with a sticky completion
//! bit (ADR 0028 §1).  TLS identity for `current()` uses the FreeRTOS
//! thread-local-storage pointer slot (ADR 0028 §3).
//!
//! # Completion state machine
//!
//! ```text
//! Running ──(task entry returns)──→ Finished(code)
//! ```
//!
//! No `Joining` state — EventGroup is multi-consumer (ADR 0028 §2).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::sync::Arc;
use core::ffi::c_void;
use core::sync::atomic::{AtomicU32, AtomicU8, AtomicUsize, Ordering};

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::types::{ExitCode, Priority, TaskHandle};
use osal_shared::runtime::RuntimeLease;
use osal_shared::validation;

use crate::wait::{self, WaitBudget, WaitOutcome};
use osal_backend_freertos_sys as sys;

// ---------------------------------------------------------------------------
// Completion state constants
// ---------------------------------------------------------------------------

const COMPLETION_RUNNING: u8 = 0;
const COMPLETION_FINISHED: u8 = 1;
const TASK_COMPLETED_BIT: u32 = 1;

// ---------------------------------------------------------------------------
// Stack bytes → words conversion (ADR 0028 §8)
// ---------------------------------------------------------------------------

/// Convert a stack size in bytes to FreeRTOS stack depth in words.
///
/// Rounds up to the next word boundary, enforces the platform minimum,
/// and checks for overflow.
fn stack_bytes_to_words(
    bytes: usize,
    word_size: usize,
    minimal_words: usize,
    max_words: usize,
) -> Result<usize> {
    if word_size == 0 {
        return Err(Error::Internal("stack word size is zero"));
    }

    let rounded = bytes
        .checked_add(word_size - 1)
        .ok_or(Error::Overflow)?
        / word_size;

    let words = rounded.max(minimal_words);

    if words > max_words {
        return Err(Error::Overflow);
    }

    Ok(words)
}

// ---------------------------------------------------------------------------
// Priority mapping (ADR 0028 §9)
// ---------------------------------------------------------------------------

/// Map a requested priority to a native FreeRTOS priority.
///
/// Saturates to `max_priorities - 1`.  The public `priority()` method
/// continues to report the requested value.
fn map_native_priority(requested: Priority, max_priorities: u32) -> u32 {
    if max_priorities <= 1 {
        return 0;
    }
    (requested as u32).min(max_priorities - 1)
}

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

static NEXT_HANDLE: AtomicUsize = AtomicUsize::new(1);

/// Number of OSAL tasks whose entry function has not yet completed.
static LIVE_COUNT: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Live task token
// ---------------------------------------------------------------------------

/// RAII guard: increments `LIVE_COUNT` on creation, decrements on drop.
struct LiveTaskToken;

impl LiveTaskToken {
    fn acquire() -> Self {
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
// TaskIdentity — per-task metadata, Arc-shared
// ---------------------------------------------------------------------------

struct TaskIdentity {
    handle: TaskHandle,
    requested_priority: Priority,
    /// Held for the lifetime of the Task handle (ADR 0019 §6).
    _runtime_lease: RuntimeLease<'static>,
}

// Safety: read-only after construction.
unsafe impl Send for TaskIdentity {}
unsafe impl Sync for TaskIdentity {}

// ---------------------------------------------------------------------------
// TaskCompletion — Arc-shared completion state
// ---------------------------------------------------------------------------

struct TaskCompletion {
    /// Native FreeRTOS EventGroup used for join signalling.
    event_group: Option<sys::EventGroupHandle>,

    /// Completion state: `COMPLETION_RUNNING` → `COMPLETION_FINISHED`.
    state: AtomicU8,

    /// Cached exit code, written before `state` transitions to Finished.
    exit_code: AtomicU32,
}

impl TaskCompletion {
    /// Check whether the task has finished and return the cached exit code.
    fn finished_code(&self) -> Option<ExitCode> {
        if self.state.load(Ordering::Acquire) == COMPLETION_FINISHED {
            let raw = self.exit_code.load(Ordering::Acquire);
            Some(ExitCode::new(raw))
        } else {
            None
        }
    }

    /// Publish completion (called from the trampoline, ADR 0028 §6).
    fn publish(&self, code: ExitCode) {
        // 1. Write the exit code.
        self.exit_code.store(code.code(), Ordering::Release);

        // 2. Publish Finished state.
        self.state
            .store(COMPLETION_FINISHED, Ordering::Release);

        // 3. Set the EventGroup completion bit — wakes all joiners.
        if let Some(ref eg) = self.event_group {
            sys::event_group_set_bits(eg, TASK_COMPLETED_BIT);
        }
    }
}

impl Drop for TaskCompletion {
    fn drop(&mut self) {
        if let Some(h) = self.event_group.take() {
            sys::event_group_delete(h);
        }
    }
}

// Safety: atomic state + EventGroup handle is Send+Sync.
unsafe impl Send for TaskCompletion {}
unsafe impl Sync for TaskCompletion {}

// ---------------------------------------------------------------------------
// Task start payload (ADR 0028 §5)
// ---------------------------------------------------------------------------

struct TaskStart<F> {
    identity: Arc<TaskIdentity>,
    completion: Arc<TaskCompletion>,
    entry: Option<F>,
}

// ---------------------------------------------------------------------------
// Generic trampoline (ADR 0028 §5)
// ---------------------------------------------------------------------------

unsafe extern "C" fn task_trampoline<F>(parameter: *mut c_void)
where
    F: FnOnce() + Send + 'static,
{
    let mut start: Box<TaskStart<F>> = Box::from_raw(parameter.cast());

    // 1. Install TLS current identity.
    sys::task_set_current_context(
        Arc::as_ptr(&start.identity).cast_mut().cast::<c_void>(),
    );

    // 2. Register live count.
    let live_token = LiveTaskToken::acquire();

    // 3. Execute the user entry.
    let entry = start
        .entry
        .take()
        .expect("FreeRTOS task entry already consumed");
    entry();

    // 4. Drop live token (count decremented — ADR 0028 §6 step 2).
    drop(live_token);

    // 5. Clear TLS identity (ADR 0028 §6 step 3).
    sys::task_set_current_context(core::ptr::null_mut());

    // 6-7. Publish completion (ADR 0028 §6 steps 4-6).
    start.completion.publish(ExitCode::SUCCESS);

    // 8. Release task-owned Arcs.
    drop(start);

    // 9. Self-delete (native) or return (fixture).
    // vTaskDelete(NULL) never returns on real FreeRTOS.
    // In the test fixture, the thread simply exits its closure.
    #[cfg(not(feature = "test-fixture"))]
    {
        sys::task_delete_current();
    }
}

// ---------------------------------------------------------------------------
// FreeRtosTaskBuilder
// ---------------------------------------------------------------------------

/// Builder for configuring and spawning a [`FreeRtosTask`].
pub struct FreeRtosTaskBuilder {
    name: String,
    stack_size: usize,
    priority: Priority,
}

impl TaskBuilder for FreeRtosTaskBuilder {
    type Task = FreeRtosTask;

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
        // 1. Validate parameters first (ADR 0019 §6).
        validation::validate_task_config(&self.name, self.stack_size)?;

        // 2. Acquire a runtime lease.
        let runtime = crate::runtime::acquire_object()?;

        // 3. Probe capabilities for stack conversion and priority mapping.
        let caps = crate::runtime::capabilities()
            .expect("spawn requires osal::initialize()");

        let words = stack_bytes_to_words(
            self.stack_size,
            caps.stack_word_size as usize,
            caps.minimal_stack_depth_words as usize,
            caps.max_stack_depth_words as usize,
        )?;

        let native_priority = map_native_priority(self.priority, caps.max_priorities);

        // 4. Create EventGroup (fallible — before the handle alloc).
        let event_group =
            sys::event_group_create().ok_or(Error::OutOfMemory)?;

        // 5. Allocate OSAL TaskHandle.
        let handle = allocate_task_handle()?;

        // 6. Construct identity and completion Arcs.
        let identity = Arc::new(TaskIdentity {
            handle,
            requested_priority: self.priority,
            _runtime_lease: runtime,
        });

        let completion = Arc::new(TaskCompletion {
            event_group: Some(event_group),
            state: AtomicU8::new(COMPLETION_RUNNING),
            exit_code: AtomicU32::new(0),
        });

        // 7. Box up the start payload.
        let start = Box::new(TaskStart {
            identity: Arc::clone(&identity),
            completion: Arc::clone(&completion),
            entry: Some(entry),
        });

        let raw_start = Box::into_raw(start).cast::<c_void>();

        // 8. Prepare C-compatible name (NUL-terminated, truncated).
        let mut name_buf = [0u8; 32]; // max 31 chars + NUL
        let name_len = self.name.len().min(caps.max_task_name_len as usize - 1).min(31);
        name_buf[..name_len].copy_from_slice(&self.name.as_bytes()[..name_len]);
        // name_buf is zero-initialized, so NUL is guaranteed at name_len.

        // 9. Call xTaskCreate.
        // Safety: all shared state is fully constructed before this call.
        // If creation fails, we reclaim the Box and roll back.
        let status = unsafe {
            sys::task_create(
                task_trampoline::<F>,
                name_buf.as_ptr().cast::<core::ffi::c_char>(),
                words as u32,
                raw_start,
                native_priority,
            )
        };

        match status {
            sys::TaskCreateStatus::Ok => Ok(FreeRtosTask {
                identity,
                completion,
            }),
            sys::TaskCreateStatus::OutOfMemory => {
                // Reclaim the Box — LiveTaskToken was never registered
                // (only the trampoline does that), so no count leak.
                unsafe {
                    drop(Box::from_raw(raw_start.cast::<TaskStart<F>>()));
                }
                // identity and completion Arcs drop here —
                // RuntimeLease and EventGroup are cleaned up.
                Err(Error::OutOfMemory)
            }
            sys::TaskCreateStatus::Invalid => {
                unsafe {
                    drop(Box::from_raw(raw_start.cast::<TaskStart<F>>()));
                }
                Err(Error::InvalidParameter)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// FreeRtosTask
// ---------------------------------------------------------------------------

/// A FreeRTOS task handle.
///
/// Created by [`FreeRtosTaskBuilder::spawn`].
#[derive(Clone)]
pub struct FreeRtosTask {
    identity: Arc<TaskIdentity>,
    completion: Arc<TaskCompletion>,
}

impl Task for FreeRtosTask {
    fn join(&self, timeout: Timeout) -> Result<ExitCode> {
        // 1. Self-join guard (ADR 0028 §7).
        if Self::current() == Some(self.handle()) {
            return Err(Error::Busy);
        }

        // 2. Fast path — check cached state.
        if let Some(code) = self.completion.finished_code() {
            return Ok(code);
        }

        // 3. Non-blocking variants.
        match timeout {
            Timeout::NoWait | Timeout::After(core::time::Duration::ZERO) => {
                return Err(Error::Timeout);
            }
            _ => {}
        }

        // 4. Blocking — prepare budget.
        let mut budget = WaitBudget::new(timeout);
        budget.prepare_blocking()?;

        let event_group = self
            .completion
            .event_group
            .as_ref()
            .expect("task completion event group already deleted");

        loop {
            // Re-check after any wake.
            if let Some(code) = self.completion.finished_code() {
                return Ok(code);
            }

            match budget.wait_once(|ticks| {
                let status = sys::event_group_wait_bits(
                    event_group,
                    TASK_COMPLETED_BIT,
                    false, // clear_on_exit
                    true,  // wait_for_all
                    ticks,
                );
                match status {
                    sys::EventGroupWaitStatus::Ok => sys::TakeStatus::Acquired,
                    sys::EventGroupWaitStatus::Timeout => sys::TakeStatus::Timeout,
                    sys::EventGroupWaitStatus::Invalid => sys::TakeStatus::Invalid,
                }
            })? {
                WaitOutcome::Acquired => {
                    // EventGroup bit was set — Finished must be visible.
                    if let Some(code) = self.completion.finished_code() {
                        return Ok(code);
                    }
                    panic!("completion bit set before Finished state");
                }
                WaitOutcome::Unavailable => {
                    // Timeout — check one last time.
                    if let Some(code) = self.completion.finished_code() {
                        return Ok(code);
                    }
                    return Err(Error::Timeout);
                }
            }
        }
    }

    fn handle(&self) -> TaskHandle {
        self.identity.handle
    }

    fn priority(&self) -> Priority {
        self.identity.requested_priority
    }

    fn current() -> Option<TaskHandle> {
        let ptr = sys::task_current_context();
        if ptr.is_null() {
            return None;
        }
        // Safety: ptr was set by the trampoline, which holds an
        // Arc<TaskIdentity> keeping the pointee alive for the
        // duration of the entry function.
        let identity = unsafe { &*ptr.cast::<TaskIdentity>() };
        Some(identity.handle)
    }

    fn count() -> usize {
        LIVE_COUNT.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

#[cfg(feature = "testkit")]
pub struct FreeRtosTaskFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::TaskFactory for FreeRtosTaskFactory {
    type Task = FreeRtosTask;
    type TaskBuilder = FreeRtosTaskBuilder;

    fn task_builder(&self) -> Self::TaskBuilder {
        FreeRtosTaskBuilder::new()
    }
}
