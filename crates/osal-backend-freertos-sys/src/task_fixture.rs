//! Host task and EventGroup fixture for FreeRTOS task backend.
//!
//! Uses `std::thread::spawn` to simulate `xTaskCreate`, `std::sync::Mutex` +
//! `Condvar` for EventGroup wait, and `thread_local!` for TLS current context.
//! Only compiled when `test-fixture` is enabled.

#![allow(unsafe_op_in_unsafe_fn)]

extern crate std;

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::collections::HashMap;
use std::sync::{Condvar, LazyLock, Mutex};
use std::time::Duration;
use std::vec::Vec;

use super::{EventGroupHandle, TaskCreateStatus, TaskEntry};

// ---------------------------------------------------------------------------
// Global fixture state
// ---------------------------------------------------------------------------

struct FixtureEventGroupEntry {
    bits: u32,
    deleted: bool,
    waiters: usize,
    blocked_count: usize,
}

struct FixtureTaskEntry {
    #[allow(dead_code)]
    running: bool,
    started: bool,
    #[allow(dead_code)]
    deleted: bool,
}

struct FixtureTaskState {
    event_groups: HashMap<usize, FixtureEventGroupEntry>,
    tasks: HashMap<usize, FixtureTaskEntry>,
    next_id: usize,
    eg_create_count: usize,
    eg_delete_count: usize,
    task_create_count: usize,
    /// Tasks that were created before the scheduler started — they are
    /// queued and start when the scheduler transitions to Running.
    pending_tasks: Vec<(TaskEntry, u32, usize)>,
    scheduler_running: bool,
}

impl Default for FixtureTaskState {
    fn default() -> Self {
        Self {
            event_groups: HashMap::new(),
            tasks: HashMap::new(),
            next_id: 1,
            eg_create_count: 0,
            eg_delete_count: 0,
            task_create_count: 0,
            pending_tasks: Vec::new(),
            scheduler_running: true,
        }
    }
}

static TASK_FIXTURE: LazyLock<(Mutex<FixtureTaskState>, Condvar)> =
    LazyLock::new(|| (Mutex::new(FixtureTaskState::default()), Condvar::new()));

/// Count of threads currently inside a Condvar wait (for event group waits).
pub(super) static TASK_BLOCKED_COUNT: AtomicU64 = AtomicU64::new(0);

static FAIL_NEXT_EG_CREATE: AtomicBool = AtomicBool::new(false);
static FAIL_NEXT_TASK_CREATE: AtomicBool = AtomicBool::new(false);

/// Last recorded native parameters (for test assertions).
pub static LAST_STACK_WORDS: AtomicU32 = AtomicU32::new(0);
pub static LAST_NATIVE_PRIORITY: AtomicU32 = AtomicU32::new(0);

// ---------------------------------------------------------------------------
// Thread-local current-context pointer (ADR 0028 §3)
// ---------------------------------------------------------------------------

std::thread_local! {
    static CURRENT_CONTEXT: std::cell::Cell<*mut core::ffi::c_void> =
        std::cell::Cell::new(core::ptr::null_mut());
}

// ---------------------------------------------------------------------------
// Handle tagging
// ---------------------------------------------------------------------------

fn id_from_eg_handle(h: &EventGroupHandle) -> usize {
    h.raw.as_ptr() as usize
}

fn make_eg_handle(id: usize) -> EventGroupHandle {
    EventGroupHandle {
        raw: unsafe { core::ptr::NonNull::new_unchecked(id as *mut core::ffi::c_void) },
    }
}

// ---------------------------------------------------------------------------
// EventGroup fixture
// ---------------------------------------------------------------------------

pub fn event_group_create() -> Option<EventGroupHandle> {
    if FAIL_NEXT_EG_CREATE.swap(false, Ordering::Relaxed) {
        return None;
    }
    let (lock, _cvar) = &*TASK_FIXTURE;
    let mut state = lock.lock().unwrap();
    let id = state.next_id;
    state.next_id += 1;
    state.eg_create_count += 1;
    state.event_groups.insert(
        id,
        FixtureEventGroupEntry {
            bits: 0,
            deleted: false,
            waiters: 0,
            blocked_count: 0,
        },
    );
    Some(make_eg_handle(id))
}

pub fn event_group_set_bits(handle: &EventGroupHandle, bits: u32) -> u32 {
    let id = id_from_eg_handle(handle);
    let (lock, cvar) = &*TASK_FIXTURE;
    let mut state = lock.lock().unwrap();
    let entry = state
        .event_groups
        .get_mut(&id)
        .expect("event group not found");
    if entry.deleted {
        return super::EVENT_GROUP_INVALID;
    }
    entry.bits |= bits;
    // Wake all waiters — the bit is sticky (ADR 0028 §1).
    cvar.notify_all();
    super::EVENT_GROUP_OK
}

pub fn event_group_wait_bits(
    handle: &EventGroupHandle,
    bits: u32,
    _clear_on_exit: bool,
    _wait_for_all: bool,
    ticks: u64,
) -> super::EventGroupWaitStatus {
    let id = id_from_eg_handle(handle);
    let (lock, cvar) = &*TASK_FIXTURE;

    loop {
        let mut state = lock.lock().unwrap();
        let entry = state
            .event_groups
            .get_mut(&id)
            .expect("event group not found");
        if entry.deleted {
            return super::EventGroupWaitStatus::Invalid;
        }

        // Check if all requested bits are set.
        if (entry.bits & bits) == bits {
            return super::EventGroupWaitStatus::Ok;
        }

        if ticks == 0 {
            return super::EventGroupWaitStatus::Timeout;
        }

        let max_ticks = super::max_finite_delay_ticks();
        let wait_ticks = ticks.min(max_ticks);
        let timeout = Duration::from_micros((wait_ticks as u128 * 1_000_000 / 1000) as u64);

        entry.waiters += 1;
        entry.blocked_count += 1;
        TASK_BLOCKED_COUNT.fetch_add(1, Ordering::Relaxed);
        let (_state, wait_result) = cvar.wait_timeout(state, timeout).unwrap();
        TASK_BLOCKED_COUNT.fetch_sub(1, Ordering::Relaxed);
        state = _state;

        let entry = state.event_groups.get_mut(&id).unwrap();
        entry.blocked_count = entry.blocked_count.saturating_sub(1);
        entry.waiters = entry.waiters.saturating_sub(1);

        if wait_result.timed_out() {
            // Re-check after timeout.
            if (entry.bits & bits) == bits {
                return super::EventGroupWaitStatus::Ok;
            }
            return super::EventGroupWaitStatus::Timeout;
        }
        // Spurious or notified — loop back and re-check.
    }
}

pub fn event_group_delete(handle: EventGroupHandle) {
    let id = id_from_eg_handle(&handle);
    let (lock, _cvar) = &*TASK_FIXTURE;
    let mut state = lock.lock().unwrap();
    state.eg_delete_count += 1;
    if let Some(entry) = state.event_groups.get(&id) {
        assert!(!entry.deleted, "event group already deleted");
    }
    state.event_groups.remove(&id);
}

// ---------------------------------------------------------------------------
// Task fixture
// ---------------------------------------------------------------------------

pub fn task_create(
    entry: TaskEntry,
    _name: *const core::ffi::c_char,
    stack_depth_words: u32,
    parameter: *mut core::ffi::c_void,
    priority: u32,
) -> TaskCreateStatus {
    if FAIL_NEXT_TASK_CREATE.swap(false, Ordering::Relaxed) {
        return TaskCreateStatus::OutOfMemory;
    }

    LAST_STACK_WORDS.store(stack_depth_words, Ordering::Relaxed);
    LAST_NATIVE_PRIORITY.store(priority, Ordering::Relaxed);

    let (lock, _cvar) = &*TASK_FIXTURE;
    let mut state = lock.lock().unwrap();
    state.task_create_count += 1;

    let id = state.next_id;
    state.next_id += 1;
    state.tasks.insert(
        id,
        FixtureTaskEntry {
            running: false,
            started: false,
            deleted: false,
        },
    );

    if !state.scheduler_running {
        // Queue the task to start when the scheduler goes Running.
        state
            .pending_tasks
            .push((entry, id as u32, parameter as usize));
        // Mark as started so the caller can proceed.
        if let Some(t) = state.tasks.get_mut(&id) {
            t.started = true;
        }
        drop(state);
        return TaskCreateStatus::Ok;
    }

    // Scheduler is running — start the thread immediately.
    if let Some(t) = state.tasks.get_mut(&id) {
        t.started = true;
    }
    drop(state);

    let entry_copy = entry;
    let param_addr = parameter as usize;
    std::thread::spawn(move || {
        let ptr = param_addr as *mut core::ffi::c_void;
        // Set TLS context before calling the entry.
        CURRENT_CONTEXT.with(|cell| cell.set(ptr));
        unsafe { entry_copy(ptr) };
    });

    TaskCreateStatus::Ok
}

pub fn task_delete_current() -> ! {
    // In the fixture, we just kill the current thread.
    // The trampoline should call this after the entry returns.
    // We simulate the "never returns" semantics by aborting the thread.
    // Simulate vTaskDelete(NULL) — in the fixture, the trampoline
    // returns normally (see cfg gate in task.rs).  This function
    // should never be called in fixture mode.
    panic!("task_delete_current called in fixture — thread should exit");
}

pub fn task_set_current_context(ptr: *mut core::ffi::c_void) {
    CURRENT_CONTEXT.with(|cell| cell.set(ptr));
}

pub fn task_current_context() -> *mut core::ffi::c_void {
    CURRENT_CONTEXT.with(|cell| cell.get())
}

// ---------------------------------------------------------------------------
// Fixture control API
// ---------------------------------------------------------------------------

pub fn task_fixture_reset() {
    FAIL_NEXT_EG_CREATE.store(false, Ordering::Relaxed);
    FAIL_NEXT_TASK_CREATE.store(false, Ordering::Relaxed);
    LAST_STACK_WORDS.store(0, Ordering::Relaxed);
    LAST_NATIVE_PRIORITY.store(0, Ordering::Relaxed);

    let (lock, _cvar) = &*TASK_FIXTURE;
    let mut state = match lock.lock() {
        Ok(g) => g,
        Err(e) => {
            lock.clear_poison();
            e.into_inner()
        }
    };
    state.event_groups.clear();
    state.tasks.clear();
    state.next_id = 1;
    state.eg_create_count = 0;
    state.eg_delete_count = 0;
    state.task_create_count = 0;
    state.pending_tasks.clear();
    state.scheduler_running = true;

    // Assert no threads are still blocked.
    let blocked = TASK_BLOCKED_COUNT.load(Ordering::SeqCst);
    assert_eq!(
        blocked, 0,
        "task fixture reset while {blocked} thread(s) still blocked — \
         join all worker threads before reset"
    );

    // Clear TLS for the current (main) thread.
    CURRENT_CONTEXT.with(|cell| cell.set(core::ptr::null_mut()));
}

/// Notify the fixture that the scheduler state has changed.
/// When transitioning to Running, any pending tasks are started.
pub fn task_fixture_notify_scheduler_state(running: bool) {
    let (lock, _cvar) = &*TASK_FIXTURE;
    let mut state = lock.lock().unwrap();
    state.scheduler_running = running;

    if running {
        // Start all pending tasks.
        let pending: Vec<_> = state.pending_tasks.drain(..).collect();
        drop(state);

        for (entry, _id, param_addr) in pending {
            std::thread::spawn(move || {
                let ptr = param_addr as *mut core::ffi::c_void;
                CURRENT_CONTEXT.with(|cell| cell.set(ptr));
                unsafe { entry(ptr) };
            });
        }
    }
}

// --- Fault injection controls ---

pub fn task_fixture_set_fail_next_event_group_create(fail: bool) {
    FAIL_NEXT_EG_CREATE.store(fail, Ordering::Relaxed);
}

pub fn task_fixture_set_fail_next_task_create(fail: bool) {
    FAIL_NEXT_TASK_CREATE.store(fail, Ordering::Relaxed);
}

// --- Counters for test assertions ---

pub fn task_fixture_eg_create_count() -> usize {
    let (lock, _cvar) = &*TASK_FIXTURE;
    lock.lock().unwrap().eg_create_count
}

pub fn task_fixture_eg_delete_count() -> usize {
    let (lock, _cvar) = &*TASK_FIXTURE;
    lock.lock().unwrap().eg_delete_count
}

pub fn task_fixture_task_create_count() -> usize {
    let (lock, _cvar) = &*TASK_FIXTURE;
    lock.lock().unwrap().task_create_count
}

/// Number of threads currently blocked on event group waits.
pub fn task_fixture_blocked_count() -> u64 {
    TASK_BLOCKED_COUNT.load(Ordering::Relaxed)
}

/// Per-event-group blocked count.
pub fn task_fixture_eg_blocked_count(handle: &EventGroupHandle) -> usize {
    let id = id_from_eg_handle(handle);
    let (lock, _cvar) = &*TASK_FIXTURE;
    lock.lock()
        .unwrap()
        .event_groups
        .get(&id)
        .map(|e| e.blocked_count)
        .unwrap_or(0)
}
