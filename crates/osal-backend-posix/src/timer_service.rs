//! POSIX Timer Service — single joinable pthread for timer callbacks.
//!
//! Managed through `timer_control::TimerServiceControl` (ADR 0018).
//! The service instance is explicitly created on `initialize()` and
//! destroyed on `shutdown()`.  The worker thread is joinable and holds
//! an `Arc<TimerService>` for its lifetime.
//!
//! # Lock ordering
//!
//! ```text
//! Timer API:       control mutex → service mutex
//! shutdown phase1: control mutex → service mutex
//!                   release control lock
//! shutdown phase2: pthread_join worker (outside all locks)
//! shutdown phase3: control mutex → Stopped
//! worker loop:     only service mutex
//! callback:        holds neither lock
//! ```

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::ffi::c_void;
use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::traits::timer::TimerCallback;
use osal_portable::timer_state::TimerState;

use crate::sys::condvar::PosixCondvar;
use crate::sys::mutex::PosixMutex;
use crate::sys::task::PosixThread;
use crate::sys::time;
use crate::timer_control::{self, ServiceSlot};

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

struct TimerEntry {
    id: u64,
    state: TimerState,
    callback: Option<TimerCallback>,
    deleted: bool,
}

// ---------------------------------------------------------------------------
// Service state
// ---------------------------------------------------------------------------

pub(crate) struct TimerServiceState {
    timers: Vec<TimerEntry>,
    next_id: u64,
    stop_requested: bool,
}

// ---------------------------------------------------------------------------
// Service instance
// ---------------------------------------------------------------------------

pub(crate) struct TimerService {
    mutex: PosixMutex,
    condvar: PosixCondvar,
    state: UnsafeCell<TimerServiceState>,
}

impl TimerService {
    fn new() -> Result<Self> {
        Ok(Self {
            mutex: PosixMutex::new()?,
            condvar: PosixCondvar::new()?,
            state: UnsafeCell::new(TimerServiceState {
                timers: Vec::new(),
                next_id: 1,
                stop_requested: false,
            }),
        })
    }

    /// Worker loop.  Returns when `stop_requested` is set.
    fn run(&self) {
        loop {
            let mut guard = self.mutex.lock_guard().unwrap();

            {
                let state = unsafe { &mut *self.state.get() };
                if state.stop_requested {
                    return;
                }
                state.timers.retain(|e| !e.deleted);
            }

            let state = unsafe { &mut *self.state.get() };
            let now = time::monotonic_now();
            let mut earliest: Option<Duration> = None;
            for e in &state.timers {
                if let Some(d) = e.state.deadline() {
                    match earliest {
                        None => earliest = Some(d),
                        Some(cur) if d < cur => earliest = Some(d),
                        _ => {}
                    }
                }
            }

            match earliest {
                None => {
                    let _ = self.condvar.wait(&mut guard);
                }
                Some(deadline) if deadline <= now => {
                    drop(guard);
                    self.dispatch_one();
                }
                Some(deadline) => {
                    let timeout = deadline.saturating_sub(now);
                    let abs = time::abs_deadline(timeout);
                    let _ = self.condvar.timed_wait(&mut guard, &abs);
                }
            }
        }
    }

    /// Dispatch ONE expired callback.  Callback executes outside all locks.
    fn dispatch_one(&self) {
        let (id, mut callback) = {
            let _guard = self.mutex.lock_guard().unwrap();
            let state = unsafe { &mut *self.state.get() };
            let now = time::monotonic_now();

            let mut best_idx: Option<usize> = None;
            for (i, e) in state.timers.iter().enumerate() {
                if e.deleted || e.callback.is_none() {
                    continue;
                }
                if let Some(d) = e.state.deadline() {
                    if d <= now {
                        best_idx = Some(i);
                        break;
                    }
                }
            }

            let Some(idx) = best_idx else { return };
            let entry = &mut state.timers[idx];
            if !entry.state.advance_on_expiry(now) {
                return;
            }
            (entry.id, entry.callback.take().unwrap())
        };

        callback();

        let _guard = self.mutex.lock_guard().unwrap();
        let state = unsafe { &mut *self.state.get() };
        if let Some(entry) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            if entry.callback.is_none() {
                entry.callback = Some(callback);
            }
        }
    }
}

unsafe impl Send for TimerService {}
unsafe impl Sync for TimerService {}

// ---------------------------------------------------------------------------
// Worker entry point
// ---------------------------------------------------------------------------

extern "C" fn timer_worker(arg: *mut c_void) -> *mut c_void {
    let service = unsafe { Arc::from_raw(arg.cast::<TimerService>()) };
    service.run();
    core::ptr::null_mut()
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn with_service<R>(f: impl FnOnce(&TimerService, &mut TimerServiceState) -> Result<R>) -> Result<R> {
    timer_control::with_control(|ctrl| match &ctrl.slot {
        ServiceSlot::Running { service, .. } => {
            let _guard = service.mutex.lock_guard()?;
            let state = unsafe { &mut *service.state.get() };
            f(service, state)
        }
        ServiceSlot::Stopped => Err(Error::NotInitialized),
        ServiceSlot::Stopping { .. } => Err(Error::Busy),
    })
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn initialize() -> Result<()> {
    timer_control::with_control(|ctrl| match &ctrl.slot {
        ServiceSlot::Stopped => {
            let service = Arc::new(TimerService::new()?);
            let worker_ref = Arc::into_raw(Arc::clone(&service)).cast_mut().cast::<c_void>();
            let cfg = crate::sys::task::PosixThreadConfig { stack_size: 4096 };
            let worker = PosixThread::spawn(&cfg, timer_worker, worker_ref).map_err(|_| {
                unsafe {
                    drop(Arc::from_raw(worker_ref.cast::<TimerService>()));
                }
                Error::Internal("timer worker spawn failed")
            })?;
            let generation = ctrl.next_generation;
            ctrl.next_generation = ctrl
                .next_generation
                .checked_add(1)
                .ok_or(Error::Overflow)?;
            ctrl.slot = ServiceSlot::Running {
                service,
                worker,
                generation,
            };
            Ok(())
        }
        ServiceSlot::Running { .. } => Err(Error::AlreadyInitialized),
        ServiceSlot::Stopping { .. } => Err(Error::Busy),
    })
}

pub fn shutdown() -> Result<()> {
    // Phase 1: request stop under control lock, extract worker.
    let mut worker = {
        timer_control::with_control(|ctrl| {
            let (service, generation) = match &ctrl.slot {
                ServiceSlot::Stopped => return Err(Error::NotInitialized),
                ServiceSlot::Stopping { .. } => return Err(Error::Busy),
                ServiceSlot::Running {
                    service, generation, ..
                } => (Arc::clone(service), *generation),
            };

            // Check for live timers (including stopped-but-not-dropped).
            {
                let _guard = service.mutex.lock_guard()?;
                let state = unsafe { &mut *service.state.get() };
                state.timers.retain(|e| !e.deleted);
                if !state.timers.is_empty() {
                    return Err(Error::Busy);
                }
                state.stop_requested = true;
                service
                    .condvar
                    .broadcast()
                    .expect("timer shutdown broadcast failed");
            }

            let old = core::mem::replace(&mut ctrl.slot, ServiceSlot::Stopping { generation });
            match old {
                ServiceSlot::Running { worker, .. } => Ok(worker),
                _ => unreachable!(),
            }
        })?
    };

    // Phase 2: join OUTSIDE the control lock.
    worker
        .try_join()
        .expect("timer worker join invariant violated");

    // Phase 3: transition to Stopped.
    timer_control::with_control(|ctrl| match &ctrl.slot {
        ServiceSlot::Stopping { .. } => {
            ctrl.slot = ServiceSlot::Stopped;
            Ok(())
        }
        _ => Err(Error::Internal("timer slot state inconsistent")),
    })
}

#[allow(dead_code)]
pub fn is_running() -> bool {
    timer_control::with_control(|ctrl| matches!(ctrl.slot, ServiceSlot::Running { .. }))
}

// ---------------------------------------------------------------------------
// Timer operations
// ---------------------------------------------------------------------------

pub fn register(
    period: Duration,
    mode: osal_api::types::TimerMode,
    callback: TimerCallback,
) -> Result<u64> {
    with_service(|svc, state| {
        let id = state.next_id;
        state.next_id = state
            .next_id
            .checked_add(1)
            .ok_or(Error::Overflow)?;
        debug_assert_ne!(id, 0, "timer ID 0 is reserved");
        state.timers.push(TimerEntry {
            id,
            state: TimerState::new(period, mode).map_err(|_| Error::InvalidParameter)?,
            callback: Some(callback),
            deleted: false,
        });
        svc.condvar.signal()?;
        Ok(id)
    })
}

pub fn start(id: u64) -> Result<()> {
    with_service(|svc, state| {
        let now = time::monotonic_now();
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.start(now)?;
            svc.condvar.signal()?;
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn stop(id: u64) -> Result<()> {
    with_service(|svc, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.stop();
            svc.condvar.signal()?;
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn reset(id: u64) -> Result<()> {
    with_service(|svc, state| {
        let now = time::monotonic_now();
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.reset(now)?;
            svc.condvar.signal()?;
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn change_period(id: u64, new_period: Duration) -> Result<()> {
    with_service(|svc, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.change_period(new_period)?;
            svc.condvar.signal()?;
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}

pub fn deregister(id: u64) -> Result<()> {
    with_service(|svc, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.deleted = true;
            e.state.stop();
            e.callback = None;
            svc.condvar.signal()?;
            Ok(())
        } else {
            Err(Error::NotFound)
        }
    })
}
