//! Timer service control block — process-lifetime, restart-safe.
//!
//! A single process-lifetime `static` holds a mutex-protected
//! `ServiceSlot`.  The actual `TimerService` (timers, worker
//! thread) is created and destroyed inside the slot; the control
//! block itself persists across restarts.
//!
//! # Lock ordering (ADR 0018)
//!
//! ```text
//! Timer API:       control mutex → service mutex
//! shutdown:        control mutex → service mutex
//! worker loop:     only service mutex
//! callback:        holds neither lock
//! ```

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicBool, Ordering};

use alloc::sync::Arc;

// ---------------------------------------------------------------------------
// Service slot
// ---------------------------------------------------------------------------

pub(crate) enum ServiceSlot {
    Stopped,
    Running {
        service: Arc<super::timer_service::TimerService>,
        worker: super::sys::task::PosixThread,
        generation: u64,
    },
    Stopping {
        generation: u64,
    },
}

// ---------------------------------------------------------------------------
// Control state
// ---------------------------------------------------------------------------

pub(crate) struct TimerControlState {
    pub slot: ServiceSlot,
    pub next_generation: u64,
}

// ---------------------------------------------------------------------------
// Control block
// ---------------------------------------------------------------------------

pub(crate) struct TimerServiceControl {
    mutex: UnsafeCell<MaybeUninit<libc::pthread_mutex_t>>,
    state: UnsafeCell<TimerControlState>,
    ready: AtomicBool,
}

unsafe impl Sync for TimerServiceControl {}

impl TimerServiceControl {
    pub const fn new() -> Self {
        Self {
            mutex: UnsafeCell::new(MaybeUninit::uninit()),
            state: UnsafeCell::new(TimerControlState {
                slot: ServiceSlot::Stopped,
                next_generation: 1,
            }),
            ready: AtomicBool::new(false),
        }
    }

    fn ensure_init(&self) {
        if self.ready.load(Ordering::Acquire) {
            return;
        }
        // CAS to claim initialisation; loser spins.
        if self
            .ready
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            unsafe {
                libc::pthread_mutex_init((*self.mutex.get()).as_mut_ptr(), core::ptr::null());
            }
            self.ready.store(true, Ordering::Release);
        } else {
            while !self.ready.load(Ordering::Acquire) {
                core::hint::spin_loop();
            }
        }
    }

    fn lock(&self) {
        self.ensure_init();
        unsafe {
            libc::pthread_mutex_lock((*self.mutex.get()).as_mut_ptr());
        }
    }

    fn unlock(&self) {
        unsafe {
            libc::pthread_mutex_unlock((*self.mutex.get()).as_mut_ptr());
        }
    }

    pub fn with_state<R>(&self, f: impl FnOnce(&mut TimerControlState) -> R) -> R {
        self.lock();
        let state = unsafe { &mut *self.state.get() };
        let result = f(state);
        self.unlock();
        result
    }
}

// ---------------------------------------------------------------------------
// Global control block
// ---------------------------------------------------------------------------

static CONTROL: TimerServiceControl = TimerServiceControl::new();

pub(crate) fn with_control<R>(f: impl FnOnce(&mut TimerControlState) -> R) -> R {
    CONTROL.with_state(f)
}
