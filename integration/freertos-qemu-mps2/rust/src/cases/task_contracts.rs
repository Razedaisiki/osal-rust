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
    fn osal_test_diag_task_create_calls() -> u32;
    fn osal_test_diag_event_group_creates() -> u32;
    fn osal_test_diag_event_group_deletes() -> u32;
    fn osal_test_diag_last_stack_words() -> u32;
    fn osal_test_diag_last_native_priority() -> u32;
    fn osal_test_diag_last_name_len() -> u32;
    fn osal_test_diag_reset();
    fn osal_test_task_stack_hwm() -> u32;
    fn osal_test_scheduler_resume();
    fn non_osal_context_helper(context: *mut c_void);
}

fn diag_task_create_calls() -> u32 { unsafe { osal_test_diag_task_create_calls() } }
fn diag_event_group_creates() -> u32 { unsafe { osal_test_diag_event_group_creates() } }
fn diag_event_group_deletes() -> u32 { unsafe { osal_test_diag_event_group_deletes() } }
fn diag_last_stack_words() -> u32 { unsafe { osal_test_diag_last_stack_words() } }
fn diag_last_native_priority() -> u32 { unsafe { osal_test_diag_last_native_priority() } }
fn diag_last_name_len() -> u32 { unsafe { osal_test_diag_last_name_len() } }
fn diag_reset() { unsafe { osal_test_diag_reset() } }
fn task_stack_hwm() -> u32 { unsafe { osal_test_task_stack_hwm() } }

struct DiagSnapshot { task_create_calls: u32, event_group_creates: u32, event_group_deletes: u32 }

fn read_diag() -> DiagSnapshot {
    DiagSnapshot {
        task_create_calls: diag_task_create_calls(),
        event_group_creates: diag_event_group_creates(),
        event_group_deletes: diag_event_group_deletes(),
    }
}

fn diag_create_delta(after: &DiagSnapshot, before: &DiagSnapshot) -> u32 {
    after.task_create_calls.wrapping_sub(before.task_create_calls)
}
fn diag_eg_create_delta(after: &DiagSnapshot, before: &DiagSnapshot) -> u32 {
    after.event_group_creates.wrapping_sub(before.event_group_creates)
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

/// Spawn a simple task, join it, drop it, and verify heap recovery.
/// Returns after heap is recovered and state dropped.
fn spawn_join_drop_recover(
    name: &str,
    tick_bits: u8,
    entry: impl FnOnce() + Send + 'static,
) -> TestResult {
    let t = FreeRtosTaskBuilder::new()
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
        spawn_join_drop_recover("", tick_bits, || {})?;
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
        if diag_create_delta(&diag_after, &diag_before) != 0 {
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
        if diag_create_delta(&read_diag(), &diag_before) != 0
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
        if diag_create_delta(&read_diag(), &diag_before) != 0
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
        if diag_create_delta(&diag_after, &diag_before) != 0
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

// ------------------------------------------------------------------
// Dispatcher
// ------------------------------------------------------------------

pub fn run_task_cases(tick_bits: u8) -> TestResult {
    diag_reset();

    case_builder_core(tick_bits)?;
    case_entry_once(tick_bits)?;
    case_handle_unique(tick_bits)?;
    case_stack_mapping(tick_bits)?;
    case_priority_mapping(tick_bits)?;
    case_current_identity(tick_bits)?;
    case_non_osal_context(tick_bits)?;
    case_live_count(tick_bits)?;

    Ok(())
}
