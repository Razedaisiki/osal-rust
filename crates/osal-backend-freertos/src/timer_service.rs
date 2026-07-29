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
        let status = sys::mutex_take(mtx, sys::max_finite_delay_ticks() + 1);
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

enum ServiceAction {
    Dispatch { id: u64, callback: TimerCallback },
    WaitUntil(Duration),
    WaitForever,
    Rescan,
    Stop,
}

impl TimerService {
    fn run(&self) {
        let wake_sem_ptr =
            self.wake_sem.as_ref().expect("wake sem missing") as *const sys::SemaphoreHandle;

        loop {
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
                    wait_until_deadline(wake_sem, deadline);
                }
                ServiceAction::WaitForever => {
                    wait_forever(wake_sem);
                }
                ServiceAction::Rescan => {}
                ServiceAction::Stop => {
                    let eg = self.completion_eg.as_ref().expect("completion EG missing");
                    let status = sys::event_group_set_bits(eg, WORKER_COMPLETED_BIT);
                    assert_eq!(status, sys::EVENT_GROUP_OK, "completion EG invalid");
                    sys::task_delete_current();
                }
            }
        }
    }

    fn dispatch_one(&self, id: u64, mut callback: TimerCallback) {
        callback();

        // Restore callback unless deleted during execution.
        self.with_lock(|state| {
            if let Some(entry) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
                if entry.callback.is_none() {
                    entry.callback = Some(callback);
                    return;
                }
            }
            // Entry deleted — drop outside lock.
            drop(callback);
        });
    }
}

// ---------------------------------------------------------------------------
// Deadline waiting
// ---------------------------------------------------------------------------

fn wait_until_deadline(wake_sem: &sys::SemaphoreHandle, deadline: Duration) {
    let max_payload = sys::max_finite_delay_ticks();
    let guard_tick: u64 = 1;

    loop {
        let now = FreeRtosClock::now();
        if now >= deadline {
            return;
        }

        let remaining = deadline.saturating_sub(now);
        let caps = crate::runtime::capabilities().expect("capabilities missing");
        let remaining_ticks = duration_to_ticks_ceil(remaining, caps.tick_rate_hz);

        let Ok(remaining_ticks) = remaining_ticks else {
            return; // overflow → treat as expired
        };

        let max_payload_u128 = max_payload as u128;
        let chunk = if remaining_ticks == 0 {
            0u128
        } else {
            remaining_ticks.min(max_payload_u128.saturating_sub(guard_tick as u128))
        };
        let wait_ticks: u64 =
            (chunk.saturating_add(guard_tick as u128)).min(u64::MAX as u128) as u64;

        match sys::semaphore_take(wake_sem, wait_ticks) {
            sys::TakeStatus::Acquired => return,
            sys::TakeStatus::Timeout => continue,
            sys::TakeStatus::Invalid => panic!("wake sem invalid"),
        }
    }
}

fn wait_forever(wake_sem: &sys::SemaphoreHandle) {
    loop {
        match sys::semaphore_take(wake_sem, sys::max_finite_delay_ticks() + 1) {
            sys::TakeStatus::Acquired => return,
            sys::TakeStatus::Timeout => continue,
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
    drop(service);
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
        ServiceSlot::Stopping => Err(Error::Busy),
    })?;

    service.with_lock(|state| f(&service, state))
}

// ---------------------------------------------------------------------------
// Lifecycle API
// ---------------------------------------------------------------------------

pub fn initialize() -> Result<()> {
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
        ServiceSlot::Stopping => Err(Error::Busy),
    })
}

pub fn shutdown() -> Result<()> {
    // Phase 1: request stop under registry lock.
    let (has_worker, service) = timer_control::with_slot(|slot| {
        let (has_worker, service) = match slot {
            ServiceSlot::Stopped => return Err(Error::NotInitialized),
            ServiceSlot::Stopping => return Err(Error::Busy),
            ServiceSlot::Running { service, worker } => (worker.is_some(), Arc::clone(service)),
        };

        // FIXME(P7F): self-shutdown detection — compare stored worker
        // handle with current native task handle and return Busy if
        // the caller is the timer worker.  Requires exposing raw
        // pointers from InternalTaskHandle/NativeTaskHandle for comparison.

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
        *slot = ServiceSlot::Stopping;

        Ok((has_worker, service))
    })?;

    // Phase 2: wait for worker completion (if one existed).
    if has_worker {
        let eg = service
            .completion_eg
            .as_ref()
            .expect("completion EG missing");
        let status = sys::event_group_wait_bits(
            eg,
            WORKER_COMPLETED_BIT,
            false, // don't clear
            true,  // wait for all bits
            sys::max_finite_delay_ticks() + 1,
        );
        assert_eq!(
            status,
            sys::EventGroupWaitStatus::Ok,
            "timer worker did not signal completion"
        );
    }

    // Phase 3: clean up.
    drop(service); // Arc::drop → TimerService::drop → delete resources

    timer_control::with_slot(|slot| match slot {
        ServiceSlot::Stopping => {
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
            if worker.is_some() {
                return Ok(());
            }

            // Scheduler state preflight.
            match sys::scheduler_state() {
                sys::SchedulerState::NotStarted => return Err(Error::NotInitialized),
                sys::SchedulerState::Suspended => return Err(Error::Busy),
                sys::SchedulerState::Running => {}
                sys::SchedulerState::Unknown(_) => {
                    return Err(Error::Internal("unknown scheduler state"));
                }
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
            let raw = Arc::into_raw(Arc::clone(service))
                .cast_mut()
                .cast::<c_void>();

            let name = c"roussatl-timer".as_ptr();
            let handle = unsafe {
                sys::internal_task_create(timer_worker, name, stack_words as u32, raw, priority)
            }
            .ok_or(Error::OutOfMemory)?;

            *worker = Some(handle);
            Ok(())
        }
        ServiceSlot::Stopped => Err(Error::NotInitialized),
        ServiceSlot::Stopping => Err(Error::Busy),
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
    with_registry(|service, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.deleted = true;
            e.state.stop();
            // Take callback so it's dropped when the caller releases the lock.
            // The callback is NOT in flight (dispatch_one takes it before
            // unlocking), so taking it here is safe.
            e.callback = None;
            service.signal_wake();
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}
