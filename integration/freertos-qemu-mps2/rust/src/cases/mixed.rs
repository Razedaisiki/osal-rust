//! FreeRTOS mixed-object real-kernel integration contracts.
//!
//! Validates that Mutex, CountingSemaphore, BinarySemaphore, Queue,
//! Task, and Timer compose correctly in a single runtime session.

use alloc::boxed::Box;
use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use core::time::Duration;

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
}

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

pub fn run_mixed_cases(tick_bits: u8) -> Result<(), MixedError> {
    mixed_native_create_rollback(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_native_create_rollback");

    mixed_resource_pressure_recovery(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_resource_pressure_recovery");

    mixed_object_pipeline(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_object_pipeline");
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
    let probe_stack = (pressured_free as usize).saturating_add(4096);

    let oom_guard = ExpectedMallocFailureGuard::arm();
    let result = FreeRtosTaskBuilder::new()
        .stack_size(probe_stack)
        .priority(2)
        .spawn(move || {});
    let consumed = oom_guard.consumed();
    drop(oom_guard);

    if consumed != 1 {
        return Err(MixedError::PressureOomHookNotConsumed);
    }
    if !matches!(result, Err(osal_api::error::Error::OutOfMemory)) {
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

fn mixed_object_pipeline(tick_bits: u8) -> Result<(), MixedError> {
    let public_task_baseline = FreeRtosTask::count();
    let state = Arc::new(PipelineState::new());

    let mtx = osal::backend::Mutex::new(0u32).map_err(|_| MixedError::MutexCreate)?;
    let binary = osal::backend::BinarySemaphore::new()
        .map_err(|_| MixedError::BinaryCreate)?;
    let counting =
        osal::backend::CountingSemaphore::new(1, 0)
            .map_err(|_| MixedError::CountingCreate)?;
    let q = osal::backend::Queue::new(1, 4).map_err(|_| MixedError::QueueCreate)?;

    // Task B: recv from Queue, increment Mutex, release CountingSemaphore
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

    // Task A: acquire BinarySemaphore, send M0 to Queue
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
                .send(&M0, Timeout::After(Duration::from_millis(100)))
                .is_err()
            {
                s_a.task_a_done.store(3, Ordering::Release);
                return;
            }
            s_a.task_a_done.store(1, Ordering::Release);
        })
        .map_err(|_| MixedError::TaskSpawnFailed)?;

    if !bounded_wait_bool(&state.task_b_started, true, 80, tick_bits) {
        return Err(MixedError::PipelineTimeout);
    }
    if !bounded_wait_bool(&state.task_a_started, true, 80, tick_bits) {
        return Err(MixedError::PipelineTimeout);
    }

    // Timer: release BinarySemaphore (unblocks Task A)
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
    timer.start().map_err(|_| MixedError::TimerStart)?;

    // Controller waits on CountingSemaphore (released by Task B)
    counting
        .acquire(Timeout::After(Duration::from_millis(200)))
        .map_err(|_| MixedError::PipelineTimeout)?;

    ta.join(Timeout::After(Duration::from_millis(100)))
        .map_err(|_| MixedError::TaskJoinFailed)?;
    tb.join(Timeout::After(Duration::from_millis(100)))
        .map_err(|_| MixedError::TaskJoinFailed)?;

    // Verify
    if state.timer_callback_count.load(Ordering::Acquire) != 1 {
        return Err(MixedError::TimerCountWrong);
    }
    if state.binary_release_ok.load(Ordering::Acquire) != 1 {
        return Err(MixedError::BinaryReleaseFailed);
    }
    if state.task_a_done.load(Ordering::Acquire) != 1 {
        return Err(MixedError::TaskAOperationFailed);
    }
    if state.task_b_done.load(Ordering::Acquire) != 1 {
        return Err(MixedError::TaskBOperationFailed);
    }
    if state.b_received_word.load(Ordering::Acquire) != u32::from_le_bytes(M0) {
        return Err(MixedError::PayloadMismatch);
    }
    if state.b_counter.load(Ordering::Acquire) != 1 {
        return Err(MixedError::CounterMismatch);
    }
    if q.len().map_err(|_| MixedError::QueueCreate)? != 0 {
        return Err(MixedError::PayloadMismatch);
    }

    drop(ta);
    drop(tb);
    drop(timer);
    drop(q);
    drop(binary);
    drop(counting);
    drop(mtx);
    drop(state);

    if FreeRtosTask::count() != public_task_baseline {
        return Err(MixedError::TaskCountWrong);
    }
    Ok(())
}
