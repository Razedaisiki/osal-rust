//! Deterministic native helper-task harness (P7G Step 4-0).
//!
//! Provides `CaseState` with phase/result/tick atomics and a visited
//! bitmap, `wait_until_phase` with real tick deadlines, and
//! context-aware extern "C" bridges so multiple independent native
//! FreeRTOS helper tasks can report progress without cross-talk.
//!
//! ## Phase lifecycle
//!
//! ```text
//! CREATED → STARTED → BEFORE_OPERATION → OPERATION_COMPLETED → EXITING → DONE
//! ```
//!
//! Native helpers set STARTED through EXITING.  The Rust controller
//! sets DONE after confirming Idle task heap recovery.
//!
//! Each `CaseState` is a static so that context pointers remain valid
//! even if the controller returns early on an error path while a
//! spawned native helper is still running.  Native helpers receive an
//! opaque `*mut c_void` context and pass it back to the bridges.

use core::ffi::{c_void, CStr};
use core::sync::atomic::{AtomicI32, AtomicU32, Ordering};

use osal_backend_freertos_sys as sys;
use sys::{DelayStatus, TickSnapshot};

// ------------------------------------------------------------------
// Phase constants — keep in sync with test_task.h enum.
// ------------------------------------------------------------------
pub const PHASE_CREATED: u32 = 0;
pub const PHASE_STARTED: u32 = 1;
pub const PHASE_BEFORE_OPERATION: u32 = 2;
pub const PHASE_OPERATION_COMPLETED: u32 = 3;
pub const PHASE_EXITING: u32 = 4;
pub const PHASE_DONE: u32 = 5;

/// Internal sentinel: a phase transition violated the state machine.
const RESULT_INVALID_PHASE: i32 = -2;

/// The phases a native helper must visit (in order).
const REQUIRED_HELPER_PHASES: &[u32] = &[
    PHASE_STARTED,
    PHASE_BEFORE_OPERATION,
    PHASE_OPERATION_COMPLETED,
    PHASE_EXITING,
];

// ------------------------------------------------------------------
// Error codes — returned through osal_test_object_entry()
// ------------------------------------------------------------------
#[repr(i32)]
pub enum HarnessError {
    SpawnFailed = 100,
    Timeout = 101,
    HeapLeak = 102,
    HelperResult = 103,
    TickStalled = 104,
    PhaseNotVisited = 105,
    StateIsolation = 106,
}

// ------------------------------------------------------------------
// C bridge declarations (module-level)
// ------------------------------------------------------------------

/// Type matching C `osal_test_task_entry_t`: `void (*)(void *context)`.
type NativeTaskEntry = unsafe extern "C" fn(*mut c_void);

unsafe extern "C" {
    /// Console line output (defined in main.c).
    fn osal_test_console_line(line: *const i8);

    /// Spawn a native FreeRTOS helper task (defined in test_task.c).
    fn osal_test_task_spawn(
        entry: NativeTaskEntry,
        context: *mut c_void,
        stack_words: u32,
        priority: u32,
    ) -> i32;

    /// Harness smoke native helper entry (defined in main.c).
    fn harness_smoke_helper(context: *mut c_void);
}

// ------------------------------------------------------------------
// CaseState — per-helper progress with visited bitmap.
// ------------------------------------------------------------------
pub struct CaseState {
    phase: AtomicU32,
    /// Bitmap: bit N = phase N was set at least once.
    visited: AtomicU32,
    result: AtomicI32,
    start_tick: AtomicU32,
    end_tick: AtomicU32,
}

impl CaseState {
    pub const fn new() -> Self {
        Self {
            phase: AtomicU32::new(PHASE_CREATED),
            visited: AtomicU32::new(0),
            result: AtomicI32::new(0),
            start_tick: AtomicU32::new(0),
            end_tick: AtomicU32::new(0),
        }
    }

    /// Record a phase transition.
    ///
    /// The phase must be exactly `current + 1` and within the valid
    /// range `CREATED..=DONE`.  Out-of-order, duplicate, backward, and
    /// skipped transitions set `result` to `RESULT_INVALID_PHASE` and
    /// are otherwise ignored.
    ///
    /// The visited bit is only set on a successful transition.
    pub fn record_phase(&self, next: u32) {
        // Guard: range check (also prevents shift overflow below).
        if next > PHASE_DONE {
            self.set_result(RESULT_INVALID_PHASE);
            return;
        }

        let current = self.phase.load(Ordering::Acquire);

        // Strict sequential advance — no skipping, no going backward.
        if next != current + 1 {
            self.set_result(RESULT_INVALID_PHASE);
            return;
        }

        self.phase.store(next, Ordering::Release);
        // Only record the visited bit on a valid transition.
        self.visited
            .fetch_or(1u32 << next, Ordering::Release);
    }

    pub fn get_phase(&self) -> u32 {
        self.phase.load(Ordering::Acquire)
    }

    /// Record a failure code, keeping the first (root-cause) error.
    pub fn set_result(&self, result: i32) {
        let _ = self.result.compare_exchange(
            0,
            result,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
    }

    pub fn get_result(&self) -> i32 {
        self.result.load(Ordering::Acquire)
    }

    pub fn record_start(&self, tick: u32) {
        self.start_tick.store(tick, Ordering::Release);
    }

    pub fn record_end(&self, tick: u32) {
        self.end_tick.store(tick, Ordering::Release);
    }

    pub fn reset(&self) {
        self.phase.store(PHASE_CREATED, Ordering::Release);
        self.visited.store(0, Ordering::Release);
        self.result.store(0, Ordering::Release);
        self.start_tick.store(0, Ordering::Release);
        self.end_tick.store(0, Ordering::Release);
    }

    /// True if every phase in `required` has its visited bit set.
    pub fn all_visited(&self, required: &[u32]) -> bool {
        let visited = self.visited.load(Ordering::Acquire);
        required.iter().all(|&p| visited & (1u32 << p) != 0)
    }

    /// Return this CaseState's address as an opaque context pointer.
    pub fn as_context(&self) -> *mut c_void {
        (self as *const CaseState).cast_mut().cast::<c_void>()
    }
}

// ------------------------------------------------------------------
// Context-aware extern "C" bridges — called by native helper tasks.
// ------------------------------------------------------------------

/// # Safety
/// `context` must point to a valid **static** `CaseState` or be null.
/// The caller must ensure the pointee outlives every native helper
/// task that holds this context — even across early-return paths.
unsafe fn state_from_context(context: *mut c_void) -> Option<&'static CaseState> {
    if context.is_null() {
        return None;
    }
    Some(unsafe { &*(context as *const CaseState) })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn osal_test_harness_set_phase(context: *mut c_void, phase: u32) {
    if let Some(state) = unsafe { state_from_context(context) } {
        state.record_phase(phase);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn osal_test_harness_set_result(context: *mut c_void, result: i32) {
    if let Some(state) = unsafe { state_from_context(context) } {
        state.set_result(result);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn osal_test_harness_record_start(context: *mut c_void, tick: u32) {
    if let Some(state) = unsafe { state_from_context(context) } {
        state.record_start(tick);
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn osal_test_harness_record_end(context: *mut c_void, tick: u32) {
    if let Some(state) = unsafe { state_from_context(context) } {
        state.record_end(tick);
    }
}

// ------------------------------------------------------------------
// Controller-side helpers
// ------------------------------------------------------------------

/// Compute elapsed ticks between two snapshots (handles overflow).
fn total_ticks_diff(after: TickSnapshot, before: TickSnapshot, bits: u8) -> u128 {
    let ta = ((after.overflow_count as u128) << bits) | after.tick_count as u128;
    let tb = ((before.overflow_count as u128) << bits) | before.tick_count as u128;
    ta.saturating_sub(tb)
}

/// Wait for the harness phase to reach at least `expected`, polling
/// every tick.  Also fails fast if the helper reports a non-zero
/// result.
///
/// Returns `Ok(())` when `state.get_phase() >= expected`.
fn wait_until_phase(
    state: &CaseState,
    expected: u32,
    timeout_ticks: u32,
    tick_bits: u8,
) -> Result<(), HarnessError> {
    let start = sys::tick_snapshot();

    loop {
        // Fail-fast: helper reported an error.
        if state.get_result() != 0 {
            return Err(HarnessError::HelperResult);
        }

        if state.get_phase() >= expected {
            return Ok(());
        }

        let now = sys::tick_snapshot();
        let elapsed = total_ticks_diff(now, start, tick_bits);
        if elapsed >= timeout_ticks as u128 {
            return Err(HarnessError::Timeout);
        }

        if sys::delay_ticks(1) != DelayStatus::Ok {
            return Err(HarnessError::TickStalled);
        }
    }
}

/// Poll for heap to return to `baseline`, yielding 1 tick per iteration.
fn wait_until_heap_recovered(
    baseline: u64,
    deadline_ticks: u32,
    tick_bits: u8,
) -> Result<(), HarnessError> {
    let start = sys::tick_snapshot();

    loop {
        if sys::heap_free() == baseline {
            return Ok(());
        }

        let now = sys::tick_snapshot();
        let elapsed = total_ticks_diff(now, start, tick_bits);
        if elapsed >= deadline_ticks as u128 {
            return Err(HarnessError::HeapLeak);
        }

        if sys::delay_ticks(1) != DelayStatus::Ok {
            return Err(HarnessError::TickStalled);
        }
    }
}

/// Emit a line to the UART via the C console bridge.
fn console_line(text: &CStr) {
    unsafe { osal_test_console_line(text.as_ptr().cast::<i8>()); }
}

// ------------------------------------------------------------------
// Per-helper validation
// ------------------------------------------------------------------

/// Validate a helper's lifecycle: phase coverage, tick advance, result.
///
/// State isolation between independent helpers is proven by the fact
/// each uses a different context pointer — if any bridge ignored its
/// context, the other helper would time out or fail its tick check.
fn validate_helper(state: &CaseState) -> Result<(), HarnessError> {
    // All required phases visited.
    if !state.all_visited(REQUIRED_HELPER_PHASES) {
        return Err(HarnessError::PhaseNotVisited);
    }

    // Tick must advance — at least 1 tick between start and end.
    let start = state.start_tick.load(Ordering::Acquire);
    let end = state.end_tick.load(Ordering::Acquire);
    if end.wrapping_sub(start) < 1 {
        return Err(HarnessError::TickStalled);
    }

    // Helper must not report an error.
    if state.get_result() != 0 {
        return Err(HarnessError::HelperResult);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Static CaseState slots — stay valid across early-return paths.
// ------------------------------------------------------------------

static STATE_A: CaseState = CaseState::new();
static STATE_B: CaseState = CaseState::new();

// ------------------------------------------------------------------
// Harness smoke — two independent native helpers.
// ------------------------------------------------------------------

pub fn run_harness_smoke(tick_bits: u8) -> Result<(), HarnessError> {
    console_line(c"OSAL_OBJECT_BEGIN");

    let baseline = sys::heap_free();

    // Reset both static states for a fresh run.
    STATE_A.reset();
    STATE_B.reset();

    let ctx_a = STATE_A.as_context();
    let ctx_b = STATE_B.as_context();

    // Spawn helper A.
    let rc = unsafe { osal_test_task_spawn(harness_smoke_helper, ctx_a, 512, 2) };
    if rc != 0 {
        return Err(HarnessError::SpawnFailed);
    }

    // Spawn helper B — proves state isolation in a single harness.
    let rc = unsafe { osal_test_task_spawn(harness_smoke_helper, ctx_b, 512, 2) };
    if rc != 0 {
        return Err(HarnessError::SpawnFailed);
    }

    // Wait for both helpers to reach EXITING.
    wait_until_phase(&STATE_A, PHASE_EXITING, 100, tick_bits)?;
    wait_until_phase(&STATE_B, PHASE_EXITING, 100, tick_bits)?;

    // Give Idle task time to reclaim both TCBs and stacks.
    wait_until_heap_recovered(baseline, 100, tick_bits)?;

    // Set DONE on both states.
    STATE_A.record_phase(PHASE_DONE);
    STATE_B.record_phase(PHASE_DONE);

    // Validate each helper independently.
    validate_helper(&STATE_A)?;
    validate_helper(&STATE_B)?;

    // State isolation: verify both states have independent visited
    // bitmaps covering the full lifecycle.
    let expected_mask: u32 =
        (1u32 << PHASE_STARTED)
        | (1u32 << PHASE_BEFORE_OPERATION)
        | (1u32 << PHASE_OPERATION_COMPLETED)
        | (1u32 << PHASE_EXITING)
        | (1u32 << PHASE_DONE);

    let visited_a = STATE_A.visited.load(Ordering::Acquire);
    let visited_b = STATE_B.visited.load(Ordering::Acquire);

    if (visited_a & expected_mask) != expected_mask {
        return Err(HarnessError::StateIsolation);
    }
    if (visited_b & expected_mask) != expected_mask {
        return Err(HarnessError::StateIsolation);
    }
    // Both states completed independently — different static addresses
    // mean different memory; both produced correct, self-consistent
    // visited bitmaps so no cross-talk occurred.

    // --- case pass ---
    console_line(c"OSAL_CASE_PASS name=harness_native_task");

    // --- object pass ---
    console_line(
        c"OSAL_OBJECT_PASS harness=true helper_self_delete=true idle_cleanup=true heap_recovered=true multi_helper=true tick_advance=true",
    );

    // --- end object protocol ---
    console_line(c"OSAL_OBJECT_END status=pass");

    Ok(())
}
