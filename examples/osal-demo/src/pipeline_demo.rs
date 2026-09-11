//! Portable multi-task pipeline demo.
//!
//! ```text
//!   Producers (×2) ──send──> Queue ──recv──> Consumers (×3)
//!                                               │
//!                                               ▼
//!                                        Shared Stats (Mutex)
//!
//!   Timer ──callback──> Monitor ──reads──> Stats
//!   Supervisor controls START / STOP via event bits
//! ```
//!
//! The supervisor runs on the caller's context (host `main` thread or the
//! FreeRTOS boot task); workers are spawned through the OSAL `Task` API,
//! so the same code becomes pthreads or FreeRTOS tasks.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use core::time::Duration;

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};
use crate::runtime::with_runtime;

// ---------------------------------------------------------------------------
// Parameters — identical on every backend
// ---------------------------------------------------------------------------

const PACKET_SIZE: usize = 16;
const QUEUE_CAPACITY: usize = 128;

const PRODUCER_COUNT: u32 = 2;
const CONSUMER_COUNT: u32 = 3;
/// Tasks that signal the ready semaphore (2 producers + 3 consumers + 1 monitor).
const TOTAL_READY_TASKS: u32 = PRODUCER_COUNT + CONSUMER_COUNT + 1;

const PRODUCER_HEAD_START_MS: u64 = 1000;
const PRODUCER_PERIOD_MS: u64 = 25;
const CONSUMER_PROCESS_MS: u64 = 30;
const QUEUE_FETCH_TIMEOUT_MS: u64 = 100;
const QUEUE_POST_TIMEOUT_MS: u64 = 100;
const DEMO_FIRST_PHASE_MS: u64 = 2001;
const DEMO_SECOND_PHASE_MS: u64 = 3001;
const MONITOR_WAIT_MS: u64 = 2000;
const TIMER_PERIOD_MS: u64 = 1000;
const HEARTBEAT_FAST_MS: u64 = 500;

/// Worker join budget during shutdown.
const JOIN_TIMEOUT_SECS: u64 = 2;

/// Event bits (replaces the legacy EventGroup).
const START_BIT: u8 = 1 << 0;
const STOP_BIT: u8 = 1 << 1;
const CONSUMER_GO_BIT: u8 = 1 << 2;

// ---------------------------------------------------------------------------
// Worker error codes — recorded once, first failure wins
// ---------------------------------------------------------------------------

#[repr(u32)]
enum WorkerError {
    ReadyRelease = 1,
    StatsLock = 2,
    QueueSend = 3,
    QueueRecv = 4,
}

fn record_worker_error(state: &AppState, error: WorkerError) {
    let _ =
        state
            .worker_error
            .compare_exchange(0, error as u32, Ordering::AcqRel, Ordering::Acquire);
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// Outcome of the pipeline demo.
///
/// Counts are backend- and timing-dependent; only the invariants checked in
/// [`run`] are portable.
pub struct PipelineReport {
    /// Packets accepted by the queue.
    pub produced: u32,
    /// Packets received by consumers.
    pub consumed: u32,
    /// Sends that timed out because the queue was saturated.
    pub dropped: u32,
    /// Receives that timed out because the queue was empty.
    pub queue_timeout: u32,
    /// Consumers that received a corrupt packet.
    pub checksum_error: u32,
    /// Timer callback invocations.
    pub timer_fires: u32,
    /// Monitor wakeups that read the shared stats.
    pub monitor_samples: u32,
}

impl fmt::Display for PipelineReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[pipeline] produced={}\n\
             [pipeline] consumed={}\n\
             [pipeline] dropped={}\n\
             [pipeline] timeout={}\n\
             [pipeline] checksum_error={}\n\
             [pipeline] timer_fires={}\n\
             [pipeline] monitor_samples={}\n\
             [pipeline] invariants OK",
            self.produced,
            self.consumed,
            self.dropped,
            self.queue_timeout,
            self.checksum_error,
            self.timer_fires,
            self.monitor_samples
        )
    }
}

// ---------------------------------------------------------------------------
// Shared statistics
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Stats {
    produced: u32,
    consumed: u32,
    dropped: u32,
    checksum_error: u32,
    queue_timeout: u32,
}

struct StatsSnapshot {
    produced: u32,
    consumed: u32,
    dropped: u32,
    checksum_error: u32,
    queue_timeout: u32,
}

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

struct AppState {
    queue: Queue,
    stats: Mutex<Stats>,
    ready_sem: CountingSemaphore,
    events: AtomicU8,
    worker_error: AtomicU32,
    monitor_samples: AtomicU32,
    timer_fires: AtomicU32,
}

// ---------------------------------------------------------------------------
// Timing helpers
// ---------------------------------------------------------------------------

fn delay_until(wake: &mut Duration, period: Duration) {
    let now = Clock::now();
    if *wake > now {
        Clock::delay(*wake - now);
    }
    *wake += period;
}

// ---------------------------------------------------------------------------
// Packet helpers (16-byte fixed-length message): [pid][seq][checksum]
// ---------------------------------------------------------------------------

fn build_packet(producer_id: u32, sequence_id: u32) -> [u8; PACKET_SIZE] {
    let checksum = producer_id ^ sequence_id;
    let mut buf = [0u8; PACKET_SIZE];
    let pid = producer_id.to_le_bytes();
    let seq = sequence_id.to_le_bytes();
    let cks = checksum.to_le_bytes();

    let mut i = 0;
    while i < 4 {
        buf[i] = pid[i];
        buf[4 + i] = seq[i];
        buf[8 + i] = cks[i];
        i += 1;
    }
    buf
}

fn read_u32_le(buf: &[u8; PACKET_SIZE], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *buf.get(offset)?,
        *buf.get(offset + 1)?,
        *buf.get(offset + 2)?,
        *buf.get(offset + 3)?,
    ]))
}

fn verify_packet(buf: &[u8; PACKET_SIZE]) -> bool {
    match (
        read_u32_le(buf, 0),
        read_u32_le(buf, 4),
        read_u32_le(buf, 8),
    ) {
        (Some(pid), Some(seq), Some(checksum)) => checksum == pid ^ seq,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Static task names — avoids per-spawn allocation
// ---------------------------------------------------------------------------

fn producer_name(id: u32) -> &'static str {
    match id {
        0 => "prod-0",
        1 => "prod-1",
        _ => "producer",
    }
}

fn consumer_name(id: u32) -> &'static str {
    match id {
        0 => "cons-0",
        1 => "cons-1",
        2 => "cons-2",
        _ => "consumer",
    }
}

// ---------------------------------------------------------------------------
// Ready / start gate
// ---------------------------------------------------------------------------

fn signal_ready(state: &AppState) -> bool {
    if state.ready_sem.release().is_err() {
        record_worker_error(state, WorkerError::ReadyRelease);
        return false;
    }
    true
}

fn wait_for_start(state: &AppState) -> bool {
    loop {
        let bits = state.events.load(Ordering::Acquire);
        if bits & STOP_BIT != 0 {
            return false;
        }
        if bits & START_BIT != 0 {
            return true;
        }
        Clock::delay(Duration::from_millis(1));
    }
}

// ---------------------------------------------------------------------------
// Producer task
// ---------------------------------------------------------------------------

/// Send one packet. Returns `false` if the worker must abort.
fn produce_once(state: &AppState, id: u32, seq: u32) -> bool {
    let packet = build_packet(id, seq);

    match state.queue.send(
        &packet,
        Timeout::After(Duration::from_millis(QUEUE_POST_TIMEOUT_MS)),
    ) {
        Ok(()) => match state.stats.lock(Timeout::Forever) {
            Ok(mut guard) => {
                guard.produced += 1;
                true
            }
            Err(_) => {
                record_worker_error(state, WorkerError::StatsLock);
                false
            }
        },
        // A saturated queue is expected demo behaviour, not a failure.
        Err(Error::Timeout) => match state.stats.lock(Timeout::Forever) {
            Ok(mut guard) => {
                guard.dropped += 1;
                true
            }
            Err(_) => {
                record_worker_error(state, WorkerError::StatsLock);
                false
            }
        },
        // QueueClosed / Internal / NotInitialized must not be masked as a drop.
        Err(_) => {
            record_worker_error(state, WorkerError::QueueSend);
            false
        }
    }
}

fn producer_task(id: u32, state: Arc<AppState>, start: Duration) {
    if !signal_ready(&state) {
        return;
    }
    if !wait_for_start(&state) {
        return;
    }

    let mut seq = 0u32;
    let mut last_wake = Clock::now();
    let head_start_end = start + Duration::from_millis(PRODUCER_HEAD_START_MS);

    // Head start: produce alone so consumers begin against a non-empty queue.
    while Clock::now() < head_start_end {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            return;
        }
        if !produce_once(&state, id, seq) {
            return;
        }
        seq = seq.wrapping_add(1);
        delay_until(&mut last_wake, Duration::from_millis(PRODUCER_PERIOD_MS));
    }

    // Release the consumers.
    state.events.fetch_or(CONSUMER_GO_BIT, Ordering::Release);

    loop {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            return;
        }
        if !produce_once(&state, id, seq) {
            return;
        }
        seq = seq.wrapping_add(1);
        delay_until(&mut last_wake, Duration::from_millis(PRODUCER_PERIOD_MS));
    }
}

// ---------------------------------------------------------------------------
// Consumer task
// ---------------------------------------------------------------------------

fn consumer_task(_id: u32, state: Arc<AppState>) {
    if !signal_ready(&state) {
        return;
    }

    // Wait for START and for a producer to signal CONSUMER_GO.
    loop {
        let bits = state.events.load(Ordering::Acquire);
        if bits & STOP_BIT != 0 {
            return;
        }
        if bits & START_BIT != 0 && bits & CONSUMER_GO_BIT != 0 {
            break;
        }
        Clock::delay(Duration::from_millis(1));
    }

    let mut packet = [0u8; PACKET_SIZE];

    loop {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            return;
        }

        match state.queue.recv(
            &mut packet,
            Timeout::After(Duration::from_millis(QUEUE_FETCH_TIMEOUT_MS)),
        ) {
            Ok(()) => {
                let valid = verify_packet(&packet);
                match state.stats.lock(Timeout::Forever) {
                    Ok(mut guard) => {
                        guard.consumed += 1;
                        if !valid {
                            guard.checksum_error += 1;
                        }
                    }
                    Err(_) => {
                        record_worker_error(&state, WorkerError::StatsLock);
                        return;
                    }
                }
            }
            // An empty queue is expected demo behaviour.
            Err(Error::Timeout) => match state.stats.lock(Timeout::Forever) {
                Ok(mut guard) => guard.queue_timeout += 1,
                Err(_) => {
                    record_worker_error(&state, WorkerError::StatsLock);
                    return;
                }
            },
            // A closed queue is the normal shutdown signal.
            Err(Error::QueueClosed) => {
                if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
                    return;
                }
                record_worker_error(&state, WorkerError::QueueRecv);
                return;
            }
            Err(_) => {
                record_worker_error(&state, WorkerError::QueueRecv);
                return;
            }
        }

        Clock::delay(Duration::from_millis(CONSUMER_PROCESS_MS));
    }
}

// ---------------------------------------------------------------------------
// Monitor task — reads stats whenever the timer fires
// ---------------------------------------------------------------------------

fn monitor_task(state: Arc<AppState>, _start: Duration) {
    if !signal_ready(&state) {
        return;
    }
    if !wait_for_start(&state) {
        return;
    }

    loop {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            return;
        }

        let previous = state.timer_fires.load(Ordering::Acquire);
        let deadline = Clock::now() + Duration::from_millis(MONITOR_WAIT_MS);

        loop {
            if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
                return;
            }
            if state.timer_fires.load(Ordering::Acquire) != previous {
                break;
            }
            if Clock::now() >= deadline {
                break;
            }
            Clock::delay(Duration::from_millis(10));
        }

        match state.stats.lock(Timeout::Forever) {
            Ok(guard) => {
                // Touch every field so the read is observable to the optimiser.
                let _observed = (
                    guard.produced,
                    guard.consumed,
                    guard.dropped,
                    guard.queue_timeout,
                    guard.checksum_error,
                );
                drop(guard);
                state.monitor_samples.fetch_add(1, Ordering::Release);
            }
            Err(_) => {
                record_worker_error(&state, WorkerError::StatsLock);
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Supervisor — lifecycle controller
// ---------------------------------------------------------------------------

fn supervisor_main(state: &Arc<AppState>, timer: &Timer) -> DemoResult<()> {
    // Phase 0 — every worker signals ready.
    for _ in 0..TOTAL_READY_TASKS {
        state
            .ready_sem
            .acquire(Timeout::After(Duration::from_secs(5)))
            .demo_context("pipeline.ready")?;
    }

    state.events.fetch_or(START_BIT, Ordering::Release);
    timer.start().demo_context("pipeline.timer.start")?;

    // Phase 1 — producer head start.
    Clock::delay(Duration::from_millis(PRODUCER_HEAD_START_MS));

    // Phase 2 — default heartbeat period.
    Clock::delay(Duration::from_millis(DEMO_FIRST_PHASE_MS));

    timer
        .change_period(Duration::from_millis(HEARTBEAT_FAST_MS))
        .demo_context("pipeline.timer.change_period")?;
    timer.reset().demo_context("pipeline.timer.reset")?;

    // Phase 3 — faster heartbeat period.
    Clock::delay(Duration::from_millis(DEMO_SECOND_PHASE_MS));

    // Phase 4 — stop.
    state.events.fetch_or(STOP_BIT, Ordering::Release);
    timer.stop().demo_context("pipeline.timer.stop")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Worker set
// ---------------------------------------------------------------------------

fn spawn_workers(
    state: &Arc<AppState>,
    tasks: &mut Vec<Task>,
    demo_start: Duration,
) -> DemoResult<()> {
    for id in 0..PRODUCER_COUNT {
        let worker_state = Arc::clone(state);
        let task = TaskBuilder::new()
            .name(producer_name(id))
            .priority(3)
            .spawn(move || producer_task(id, worker_state, demo_start))
            .demo_context("pipeline.producer.spawn")?;
        tasks.push(task);
    }

    for id in 0..CONSUMER_COUNT {
        let worker_state = Arc::clone(state);
        let task = TaskBuilder::new()
            .name(consumer_name(id))
            .priority(3)
            .spawn(move || consumer_task(id, worker_state))
            .demo_context("pipeline.consumer.spawn")?;
        tasks.push(task);
    }

    let worker_state = Arc::clone(state);
    let task = TaskBuilder::new()
        .name("monitor")
        .priority(2)
        .spawn(move || monitor_task(worker_state, demo_start))
        .demo_context("pipeline.monitor.spawn")?;
    tasks.push(task);

    Ok(())
}

/// Stop and join every spawned worker, then release the queue.
///
/// Safe to call more than once and on a partially spawned worker set — used
/// both for normal shutdown and for rollback after a spawn failure.
fn stop_and_join(state: &Arc<AppState>, timer: &Timer, tasks: &[Task]) -> DemoResult<()> {
    state.events.fetch_or(STOP_BIT, Ordering::Release);

    // Closing wakes workers blocked in send/recv immediately.
    let _ = state.queue.close();
    let _ = timer.stop();

    let mut first_error: Option<DemoError> = None;
    for task in tasks {
        if let Err(error) = task.join(Timeout::After(Duration::from_secs(JOIN_TIMEOUT_SECS))) {
            if first_error.is_none() {
                first_error = Some(DemoError::Osal {
                    operation: "pipeline.join",
                    error,
                });
            }
        }
    }

    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

fn read_stats(state: &AppState) -> DemoResult<StatsSnapshot> {
    let guard = state
        .stats
        .lock(Timeout::After(Duration::from_millis(100)))
        .demo_context("pipeline.stats.lock")?;

    Ok(StatsSnapshot {
        produced: guard.produced,
        consumed: guard.consumed,
        dropped: guard.dropped,
        checksum_error: guard.checksum_error,
        queue_timeout: guard.queue_timeout,
    })
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the pipeline demo against the active backend.
pub fn run() -> DemoResult<PipelineReport> {
    with_runtime(|| {
        let queue =
            Queue::new(QUEUE_CAPACITY, PACKET_SIZE).demo_context("pipeline.queue.create")?;
        let stats = Mutex::new(Stats::default()).demo_context("pipeline.stats.create")?;
        let ready_sem =
            CountingSemaphore::new(TOTAL_READY_TASKS, 0).demo_context("pipeline.ready.create")?;

        let state = Arc::new(AppState {
            queue,
            stats,
            ready_sem,
            events: AtomicU8::new(0),
            worker_error: AtomicU32::new(0),
            monitor_samples: AtomicU32::new(0),
            timer_fires: AtomicU32::new(0),
        });

        let timer_state = Arc::clone(&state);
        let timer = Timer::new(
            "heartbeat",
            Duration::from_millis(TIMER_PERIOD_MS),
            TimerMode::Periodic,
            Box::new(move || {
                timer_state.timer_fires.fetch_add(1, Ordering::Release);
            }),
        )
        .demo_context("pipeline.timer.create")?;

        let demo_start = Clock::now();
        let mut tasks: Vec<Task> = Vec::new();

        if let Err(error) = spawn_workers(&state, &mut tasks, demo_start) {
            // Reap whatever did spawn so no lease outlives the demo.
            let _ = stop_and_join(&state, &timer, &tasks);
            return Err(error);
        }

        if let Err(error) = supervisor_main(&state, &timer) {
            let _ = stop_and_join(&state, &timer, &tasks);
            return Err(error);
        }

        stop_and_join(&state, &timer, &tasks)?;

        let snapshot = read_stats(&state)?;
        let timer_fires = state.timer_fires.load(Ordering::Acquire);
        let monitor_samples = state.monitor_samples.load(Ordering::Acquire);
        let worker_error = state.worker_error.load(Ordering::Acquire);
        let final_len = state.queue.len().demo_context("pipeline.queue.len")?;

        drop(tasks);
        drop(timer);
        drop(state);

        // Portable invariants only — absolute counts differ per backend.
        if let Some(reason) =
            pipeline_invariant_failure(&snapshot, timer_fires, monitor_samples, worker_error)
        {
            return Err(reason);
        }

        if final_len > QUEUE_CAPACITY {
            return Err(DemoError::Check("queue exceeded capacity"));
        }

        Ok(PipelineReport {
            produced: snapshot.produced,
            consumed: snapshot.consumed,
            dropped: snapshot.dropped,
            queue_timeout: snapshot.queue_timeout,
            checksum_error: snapshot.checksum_error,
            timer_fires,
            monitor_samples,
        })
    })
}

fn pipeline_invariant_failure(
    snapshot: &StatsSnapshot,
    timer_fires: u32,
    monitor_samples: u32,
    worker_error: u32,
) -> Option<DemoError> {
    if snapshot.produced == 0 {
        return Some(DemoError::Check("pipeline produced no packets"));
    }
    if snapshot.consumed == 0 {
        return Some(DemoError::Check("pipeline consumed no packets"));
    }
    if snapshot.consumed > snapshot.produced {
        return Some(DemoError::Check("pipeline consumed more than produced"));
    }
    if snapshot.checksum_error != 0 {
        return Some(DemoError::Check("pipeline payload checksum mismatch"));
    }
    if timer_fires == 0 {
        return Some(DemoError::Check("pipeline timer never fired"));
    }
    if monitor_samples == 0 {
        return Some(DemoError::Check("pipeline monitor never sampled stats"));
    }
    if worker_error != 0 {
        return Some(DemoError::Worker { code: worker_error });
    }
    None
}
