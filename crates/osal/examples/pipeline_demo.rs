//! Portable OSAL Multi-Task Pipeline Integration Demo
//!
//! Simulates an embedded data-processing pipeline:
//!
//! ```text
//!   Producers (×2)  ──send──>  Queue  ──recv──>  Consumers (×3)
//!                                                     │
//!                                                     ▼
//!                                              Shared Stats (Mutex)
//!
//!   Timer ──callback──> Monitor ──reads──> Stats
//!   Supervisor controls START / STOP via event bits
//! ```
//!
//! # Build & Run (POSIX backend only)
//!
//! ```bash
//! cargo run -p osal --example pipeline_demo
//! ```

extern crate alloc;

use alloc::sync::Arc;
use core::sync::atomic::{AtomicU8, AtomicU32, Ordering};
use core::time::Duration;

use osal::prelude::*;

// ---------------------------------------------------------------------------
// Constants (matched to legacy portable_osal_integration_demo)
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

/// Event bits (replaces old EventGroup)
const START_BIT: u8 = 1 << 0;
const STOP_BIT: u8 = 1 << 1;
const CONSUMER_GO_BIT: u8 = 1 << 2;

// ---------------------------------------------------------------------------
// Tick helper — maps Clock::now() to a millisecond counter
// ---------------------------------------------------------------------------

fn tick_ms(start: Duration) -> u32 {
    Clock::now().saturating_sub(start).as_millis() as u32
}

fn delay_until(wake: &mut Duration, period: Duration) {
    let now = Clock::now();
    if *wake > now {
        Clock::delay(*wake - now);
    }
    *wake += period;
}

// ---------------------------------------------------------------------------
// Packet helpers (16-byte fixed-length message)
// ---------------------------------------------------------------------------

fn build_packet(producer_id: u32, sequence_id: u32) -> [u8; PACKET_SIZE] {
    let checksum = producer_id ^ sequence_id;
    let mut buf = [0u8; PACKET_SIZE];
    buf[0..4].copy_from_slice(&producer_id.to_le_bytes());
    buf[4..8].copy_from_slice(&sequence_id.to_le_bytes());
    buf[8..12].copy_from_slice(&checksum.to_le_bytes());
    buf
}

fn verify_packet(buf: &[u8; PACKET_SIZE]) -> bool {
    let pid = u32::from_le_bytes(buf[..4].try_into().unwrap());
    let seq = u32::from_le_bytes(buf[4..8].try_into().unwrap());
    let cksum = u32::from_le_bytes(buf[8..12].try_into().unwrap());
    cksum == pid ^ seq
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

// ---------------------------------------------------------------------------
// Shared application state
// ---------------------------------------------------------------------------

struct AppState {
    queue: Queue,
    stats: Mutex<Stats>,
    ready_sem: CountingSemaphore,
    events: AtomicU8,
}

// ---------------------------------------------------------------------------
// Producer task
// ---------------------------------------------------------------------------

fn producer_task(id: u32, state: Arc<AppState>, start: Duration) {
    state.ready_sem.release().ok();

    // Wait for START (or immediate STOP).
    loop {
        let bits = state.events.load(Ordering::Acquire);
        if bits & STOP_BIT != 0 {
            return;
        }
        if bits & START_BIT != 0 {
            break;
        }
        Clock::delay(Duration::from_millis(1));
    }

    let mut seq = 0u32;
    let mut last_wake = Clock::now();
    let head_start_end = start + Duration::from_millis(PRODUCER_HEAD_START_MS);

    // — Head-start: produce alone for PRODUCER_HEAD_START_MS.
    while Clock::now() < head_start_end {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            return;
        }

        let packet = build_packet(id, seq);
        match state.queue.send(
            &packet,
            Timeout::After(Duration::from_millis(QUEUE_POST_TIMEOUT_MS)),
        ) {
            Ok(()) => {
                let mut guard = state.stats.lock(Timeout::Forever).unwrap();
                guard.produced += 1;
            }
            Err(_) => {
                let mut guard = state.stats.lock(Timeout::Forever).unwrap();
                guard.dropped += 1;
            }
        }

        seq = seq.wrapping_add(1);
        delay_until(&mut last_wake, Duration::from_millis(PRODUCER_PERIOD_MS));
    }

    // Signal consumers that they can start.
    state.events.fetch_or(CONSUMER_GO_BIT, Ordering::Release);

    // — Main loop: keep producing until STOP_BIT is set.
    loop {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            break;
        }

        let packet = build_packet(id, seq);
        match state.queue.send(
            &packet,
            Timeout::After(Duration::from_millis(QUEUE_POST_TIMEOUT_MS)),
        ) {
            Ok(()) => {
                let mut guard = state.stats.lock(Timeout::Forever).unwrap();
                guard.produced += 1;
            }
            Err(_) => {
                let mut guard = state.stats.lock(Timeout::Forever).unwrap();
                guard.dropped += 1;
            }
        }

        seq = seq.wrapping_add(1);
        delay_until(&mut last_wake, Duration::from_millis(PRODUCER_PERIOD_MS));
    }
}

// ---------------------------------------------------------------------------
// Consumer task
// ---------------------------------------------------------------------------

fn consumer_task(_id: u32, state: Arc<AppState>) {
    state.ready_sem.release().ok();

    // Wait for START and CONSUMER_GO (or immediate STOP).
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
            break;
        }

        match state.queue.recv(
            &mut packet,
            Timeout::After(Duration::from_millis(QUEUE_FETCH_TIMEOUT_MS)),
        ) {
            Ok(_) => {
                let valid = verify_packet(&packet);

                let mut guard = state.stats.lock(Timeout::Forever).unwrap();
                guard.consumed += 1;

                if !valid {
                    guard.checksum_error += 1;
                }
            }
            Err(_) => {
                let mut guard = state.stats.lock(Timeout::Forever).unwrap();
                guard.queue_timeout += 1;
            }
        }

        Clock::delay(Duration::from_millis(CONSUMER_PROCESS_MS));
    }
}

// ---------------------------------------------------------------------------
// Monitor task — reads stats when the timer fires
// ---------------------------------------------------------------------------

fn monitor_task(state: Arc<AppState>, timer_fired: Arc<AtomicU32>, start: Duration) {
    state.ready_sem.release().ok();

    // Wait for START (or immediate STOP).
    loop {
        let bits = state.events.load(Ordering::Acquire);
        if bits & STOP_BIT != 0 {
            return;
        }
        if bits & START_BIT != 0 {
            break;
        }
        Clock::delay(Duration::from_millis(1));
    }

    loop {
        if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
            break;
        }

        // Wait for a timer tick with timeout.
        let prev = timer_fired.load(Ordering::Acquire);
        let deadline = Clock::now() + Duration::from_millis(MONITOR_WAIT_MS);
        loop {
            if state.events.load(Ordering::Acquire) & STOP_BIT != 0 {
                return;
            }
            if timer_fired.load(Ordering::Acquire) != prev {
                break;
            }
            if Clock::now() >= deadline {
                break;
            }
            Clock::delay(Duration::from_millis(10));
        }

        let tick = tick_ms(start);
        let guard = state.stats.lock(Timeout::Forever).unwrap();
        println!(
            "[monitor] tick={:5} produced={} consumed={} dropped={} timeout={} checksum_error={}",
            tick,
            guard.produced,
            guard.consumed,
            guard.dropped,
            guard.queue_timeout,
            guard.checksum_error,
        );
        drop(guard);
    }
}

// ---------------------------------------------------------------------------
// Supervisor — lifecycle controller (runs on the main thread)
// ---------------------------------------------------------------------------

fn supervisor_main(state: Arc<AppState>, timer: Timer) {
    // Phase 0 — wait for all tasks to signal ready.
    for _ in 0..TOTAL_READY_TASKS {
        state.ready_sem.acquire(Timeout::Forever).unwrap();
    }

    println!("[supervisor] all tasks ready, set START_BIT");

    state.events.fetch_or(START_BIT, Ordering::Release);
    timer.start().unwrap();

    // Phase 1 — producer head-start.
    Clock::delay(Duration::from_millis(PRODUCER_HEAD_START_MS));

    // Phase 2 — default timer period.
    Clock::delay(Duration::from_millis(DEMO_FIRST_PHASE_MS));

    // Change period mid-demo.
    timer
        .change_period(Duration::from_millis(HEARTBEAT_FAST_MS))
        .ok();
    timer.reset().ok();

    // Phase 3 — faster timer period.
    Clock::delay(Duration::from_millis(DEMO_SECOND_PHASE_MS));

    // Phase 4 — graceful shutdown.
    println!("[supervisor] set STOP_BIT");
    state.events.fetch_or(STOP_BIT, Ordering::Release);

    timer.stop().unwrap();

    // Print final summary.
    {
        let guard = state.stats.lock(Timeout::Forever).unwrap();
        println!(
            "[summary] produced={} consumed={} dropped={} timeout={} checksum_error={}",
            guard.produced,
            guard.consumed,
            guard.dropped,
            guard.queue_timeout,
            guard.checksum_error,
        );
        println!("[summary] demo finished");
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> Result<()> {
    println!("[main] run portable demo on posix backend");

    osal::initialize()?;

    let demo_start = Clock::now();

    println!("[init] Portable OSAL Integration Demo");

    // — Create OSAL resources ------------------------------------------------

    let queue = Queue::new(QUEUE_CAPACITY, PACKET_SIZE)?;
    println!(
        "[init] queue capacity={} message_size={}",
        QUEUE_CAPACITY, PACKET_SIZE
    );

    let stats = Mutex::new(Stats::default())?;
    println!("[init] stats mutex");

    let ready_sem = CountingSemaphore::new(TOTAL_READY_TASKS, 0)?;
    println!("[init] ready semaphore max_count={}", TOTAL_READY_TASKS);

    let state = Arc::new(AppState {
        queue,
        stats,
        ready_sem,
        events: AtomicU8::new(0),
    });
    println!("[init] event group");

    // — Timer — callback notifies the monitor via a shared counter -----------

    let timer_fired = Arc::new(AtomicU32::new(0));
    let tf = Arc::clone(&timer_fired);
    let timer = Timer::new(
        "heartbeat",
        Duration::from_millis(TIMER_PERIOD_MS),
        TimerMode::Periodic,
        Box::new(move || {
            tf.fetch_add(1, Ordering::Release);
        }),
    )?;
    println!("[init] heartbeat timer period={}ms", TIMER_PERIOD_MS);

    // — Spawn workers --------------------------------------------------------

    let mut tasks = Vec::new();

    for id in 0..PRODUCER_COUNT {
        let s = Arc::clone(&state);
        tasks.push(
            TaskBuilder::new()
                .name(&alloc::format!("prod-{}", id))
                .priority(3)
                .spawn(move || producer_task(id, s, demo_start))?,
        );
        println!("[init] producer-{} spawned", id);
    }

    for id in 0..CONSUMER_COUNT {
        let s = Arc::clone(&state);
        tasks.push(
            TaskBuilder::new()
                .name(&alloc::format!("cons-{}", id))
                .priority(3)
                .spawn(move || consumer_task(id, s))?,
        );
        println!("[init] consumer-{} spawned", id);
    }

    {
        let s = Arc::clone(&state);
        let tf = Arc::clone(&timer_fired);
        tasks.push(
            TaskBuilder::new()
                .name("monitor")
                .priority(2)
                .spawn(move || monitor_task(s, tf, demo_start))?,
        );
        println!("[init] monitor spawned");
    }

    println!("[init] supervisor spawned");

    // — Run supervisor on main thread, then join all workers -----------------

    supervisor_main(Arc::clone(&state), timer);

    for t in &tasks {
        t.join(Timeout::Forever).ok();
    }
    drop(tasks);
    drop(state);
    drop(timer_fired);

    osal::shutdown()?;

    println!("[main] demo completed successfully");
    Ok(())
}
