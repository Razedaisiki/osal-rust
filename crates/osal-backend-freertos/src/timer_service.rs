//! FreeRTOS Timer Service — single service task for timer callbacks.
//!
//! The service task is created lazily on the first `start()`/`reset()`.
//! It waits on a binary wake semaphore and dispatches expired callbacks
//! one at a time using the take-execute-restore pattern.
//!
//! # Lock ordering
//!
//! ```text
//! Timer API:       control mutex → extract Arc → drop control → registry mutex
//! shutdown phase1: control mutex → registry mutex → signal → release both
//! shutdown phase2: wait completion EventGroup (lock-free)
//! shutdown phase3: control mutex → Stopped
//! worker loop:     only registry mutex
//! callback:        holds neither lock
//! ```

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
#[cfg(feature = "test-fixture")]
use core::sync::atomic::{AtomicU64, Ordering};
use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::traits::clock::Clock as _;
use osal_api::traits::timer::TimerCallback;
use osal_portable::tick_time::duration_to_ticks_ceil;
use osal_portable::timer_state::TimerState;

use osal_backend_freertos_sys as sys;

use crate::clock::FreeRtosClock;
use crate::timer_control::{self, ServiceSlot};

// ---------------------------------------------------------------------------
// Timer entry
// ---------------------------------------------------------------------------

pub(crate) struct TimerEntry {
    pub id: u64,
    pub state: TimerState,
    pub callback: Option<TimerCallback>,
    pub deleted: bool,
}

// ---------------------------------------------------------------------------
// Service state (registry)
// ---------------------------------------------------------------------------

pub(crate) struct TimerServiceState {
    pub timers: Vec<TimerEntry>,
    pub next_id: u64,
    pub stop_requested: bool,
}

// ---------------------------------------------------------------------------
// Timer service — resource container
// ---------------------------------------------------------------------------

/// Completion bit set by the worker before self-deleting.
const WORKER_COMPLETED_BIT: u32 = 1;

// ---------------------------------------------------------------------------
// Fixture progress tracking //
// ---------------------------------------------------------------------------

/// Request/ack protocol for deterministic flush.
///
/// The test advances ticks, then calls `timer_flush_request()` to wake
/// the worker and obtain a target.  The worker acknowledges the target
/// when it reaches a quiescent state: no due callbacks, about to enter
/// a waiting state.  `flush_timer_service(target)` blocks until the
/// ack is observed and no dispatch is in flight.
#[cfg(feature = "test-fixture")]
static FLUSH_REQUEST: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "test-fixture")]
static FLUSH_ACK: AtomicU64 = AtomicU64::new(0);

/// Count of wake-semaphore finite-wait calls made by the worker in
/// `wait_until_deadline`.  Each call records its tick count via
/// `record_wake_wait()`.  Tests use `fixture_wake_wait_count()` and
/// `fixture_wake_wait_ticks_max()` to assert finite-chunk behavior.
#[cfg(feature = "test-fixture")]
static WAKE_WAIT_COUNT: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "test-fixture")]
static WAKE_WAIT_MAX_TICKS: AtomicU64 = AtomicU64::new(0);

#[cfg(feature = "test-fixture")]
fn record_wake_wait(wait_ticks: u64) {
    WAKE_WAIT_COUNT.fetch_add(1, Ordering::Relaxed);
    WAKE_WAIT_MAX_TICKS.fetch_max(wait_ticks, Ordering::Relaxed);
}

/// Number of dispatched callbacks since startup.
#[cfg(feature = "test-fixture")]
static DISPATCH_STARTED: AtomicU64 = AtomicU64::new(0);

/// Number of completed dispatches (callback returned and restored/removed).
#[cfg(feature = "test-fixture")]
static DISPATCH_COMPLETED: AtomicU64 = AtomicU64::new(0);

/// Signal the worker if the timer service is running.
#[cfg(feature = "testkit")]
fn signal_worker_for_flush() {
    if let Ok(()) = crate::timer_control::with_slot(|slot| match slot {
        crate::timer_control::ServiceSlot::Running { service, .. } => {
            service.signal_wake();
            Ok(())
        }
        _ => Ok(()),
    }) {}
}

/// Bump the flush request counter, signal the worker to wake up and
/// scan, and return the new target value.  The caller must have
/// already advanced ticks before calling this.
///
/// Call `flush_timer_service(target)` to wait for the worker to
/// acknowledge.
#[cfg(feature = "testkit")]
pub fn timer_flush_request() -> u64 {
    let target = FLUSH_REQUEST.fetch_add(1, Ordering::SeqCst) + 1;
    signal_worker_for_flush();
    target
}

/// Block until the worker has acknowledged the given flush target
/// AND no callback dispatch is in flight.
#[cfg(feature = "testkit")]
pub fn flush_timer_service(target: u64) {
    extern crate std;
    // If there's no worker, there's nothing to flush — return immediately.
    let has_worker = crate::timer_control::with_slot(|slot| match slot {
        crate::timer_control::ServiceSlot::Running { worker, .. } => Ok(worker.is_some()),
        _ => Ok(false),
    })
    .unwrap_or(false);
    if !has_worker {
        return;
    }

    let start = std::time::Instant::now();
    let watchdog = core::time::Duration::from_secs(2);
    loop {
        let ack = FLUSH_ACK.load(Ordering::SeqCst);
        let started = DISPATCH_STARTED.load(Ordering::SeqCst);
        let completed = DISPATCH_COMPLETED.load(Ordering::SeqCst);
        if ack >= target && started == completed {
            return;
        }
        if start.elapsed() > watchdog {
            panic!(
                "flush_timer_service stalled: ack={ack} target={target} \
                 dispatched={started}/{completed}"
            );
        }
        std::thread::sleep(core::time::Duration::from_micros(100));
    }
}

// ---------------------------------------------------------------------------
// Fixture-only test hooks //
// ---------------------------------------------------------------------------

/// Fault: make the next registry reserve fail with OutOfMemory.
#[cfg(feature = "testkit")]
static FAIL_NEXT_REGISTRY_RESERVE: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

/// Set when shutdown enters its EventGroup wait; cleared on exit.
/// Tests poll this instead of using fixed sleeps.
#[cfg(feature = "test-fixture")]
static SHUTDOWN_WAITING: core::sync::atomic::AtomicBool =
    core::sync::atomic::AtomicBool::new(false);

#[cfg(feature = "testkit")]
pub fn fixture_wake_wait_count() -> u64 {
    WAKE_WAIT_COUNT.load(Ordering::Relaxed)
}

#[cfg(feature = "testkit")]
pub fn fixture_wake_wait_max_ticks() -> u64 {
    WAKE_WAIT_MAX_TICKS.load(Ordering::Relaxed)
}

#[cfg(feature = "testkit")]
pub fn fixture_clear_wake_wait_ticks() {
    WAKE_WAIT_COUNT.store(0, Ordering::Relaxed);
    WAKE_WAIT_MAX_TICKS.store(0, Ordering::Relaxed);
}

#[cfg(feature = "testkit")]
pub fn fixture_reset_timer_hooks() {
    FAIL_NEXT_REGISTRY_RESERVE.store(false, Ordering::SeqCst);
    #[cfg(feature = "test-fixture")]
    {
        SHUTDOWN_WAITING.store(false, Ordering::SeqCst);
        WAKE_WAIT_COUNT.store(0, Ordering::Relaxed);
        WAKE_WAIT_MAX_TICKS.store(0, Ordering::Relaxed);
    }
}

#[cfg(feature = "testkit")]
pub fn fixture_fail_next_registry_reserve() {
    FAIL_NEXT_REGISTRY_RESERVE.store(true, Ordering::SeqCst);
}

#[cfg(feature = "testkit")]
pub fn fixture_set_next_timer_id(id: u64) {
    with_registry(|_, state| {
        state.next_id = id;
        Ok(())
    })
    .expect("timer service must be running");
}

#[cfg(feature = "testkit")]
pub fn fixture_registry_len() -> usize {
    with_registry(|_, state| Ok(state.timers.len())).expect("timer service must be running")
}

#[cfg(feature = "testkit")]
pub fn fixture_worker_exists() -> bool {
    crate::timer_control::with_slot(|slot| {
        Ok(matches!(
            slot,
            crate::timer_control::ServiceSlot::Running {
                worker: Some(_),
                ..
            }
        ))
    })
    .unwrap_or(false)
}

/// Return the number of threads blocked on the completion EventGroup.
/// Used by shutdown tests instead of fixed sleeps.
#[cfg(feature = "testkit")]
pub fn fixture_completion_waiter_count() -> usize {
    use osal_backend_freertos_sys as sys;
    crate::timer_control::with_slot(|slot| match slot {
        crate::timer_control::ServiceSlot::Running { service, .. }
        | crate::timer_control::ServiceSlot::Stopping { service } => {
            let eg = service
                .completion_eg
                .as_ref()
                .expect("completion EG missing");
            Ok(sys::fixture::event_group_blocked_count(eg))
        }
        crate::timer_control::ServiceSlot::Stopped => Ok(0),
    })
    .unwrap_or(0)
}

/// Return true if shutdown has entered its completion EventGroup wait.
#[cfg(feature = "testkit")]
pub fn fixture_shutdown_waiting() -> bool {
    SHUTDOWN_WAITING.load(Ordering::SeqCst)
}

pub(crate) struct TimerService {
    registry_mutex: Option<sys::MutexHandle>,
    wake_sem: Option<sys::SemaphoreHandle>,
    completion_eg: Option<sys::EventGroupHandle>,
    state: UnsafeCell<TimerServiceState>,
}

impl TimerService {
    fn new() -> Result<Self> {
        let registry_mutex = sys::mutex_create().ok_or(Error::OutOfMemory)?;
        let wake_sem = match sys::binary_semaphore_create() {
            Some(s) => s,
            None => {
                sys::mutex_delete(registry_mutex);
                return Err(Error::OutOfMemory);
            }
        };
        let completion_eg = match sys::event_group_create() {
            Some(eg) => eg,
            None => {
                sys::semaphore_delete(wake_sem);
                sys::mutex_delete(registry_mutex);
                return Err(Error::OutOfMemory);
            }
        };

        Ok(Self {
            registry_mutex: Some(registry_mutex),
            wake_sem: Some(wake_sem),
            completion_eg: Some(completion_eg),
            state: UnsafeCell::new(TimerServiceState {
                timers: Vec::new(),
                next_id: 1,
                stop_requested: false,
            }),
        })
    }

    /// Lock the registry mutex, run `f`, unlock.
    fn with_lock<R>(&self, f: impl FnOnce(&mut TimerServiceState) -> R) -> R {
        let mtx = self
            .registry_mutex
            .as_ref()
            .expect("registry mutex missing");
        // When the scheduler is suspended the worker cannot run,
        // so the mutex must be free.  Use a zero-timeout try to
        // avoid the FreeRTOS assertion that forbids non-zero waits
        // while suspended.
        let timeout = match sys::scheduler_state() {
            sys::SchedulerState::Suspended => 0,
            _ => sys::max_finite_delay_ticks() + 1,
        };
        let status = sys::mutex_take(mtx, timeout);
        assert_eq!(status, sys::TakeStatus::Acquired, "registry mutex dead");
        let result = f(unsafe { &mut *self.state.get() });
        let status = sys::mutex_give(mtx);
        assert_eq!(status, sys::GiveStatus::Ok, "registry mutex unlock failed");
        result
    }

    fn signal_wake(&self) {
        let sem = self.wake_sem.as_ref().expect("wake sem missing");
        match sys::semaphore_give(sem) {
            sys::GiveStatus::Ok | sys::GiveStatus::Full => {}
            sys::GiveStatus::Invalid => panic!("wake semaphore invalid"),
        }
    }

    fn delete_resources(&mut self) {
        if let Some(eg) = self.completion_eg.take() {
            sys::event_group_delete(eg);
        }
        if let Some(sem) = self.wake_sem.take() {
            sys::semaphore_delete(sem);
        }
        if let Some(mtx) = self.registry_mutex.take() {
            sys::mutex_delete(mtx);
        }
    }
}

impl Drop for TimerService {
    fn drop(&mut self) {
        self.delete_resources();
    }
}

unsafe impl Send for TimerService {}
unsafe impl Sync for TimerService {}

// ---------------------------------------------------------------------------
// Worker loop
// ---------------------------------------------------------------------------

/// Acknowledge a specific flush request value.  Called at quiescent
/// points only — worker has scanned timers with the time that was
/// current when `observed_flush` was captured.
#[cfg(feature = "test-fixture")]
fn ack_flush(observed: u64) {
    FLUSH_ACK.store(observed, Ordering::SeqCst);
}

enum ServiceAction {
    Dispatch { id: u64, callback: TimerCallback },
    WaitUntil(Duration),
    WaitForever,
    Rescan,
    Stop,
}

/// The flush-request value the worker captured before this scan.
/// Carried through to ack_flush at quiescent points.
#[cfg(feature = "test-fixture")]
struct ScanContext {
    observed_flush: u64,
}

impl TimerService {
    fn run(&self) {
        let wake_sem_ptr =
            self.wake_sem.as_ref().expect("wake sem missing") as *const sys::SemaphoreHandle;

        loop {
            // Capture the flush-request value BEFORE scanning timers.
            // Only this captured value may be acknowledged after the
            // scan.  Requests arriving mid-scan must wait for the
            // next scan iteration.
            #[cfg(feature = "test-fixture")]
            let scan_ctx = ScanContext {
                observed_flush: FLUSH_REQUEST.load(Ordering::SeqCst),
            };

            let action = self.with_lock(|state| {
                state.timers.retain(|e| !e.deleted);

                if state.stop_requested {
                    return ServiceAction::Stop;
                }

                let now = FreeRtosClock::now();

                // Find earliest expired timer by (deadline, id).
                let best = state
                    .timers
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| !e.deleted && e.callback.is_some())
                    .filter_map(|(i, e)| e.state.deadline().map(|d| (i, d, e.id)))
                    .filter(|(_, d, _)| *d <= now)
                    .min_by_key(|(_, d, id)| (*d, *id));

                if let Some((idx, _, _)) = best {
                    let entry = &mut state.timers[idx];
                    if entry.state.advance_on_expiry(now) {
                        let cb = entry.callback.take().expect("callback missing");
                        return ServiceAction::Dispatch {
                            id: entry.id,
                            callback: cb,
                        };
                    }
                    return ServiceAction::Rescan;
                }

                // No expired timer — find earliest future deadline.
                let earliest = state
                    .timers
                    .iter()
                    .filter(|e| !e.deleted)
                    .filter_map(|e| e.state.deadline())
                    .min();

                match earliest {
                    Some(d) => ServiceAction::WaitUntil(d),
                    None => ServiceAction::WaitForever,
                }
            });

            let wake_sem = unsafe { &*wake_sem_ptr };

            match action {
                ServiceAction::Dispatch { id, callback } => {
                    self.dispatch_one(id, callback);
                }
                ServiceAction::WaitUntil(deadline) => {
                    #[cfg(feature = "test-fixture")]
                    ack_flush(scan_ctx.observed_flush);
                    wait_until_deadline(wake_sem, deadline);
                }
                ServiceAction::WaitForever => {
                    #[cfg(feature = "test-fixture")]
                    ack_flush(scan_ctx.observed_flush);
                    wait_forever(wake_sem);
                }
                ServiceAction::Rescan => {}
                ServiceAction::Stop => {
                    #[cfg(feature = "test-fixture")]
                    ack_flush(scan_ctx.observed_flush);
                    let eg = self.completion_eg.as_ref().expect("completion EG missing");
                    let status = sys::event_group_set_bits(eg, WORKER_COMPLETED_BIT);
                    assert_eq!(status, sys::EVENT_GROUP_OK, "completion EG invalid");
                    // Trampoline handles drop(service) and task_delete_current.
                    return;
                }
            }
        }
    }

    fn dispatch_one(&self, id: u64, mut callback: TimerCallback) {
        #[cfg(feature = "test-fixture")]
        DISPATCH_STARTED.fetch_add(1, Ordering::SeqCst);

        callback();

        // Try to restore callback; if the entry was deleted during
        // execution, the callback must be dropped OUTSIDE the registry
        // lock to avoid deadlock (captured Timer handles may call back
        // into the registry on drop).
        let callback_to_drop = self.with_lock(|state| {
            if let Some(entry) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
                if entry.callback.is_none() {
                    entry.callback = Some(callback);
                    return None;
                }
            }
            Some(callback)
        });

        drop(callback_to_drop);

        #[cfg(feature = "test-fixture")]
        DISPATCH_COMPLETED.fetch_add(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// Deadline waiting
// ---------------------------------------------------------------------------

fn wait_until_deadline(wake_sem: &sys::SemaphoreHandle, deadline: Duration) {
    let max_payload = sys::max_finite_delay_ticks();
    let guard_tick: u64 = 1;
    let max_payload_u128 = max_payload as u128;

    loop {
        let now = FreeRtosClock::now();
        if now >= deadline {
            return;
        }

        let remaining = deadline.saturating_sub(now);
        let caps = crate::runtime::capabilities().expect("capabilities missing");
        let remaining_ticks = duration_to_ticks_ceil(remaining, caps.tick_rate_hz);

        // On overflow, wait the max payload and re-read now — don't
        // hot-loop treating overflow as "expired".
        let chunk = match remaining_ticks {
            Ok(0) => 0u128,
            Ok(t) => t.min(max_payload_u128.saturating_sub(guard_tick as u128)),
            Err(Error::Overflow) => max_payload_u128.saturating_sub(guard_tick as u128),
            Err(_) => 0u128, // defensive: shouldn't happen
        };
        let wait_ticks: u64 =
            (chunk.saturating_add(guard_tick as u128)).min(u64::MAX as u128) as u64;

        #[cfg(feature = "test-fixture")]
        record_wake_wait(wait_ticks);

        match sys::semaphore_take(wake_sem, wait_ticks) {
            sys::TakeStatus::Acquired => return,
            sys::TakeStatus::Timeout => {
                continue;
            }
            sys::TakeStatus::Invalid => panic!("wake sem invalid"),
        }
    }
}

fn wait_forever(wake_sem: &sys::SemaphoreHandle) {
    loop {
        match sys::semaphore_take(wake_sem, sys::max_finite_delay_ticks() + 1) {
            sys::TakeStatus::Acquired => return,
            sys::TakeStatus::Timeout => {
                continue;
            }
            sys::TakeStatus::Invalid => panic!("wake sem invalid"),
        }
    }
}

// ---------------------------------------------------------------------------
// Worker entry point
// ---------------------------------------------------------------------------

unsafe extern "C" fn timer_worker(param: *mut c_void) {
    let service = unsafe { Arc::from_raw(param.cast::<TimerService>()) };
    service.run();
    // Drop the worker-owned Arc BEFORE self-deleting.  On real FreeRTOS
    // this runs TimerService::drop (deletes mutex/semaphore/EventGroup).
    // In the fixture, the thread returns normally and the JoinHandle is
    // cleaned up by internal_task_fixture_reset.
    drop(service);

    #[cfg(not(feature = "test-fixture"))]
    sys::task_delete_current();
}

// ---------------------------------------------------------------------------
// Registry access helper
// ---------------------------------------------------------------------------

/// Extract the service Arc from the control slot, then run `f` with
/// the registry mutex held.  The control lock is released before the
/// registry lock is acquired (the Arc keeps the service alive).
fn with_registry<R>(
    f: impl FnOnce(&TimerService, &mut TimerServiceState) -> Result<R>,
) -> Result<R> {
    let service = timer_control::with_slot(|slot| match slot {
        ServiceSlot::Running { service, .. } => Ok(Arc::clone(service)),
        ServiceSlot::Stopped => Err(Error::NotInitialized),
        ServiceSlot::Stopping { .. } => Err(Error::Busy),
    })?;

    service.with_lock(|state| f(&service, state))
}

// ---------------------------------------------------------------------------
// Lifecycle API
// ---------------------------------------------------------------------------

pub fn initialize() -> Result<()> {
    #[cfg(feature = "testkit")]
    fixture_reset_timer_hooks();

    timer_control::with_slot(|slot| match slot {
        ServiceSlot::Stopped => {
            let service = Arc::new(TimerService::new()?);
            *slot = ServiceSlot::Running {
                service,
                worker: None,
            };
            Ok(())
        }
        ServiceSlot::Running { .. } => Err(Error::AlreadyInitialized),
        ServiceSlot::Stopping { .. } => Err(Error::Busy),
    })
}

pub fn shutdown() -> Result<()> {
    // Scheduler preflight — checked BEFORE any state mutation to
    // guarantee failure-atomicity.  Only relevant when a worker task
    // exists (needs the scheduler to process the stop signal).
    let has_worker = timer_control::with_slot(|slot| match slot {
        ServiceSlot::Running { worker, .. } => Ok(worker.is_some()),
        ServiceSlot::Stopped => Ok(false),
        ServiceSlot::Stopping { .. } => Err(Error::Busy),
    })?;

    if has_worker {
        match sys::scheduler_state() {
            sys::SchedulerState::Running => {}
            sys::SchedulerState::NotStarted => return Err(Error::NotInitialized),
            sys::SchedulerState::Suspended => return Err(Error::Busy),
            sys::SchedulerState::Unknown(_) => {
                return Err(Error::Internal("unknown scheduler state"));
            }
        }
    }

    // Phase 1: request stop under registry lock.
    let (has_worker, service) = timer_control::with_slot(|slot| {
        let (has_worker, service) = match slot {
            ServiceSlot::Stopped => return Err(Error::NotInitialized),
            ServiceSlot::Stopping { .. } => return Err(Error::Busy),
            ServiceSlot::Running { service, worker } => (worker.is_some(), Arc::clone(service)),
        };

        // Self-shutdown detection: if the caller IS the timer worker,
        // reject with Busy to avoid deadlock (worker waiting for its
        // own completion EventGroup).
        let is_self = match slot {
            ServiceSlot::Running {
                worker: Some(w), ..
            } => {
                if let Some(current) = sys::current_native_task_handle() {
                    w.matches(&current)
                } else {
                    false
                }
            }
            _ => false,
        };
        if is_self {
            return Err(Error::Busy);
        }

        // Check live timers.
        service.with_lock(|state| {
            state.timers.retain(|e| !e.deleted);
            if !state.timers.is_empty() {
                return Err(Error::Busy);
            }
            state.stop_requested = true;
            service.signal_wake();
            Ok(())
        })?;

        // Transition to Stopping.
        *slot = ServiceSlot::Stopping {
            service: Arc::clone(&service),
        };

        Ok((has_worker, service))
    })?;

    // Phase 2: wait for worker completion (if one existed).
    // Once stop_requested is committed, this must not fail — the worker
    // is guaranteed to observe the flag and signal completion.  We wait
    // in finite chunks (never portMAX_DELAY) and retry on Timeout, so a
    // long-running in-flight callback cannot cause a spurious panic.
    #[cfg(feature = "test-fixture")]
    {
        SHUTDOWN_WAITING.store(true, Ordering::SeqCst);
    }
    if has_worker {
        let eg = service
            .completion_eg
            .as_ref()
            .expect("completion EG missing");

        loop {
            match sys::event_group_wait_bits(
                eg,
                WORKER_COMPLETED_BIT,
                false, // don't clear
                true,  // wait for all bits
                sys::max_finite_delay_ticks(),
            ) {
                sys::EventGroupWaitStatus::Ok => break,
                sys::EventGroupWaitStatus::Timeout => continue,
                sys::EventGroupWaitStatus::Invalid => {
                    panic!("live timer completion EventGroup became invalid");
                }
            }
        }
    }

    // Phase 3: clean up.
    #[cfg(feature = "test-fixture")]
    {
        SHUTDOWN_WAITING.store(false, Ordering::SeqCst);
    }
    drop(service); // Arc::drop → TimerService::drop → delete resources

    timer_control::with_slot(|slot| match slot {
        ServiceSlot::Stopping { .. } => {
            *slot = ServiceSlot::Stopped;
            Ok(())
        }
        _ => panic!("timer shutdown slot invariant violated"),
    })
}

// ---------------------------------------------------------------------------
// Ensure worker exists (lazy creation)
// ---------------------------------------------------------------------------

pub fn ensure_worker() -> Result<()> {
    timer_control::with_slot(|slot| match slot {
        ServiceSlot::Running { service, worker } => {
            // Scheduler preflight MUST precede the worker.is_some()
            // fast path.  ADR 0029 requires start/reset to return Busy
            // when the scheduler is suspended, regardless of whether
            // the worker already exists.
            match sys::scheduler_state() {
                sys::SchedulerState::NotStarted => return Err(Error::NotInitialized),
                sys::SchedulerState::Suspended => return Err(Error::Busy),
                sys::SchedulerState::Running => {}
                sys::SchedulerState::Unknown(_) => {
                    return Err(Error::Internal("unknown scheduler state"));
                }
            }

            if worker.is_some() {
                return Ok(());
            }

            let caps = crate::runtime::capabilities().ok_or(Error::NotInitialized)?;

            let stack_words = crate::task::stack_bytes_to_words(
                4096,
                caps.stack_word_size as usize,
                caps.minimal_stack_depth_words as usize,
                caps.max_stack_depth_words as usize,
            )?;

            let priority = caps.max_priorities.saturating_sub(1);

            // Arc::into_raw for passing to the worker trampoline.
            // If internal_task_create fails, we MUST reclaim this Arc
            // to avoid a permanent strong-reference leak.
            let raw = Arc::into_raw(Arc::clone(service))
                .cast_mut()
                .cast::<c_void>();

            let name = c"osal-timer".as_ptr();
            let handle = match unsafe {
                sys::internal_task_create(timer_worker, name, stack_words as u32, raw, priority)
            } {
                Some(h) => h,
                None => {
                    // Reclaim the leaked Arc — the worker will never
                    // run, so we must decrement the refcount here.
                    unsafe {
                        drop(Arc::from_raw(raw.cast::<TimerService>()));
                    }
                    return Err(Error::OutOfMemory);
                }
            };

            *worker = Some(handle);
            Ok(())
        }
        ServiceSlot::Stopped => Err(Error::NotInitialized),
        ServiceSlot::Stopping { .. } => Err(Error::Busy),
    })
}

// ---------------------------------------------------------------------------
// Timer operations
// ---------------------------------------------------------------------------

pub fn register(
    period: Duration,
    mode: osal_api::types::TimerMode,
    callback: TimerCallback,
) -> Result<u64> {
    with_registry(|service, state| {
        let timer_state = TimerState::new(period, mode)?;

        #[cfg(feature = "testkit")]
        if FAIL_NEXT_REGISTRY_RESERVE.swap(false, Ordering::SeqCst) {
            return Err(Error::OutOfMemory);
        }

        state
            .timers
            .try_reserve(1)
            .map_err(|_| Error::OutOfMemory)?;

        let id = state.next_id;
        state.next_id = id.checked_add(1).ok_or(Error::Overflow)?;

        state.timers.push(TimerEntry {
            id,
            state: timer_state,
            callback: Some(callback),
            deleted: false,
        });

        service.signal_wake();
        Ok(id)
    })
}

pub fn start(id: u64) -> Result<()> {
    ensure_worker()?;

    with_registry(|service, state| {
        let now = FreeRtosClock::now();
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.start(now)?;
            service.signal_wake();
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn stop(id: u64) -> Result<()> {
    with_registry(|service, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.stop();
            service.signal_wake();
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn reset(id: u64) -> Result<()> {
    ensure_worker()?;

    with_registry(|service, state| {
        let now = FreeRtosClock::now();
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.reset(now)?;
            service.signal_wake();
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn change_period(id: u64, new_period: Duration) -> Result<()> {
    with_registry(|_service, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.change_period(new_period)?;
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn deregister(id: u64) -> Result<()> {
    // Take the callback from the registry inside the lock, then drop it
    // outside to avoid deadlock if it captures other OSAL objects.
    let dropped_cb = with_registry(|service, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.deleted = true;
            e.state.stop();
            // Take callback — safe because dispatch_one takes it before
            // unlocking, so it cannot be in flight right now.
            let cb = e.callback.take();
            service.signal_wake();
            Ok(cb)
        } else {
            Err(Error::NotFound)
        }
    })?;

    // Drop outside all locks.
    drop(dropped_cb);
    Ok(())
}
