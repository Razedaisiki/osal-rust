//! POSIX Timer Service — single background pthread for timer callbacks.
//!
//! Lazy-initialized via `pthread_once`. State is protected by `PosixMutex`.
//! Callbacks execute outside the mutex lock.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::time::Duration;

use osal_api::traits::timer::TimerCallback;
use osal_portable::timer_state::TimerState;

use crate::sys::condvar::PosixCondvar;
use crate::sys::mutex::PosixMutex;
use crate::sys::time;

// ---------------------------------------------------------------------------
// Service state
// ---------------------------------------------------------------------------

struct TimerEntry {
    id: u64,
    state: TimerState,
    callback: Option<TimerCallback>,
    deleted: bool,
}

struct TimerServiceState {
    timers: Vec<TimerEntry>,
    next_id: u64,
    initialized: bool,
}

struct TimerService {
    mutex: PosixMutex,
    condvar: PosixCondvar,
    state: UnsafeCell<TimerServiceState>,
}

// Safety: all state access is guarded by the mutex.
unsafe impl Send for TimerService {}
unsafe impl Sync for TimerService {}

// ---------------------------------------------------------------------------
// Singleton via pthread_once
// ---------------------------------------------------------------------------

static mut SERVICE_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;
static mut SERVICE: MaybeUninit<TimerService> = MaybeUninit::uninit();
static mut SERVICE_OK: bool = false;

extern "C" fn init_service() {
    unsafe {
        let mutex = match PosixMutex::new() {
            Ok(m) => m,
            Err(_) => return,
        };
        let condvar = match PosixCondvar::new() {
            Ok(c) => c,
            Err(_) => return,
        };
        SERVICE.write(TimerService {
            mutex,
            condvar,
            state: UnsafeCell::new(TimerServiceState {
                timers: Vec::new(),
                next_id: 1,
                initialized: false,
            }),
        });
        // Only mark OK after thread creation succeeds
        if spawn_service_thread() {
            SERVICE_OK = true;
        }
    }
}

fn ensure_init() -> bool {
    unsafe {
        libc::pthread_once(&raw mut SERVICE_ONCE, Some(init_service));
        SERVICE_OK
    }
}

fn with_service<R>(f: impl FnOnce(&TimerService) -> R) -> Option<R> {
    if !ensure_init() {
        return None;
    }
    unsafe { Some(f(&*SERVICE.as_ptr())) }
}

fn with_service_locked<R>(f: impl FnOnce(&TimerService, &mut TimerServiceState) -> R) -> Option<R> {
    with_service(|svc| {
        let guard = svc.mutex.lock_guard().ok()?;
        let state = unsafe { &mut *svc.state.get() };
        let result = f(svc, state);
        drop(guard);
        Some(result)
    })?
}

// ---------------------------------------------------------------------------
// Service thread
// ---------------------------------------------------------------------------

fn spawn_service_thread() -> bool {
    unsafe {
        let mut attr: libc::pthread_attr_t = core::mem::zeroed();
        libc::pthread_attr_init(&mut attr);
        libc::pthread_attr_setdetachstate(&mut attr, libc::PTHREAD_CREATE_DETACHED);
        let mut thread: libc::pthread_t = core::mem::zeroed();
        let ret = libc::pthread_create(&mut thread, &attr, service_loop, core::ptr::null_mut());
        libc::pthread_attr_destroy(&mut attr);
        ret == 0
    }
}

extern "C" fn service_loop(_arg: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
    loop {
        let svc = unsafe { &*SERVICE.as_ptr() };
        let mut guard = svc.mutex.lock_guard().unwrap();
        let state = unsafe { &mut *svc.state.get() };

        // Clean up deleted timers
        state.timers.retain(|e| !e.deleted);

        // Find earliest deadline
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
                let _ = svc.condvar.wait(&mut guard);
            }
            Some(deadline) if deadline <= now => {
                drop(guard);
                // Dispatch one callback outside lock
                dispatch_one(svc);
            }
            Some(deadline) => {
                let timeout = deadline.saturating_sub(now);
                let abs = time::abs_deadline(timeout);
                let _ = svc.condvar.timed_wait(&mut guard, &abs);
            }
        }
    }
}

/// Dispatch ONE expired timer. Locks, finds earliest expired, takes
/// callback, unlocks, executes, re-locks, restores callback.
fn dispatch_one(svc: &TimerService) {
    let mut guard = svc.mutex.lock_guard().unwrap();
    let state = unsafe { &mut *svc.state.get() };
    let now = time::monotonic_now();

    // Find earliest expired with callback present
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

    let idx = match best_idx {
        Some(i) => i,
        None => return,
    };

    let entry = &mut state.timers[idx];
    if !entry.state.advance_on_expiry(now) {
        return;
    }
    let id = entry.id;
    let mut callback = entry.callback.take().unwrap();

    // Release ALL borrows before executing callback
    drop(state);
    drop(guard);

    callback();

    // Re-acquire and restore
    let mut guard2 = svc.mutex.lock_guard().unwrap();
    let state2 = unsafe { &mut *svc.state.get() };
    if let Some(entry) = state2.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
        if entry.callback.is_none() {
            entry.callback = Some(callback);
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn register(period: Duration, mode: osal_api::types::TimerMode, callback: TimerCallback) -> Option<u64> {
    with_service_locked(|svc, state| {
        let id = state.next_id;
        state.next_id += 1;
        state.timers.push(TimerEntry {
            id,
            state: TimerState::new(period, mode).expect("period validated by caller"),
            callback: Some(callback),
            deleted: false,
        });
        let _ = svc.condvar.signal();
        id
    })
}

pub fn start(id: u64) {
    with_service_locked(|svc, state| {
        let now = time::monotonic_now();
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            let _ = e.state.start(now);
            let _ = svc.condvar.signal();
        }
    });
}

pub fn stop(id: u64) {
    with_service_locked(|svc, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.stop();
            let _ = svc.condvar.signal();
        }
    });
}

pub fn reset(id: u64) {
    with_service_locked(|svc, state| {
        let now = time::monotonic_now();
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            let _ = e.state.reset(now);
            let _ = svc.condvar.signal();
        }
    });
}

pub fn change_period(id: u64, new_period: Duration) {
    with_service_locked(|svc, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            let _ = e.state.change_period(new_period);
            let _ = svc.condvar.signal();
        }
    });
}

pub fn deregister(id: u64) {
    with_service_locked(|svc, state| {
        if let Some(e) = state.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.deleted = true;
            e.state.stop();
            e.callback = None;
            let _ = svc.condvar.signal();
        }
    });
}
