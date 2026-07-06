//! POSIX Timer Service — single background pthread for timer callbacks.
//!
//! Lazy-initialized on first timer creation. Uses `pthread_mutex_t` +
//! `pthread_cond_t` (CLOCK_MONOTONIC) for waiting. Callbacks execute
//! outside the registry lock.

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::time::Duration;

use osal_api::traits::timer::TimerCallback;
use osal_api::types::TimerMode;
use osal_portable::timer_state::TimerState;

use crate::sys::condvar::PosixCondvar;
use crate::sys::mutex::PosixMutex;
use crate::sys::time;

// ---------------------------------------------------------------------------
// Registry entry
// ---------------------------------------------------------------------------

struct TimerEntry {
    id: u64,
    state: TimerState,
    callback: Option<TimerCallback>,
    deleted: bool,
}

// ---------------------------------------------------------------------------
// Service singleton
// ---------------------------------------------------------------------------

struct ServiceInner {
    mutex: PosixMutex,
    condvar: PosixCondvar,
    timers: Vec<TimerEntry>,
    next_id: u64,
    initialized: bool,
}

static mut SERVICE_PTR: *mut ServiceInner = core::ptr::null_mut();

fn with_service<R>(f: impl FnOnce(&mut ServiceInner) -> R) -> R {
    unsafe { f(&mut *SERVICE_PTR) }
}

fn ensure_initialized() {
    unsafe {
        if SERVICE_PTR.is_null() {
            let inner = Box::new(ServiceInner {
                mutex: PosixMutex::new().expect("TimerService mutex"),
                condvar: PosixCondvar::new().expect("TimerService condvar"),
                timers: Vec::new(),
                next_id: 1,
                initialized: false,
            });
            SERVICE_PTR = Box::into_raw(inner);
            spawn_service_thread();
        }
    }
}

// ---------------------------------------------------------------------------
// Service thread
// ---------------------------------------------------------------------------

fn spawn_service_thread() {
    unsafe {
        let mut attr: libc::pthread_attr_t = core::mem::zeroed();
        libc::pthread_attr_init(&mut attr);
        libc::pthread_attr_setdetachstate(&mut attr, libc::PTHREAD_CREATE_DETACHED);
        let mut thread: libc::pthread_t = core::mem::zeroed();
        let ret = libc::pthread_create(&mut thread, &attr, service_loop, core::ptr::null_mut());
        libc::pthread_attr_destroy(&mut attr);
        assert_eq!(ret, 0, "pthread_create for TimerService failed");
        // Mark as initialized so we don't spawn another thread
        with_service(|s| s.initialized = true);
    }
}

extern "C" fn service_loop(_arg: *mut core::ffi::c_void) -> *mut core::ffi::c_void {
    loop {
        // Access raw pointer to avoid double-borrow issues with lock_guard
        let s = unsafe { &mut *SERVICE_PTR };

        // Clean up deleted timers
        s.timers.retain(|e| !e.deleted);

        // Acquire lock for condvar wait
        let mut guard = s.mutex.lock_guard().unwrap();

        // Find earliest deadline
        let now = time::monotonic_now();
        let mut earliest: Option<Duration> = None;
        for e in &s.timers {
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
                let _ = s.condvar.wait(&mut guard);
            }
            Some(deadline) if deadline <= now => {
                drop(guard);
                dispatch_expired(s);
            }
            Some(deadline) => {
                let timeout = deadline.saturating_sub(now);
                let abs = time::abs_deadline(timeout);
                let _ = s.condvar.timed_wait(&mut guard, &abs);
            }
        }
    }
}

fn dispatch_expired(s: &mut ServiceInner) {
    let now = time::monotonic_now();

    // Find the timer with earliest deadline <= now
    let mut best_idx: Option<usize> = None;
    for (i, e) in s.timers.iter().enumerate() {
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

    let entry = &mut s.timers[idx];

    // Pre-advance state before callback
    if !entry.state.advance_on_expiry(now) {
        return;
    }
    let mut callback = entry.callback.take().unwrap();

    // Execute outside lock: we need to drop the borrow on `s`.
    // A full implementation would release the mutex here.
    // (Will be fixed in a subsequent commit.)
    callback();

    // Restore callback if entry still exists (both OneShot and Periodic)
    if let Some(entry) = s.timers.get_mut(idx) {
        if !entry.deleted && entry.callback.is_none() {
            entry.callback = Some(callback);
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn register(period: Duration, mode: TimerMode, callback: TimerCallback) -> u64 {
    ensure_initialized();
    with_service(|s| {
        let id = s.next_id;
        s.next_id += 1;
        s.timers.push(TimerEntry {
            id,
            state: TimerState::new(period, mode).expect("period validated by caller"),
            callback: Some(callback),
            deleted: false,
        });
        // Signal service thread to recompute earliest deadline
        let _ = s.condvar.signal();
        id
    })
}

pub fn start(id: u64) {
    with_service(|s| {
        if let Some(e) = s.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            let now = time::monotonic_now();
            let _ = e.state.start(now);
            let _ = s.condvar.signal();
        }
    });
}

pub fn stop(id: u64) {
    with_service(|s| {
        if let Some(e) = s.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.state.stop();
            let _ = s.condvar.signal();
        }
    });
}

pub fn reset(id: u64) {
    with_service(|s| {
        if let Some(e) = s.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            let now = time::monotonic_now();
            let _ = e.state.reset(now);
            let _ = s.condvar.signal();
        }
    });
}

pub fn change_period(id: u64, new_period: Duration) {
    with_service(|s| {
        if let Some(e) = s.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            let _ = e.state.change_period(new_period);
            let _ = s.condvar.signal();
        }
    });
}

pub fn deregister(id: u64) {
    with_service(|s| {
        if let Some(e) = s.timers.iter_mut().find(|e| e.id == id && !e.deleted) {
            e.deleted = true;
            e.state.stop();
            e.callback = None;
            let _ = s.condvar.signal();
        }
    });
}
