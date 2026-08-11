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

pub fn run_mixed_cases(tick_bits: u8) -> Result<(), MixedError> {
    mixed_object_pipeline(tick_bits)?;
    harness::console_line(c"OSAL_CASE_PASS name=mixed_object_pipeline");
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
