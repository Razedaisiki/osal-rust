//! Task real-kernel contracts (P7G Step 4D).
//!
//! Commit 2 — Builder, identity, mapping, and live count cases (1–8).

use alloc::sync::Arc;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::types::{ExitCode, TaskHandle};
use osal_api::traits::task::Task as _;
use osal_api::traits::task::TaskBuilder as _;
use osal_backend_freertos_sys as sys;
use osal_backend_freertos::task::{FreeRtosTask, FreeRtosTaskBuilder};

use crate::harness::{self, CaseState, PHASE_DONE, PHASE_EXITING};

// ------------------------------------------------------------------
// Diagnostics FFI — C observer getters (only linked in suite-task).
// ------------------------------------------------------------------
unsafe extern "C" {
    fn osal_test_diag_task_create_attempts() -> u32;
    fn osal_test_diag_task_create_successes() -> u32;
    fn osal_test_diag_event_group_creates() -> u32;
    fn osal_test_diag_event_group_deletes() -> u32;
    fn osal_test_diag_last_stack_words() -> u32;
    fn osal_test_diag_last_native_priority() -> u32;
    fn osal_test_diag_last_name_len() -> u32;
    fn osal_test_diag_reset();
    fn osal_test_expect_malloc_failure();
    fn osal_test_expected_malloc_failure_consumed() -> u32;
    fn osal_test_clear_expected_malloc_failure();
    fn osal_test_task_stack_hwm() -> u32;
    fn osal_test_scheduler_suspend();
    fn osal_test_scheduler_resume();
    fn non_osal_context_helper(context: *mut c_void);
}

fn diag_task_create_attempts() -> u32 { unsafe { osal_test_diag_task_create_attempts() } }
fn diag_task_create_successes() -> u32 { unsafe { osal_test_diag_task_create_successes() } }
fn diag_event_group_creates() -> u32 { unsafe { osal_test_diag_event_group_creates() } }
fn diag_event_group_deletes() -> u32 { unsafe { osal_test_diag_event_group_deletes() } }
fn diag_last_stack_words() -> u32 { unsafe { osal_test_diag_last_stack_words() } }
fn diag_last_native_priority() -> u32 { unsafe { osal_test_diag_last_native_priority() } }
fn diag_last_name_len() -> u32 { unsafe { osal_test_diag_last_name_len() } }
fn diag_reset() { unsafe { osal_test_diag_reset() } }
fn task_stack_hwm() -> u32 { unsafe { osal_test_task_stack_hwm() } }

struct DiagSnapshot {
    task_create_attempts: u32,
    task_create_successes: u32,
    event_group_creates: u32,
    event_group_deletes: u32,
}

fn read_diag() -> DiagSnapshot {
    DiagSnapshot {
        task_create_attempts: diag_task_create_attempts(),
        task_create_successes: diag_task_create_successes(),
        event_group_creates: diag_event_group_creates(),
        event_group_deletes: diag_event_group_deletes(),
    }
}

fn diag_create_attempt_delta(after: &DiagSnapshot, before: &DiagSnapshot) -> u32 {
    after.task_create_attempts.wrapping_sub(before.task_create_attempts)
}
fn diag_create_success_delta(after: &DiagSnapshot, before: &DiagSnapshot) -> u32 {
    after.task_create_successes.wrapping_sub(before.task_create_successes)
}
fn diag_eg_create_delta(after: &DiagSnapshot, before: &DiagSnapshot) -> u32 {
    after.event_group_creates.wrapping_sub(before.event_group_creates)
}
fn diag_eg_delete_delta(after: &DiagSnapshot, before: &DiagSnapshot) -> u32 {
    after.event_group_deletes.wrapping_sub(before.event_group_deletes)
}

// ------------------------------------------------------------------
// Error codes.
// ------------------------------------------------------------------
#[repr(i32)]
pub enum TaskContractError {
    SpawnFailed = 500,
    JoinFailed = 501,
    EntryCountMismatch = 502,
    HandleInvalid = 503,
    CurrentIdentityMismatch = 504,
    LiveCountMismatch = 505,
    HeapNotRecovered = 506,
    DiagCreateDeltaNonZero = 507,
    DiagEgCreateDeltaNonZero = 508,
    InvalidParamNotReturned = 509,
    OverflowNotReturned = 510,
    PriorityMappingWrong = 511,
    StackMappingWrong = 512,
    NameTruncationWrong = 513,
    #[allow(dead_code)] RuntimeNotRunning = 514,
    #[allow(dead_code)] StackHwmTooSmall = 515,
    CachedResultMismatch = 516,
}

// ------------------------------------------------------------------
// TaskCaseState — per-task atomics shared via Arc.
// ------------------------------------------------------------------
struct TaskCaseState {
    started: AtomicBool,
    release_gate: AtomicBool,
    entry_count: AtomicU32,
    current_handle: AtomicUsize,
    native_priority: AtomicU32,
    #[allow(dead_code)]
    stack_hwm: AtomicU32,
}

impl TaskCaseState {
    fn new() -> Self {
        Self {
            started: AtomicBool::new(false),
            release_gate: AtomicBool::new(false),
            entry_count: AtomicU32::new(0),
            current_handle: AtomicUsize::new(0),
            native_priority: AtomicU32::new(0),
            stack_hwm: AtomicU32::new(0),
        }
    }
}

// ------------------------------------------------------------------
// Helpers
// ------------------------------------------------------------------

fn wait_gate(state: &TaskCaseState) {
    while !state.release_gate.load(Ordering::Acquire) {
        let _ = sys::delay_ticks(1);
    }
}

#[allow(dead_code)]
const HWM_MIN_WORDS: u32 = 64;
const HEAP_RECOVERY_TICKS: u32 = 500;
/// Test task stack size — small enough to minimize heap pressure but
/// well above the 128-word platform minimum.
const TEST_STACK_BYTES: usize = 1536;

// ------------------------------------------------------------------
// PublishedTaskSlot — single-writer, single-reader slot for self-join.
// ------------------------------------------------------------------
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

struct PublishedTaskSlot {
    ready: AtomicBool,
    slot: UnsafeCell<MaybeUninit<FreeRtosTask>>,
}

unsafe impl Sync for PublishedTaskSlot {}

impl PublishedTaskSlot {
    const fn new() -> Self {
        Self {
            ready: AtomicBool::new(false),
            slot: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Controller: publish a task clone (called once before task reads).
    fn publish(&self, task: FreeRtosTask) {
        unsafe { (*self.slot.get()).write(task); }
        self.ready.store(true, Ordering::Release);
    }

    /// Task: take the clone.  Must only be called once, after ready.
    /// The caller must copy the task out of the slot before calling join().
    unsafe fn take(&self) -> FreeRtosTask {
        while !self.ready.load(Ordering::Acquire) {
            let _ = sys::delay_ticks(1);
        }
        unsafe { (*self.slot.get()).assume_init_read() }
    }
}

// ------------------------------------------------------------------
// Scheduler resume guard — RAII resume.
// ------------------------------------------------------------------
struct SchedulerResumeGuard;

impl SchedulerResumeGuard {
    /// # Safety: vTaskSuspendAll must have been called before this.
    unsafe fn new() -> Self {
        Self
    }
}

impl Drop for SchedulerResumeGuard {
    fn drop(&mut self) {
        unsafe { osal_test_scheduler_resume(); }
    }
}

// ------------------------------------------------------------------
// Non-OSAL context checker — called from native helper via C bridge.
// ------------------------------------------------------------------
static NON_OSAL_CURRENT: AtomicU32 = AtomicU32::new(0);
static NON_OSAL_COUNT: AtomicU32 = AtomicU32::new(0);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn osal_test_record_non_osal_identity() {
    let is_some = FreeRtosTask::current().is_some();
    NON_OSAL_CURRENT.store(if is_some { 1 } else { 0 }, Ordering::Release);
    NON_OSAL_COUNT.store(FreeRtosTask::count() as u32, Ordering::Release);
}

// ------------------------------------------------------------------
// Cases
// ------------------------------------------------------------------

type TestResult = core::result::Result<(), TaskContractError>;

/// Spawn a task, join it, drop the handle.  Heap recovery is the
/// caller's responsibility so the baseline can be captured between
/// state allocation and spawn.
fn spawn_join_drop(
    name: &str,
    entry: impl FnOnce() + Send + 'static,
) -> TestResult {
    let t = FreeRtosTaskBuilder::new()
        .stack_size(TEST_STACK_BYTES)
        .name(name)
        .spawn(entry)
        .map_err(|_| TaskContractError::SpawnFailed)?;
    t.join(Timeout::Forever)
        .map_err(|_| TaskContractError::JoinFailed)?;
    drop(t);
    Ok(())
}

/// Case 1: Builder parameter validation.
fn case_builder_core(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let count_baseline = FreeRtosTask::count();

    // --- default builder ---
    {
        let state = Arc::new(TaskCaseState::new());
        let s = Arc::clone(&state);
        let sub_baseline = sys::heap_free();
        let t = FreeRtosTaskBuilder::new()
            .name("builder_core")
            .spawn(move || {
                s.started.store(true, Ordering::Release);
                s.entry_count.fetch_add(1, Ordering::Release);
                s.current_handle.store(
                    FreeRtosTask::current().map(|h| h.get()).unwrap_or(0),
                    Ordering::Release,
                );
                s.stack_hwm.store(task_stack_hwm(), Ordering::Release);
            })
            .map_err(|_| TaskContractError::SpawnFailed)?;

        t.join(Timeout::Forever)
            .map_err(|_| TaskContractError::JoinFailed)?;

        if state.entry_count.load(Ordering::Acquire) != 1 {
            return Err(TaskContractError::EntryCountMismatch);
        }
        let handle = TaskHandle::from_raw(state.current_handle.load(Ordering::Acquire));
        if handle.is_none() || t.handle() != handle.unwrap() {
            return Err(TaskContractError::HandleInvalid);
        }
        if state.stack_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
            return Err(TaskContractError::StackHwmTooSmall);
        }

        drop(t);
        if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
        drop(state);
    }

    // --- explicit empty name ---
    {
        let sub_baseline = sys::heap_free();
        spawn_join_drop("", || {})?;
        if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    // --- 31-byte name (accepted by API, truncated to 15 at C boundary) ---
    {
        let name31 = "abcdefghijklmnopqrstuvwxyz01234"; // 31 chars
        let sub_baseline = sys::heap_free();
        let t = FreeRtosTaskBuilder::new()
            .name(name31)
            .spawn(|| {})
            .map_err(|_| TaskContractError::SpawnFailed)?;
        t.join(Timeout::Forever)
            .map_err(|_| TaskContractError::JoinFailed)?;
        drop(t);
        if diag_last_name_len() != 15 {
            return Err(TaskContractError::NameTruncationWrong);
        }
        if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    // --- embedded NUL (rejected, no side effects) ---
    {
        let sub_baseline = sys::heap_free();
        let count_before = FreeRtosTask::count();
        let diag_before = read_diag();
        match FreeRtosTaskBuilder::new().name("bad\0name").spawn(|| {}) {
            Err(Error::InvalidParameter) => {}
            _ => return Err(TaskContractError::InvalidParamNotReturned),
        }
        let diag_after = read_diag();
        if diag_create_attempt_delta(&diag_after, &diag_before) != 0 {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        if diag_eg_create_delta(&diag_after, &diag_before) != 0 {
            return Err(TaskContractError::DiagEgCreateDeltaNonZero);
        }
        if sys::heap_free() != sub_baseline {
            return Err(TaskContractError::HeapNotRecovered);
        }
        if FreeRtosTask::count() != count_before {
            return Err(TaskContractError::LiveCountMismatch);
        }
    }

    // --- 32-byte name (rejected: > 31) ---
    {
        let name32 = "abcdefghijklmnopqrstuvwxyz012345"; // 32 chars
        let sub_baseline = sys::heap_free();
        let count_before = FreeRtosTask::count();
        let diag_before = read_diag();
        match FreeRtosTaskBuilder::new().name(name32).spawn(|| {}) {
            Err(Error::InvalidParameter) => {}
            _ => return Err(TaskContractError::InvalidParamNotReturned),
        }
        if diag_create_attempt_delta(&read_diag(), &diag_before) != 0
            || diag_eg_create_delta(&read_diag(), &diag_before) != 0
        {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        if sys::heap_free() != sub_baseline || FreeRtosTask::count() != count_before {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    // --- zero stack (rejected) ---
    {
        let sub_baseline = sys::heap_free();
        let count_before = FreeRtosTask::count();
        let diag_before = read_diag();
        match FreeRtosTaskBuilder::new().stack_size(0).spawn(|| {}) {
            Err(Error::InvalidParameter) => {}
            _ => return Err(TaskContractError::InvalidParamNotReturned),
        }
        if diag_create_attempt_delta(&read_diag(), &diag_before) != 0
            || diag_eg_create_delta(&read_diag(), &diag_before) != 0
        {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        if sys::heap_free() != sub_baseline || FreeRtosTask::count() != count_before {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    // Final full heap gate.
    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }
    if FreeRtosTask::count() != count_baseline {
        return Err(TaskContractError::LiveCountMismatch);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_builder_core");
    Ok(())
}

/// Case 2: Entry executes exactly once, join caches result.
fn case_entry_once(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let task = FreeRtosTaskBuilder::new()
        .name("entry_once")
        .spawn(move || { s.entry_count.fetch_add(1, Ordering::Release); })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    let result = task.join(Timeout::Forever)
        .map_err(|_| TaskContractError::JoinFailed)?;
    if result != ExitCode::SUCCESS {
        return Err(TaskContractError::CachedResultMismatch);
    }
    if state.entry_count.load(Ordering::Acquire) != 1 {
        return Err(TaskContractError::EntryCountMismatch);
    }

    // Repeat joins return cached result.
    for _ in 0..3 {
        let r = task.join(Timeout::NoWait)
            .map_err(|_| TaskContractError::CachedResultMismatch)?;
        if r != ExitCode::SUCCESS {
            return Err(TaskContractError::CachedResultMismatch);
        }
    }
    let r4 = task.join(Timeout::Forever)
        .map_err(|_| TaskContractError::CachedResultMismatch)?;
    if r4 != ExitCode::SUCCESS {
        return Err(TaskContractError::CachedResultMismatch);
    }

    if state.entry_count.load(Ordering::Acquire) != 1 {
        return Err(TaskContractError::EntryCountMismatch);
    }

    drop(task);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);

    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_entry_once");
    Ok(())
}

/// Case 3: Three concurrent tasks have distinct handles.
fn case_handle_unique(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    let s1 = Arc::new(TaskCaseState::new());
    let s2 = Arc::new(TaskCaseState::new());
    let s3 = Arc::new(TaskCaseState::new());
    let c1 = Arc::clone(&s1);
    let c2 = Arc::clone(&s2);
    let c3 = Arc::clone(&s3);

    let sub_baseline = sys::heap_free();

    let t1 = FreeRtosTaskBuilder::new().name("unique_a").stack_size(2048).spawn(move || {
        c1.started.store(true, Ordering::Release);
        c1.current_handle.store(
            FreeRtosTask::current().map(|h| h.get()).unwrap_or(0), Ordering::Release);
        wait_gate(&c1);
    }).map_err(|_| TaskContractError::SpawnFailed)?;

    let t2 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("unique_b").spawn(move || {
        c2.started.store(true, Ordering::Release);
        c2.current_handle.store(
            FreeRtosTask::current().map(|h| h.get()).unwrap_or(0), Ordering::Release);
        wait_gate(&c2);
    }).map_err(|_| TaskContractError::SpawnFailed)?;

    let t3 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("unique_c").spawn(move || {
        c3.started.store(true, Ordering::Release);
        c3.current_handle.store(
            FreeRtosTask::current().map(|h| h.get()).unwrap_or(0), Ordering::Release);
        wait_gate(&c3);
    }).map_err(|_| TaskContractError::SpawnFailed)?;

    // Wait for all to start.
    while !s1.started.load(Ordering::Acquire)
        || !s2.started.load(Ordering::Acquire)
        || !s3.started.load(Ordering::Acquire)
    {
        let _ = sys::delay_ticks(1);
    }

    let h1 = s1.current_handle.load(Ordering::Acquire);
    let h2 = s2.current_handle.load(Ordering::Acquire);
    let h3 = s3.current_handle.load(Ordering::Acquire);

    if h1 == 0 || h2 == 0 || h3 == 0 { return Err(TaskContractError::HandleInvalid); }
    if h1 == h2 || h1 == h3 || h2 == h3 { return Err(TaskContractError::HandleInvalid); }
    if t1.handle().get() != h1 || t2.handle().get() != h2 || t3.handle().get() != h3 {
        return Err(TaskContractError::CurrentIdentityMismatch);
    }

    s1.release_gate.store(true, Ordering::Release);
    s2.release_gate.store(true, Ordering::Release);
    s3.release_gate.store(true, Ordering::Release);

    t1.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    t2.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    t3.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    drop(t1); drop(t2); drop(t3);
    // Check sub_baseline while Arcs are still alive.
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    drop(s1); drop(s2); drop(s3);
    // After Arcs dropped, heap must match global_baseline.
    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_handle_unique");
    Ok(())
}

/// Case 4: Stack bytes → words mapping and overflow rejection.
fn case_stack_mapping(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    // 4097 bytes → ceil(4097/4) = 1025 words.
    {
        let sub_baseline = sys::heap_free();
        let t = FreeRtosTaskBuilder::new()
            .name("stack_map").stack_size(4097).spawn(|| {})
            .map_err(|_| TaskContractError::SpawnFailed)?;
        if diag_last_stack_words() != 1025 {
            return Err(TaskContractError::StackMappingWrong);
        }
        t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
        drop(t);
        if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    // usize::MAX → Overflow, no side effects.
    {
        let sub_baseline = sys::heap_free();
        let count_before = FreeRtosTask::count();
        let diag_before = read_diag();
        match FreeRtosTaskBuilder::new().stack_size(usize::MAX).spawn(|| {}) {
            Err(Error::Overflow) => {}
            _ => return Err(TaskContractError::OverflowNotReturned),
        }
        let diag_after = read_diag();
        if diag_create_attempt_delta(&diag_after, &diag_before) != 0
            || diag_eg_create_delta(&diag_after, &diag_before) != 0
        {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        if sys::heap_free() != sub_baseline || FreeRtosTask::count() != count_before {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_stack_mapping");
    Ok(())
}

/// Case 5: Priority mapping — requested u32::MAX → native 7.
/// No controller gate to avoid starvation.
fn case_priority_mapping(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new()
        .name("prio_map").priority(u32::MAX)
        .spawn(move || {
            s.native_priority.store(diag_last_native_priority(), Ordering::Release);
            s.stack_hwm.store(task_stack_hwm(), Ordering::Release);
            s.entry_count.fetch_add(1, Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    if t.priority() != u32::MAX {
        return Err(TaskContractError::PriorityMappingWrong);
    }
    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    if state.native_priority.load(Ordering::Acquire) != 7 {
        return Err(TaskContractError::PriorityMappingWrong);
    }
    if state.entry_count.load(Ordering::Acquire) != 1 {
        return Err(TaskContractError::EntryCountMismatch);
    }

    drop(t);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);

    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_priority_mapping");
    Ok(())
}

/// Case 6: Task::current() returns Some(handle) inside OSAL task.
fn case_current_identity(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    if FreeRtosTask::current().is_some() {
        return Err(TaskContractError::CurrentIdentityMismatch);
    }

    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new()
        .name("current_id")
        .spawn(move || {
            s.current_handle.store(
                FreeRtosTask::current().map(|h| h.get()).unwrap_or(0), Ordering::Release);
            s.entry_count.fetch_add(1, Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    let expected = t.handle();
    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    let reported = TaskHandle::from_raw(state.current_handle.load(Ordering::Acquire));
    if reported != Some(expected) {
        return Err(TaskContractError::CurrentIdentityMismatch);
    }
    if FreeRtosTask::current().is_some() {
        return Err(TaskContractError::CurrentIdentityMismatch);
    }

    drop(t);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);

    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_current_identity");
    Ok(())
}

/// Case 7: Task::current() == None from native (non-OSAL) context.
fn case_non_osal_context(tick_bits: u8) -> TestResult {
    let baseline = sys::heap_free();
    let count_baseline = FreeRtosTask::count();

    let case_state = CaseState::new();
    let ctx = case_state.as_context();

    NON_OSAL_CURRENT.store(0, Ordering::Release);
    NON_OSAL_COUNT.store(0, Ordering::Release);

    let rc = unsafe { harness::native_task_spawn(non_osal_context_helper, ctx, 512, 2) };
    if rc != 0 { return Err(TaskContractError::SpawnFailed); }

    if harness::wait_until_phase(&case_state, PHASE_EXITING, 100, tick_bits).is_err() {
        return Err(TaskContractError::JoinFailed);
    }
    if harness::wait_until_heap_recovered(baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    case_state.record_phase(PHASE_DONE);
    if harness::validate_helper(&case_state).is_err() {
        return Err(TaskContractError::JoinFailed);
    }

    if NON_OSAL_CURRENT.load(Ordering::Acquire) != 0 {
        return Err(TaskContractError::CurrentIdentityMismatch);
    }
    if NON_OSAL_COUNT.load(Ordering::Acquire) as usize != count_baseline {
        return Err(TaskContractError::LiveCountMismatch);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_non_osal_context");
    Ok(())
}

/// Case 8: Task::count() tracks live entries, not handles.
fn case_live_count(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let count_baseline = FreeRtosTask::count();

    let s1 = Arc::new(TaskCaseState::new());
    let s2 = Arc::new(TaskCaseState::new());
    let s3 = Arc::new(TaskCaseState::new());
    let c1 = Arc::clone(&s1);
    let c2 = Arc::clone(&s2);
    let c3 = Arc::clone(&s3);

    let sub_baseline = sys::heap_free();

    let t1 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("count_a").spawn(move || {
        c1.started.store(true, Ordering::Release);
        wait_gate(&c1);
    }).map_err(|_| TaskContractError::SpawnFailed)?;

    let t2 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("count_b").spawn(move || {
        c2.started.store(true, Ordering::Release);
        wait_gate(&c2);
    }).map_err(|_| TaskContractError::SpawnFailed)?;

    let t3 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("count_c").spawn(move || {
        c3.started.store(true, Ordering::Release);
        wait_gate(&c3);
    }).map_err(|_| TaskContractError::SpawnFailed)?;

    // Wait for trampolines to run so LIVE_COUNT is incremented.
    while !s1.started.load(Ordering::Acquire)
        || !s2.started.load(Ordering::Acquire)
        || !s3.started.load(Ordering::Acquire)
    {
        let _ = sys::delay_ticks(1);
    }

    if FreeRtosTask::count() != count_baseline + 3 {
        return Err(TaskContractError::LiveCountMismatch);
    }

    s1.release_gate.store(true, Ordering::Release);
    t1.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    if FreeRtosTask::count() != count_baseline + 2 {
        return Err(TaskContractError::LiveCountMismatch);
    }

    s2.release_gate.store(true, Ordering::Release);
    s3.release_gate.store(true, Ordering::Release);
    t2.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    t3.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    if FreeRtosTask::count() != count_baseline {
        return Err(TaskContractError::LiveCountMismatch);
    }

    drop(t1); drop(t2); drop(t3);
    // Check sub_baseline while Arcs are still alive.
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    drop(s1); drop(s2); drop(s3);
    // After Arcs dropped, heap must match global_baseline.
    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_live_count");
    Ok(())
}

/// Case 9: NoWait and After(ZERO) return Timeout on a running task.
fn case_join_nowait_zero(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("nowait_z")
        .spawn(move || {
            s.started.store(true, Ordering::Release);
            wait_gate(&s);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    // Wait for task to actually start.
    while !state.started.load(Ordering::Acquire) { let _ = sys::delay_ticks(1); }

    // NoWait on running task → Timeout.
    if !matches!(t.join(Timeout::NoWait), Err(Error::Timeout)) {
        return Err(TaskContractError::CachedResultMismatch);
    }
    // After(ZERO) on running task → Timeout.
    if !matches!(t.join(Timeout::After(core::time::Duration::ZERO)), Err(Error::Timeout)) {
        return Err(TaskContractError::CachedResultMismatch);
    }

    // Release and join.
    state.release_gate.store(true, Ordering::Release);
    let r = t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    if r != ExitCode::SUCCESS { return Err(TaskContractError::CachedResultMismatch); }

    drop(t);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);
    if sys::heap_free() != global_baseline { return Err(TaskContractError::HeapNotRecovered); }

    harness::console_line(c"OSAL_CASE_PASS name=task_join_nowait_zero");
    Ok(())
}

/// Case 10: After(finite) returns Timeout while task runs, later join succeeds.
fn case_join_finite_timeout(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("fin_timeout")
        .spawn(move || {
            s.started.store(true, Ordering::Release);
            wait_gate(&s);
            s.entry_count.fetch_add(1, Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    while !state.started.load(Ordering::Acquire) { let _ = sys::delay_ticks(1); }

    let before = sys::tick_snapshot();
    if !matches!(
        t.join(Timeout::After(core::time::Duration::from_millis(5))),
        Err(Error::Timeout)
    ) {
        return Err(TaskContractError::CachedResultMismatch);
    }
    let after = sys::tick_snapshot();
    let elapsed = harness::total_ticks_diff(after, before, tick_bits);
    if elapsed < 5 {
        return Err(TaskContractError::CachedResultMismatch);
    }

    // Task closure hasn't run yet — gate still held.
    if state.entry_count.load(Ordering::Acquire) != 0 {
        return Err(TaskContractError::EntryCountMismatch);
    }

    // Release and join — must succeed.
    state.release_gate.store(true, Ordering::Release);
    let r = t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    if r != ExitCode::SUCCESS { return Err(TaskContractError::CachedResultMismatch); }
    if state.entry_count.load(Ordering::Acquire) != 1 {
        return Err(TaskContractError::EntryCountMismatch);
    }

    drop(t);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);
    if sys::heap_free() != global_baseline { return Err(TaskContractError::HeapNotRecovered); }

    harness::console_line(c"OSAL_CASE_PASS name=task_join_finite_timeout");
    Ok(())
}

/// Case 11: join(Forever) succeeds, then all subsequent joins return cached.
fn case_join_forever_cached(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    // Task delays 2 ticks, records HWM, returns.
    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("forever_cached")
        .spawn(|| { let _ = sys::delay_ticks(2); })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    let before = sys::tick_snapshot();
    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    let after = sys::tick_snapshot();
    let elapsed = harness::total_ticks_diff(after, before, tick_bits);
    if elapsed < 2 { return Err(TaskContractError::CachedResultMismatch); }

    // Subsequent joins return immediately with cached result.
    for _ in 0..3 {
        let r = t.join(Timeout::NoWait).map_err(|_| TaskContractError::CachedResultMismatch)?;
        if r != ExitCode::SUCCESS { return Err(TaskContractError::CachedResultMismatch); }
    }
    t.join(Timeout::After(core::time::Duration::ZERO)).map_err(|_| TaskContractError::CachedResultMismatch)?;
    t.join(Timeout::Forever).map_err(|_| TaskContractError::CachedResultMismatch)?;

    drop(t);
    if harness::wait_until_heap_recovered(global_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_join_forever_cached");
    Ok(())
}

/// Case 12: Self-join returns Busy from within the task entry.
fn case_self_join(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    static SLOT: PublishedTaskSlot = PublishedTaskSlot::new();

    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("self_join")
        .spawn(move || {
            s.started.store(true, Ordering::Release);
            // Take own clone from the slot (published by controller).
            let myself = unsafe { SLOT.take() };

            // All join attempts on self must return Busy.
            let r1 = myself.join(Timeout::NoWait);
            let r2 = myself.join(Timeout::After(core::time::Duration::from_millis(5)));
            let r3 = myself.join(Timeout::Forever);

            s.native_priority.store(
                if matches!(r1, Err(Error::Busy)) && matches!(r2, Err(Error::Busy)) && matches!(r3, Err(Error::Busy))
                { 1 } else { 0 },
                Ordering::Release,
            );
            s.entry_count.fetch_add(1, Ordering::Release);
            drop(myself);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    // Publish a clone into the slot.
    SLOT.publish(t.clone());

    // Wait for task to start and attempt self-join.
    while !state.started.load(Ordering::Acquire) { let _ = sys::delay_ticks(1); }

    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    if state.native_priority.load(Ordering::Acquire) != 1 {
        // native_priority is being reused here as a flag — 1 = all Busy checks passed.
        // Actually let me use a different approach...
        return Err(TaskContractError::CachedResultMismatch);
    }

    drop(t);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);
    if sys::heap_free() != global_baseline { return Err(TaskContractError::HeapNotRecovered); }

    harness::console_line(c"OSAL_CASE_PASS name=task_self_join");
    Ok(())
}

/// Case 13: Three joiners block concurrently on one gated target.
/// All three return the same ExitCode — proves EventGroup sticky bit
/// is not consumed by the first waking joiner.
fn case_concurrent_joiners(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    let target_state = Arc::new(TaskCaseState::new());
    let tc = Arc::clone(&target_state);

    let target = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("cj_target")
        .spawn(move || {
            tc.started.store(true, Ordering::Release);
            tc.stack_hwm.store(task_stack_hwm(), Ordering::Release);
            wait_gate(&tc);
            tc.entry_count.fetch_add(1, Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    // Joiner states — three independent Arcs.
    let js0 = Arc::new(TaskCaseState::new());
    let js1 = Arc::new(TaskCaseState::new());
    let js2 = Arc::new(TaskCaseState::new());
    let jc0 = Arc::clone(&js0);
    let jc1 = Arc::clone(&js1);
    let jc2 = Arc::clone(&js2);
    let tgt0 = target.clone();
    let tgt1 = target.clone();
    let tgt2 = target.clone();

    // Spawn 3 joiners — each blocks on target.join(Forever).
    let j0 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("joiner_0")
        .spawn(move || {
            jc0.started.store(true, Ordering::Release);
            let r = tgt0.join(Timeout::Forever);
            jc0.native_priority.store(if r.is_ok() { 1 } else { 0 }, Ordering::Release);
            jc0.stack_hwm.store(task_stack_hwm(), Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;
    let j1 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("joiner_1")
        .spawn(move || {
            jc1.started.store(true, Ordering::Release);
            let r = tgt1.join(Timeout::Forever);
            jc1.native_priority.store(if r.is_ok() { 1 } else { 0 }, Ordering::Release);
            jc1.stack_hwm.store(task_stack_hwm(), Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;
    let j2 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("joiner_2")
        .spawn(move || {
            jc2.started.store(true, Ordering::Release);
            let r = tgt2.join(Timeout::Forever);
            jc2.native_priority.store(if r.is_ok() { 1 } else { 0 }, Ordering::Release);
            jc2.stack_hwm.store(task_stack_hwm(), Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    // Wait for all 3 joiners to have started (blocked on EventGroup).
    while !js0.started.load(Ordering::Acquire)
        || !js1.started.load(Ordering::Acquire)
        || !js2.started.load(Ordering::Acquire)
    { let _ = sys::delay_ticks(1); }

    // Release target — all 3 joiners wake.
    target_state.release_gate.store(true, Ordering::Release);

    // All joiners must return SUCCESS.
    j0.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    j1.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
    j2.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    // All joiners got SUCCESS and HWM >= 64.
    if js0.native_priority.load(Ordering::Acquire) != 1
        || js0.stack_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS
        || js1.native_priority.load(Ordering::Acquire) != 1
        || js1.stack_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS
        || js2.native_priority.load(Ordering::Acquire) != 1
        || js2.stack_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS
    { return Err(TaskContractError::CachedResultMismatch); }

    // Target entry_count must be exactly 1 after join.
    target.join(Timeout::NoWait).map_err(|_| TaskContractError::CachedResultMismatch)?;
    if target_state.entry_count.load(Ordering::Acquire) != 1 {
        return Err(TaskContractError::EntryCountMismatch);
    }
    if target_state.stack_hwm.load(Ordering::Acquire) < HWM_MIN_WORDS {
        return Err(TaskContractError::StackHwmTooSmall);
    }

    drop(j0); drop(j1); drop(j2);
    drop(target);
    drop(target_state);
    drop(js0); drop(js1); drop(js2);

    // End-to-end recovery — all resources freed.
    if harness::wait_until_heap_recovered(global_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_concurrent_joiners");
    Ok(())
}

/// Case 14: Join after completion with scheduler suspended returns cached immediately.
fn case_late_join_cached(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    // First complete a task normally.
    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("late_join")
        .spawn(|| {})
        .map_err(|_| TaskContractError::SpawnFailed)?;
    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    // Suspend scheduler, join again — must return cached immediately.
    unsafe { osal_test_scheduler_suspend(); }
    let _guard = unsafe { SchedulerResumeGuard::new() };

    // All join variants must succeed immediately (finished fast path).
    t.join(Timeout::NoWait).map_err(|_| TaskContractError::CachedResultMismatch)?;
    t.join(Timeout::After(core::time::Duration::ZERO)).map_err(|_| TaskContractError::CachedResultMismatch)?;
    t.join(Timeout::Forever).map_err(|_| TaskContractError::CachedResultMismatch)?;

    drop(_guard); // resume scheduler

    drop(t);
    if harness::wait_until_heap_recovered(global_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_late_join_cached");
    Ok(())
}

/// Case 15: Join during scheduler suspend returns Timeout/Busy appropriately.
fn case_scheduler_suspended(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();
    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let sub_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("sched_susp")
        .spawn(move || { s.started.store(true, Ordering::Release); wait_gate(&s); })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    while !state.started.load(Ordering::Acquire) { let _ = sys::delay_ticks(1); }

    // Suspend scheduler.
    unsafe { osal_test_scheduler_suspend(); }
    let _guard = unsafe { SchedulerResumeGuard::new() };

    // Non-blocking on running+suspended → Timeout.
    if !matches!(t.join(Timeout::NoWait), Err(Error::Timeout)) {
        return Err(TaskContractError::CachedResultMismatch);
    }
    if !matches!(t.join(Timeout::After(core::time::Duration::ZERO)), Err(Error::Timeout)) {
        return Err(TaskContractError::CachedResultMismatch);
    }
    // Blocking on running+suspended → Busy.
    if !matches!(t.join(Timeout::After(core::time::Duration::from_millis(5))),
                  Err(Error::Busy))
    {
        return Err(TaskContractError::CachedResultMismatch);
    }
    if !matches!(t.join(Timeout::Forever), Err(Error::Busy)) {
        return Err(TaskContractError::CachedResultMismatch);
    }

    drop(_guard); // resume scheduler

    state.release_gate.store(true, Ordering::Release);
    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    drop(t);
    if harness::wait_until_heap_recovered(sub_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);
    if sys::heap_free() != global_baseline { return Err(TaskContractError::HeapNotRecovered); }

    harness::console_line(c"OSAL_CASE_PASS name=task_scheduler_suspended");
    Ok(())
}

/// Case 16: Drop handle without join — task still completes independently.
///
/// After the lone handle is dropped, the task must finish via self-delete
/// and the Idle task must reclaim native resources.  The suite's final
/// shutdown serves as the proof that no RuntimeLease lingers.
/// Case 16: Drop handle without join — task completes independently.
/// Shutdown must be Busy while task runs, succeed after cleanup.
fn case_drop_without_join(tick_bits: u8) -> TestResult {
    let state = Arc::new(TaskCaseState::new());
    let s = Arc::clone(&state);
    let global_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("drop_nojoin")
        .spawn(move || {
            s.started.store(true, Ordering::Release);
            wait_gate(&s);
            s.entry_count.fetch_add(1, Ordering::Release);
            s.stack_hwm.store(task_stack_hwm(), Ordering::Release);
        })
        .map_err(|_| TaskContractError::SpawnFailed)?;

    while !state.started.load(Ordering::Acquire) { let _ = sys::delay_ticks(1); }

    // Drop external handle — trampoline still holds RuntimeLease.
    drop(t);

    // Shutdown must be Busy (live RuntimeLease from trampoline's start).
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(TaskContractError::RuntimeNotRunning);
    }
    if osal::runtime_state() != osal_api::runtime::RuntimeState::Running {
        return Err(TaskContractError::RuntimeNotRunning);
    }

    // Release gate — task completes, trampoline releases lease, self-deletes.
    state.release_gate.store(true, Ordering::Release);

    while FreeRtosTask::count() > 0 { let _ = sys::delay_ticks(1); }

    if harness::wait_until_heap_recovered(global_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }
    drop(state);

    // Shutdown must now succeed (all RuntimeLeases released).
    osal::shutdown().map_err(|_| TaskContractError::RuntimeNotRunning)?;
    osal::initialize().map_err(|_| TaskContractError::RuntimeNotRunning)?;

    harness::console_line(c"OSAL_CASE_PASS name=task_drop_without_join");
    Ok(())
}

/// Case 17: Finished handle holds RuntimeLease — shutdown Busy until drop.
fn case_finished_handle_lease(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("lease")
        .spawn(|| {})
        .map_err(|_| TaskContractError::SpawnFailed)?;
    t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

    // count=0 but handle still holds RuntimeLease.
    if FreeRtosTask::count() != 0 {
        return Err(TaskContractError::LiveCountMismatch);
    }

    // Shutdown must be Busy (RuntimeLease from finished handle).
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(TaskContractError::RuntimeNotRunning);
    }
    // Runtime must still be Running after failed shutdown.
    if osal::runtime_state() != osal_api::runtime::RuntimeState::Running {
        return Err(TaskContractError::RuntimeNotRunning);
    }

    // Drop handle → RuntimeLease released → EventGroup freed.
    drop(t);

    if harness::wait_until_heap_recovered(global_baseline, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
        return Err(TaskContractError::HeapNotRecovered);
    }

    // Shutdown must now succeed — all leases released.
    osal::shutdown().map_err(|_| TaskContractError::RuntimeNotRunning)?;
    osal::initialize().map_err(|_| TaskContractError::RuntimeNotRunning)?;

    harness::console_line(c"OSAL_CASE_PASS name=task_finished_handle_lease");
    Ok(())
}

/// Case 18: Spawn rollback — overflow path with all-diag-zero proof.
///
/// Real xTaskCreate OOM is deferred — the vApplicationMallocFailedHook
/// converts any pvPortMalloc failure into a FATAL exit before Rust can
/// observe Error::OutOfMemory.  An expected-OOM arm in the hook is
/// needed before real-kernel OOM validation is possible.
fn case_spawn_rollback(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    // Overflow: stack_size(usize::MAX) → Error::Overflow.
    // All four diagnostic counters must be unchanged — overflow is
    // caught before any pvPortMalloc or xTaskCreate call.
    {
        let heap_before = sys::heap_free();
        let count_before = FreeRtosTask::count();
        let diag_before = read_diag();
        match FreeRtosTaskBuilder::new().stack_size(usize::MAX).spawn(|| {}) {
            Err(Error::Overflow) => {}
            _ => return Err(TaskContractError::OverflowNotReturned),
        }
        let diag_after = read_diag();
        if diag_create_attempt_delta(&diag_after, &diag_before) != 0
            || diag_create_success_delta(&diag_after, &diag_before) != 0
            || diag_eg_create_delta(&diag_after, &diag_before) != 0
            || diag_eg_delete_delta(&diag_after, &diag_before) != 0
        {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        if sys::heap_free() != heap_before {
            return Err(TaskContractError::HeapNotRecovered);
        }
        if FreeRtosTask::count() != count_before {
            return Err(TaskContractError::LiveCountMismatch);
        }
    }

    // Real OOM: request more stack than available heap.
    // The expected-OOM arm in hooks.c allows exactly one pvPortMalloc
    // failure from the controller task to return instead of FATAL.
    {
        let heap_before = sys::heap_free();
        let count_before = FreeRtosTask::count();
        let diag_before = read_diag();

        // Request just over the total free heap.  The EventGroup + Arc
        // pre-allocations (~300 bytes) succeed first, then xTaskCreate
        // fails on the stack+TCB allocation.
        let oom_bytes: usize = (heap_before as usize).saturating_add(4096);

        unsafe { osal_test_expect_malloc_failure(); }

        let result = FreeRtosTaskBuilder::new()
            .name("oom_probe")
            .stack_size(oom_bytes)
            .spawn(|| {});

        // Must have consumed the expected failure.
        let consumed = unsafe { osal_test_expected_malloc_failure_consumed() };
        unsafe { osal_test_clear_expected_malloc_failure(); }

        if consumed != 1 {
            return Err(TaskContractError::SpawnFailed);
        }

        match result {
            Err(Error::OutOfMemory) => {}
            _ => { return Err(TaskContractError::SpawnFailed); }
        }

        let diag_after = read_diag();
        // xTaskCreate was attempted but failed.
        if diag_create_attempt_delta(&diag_after, &diag_before) != 1 {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        if diag_create_success_delta(&diag_after, &diag_before) != 0 {
            return Err(TaskContractError::DiagCreateDeltaNonZero);
        }
        // EventGroup created (+1) then deleted (+1) during rollback.
        if diag_eg_create_delta(&diag_after, &diag_before) != 1
            || diag_eg_delete_delta(&diag_after, &diag_before) != 1
        {
            return Err(TaskContractError::DiagEgCreateDeltaNonZero);
        }
        if sys::heap_free() != heap_before {
            return Err(TaskContractError::HeapNotRecovered);
        }
        if FreeRtosTask::count() != count_before {
            return Err(TaskContractError::LiveCountMismatch);
        }
    }

    // Prove path is not corrupted: normal task still works afterward.
    {
        let sub = sys::heap_free();
        let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("recovery")
            .spawn(|| {})
            .map_err(|_| TaskContractError::SpawnFailed)?;
        t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
        drop(t);
        if harness::wait_until_heap_recovered(sub, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_spawn_rollback");
    Ok(())
}

// ------------------------------------------------------------------
// Dispatcher
// ------------------------------------------------------------------

pub fn run_task_cases(tick_bits: u8) -> TestResult {
    diag_reset();

    // OOM/rollback first — needs clean heap for small pre-allocations.
    case_spawn_rollback(tick_bits)?;
    case_builder_core(tick_bits)?;
    case_entry_once(tick_bits)?;
    case_handle_unique(tick_bits)?;
    case_stack_mapping(tick_bits)?;
    case_priority_mapping(tick_bits)?;
    case_current_identity(tick_bits)?;
    case_non_osal_context(tick_bits)?;
    case_live_count(tick_bits)?;
    case_join_nowait_zero(tick_bits)?;
    case_join_finite_timeout(tick_bits)?;
    case_join_forever_cached(tick_bits)?;
    case_self_join(tick_bits)?;
    case_concurrent_joiners(tick_bits)?;
    case_late_join_cached(tick_bits)?;
    case_scheduler_suspended(tick_bits)?;
    case_lifecycle_stress(tick_bits)?;
    // Shutdown-lease cases must be last — they call shutdown/initialize.
    case_drop_without_join(tick_bits)?;
    case_finished_handle_lease(tick_bits)?;

    Ok(())
}

// ------------------------------------------------------------------
// Case 19: lifecycle stress
// ------------------------------------------------------------------

/// Case 19: 32 sequential rounds + 8 waves of 3 — lifecycle stress.
fn case_lifecycle_stress(tick_bits: u8) -> TestResult {
    let global_baseline = sys::heap_free();

    // --- Sequential: 32 rounds of spawn/join/drop/heap-recover ---
    for _round in 0u32..32 {
        let sub = sys::heap_free();
        let t = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("str_seq")
            .spawn(|| {})
            .map_err(|_| TaskContractError::SpawnFailed)?;

        let r = t.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
        if r != ExitCode::SUCCESS { return Err(TaskContractError::CachedResultMismatch); }

        // Cached joins after completion.
        t.join(Timeout::NoWait).map_err(|_| TaskContractError::CachedResultMismatch)?;
        t.join(Timeout::After(core::time::Duration::ZERO))
            .map_err(|_| TaskContractError::CachedResultMismatch)?;

        drop(t);
        if harness::wait_until_heap_recovered(sub, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
    }

    // --- Concurrent waves: 8 waves of 3 tasks each ---
    for _wave in 0u32..8 {
        let s0 = Arc::new(TaskCaseState::new());
        let s1 = Arc::new(TaskCaseState::new());
        let s2 = Arc::new(TaskCaseState::new());
        let c0 = Arc::clone(&s0);
        let c1 = Arc::clone(&s1);
        let c2 = Arc::clone(&s2);
        let sub = sys::heap_free();

        let t0 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("str_w0")
            .spawn(move || { c0.started.store(true, Ordering::Release); wait_gate(&c0); })
            .map_err(|_| TaskContractError::SpawnFailed)?;
        let t1 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("str_w1")
            .spawn(move || { c1.started.store(true, Ordering::Release); wait_gate(&c1); })
            .map_err(|_| TaskContractError::SpawnFailed)?;
        let t2 = FreeRtosTaskBuilder::new().stack_size(TEST_STACK_BYTES).name("str_w2")
            .spawn(move || { c2.started.store(true, Ordering::Release); wait_gate(&c2); })
            .map_err(|_| TaskContractError::SpawnFailed)?;

        while !s0.started.load(Ordering::Acquire)
            || !s1.started.load(Ordering::Acquire)
            || !s2.started.load(Ordering::Acquire)
        { let _ = sys::delay_ticks(1); }

        if t0.handle() == t1.handle() || t1.handle() == t2.handle() {
            return Err(TaskContractError::HandleInvalid);
        }
        if FreeRtosTask::count() < 3 {
            return Err(TaskContractError::LiveCountMismatch);
        }

        s0.release_gate.store(true, Ordering::Release);
        s1.release_gate.store(true, Ordering::Release);
        s2.release_gate.store(true, Ordering::Release);

        t0.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
        t1.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;
        t2.join(Timeout::Forever).map_err(|_| TaskContractError::JoinFailed)?;

        drop(t0); drop(t1); drop(t2);
        if harness::wait_until_heap_recovered(sub, HEAP_RECOVERY_TICKS, tick_bits).is_err() {
            return Err(TaskContractError::HeapNotRecovered);
        }
        drop(s0); drop(s1); drop(s2);
    }

    if sys::heap_free() != global_baseline {
        return Err(TaskContractError::HeapNotRecovered);
    }

    harness::console_line(c"OSAL_CASE_PASS name=task_lifecycle_stress");
    Ok(())
}
