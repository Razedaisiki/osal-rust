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
//! shutdown:        control mutex → service mutex (phase 1)
//!                  release control lock
//!                  pthread_join worker (outside all locks)
//!                  control mutex → Stopped (phase 2)
//! worker loop:     only service mutex
//! callback:        holds neither lock
//! ```

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

use alloc::sync::Arc;

use osal_api::error::{Error, Result};

use crate::sys::task::PosixThread;

// ---------------------------------------------------------------------------
// Init states
// ---------------------------------------------------------------------------

const UNINITIALIZED: u8 = 0;
const INITIALIZING: u8 = 1;
const READY: u8 = 2;

// ---------------------------------------------------------------------------
// Service slot
// ---------------------------------------------------------------------------

pub(crate) enum ServiceSlot {
    Stopped,
    Running {
        service: Arc<super::timer_service::TimerService>,
        worker: PosixThread,
        generation: u64,
    },
    #[allow(dead_code)]
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
    init_state: AtomicU8,
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
            init_state: AtomicU8::new(UNINITIALIZED),
        }
    }

    /// Ensure the control mutex is initialised.  Idempotent and
    /// linearizable — only the thread that CASes to `INITIALIZING`
    /// calls `pthread_mutex_init`.  Other threads spin until
    /// `READY`.  On init failure the state rolls back to
    /// `UNINITIALIZED` so retries are possible.
    fn ensure_init(&self) -> Result<()> {
        loop {
            match self.init_state.load(Ordering::Acquire) {
                READY => return Ok(()),
                UNINITIALIZED => {
                    if self
                        .init_state
                        .compare_exchange(
                            UNINITIALIZED,
                            INITIALIZING,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        let rc = unsafe {
                            libc::pthread_mutex_init(
                                (*self.mutex.get()).as_mut_ptr(),
                                core::ptr::null(),
                            )
                        };
                        if rc != 0 {
                            self.init_state.store(UNINITIALIZED, Ordering::Release);
                            return Err(Error::Internal("timer control mutex init failed"));
                        }
                        self.init_state.store(READY, Ordering::Release);
                        return Ok(());
                    }
                }
                INITIALIZING => {
                    core::hint::spin_loop();
                }
                _ => unreachable!(),
            }
        }
    }

    fn lock(&self) -> Result<()> {
        self.ensure_init()?;
        let rc = unsafe { libc::pthread_mutex_lock((*self.mutex.get()).as_mut_ptr()) };
        if rc != 0 {
            return Err(Error::Internal("timer control mutex lock failed"));
        }
        Ok(())
    }

    fn unlock(&self) {
        let rc = unsafe { libc::pthread_mutex_unlock((*self.mutex.get()).as_mut_ptr()) };
        debug_assert_eq!(rc, 0, "timer control mutex unlock failed");
    }

    /// Lock the control mutex, run `f` with mutable access to the
    /// control state, then unlock.  Unlock is guaranteed even if
    /// `f` panics (via RAII guard).
    pub fn with_state<R>(&self, f: impl FnOnce(&mut TimerControlState) -> Result<R>) -> Result<R> {
        self.lock()?;
        struct Guard<'a> {
            ctrl: &'a TimerServiceControl,
        }
        impl Drop for Guard<'_> {
            fn drop(&mut self) {
                self.ctrl.unlock();
            }
        }
        let _guard = Guard { ctrl: self };
        let state = unsafe { &mut *self.state.get() };
        f(state)
    }
}

// ---------------------------------------------------------------------------
// Global control block
// ---------------------------------------------------------------------------

static CONTROL: TimerServiceControl = TimerServiceControl::new();

pub(crate) fn with_control<R>(f: impl FnOnce(&mut TimerControlState) -> Result<R>) -> Result<R> {
    CONTROL.with_state(f)
}
