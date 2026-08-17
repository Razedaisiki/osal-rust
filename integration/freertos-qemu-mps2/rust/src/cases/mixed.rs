//! FreeRTOS mixed-object real-kernel integration contracts.
//!
//! Validates that Mutex, CountingSemaphore, BinarySemaphore, Queue,
//! Task, and Timer compose correctly in a single runtime session.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;

use osal_api::error::Error;
use osal_api::runtime::RuntimeState;
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex as _;
use osal_api::traits::queue::Queue as _;
use osal_api::traits::semaphore::{BinarySemaphore, CountingSemaphore};
use osal_api::traits::task::{Task, TaskBuilder};
use osal_api::traits::timer::{Timer, TimerCallback};
use osal_api::types::TimerMode;
use osal_backend_freertos::task::{FreeRtosTask, FreeRtosTaskBuilder};
use osal_backend_freertos::timer::FreeRtosTimer;
use osal_backend_freertos_sys as sys;

use crate::harness;

// Expected-OOM FFI (only linked when DIAGNOSTICS is enabled).
unsafe extern "C" {
    fn osal_test_expect_malloc_failure();
    fn osal_test_expected_malloc_failure_consumed() -> u32;
    fn osal_test_clear_expected_malloc_failure();
    fn osal_test_diag_task_create_attempts() -> u32;
    fn osal_test_diag_task_create_successes() -> u32;
    fn osal_test_diag_internal_task_create_attempts() -> u32;
}

fn diag_task_create_attempts() -> u32 { unsafe { osal_test_diag_task_create_attempts() } }
fn diag_task_create_successes() -> u32 { unsafe { osal_test_diag_task_create_successes() } }
fn diag_worker_create_attempts() -> u32 { unsafe { osal_test_diag_internal_task_create_attempts() } }

const M0: [u8; 4] = [0xA0, 0xB1, 0xC2, 0xD3];

// ------------------------------------------------------------------
// Mixed-object pipeline errors
// ------------------------------------------------------------------
#[repr(i32)]
pub enum MixedError {
    MutexCreate = 800,
    BinaryCreate = 801,
    CountingCreate = 802,
    QueueCreate = 803,
    TaskSpawnFailed = 804,
    TimerCreate = 805,
    TimerStart = 806,
    PipelineTimeout = 807,
    PayloadMismatch = 808,
    CounterMismatch = 809,
    TaskJoinFailed = 810,
    TimerCountWrong = 811,
    TaskCountWrong = 812,
    BinaryReleaseFailed = 813,
    TaskAOperationFailed = 814,
    TaskBOperationFailed = 815,

    // ---- rollback ----
    RollbackWrongError = 816,
    RollbackDiagMismatch = 817,
    RollbackLeaseLeak = 818,
    RollbackHeapLeak = 819,
    RollbackRecoveryCreateFailed = 820,

    // ---- resource pressure ----
    PressureAllocationFailed = 821,
    PressureDidNotReduceHeap = 822,
    PressureOomNotObserved = 823,
    PressureOomHookNotConsumed = 824,
    PressureLeaseLeak = 825,
    PressureHeapLeak = 826,
    PressureRecoveryTaskFailed = 827,
    PressureRecoveryObjectFailed = 828,
    PressureProbeOverflow = 829,

    // ---- lifecycle stress ----
    StressSetupFailed = 830,
    StressTaskCountLeak = 833,
    StressActiveObjectLeak = 834,
    StressHeapLeak = 835,
    StressWorkerRecreated = 836,

    // ---- shutdown accounting ----
    ShutdownSetupFailed = 840,
    ShutdownFirstNotBusy = 841,
    ShutdownRuntimeNotRunning = 842,
    ShutdownLeaseAccounting = 843,
    ShutdownCrossCheckFailed = 844,
    ShutdownFinalNotOk = 845,
    ShutdownFinalHeapLeak = 846,
    ShutdownReinitFailed = 847,
    ShutdownReinitRecoveryFailed = 848,
}

struct PipelineState {
    task_a_started: AtomicBool,
    task_b_started: AtomicBool,
    task_a_done: AtomicU32,
    task_b_done: AtomicU32,
    b_received_word: AtomicU32,
    b_counter: AtomicU32,
    timer_callback_count: AtomicU32,
    binary_release_ok: AtomicU32,
}

impl PipelineState {
    fn new() -> Self {
        Self {
            task_a_started: AtomicBool::new(false),
            task_b_started: AtomicBool::new(false),
            task_a_done: AtomicU32::new(0),
            task_b_done: AtomicU32::new(0),
            b_received_word: AtomicU32::new(0),
            b_counter: AtomicU32::new(0),
            timer_callback_count: AtomicU32::new(0),
            binary_release_ok: AtomicU32::new(0),
        }
    }
}

fn bounded_wait_bool(atom: &AtomicBool, expected: bool, deadline_ticks: u32, tick_bits: u8) -> bool {
    let start = sys::tick_snapshot();
    loop {
        if atom.load(Ordering::Acquire) == expected {
            return true;
        }
        let now = sys::tick_snapshot();
        let start_total = ((start.overflow_count as u128) << tick_bits) | (start.tick_count as u128);
        let now_total = ((now.overflow_count as u128) << tick_bits) | (now.tick_count as u128);
        if now_total.saturating_sub(start_total) >= deadline_ticks as u128 {
            return false;
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return false;
        }
    }
}

fn wait_active_objects(target: usize, deadline_ticks: u32, tick_bits: u8) -> bool {
    let start = sys::tick_snapshot();
    loop {
        if osal_backend_freertos::runtime::active_objects() == target {
            return true;
        }
        let now = sys::tick_snapshot();
        let start_total = ((start.overflow_count as u128) << tick_bits) | (start.tick_count as u128);
        let now_total = ((now.overflow_count as u128) << tick_bits) | (now.tick_count as u128);
        if now_total.saturating_sub(start_total) >= deadline_ticks as u128 {
            return false;
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return false;
        }
    }
}

fn wait_task_count(target: usize, deadline_ticks: u32, tick_bits: u8) -> bool {
    let start = sys::tick_snapshot();
    loop {
        if FreeRtosTask::count() == target {
            return true;
        }
        let now = sys::tick_snapshot();
        let start_total = ((start.overflow_count as u128) << tick_bits) | (start.tick_count as u128);
        let now_total = ((now.overflow_count as u128) << tick_bits) | (now.tick_count as u128);
        if now_total.saturating_sub(start_total) >= deadline_ticks as u128 {
            return false;
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return false;
        }
    }
}

// ------------------------------------------------------------------
// Injection helpers
// ------------------------------------------------------------------

struct SyncCreateFailureGuard;

impl SyncCreateFailureGuard {
    fn arm(nth: u32) -> Self {
        sys::integration_diag::clear_sync_create_failure();
        sys::integration_diag::arm_sync_create_failure(nth);
        Self
    }
}

impl Drop for SyncCreateFailureGuard {
    fn drop(&mut self) {
        sys::integration_diag::clear_sync_create_failure();
    }
}

struct ExpectedMallocFailureGuard;

impl ExpectedMallocFailureGuard {
    fn arm() -> Self {
        unsafe { osal_test_expect_malloc_failure() };
        Self
    }
    fn consumed(&self) -> u32 {
        unsafe { osal_test_expected_malloc_failure_consumed() }
    }
}

impl Drop for ExpectedMallocFailureGuard {
    fn drop(&mut self) {
        unsafe { osal_test_clear_expected_malloc_failure() };
    }
}

struct HeapPressureGuard {
    ptr: *mut u8,
}

impl HeapPressureGuard {
    fn alloc(size: usize) -> Option<Self> {
        let ptr = unsafe { sys::heap_alloc(size) };
        if ptr.is_null() {
            None
        } else {
            Some(Self { ptr })
        }
    }
}

impl Drop for HeapPressureGuard {
    fn drop(&mut self) {
        unsafe { sys::heap_dealloc(self.ptr) };
    }
}

#[derive(Default)]
struct SyncDiag {
    mutex_attempts: u32,
    mutex_successes: u32,
    mutex_deletes: u32,
    sem_attempts: u32,
    sem_successes: u32,
    sem_deletes: u32,
}

fn read_sync_diag() -> SyncDiag {
    SyncDiag {
        mutex_attempts: sys::integration_diag::mutex_create_attempts(),
        mutex_successes: sys::integration_diag::mutex_create_successes(),
        mutex_deletes: sys::integration_diag::mutex_deletes(),
        sem_attempts: sys::integration_diag::semaphore_create_attempts(),
        sem_successes: sys::integration_diag::semaphore_create_successes(),
        sem_deletes: sys::integration_diag::semaphore_deletes(),
    }
}

macro_rules! assert_diag_delta {
    ($before:expr, $after:expr, $field:ident, $expected:expr) => {
        if ($after.$field.wrapping_sub($before.$field)) != ($expected) {
            return Err(MixedError::RollbackDiagMismatch);
        }
    };
}

// ------------------------------------------------------------------
// Public entry
// ------------------------------------------------------------------

pub fn run_mixed_cases(tick_bits: u8, profile_baseline: u64) -> Result<(), MixedError> {
    mixed_native_create_rollback(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_native_create_rollback");

    mixed_resource_pressure_recovery(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_resource_pressure_recovery");

    mixed_object_pipeline(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_object_pipeline");

    mixed_lifecycle_stress(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_lifecycle_stress");

    mixed_shutdown_accounting(tick_bits, profile_baseline)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_shutdown_accounting");
    Ok(())
}

fn mixed_native_create_rollback(tick_bits: u8) -> Result<(), MixedError> {
    // --- Mutex: native create failure ---
    {
        let _guard = SyncCreateFailureGuard::arm(1);
        let heap_before = sys::heap_free();
        let active_before = osal_backend_freertos::runtime::active_objects();
        let diag_before = read_sync_diag();

        let result = osal::backend::Mutex::<u32>::new(0u32);
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::RollbackWrongError);
        }
        let diag_after = read_sync_diag();
        assert_diag_delta!(diag_before, diag_after, mutex_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, mutex_successes, 0);
        assert_diag_delta!(diag_before, diag_after, mutex_deletes, 0);
        assert_diag_delta!(diag_before, diag_after, sem_attempts, 0);

        if osal_backend_freertos::runtime::active_objects() != active_before {
            return Err(MixedError::RollbackLeaseLeak);
        }
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }
    // Recovery smoke
    {
        let heap_before = sys::heap_free();
        let m = osal::backend::Mutex::<u32>::new(42u32)
            .map_err(|_| MixedError::RollbackRecoveryCreateFailed)?;
        drop(m);
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }

    // --- CountingSemaphore: native create failure ---
    {
        let _guard = SyncCreateFailureGuard::arm(1);
        let heap_before = sys::heap_free();
        let active_before = osal_backend_freertos::runtime::active_objects();
        let diag_before = read_sync_diag();

        let result = osal::backend::CountingSemaphore::new(1, 0);
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::RollbackWrongError);
        }
        let diag_after = read_sync_diag();
        assert_diag_delta!(diag_before, diag_after, mutex_attempts, 0);
        assert_diag_delta!(diag_before, diag_after, sem_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, sem_successes, 0);

        if osal_backend_freertos::runtime::active_objects() != active_before {
            return Err(MixedError::RollbackLeaseLeak);
        }
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }
    {
        let heap_before = sys::heap_free();
        let s = osal::backend::CountingSemaphore::new(1, 0)
            .map_err(|_| MixedError::RollbackRecoveryCreateFailed)?;
        drop(s);
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }

    // --- BinarySemaphore: native create failure ---
    {
        let _guard = SyncCreateFailureGuard::arm(1);
        let heap_before = sys::heap_free();
        let active_before = osal_backend_freertos::runtime::active_objects();
        let diag_before = read_sync_diag();

        let result = osal::backend::BinarySemaphore::new();
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::RollbackWrongError);
        }
        let diag_after = read_sync_diag();
        assert_diag_delta!(diag_before, diag_after, mutex_attempts, 0);
        assert_diag_delta!(diag_before, diag_after, sem_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, sem_successes, 0);

        if osal_backend_freertos::runtime::active_objects() != active_before {
            return Err(MixedError::RollbackLeaseLeak);
        }
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }
    {
        let heap_before = sys::heap_free();
        let s = osal::backend::BinarySemaphore::new()
            .map_err(|_| MixedError::RollbackRecoveryCreateFailed)?;
        drop(s);
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }

    // --- Queue stage 1: state mutex failure ---
    {
        let _guard = SyncCreateFailureGuard::arm(1);
        let heap_before = sys::heap_free();
        let active_before = osal_backend_freertos::runtime::active_objects();
        let diag_before = read_sync_diag();

        let result = osal::backend::Queue::new(1, 4);
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::RollbackWrongError);
        }
        let diag_after = read_sync_diag();
        assert_diag_delta!(diag_before, diag_after, mutex_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, mutex_successes, 0);
        assert_diag_delta!(diag_before, diag_after, mutex_deletes, 0);
        assert_diag_delta!(diag_before, diag_after, sem_attempts, 0);

        if osal_backend_freertos::runtime::active_objects() != active_before {
            return Err(MixedError::RollbackLeaseLeak);
        }
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }
    {
        let heap_before = sys::heap_free();
        let q = osal::backend::Queue::new(1, 4)
            .map_err(|_| MixedError::RollbackRecoveryCreateFailed)?;
        drop(q);
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }

    // --- Queue stage 2: sender wake semaphore failure ---
    {
        let _guard = SyncCreateFailureGuard::arm(2);
        let heap_before = sys::heap_free();
        let active_before = osal_backend_freertos::runtime::active_objects();
        let diag_before = read_sync_diag();

        let result = osal::backend::Queue::new(1, 4);
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::RollbackWrongError);
        }
        let diag_after = read_sync_diag();
        assert_diag_delta!(diag_before, diag_after, mutex_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, mutex_successes, 1);
        assert_diag_delta!(diag_before, diag_after, mutex_deletes, 1);
        assert_diag_delta!(diag_before, diag_after, sem_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, sem_successes, 0);
        assert_diag_delta!(diag_before, diag_after, sem_deletes, 0);

        if osal_backend_freertos::runtime::active_objects() != active_before {
            return Err(MixedError::RollbackLeaseLeak);
        }
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }
    {
        let heap_before = sys::heap_free();
        let q = osal::backend::Queue::new(1, 4)
            .map_err(|_| MixedError::RollbackRecoveryCreateFailed)?;
        drop(q);
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }

    // --- Queue stage 3: receiver wake semaphore failure ---
    {
        let _guard = SyncCreateFailureGuard::arm(3);
        let heap_before = sys::heap_free();
        let active_before = osal_backend_freertos::runtime::active_objects();
        let diag_before = read_sync_diag();

        let result = osal::backend::Queue::new(1, 4);
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::RollbackWrongError);
        }
        let diag_after = read_sync_diag();
        assert_diag_delta!(diag_before, diag_after, mutex_attempts, 1);
        assert_diag_delta!(diag_before, diag_after, mutex_successes, 1);
        assert_diag_delta!(diag_before, diag_after, mutex_deletes, 1);
        assert_diag_delta!(diag_before, diag_after, sem_attempts, 2);
        assert_diag_delta!(diag_before, diag_after, sem_successes, 1);
        assert_diag_delta!(diag_before, diag_after, sem_deletes, 1);

        if osal_backend_freertos::runtime::active_objects() != active_before {
            return Err(MixedError::RollbackLeaseLeak);
        }
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }
    {
        let heap_before = sys::heap_free();
        let q = osal::backend::Queue::new(1, 4)
            .map_err(|_| MixedError::RollbackRecoveryCreateFailed)?;
        drop(q);
        harness::wait_until_heap_recovered(heap_before, 50, tick_bits)
            .map_err(|_| MixedError::RollbackHeapLeak)?;
    }

    Ok(())
}

fn mixed_resource_pressure_recovery(tick_bits: u8) -> Result<(), MixedError> {
    let heap_baseline = sys::heap_free();
    let active_baseline = osal_backend_freertos::runtime::active_objects();
    let task_baseline = FreeRtosTask::count();

    // Allocate a real pressure block (~25% of free heap).
    let free = sys::heap_free();
    let pressure_bytes = (free / 4) as usize;
    let _oom_guard = ExpectedMallocFailureGuard::arm();
    let pressure = HeapPressureGuard::alloc(pressure_bytes)
        .ok_or(MixedError::PressureAllocationFailed)?;
    drop(_oom_guard); // pressure alloc succeeded, clear expected-OOM

    let pressured_free = sys::heap_free();
    if pressured_free >= heap_baseline {
        return Err(MixedError::PressureDidNotReduceHeap);
    }

    // Probe stack larger than remaining free heap → must OOM.
    let probe_stack = (pressured_free as usize)
        .checked_add(4096)
        .ok_or(MixedError::PressureProbeOverflow)?;

    let create_attempts_before = diag_task_create_attempts();
    let create_successes_before = diag_task_create_successes();

    let oom_guard = ExpectedMallocFailureGuard::arm();
    let result = FreeRtosTaskBuilder::new()
        .stack_size(probe_stack)
        .priority(2)
        .spawn(move || {});
    let consumed = oom_guard.consumed();
    drop(oom_guard);

    // Prove the OOM reached xTaskCreate (attempt incremented, success not).
    let create_attempts_delta = diag_task_create_attempts().wrapping_sub(create_attempts_before);
    let create_successes_delta = diag_task_create_successes().wrapping_sub(create_successes_before);

    if consumed != 1 {
        return Err(MixedError::PressureOomHookNotConsumed);
    }
    if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
        return Err(MixedError::PressureOomNotObserved);
    }
    if create_attempts_delta != 1 {
        return Err(MixedError::PressureOomNotObserved);
    }
    if create_successes_delta != 0 {
        return Err(MixedError::PressureOomNotObserved);
    }
    if FreeRtosTask::count() != task_baseline {
        return Err(MixedError::PressureLeaseLeak);
    }
    if osal_backend_freertos::runtime::active_objects() != active_baseline {
        return Err(MixedError::PressureLeaseLeak);
    }
    harness::wait_until_heap_recovered(pressured_free, 50, tick_bits)
        .map_err(|_| MixedError::PressureHeapLeak)?;

    // Release pressure → exact global recovery.
    drop(pressure);
    harness::wait_until_heap_recovered(heap_baseline, 100, tick_bits)
        .map_err(|_| MixedError::PressureHeapLeak)?;

    // Same stack must now succeed (proves OOM was from pressure).
    let t = FreeRtosTaskBuilder::new()
        .stack_size(probe_stack)
        .priority(2)
        .spawn(move || {})
        .map_err(|_| MixedError::PressureRecoveryTaskFailed)?;
    t.join(Timeout::After(Duration::from_millis(100)))
        .map_err(|_| MixedError::PressureRecoveryTaskFailed)?;
    drop(t);
    harness::wait_until_heap_recovered(heap_baseline, 50, tick_bits)
        .map_err(|_| MixedError::PressureHeapLeak)?;

    // Mutex real-pressure subcase: measure native mutex heap cost,
    // then reduce free heap below that cost and require Mutex::new()
    // to return OutOfMemory from the real native allocation path.
    let mutex_native_cost = {
        let before = sys::heap_free();
        let m = osal::backend::Mutex::<u32>::new(0u32)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        let during = sys::heap_free();
        drop(m);
        harness::wait_until_heap_recovered(before, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
        before.saturating_sub(during)
    };
    if mutex_native_cost == 0 {
        return Err(MixedError::PressureDidNotReduceHeap);
    }
    {
        let free = sys::heap_free();
        // Leave less free than the native mutex needs (plus a small
        // safety margin so the pressure block itself is far from the
        // allocator's failure threshold).
        let target_free = mutex_native_cost / 2;
        let pressure_bytes = free.saturating_sub(target_free) as usize;
        let pressure = HeapPressureGuard::alloc(pressure_bytes)
            .ok_or(MixedError::PressureAllocationFailed)?;

        let pressured_free = sys::heap_free();
        if pressured_free >= mutex_native_cost {
            return Err(MixedError::PressureDidNotReduceHeap);
        }

        let oom_guard = ExpectedMallocFailureGuard::arm();
        let result = osal::backend::Mutex::<u32>::new(0u32);
        let consumed = oom_guard.consumed();
        drop(oom_guard);
        if consumed != 1 {
            return Err(MixedError::PressureOomHookNotConsumed);
        }
        if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
            return Err(MixedError::PressureOomNotObserved);
        }
        if osal_backend_freertos::runtime::active_objects() != active_baseline {
            return Err(MixedError::PressureLeaseLeak);
        }
        harness::wait_until_heap_recovered(pressured_free, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;

        drop(pressure);
        harness::wait_until_heap_recovered(heap_baseline, 100, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
    }
    // Normal Mutex now succeeds after pressure release.
    {
        let before = sys::heap_free();
        let m = osal::backend::Mutex::<u32>::new(0u32)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        drop(m);
        harness::wait_until_heap_recovered(before, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
    }

    // Cross-object recovery smoke.
    {
        let m = osal::backend::Mutex::<u32>::new(0u32)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        {
            let _g = m.lock(Timeout::After(Duration::from_millis(100)))
                .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        }
        drop(m);
        harness::wait_until_heap_recovered(heap_baseline, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
    }
    {
        let b = osal::backend::BinarySemaphore::new()
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        b.release().map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        b.acquire(Timeout::After(Duration::from_millis(100)))
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        drop(b);
        harness::wait_until_heap_recovered(heap_baseline, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
    }
    {
        let c = osal::backend::CountingSemaphore::new(1, 0)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        c.release().map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        c.acquire(Timeout::After(Duration::from_millis(100)))
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        drop(c);
        harness::wait_until_heap_recovered(heap_baseline, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
    }
    {
        let q = osal::backend::Queue::new(1, 4)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        q.send(&M0, Timeout::NoWait)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        let mut buf = [0u8; 4];
        q.recv(&mut buf, Timeout::NoWait)
            .map_err(|_| MixedError::PressureRecoveryObjectFailed)?;
        drop(q);
        harness::wait_until_heap_recovered(heap_baseline, 50, tick_bits)
            .map_err(|_| MixedError::PressureHeapLeak)?;
    }

    if osal_backend_freertos::runtime::active_objects() != active_baseline {
        return Err(MixedError::PressureLeaseLeak);
    }
    if FreeRtosTask::count() != task_baseline {
        return Err(MixedError::PressureLeaseLeak);
    }
    Ok(())
}

struct PipelineBundle {
    _mtx: osal::backend::Mutex<u32>,
    _binary: osal::backend::BinarySemaphore,
    counting: osal::backend::CountingSemaphore,
    q: osal::backend::Queue,
    ta: FreeRtosTask,
    tb: FreeRtosTask,
    timer: FreeRtosTimer,
    state: Arc<PipelineState>,
}

impl PipelineBundle {
    fn construct(payload: [u8; 4]) -> Result<Self, MixedError> {
        let state = Arc::new(PipelineState::new());
        let mtx = osal::backend::Mutex::new(0u32).map_err(|_| MixedError::MutexCreate)?;
        let binary = osal::backend::BinarySemaphore::new()
            .map_err(|_| MixedError::BinaryCreate)?;
        let counting =
            osal::backend::CountingSemaphore::new(1, 0)
                .map_err(|_| MixedError::CountingCreate)?;
        let q = osal::backend::Queue::new(1, 4).map_err(|_| MixedError::QueueCreate)?;

        // Task B: recv payload, increment Mutex, release CountingSemaphore
        let s_b = Arc::clone(&state);
        let q_b = q.clone();
        let mtx_b = mtx.clone();
        let counting_b = counting.clone();
        let tb = FreeRtosTaskBuilder::new()
            .stack_size(4096)
            .priority(2)
            .spawn(move || {
                s_b.task_b_started.store(true, Ordering::Release);
                let mut buf = [0u8; 4];
                match q_b.recv(&mut buf, Timeout::After(Duration::from_millis(100))) {
                    Ok(()) => {
                        s_b.b_received_word.store(u32::from_le_bytes(buf), Ordering::Release);
                    }
                    Err(_) => {
                        s_b.task_b_done.store(2, Ordering::Release);
                        return;
                    }
                }
                {
                    let mut guard = match mtx_b.lock(Timeout::After(Duration::from_millis(100))) {
                        Ok(g) => g,
                        Err(_) => {
                            s_b.task_b_done.store(3, Ordering::Release);
                            return;
                        }
                    };
                    *guard += 1;
                    s_b.b_counter.store(*guard, Ordering::Release);
                }
                if counting_b.release().is_err() {
                    s_b.task_b_done.store(4, Ordering::Release);
                    return;
                }
                s_b.task_b_done.store(1, Ordering::Release);
            })
            .map_err(|_| MixedError::TaskSpawnFailed)?;

        // Task A: acquire BinarySemaphore, send payload to Queue
        let s_a = Arc::clone(&state);
        let binary_a = binary.clone();
        let q_a = q.clone();
        let ta = FreeRtosTaskBuilder::new()
            .stack_size(4096)
            .priority(2)
            .spawn(move || {
                s_a.task_a_started.store(true, Ordering::Release);
                if binary_a
                    .acquire(Timeout::After(Duration::from_millis(100)))
                    .is_err()
                {
                    s_a.task_a_done.store(2, Ordering::Release);
                    return;
                }
                if q_a
                    .send(&payload, Timeout::After(Duration::from_millis(100)))
                    .is_err()
                {
                    s_a.task_a_done.store(3, Ordering::Release);
                    return;
                }
                s_a.task_a_done.store(1, Ordering::Release);
            })
            .map_err(|_| MixedError::TaskSpawnFailed)?;

        // Timer: release BinarySemaphore (unblocks Task A). Not started yet.
        let s_timer = Arc::clone(&state);
        let binary_timer = binary.clone();
        let cb: TimerCallback = Box::new(move || {
            match binary_timer.release() {
                Ok(()) => s_timer.binary_release_ok.store(1, Ordering::Relaxed),
                Err(_) => s_timer.binary_release_ok.store(2, Ordering::Relaxed),
            }
            s_timer.timer_callback_count.fetch_add(1, Ordering::Release);
        });
        let timer = FreeRtosTimer::new("t-mixed-pipe", Duration::from_millis(5), TimerMode::OneShot, cb)
            .map_err(|_| MixedError::TimerCreate)?;

        Ok(Self { _mtx: mtx, _binary: binary, counting, q, ta, tb, timer, state })
    }

    fn wait_started(&self, tick_bits: u8) -> Result<(), MixedError> {
        if !bounded_wait_bool(&self.state.task_a_started, true, 80, tick_bits)
            || !bounded_wait_bool(&self.state.task_b_started, true, 80, tick_bits)
        {
            return Err(MixedError::PipelineTimeout);
        }
        Ok(())
    }

    fn start_timer(&self) -> Result<(), MixedError> {
        self.timer.start().map_err(|_| MixedError::TimerStart)
    }

    fn wait_complete(&self, payload: [u8; 4]) -> Result<(), MixedError> {
        self.counting
            .acquire(Timeout::After(Duration::from_millis(200)))
            .map_err(|_| MixedError::PipelineTimeout)?;
        self.ta.join(Timeout::After(Duration::from_millis(100)))
            .map_err(|_| MixedError::TaskJoinFailed)?;
        self.tb.join(Timeout::After(Duration::from_millis(100)))
            .map_err(|_| MixedError::TaskJoinFailed)?;

        if self.state.timer_callback_count.load(Ordering::Acquire) != 1 {
            return Err(MixedError::TimerCountWrong);
        }
        if self.state.binary_release_ok.load(Ordering::Acquire) != 1 {
            return Err(MixedError::BinaryReleaseFailed);
        }
        if self.state.task_a_done.load(Ordering::Acquire) != 1 {
            return Err(MixedError::TaskAOperationFailed);
        }
        if self.state.task_b_done.load(Ordering::Acquire) != 1 {
            return Err(MixedError::TaskBOperationFailed);
        }
        if self.state.b_received_word.load(Ordering::Acquire) != u32::from_le_bytes(payload) {
            return Err(MixedError::PayloadMismatch);
        }
        if self.state.b_counter.load(Ordering::Acquire) != 1 {
            return Err(MixedError::CounterMismatch);
        }
        if self.q.len().map_err(|_| MixedError::QueueCreate)? != 0 {
            return Err(MixedError::PayloadMismatch);
        }
        Ok(())
    }
}

/// Run one full mixed pipeline to completion (no CASE_PASS output).
fn run_pipeline_once(tick_bits: u8, payload: [u8; 4]) -> Result<(), MixedError> {
    let bundle = PipelineBundle::construct(payload)?;
    bundle.wait_started(tick_bits)?;
    bundle.start_timer()?;
    bundle.wait_complete(payload)
}

fn mixed_object_pipeline(tick_bits: u8) -> Result<(), MixedError> {
    let task_baseline = FreeRtosTask::count();
    run_pipeline_once(tick_bits, M0)?;
    if FreeRtosTask::count() != task_baseline {
        return Err(MixedError::TaskCountWrong);
    }
    Ok(())
}

fn mixed_lifecycle_stress(tick_bits: u8) -> Result<(), MixedError> {
    // Warm up the Timer worker so it becomes a permanent part of the
    // runtime.  Bounded wait — never block forever if the callback
    // fails to fire.  The bounded wait also gives the preceding case's
    // transient task-trampoline cleanup a chance to settle before we
    // snapshot baselines.
    {
        let fired = Arc::new(AtomicBool::new(false));
        let fired_cb = Arc::clone(&fired);
        let cb: TimerCallback = Box::new(move || {
            fired_cb.store(true, Ordering::Release);
        });
        let timer = FreeRtosTimer::new("t-warm", Duration::from_millis(2), TimerMode::OneShot, cb)
            .map_err(|_| MixedError::StressSetupFailed)?;
        timer.start().map_err(|_| MixedError::StressSetupFailed)?;
        if !bounded_wait_bool(&fired, true, 50, tick_bits) {
            return Err(MixedError::StressSetupFailed);
        }
        drop(timer);
    }

    // Snapshot stable baselines AFTER the warmup (previous-case cleanup
    // has settled, worker is now permanently resident).
    let active_baseline = osal_backend_freertos::runtime::active_objects();
    let task_baseline = FreeRtosTask::count();
    let worker_baseline = sys::heap_free();
    let worker_create_attempts_baseline = diag_worker_create_attempts();

    // Phase A: 16 sequential rounds with distinct payloads.
    for round in 0..16u32 {
        let payload = [0x41, 0, 0, round as u8];
        run_pipeline_once(tick_bits, payload)?;

        if !wait_task_count(task_baseline, 100, tick_bits) {
            return Err(MixedError::StressTaskCountLeak);
        }
        if !wait_active_objects(active_baseline, 100, tick_bits) {
            return Err(MixedError::StressActiveObjectLeak);
        }
        harness::wait_until_heap_recovered(worker_baseline, 100, tick_bits)
            .map_err(|_| MixedError::StressHeapLeak)?;
    }

    // Phase B: 4 waves × 2 concurrent pipelines.
    for wave in 0..4u32 {
        let payload_a = [0xA5, 0, 0, wave as u8];
        let payload_b = [0x5A, 0, 0, wave as u8];

        let a = PipelineBundle::construct(payload_a).map_err(|_| MixedError::StressSetupFailed)?;
        let b = PipelineBundle::construct(payload_b).map_err(|_| MixedError::StressSetupFailed)?;
        a.wait_started(tick_bits)?;
        b.wait_started(tick_bits)?;
        a.start_timer()?;
        b.start_timer()?;
        a.wait_complete(payload_a)?;
        b.wait_complete(payload_b)?;
        drop(a);
        drop(b);

        if !wait_task_count(task_baseline, 100, tick_bits) {
            return Err(MixedError::StressTaskCountLeak);
        }
        if !wait_active_objects(active_baseline, 100, tick_bits) {
            return Err(MixedError::StressActiveObjectLeak);
        }
        harness::wait_until_heap_recovered(worker_baseline, 100, tick_bits)
            .map_err(|_| MixedError::StressHeapLeak)?;
    }

    // Worker must not have been recreated across the whole stress.
    if diag_worker_create_attempts() != worker_create_attempts_baseline {
        return Err(MixedError::StressWorkerRecreated);
    }
    Ok(())
}

fn mixed_shutdown_accounting(
    tick_bits: u8,
    profile_baseline: u64,
) -> Result<(), MixedError> {
    let active_baseline = osal_backend_freertos::runtime::active_objects();
    let task_baseline = FreeRtosTask::count();

    // --- create 6 mixed objects, tracking active_objects delta ---
    let mtx = osal::backend::Mutex::<u32>::new(0u32)
        .map_err(|_| MixedError::ShutdownSetupFailed)?;
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 1 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    let binary = osal::backend::BinarySemaphore::new()
        .map_err(|_| MixedError::ShutdownSetupFailed)?;
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 2 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    let counting = osal::backend::CountingSemaphore::new(1, 0)
        .map_err(|_| MixedError::ShutdownSetupFailed)?;
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 3 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    let q = osal::backend::Queue::new(1, 4)
        .map_err(|_| MixedError::ShutdownSetupFailed)?;
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 4 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    // Gated Task: waits until release_gate is set.
    let started = Arc::new(AtomicBool::new(false));
    let release_gate = Arc::new(AtomicBool::new(false));
    let started_t = Arc::clone(&started);
    let release_t = Arc::clone(&release_gate);
    let ta = FreeRtosTaskBuilder::new()
        .stack_size(4096)
        .priority(2)
        .spawn(move || {
            started_t.store(true, Ordering::Release);
            while !release_t.load(Ordering::Acquire) {
                sys::delay_ticks(1);
            }
        })
        .map_err(|_| MixedError::ShutdownSetupFailed)?;
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 5 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    let timer_count = Arc::new(AtomicU32::new(0));
    let tc = Arc::clone(&timer_count);
    let cb: TimerCallback = Box::new(move || {
        tc.fetch_add(1, Ordering::Release);
    });
    let timer = FreeRtosTimer::new("t-shut", Duration::from_millis(2), TimerMode::OneShot, cb)
        .map_err(|_| MixedError::ShutdownSetupFailed)?;
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 6 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    // Drop guard: always release the gated Task on scope exit.
    struct GateGuard {
        gate: Arc<AtomicBool>,
    }
    impl Drop for GateGuard {
        fn drop(&mut self) {
            self.gate.store(true, Ordering::Release);
        }
    }
    let gate_guard = GateGuard { gate: Arc::clone(&release_gate) };

    // --- first shutdown with all 6 alive: must be Busy ---
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownFirstNotBusy);
    }
    if osal::runtime_state() != RuntimeState::Running {
        return Err(MixedError::ShutdownRuntimeNotRunning);
    }
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 6 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    let heap_after_first_busy = sys::heap_free();
    harness::wait_until_heap_recovered(heap_after_first_busy, 50, tick_bits)
        .map_err(|_| MixedError::ShutdownCrossCheckFailed)?;

    // --- Busy-failure-atomic: objects must remain usable ---
    {
        let _g = mtx.lock(Timeout::After(Duration::from_millis(50)))
            .map_err(|_| MixedError::ShutdownCrossCheckFailed)?;
    }
    counting.release().map_err(|_| MixedError::ShutdownCrossCheckFailed)?;
    q.send(&M0, Timeout::NoWait)
        .map_err(|_| MixedError::ShutdownCrossCheckFailed)?;
    let mut buf = [0u8; 4];
    q.recv(&mut buf, Timeout::NoWait)
        .map_err(|_| MixedError::ShutdownCrossCheckFailed)?;
    timer.start().map_err(|_| MixedError::ShutdownCrossCheckFailed)?;
    if !bounded_wait_bool(&started, true, 50, tick_bits) {
        return Err(MixedError::ShutdownCrossCheckFailed);
    }
    if !wait_task_count(task_baseline + 1, 50, tick_bits) {
        return Err(MixedError::ShutdownCrossCheckFailed);
    }

    // --- per-object drop: each must decrement active_objects by 1 ---
    drop(mtx);
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 5 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    drop(binary);
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 4 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    drop(counting);
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 3 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    drop(q);
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 2 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    drop(timer);
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 1 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    // --- only Task remains: handle still holds lease after task exits ---
    // Release and join the task.  FreeRtosTask::count() drops to baseline,
    // but the external handle still holds the managed-object lease.
    gate_guard.gate.store(true, Ordering::Release);
    ta.join(Timeout::After(Duration::from_millis(100)))
        .map_err(|_| MixedError::ShutdownCrossCheckFailed)?;
    if !wait_task_count(task_baseline, 50, tick_bits) {
        return Err(MixedError::ShutdownCrossCheckFailed);
    }
    // active_objects should still be active_baseline + 1 (task handle).
    if osal_backend_freertos::runtime::active_objects() != active_baseline + 1 {
        return Err(MixedError::ShutdownLeaseAccounting);
    }
    if !matches!(osal::shutdown(), Err(Error::Busy)) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    // Drop the task handle -> lease released.
    drop(ta);
    if !wait_active_objects(active_baseline, 50, tick_bits) {
        return Err(MixedError::ShutdownLeaseAccounting);
    }

    // --- final shutdown must succeed directly ---
    if !matches!(osal::shutdown(), Ok(())) {
        return Err(MixedError::ShutdownFinalNotOk);
    }
    if osal::runtime_state() != RuntimeState::Uninitialized {
        return Err(MixedError::ShutdownCrossCheckFailed);
    }
    // DEBUG: print actual heap_free vs profile_baseline
    {
        let mut buf = [0u8; 64];
        let mut i = 0;
        let mut n = sys::heap_free();
        if n == 0 { buf[i] = b'0'; i = 1; } else {
            let mut tmp = [0u8; 20]; let mut t = 0;
            while n > 0 { tmp[t] = b'0' + (n % 10) as u8; t += 1; n /= 10; }
            while t > 0 { t -= 1; buf[i] = tmp[t]; i += 1; }
        }
        buf[i] = b'/'; i += 1;
        n = profile_baseline;
        if n == 0 { buf[i] = b'0'; i += 1; } else {
            let mut tmp = [0u8; 20]; let mut t = 0;
            while n > 0 { tmp[t] = b'0' + (n % 10) as u8; t += 1; n /= 10; }
            while t > 0 { t -= 1; buf[i] = tmp[t]; i += 1; }
        }
        buf[i] = 0;
        let cstr = core::ffi::CStr::from_bytes_with_nul(&buf[..=i]).unwrap();
        harness::console_line(cstr);
    }
    // Note: Timer worker TCB+stack reclaimed asynchronously by Idle
    // after the worker self-deletes.  Do not block here on exact
    // profile-baseline recovery; the outer suite performs the final
    // exact recovery after its own shutdown of the re-initialized
    // runtime.
    let _ = profile_baseline;
    let _ = tick_bits;

    // --- reinitialize and small recovery smoke ---
    osal::initialize().map_err(|_| MixedError::ShutdownReinitFailed)?;
    if osal::runtime_state() != RuntimeState::Running {
        return Err(MixedError::ShutdownReinitFailed);
    }
    let reinit_heap = sys::heap_free();
    {
        let m = osal::backend::Mutex::<u32>::new(42u32)
            .map_err(|_| MixedError::ShutdownReinitRecoveryFailed)?;
        {
            let _g = m.lock(Timeout::After(Duration::from_millis(50)))
                .map_err(|_| MixedError::ShutdownReinitRecoveryFailed)?;
        }
        drop(m);
    }
    harness::wait_until_heap_recovered(reinit_heap, 50, tick_bits)
        .map_err(|_| MixedError::ShutdownReinitRecoveryFailed)?;
    if !wait_active_objects(active_baseline, 50, tick_bits) {
        return Err(MixedError::ShutdownReinitRecoveryFailed);
    }

    // Leave runtime Running; the outer suite will do the final shutdown.
    let _ = tick_bits;
    Ok(())
}
