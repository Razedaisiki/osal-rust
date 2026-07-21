//! Per-thread storage via `pthread_key_create` / `pthread_getspecific`.
//!
//! Used by [`crate::task`] for `current()` identity without requiring
//! `std::thread_local!`.  The TLS key is initialised on first use with
//! a CAS retry loop (no `pthread_once`) so that initialisation failure
//! can be retried.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;
use core::sync::atomic::{AtomicU8, Ordering};

use osal_api::error::{Error, Result};

// ---------------------------------------------------------------------------
// TLS key state
// ---------------------------------------------------------------------------

const UNINITIALIZED: u8 = 0;
const INITIALIZING: u8 = 1;
const READY: u8 = 2;

/// Storage for a lazily-initialised `pthread_key_t`.
///
/// Initialisation uses a CAS loop — if `pthread_key_create` fails,
/// the state returns to `UNINITIALIZED` so a subsequent caller can
/// retry.
pub struct TaskTlsSlot {
    state: AtomicU8,
    key: UnsafeCell<MaybeUninit<libc::pthread_key_t>>,
}

// Safety: once READY, the key is immutable.  Before READY, access is
// serialised by the CAS on `state`.
unsafe impl Sync for TaskTlsSlot {}

impl TaskTlsSlot {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(UNINITIALIZED),
            key: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// Return the key, initialising it if necessary.
    ///
    /// May fail if `pthread_key_create` returns an error.  On failure
    /// the slot rolls back to `UNINITIALIZED` so the caller may
    /// retry.
    pub fn get_or_init(&self) -> Result<libc::pthread_key_t> {
        // Fast path.
        if self.state.load(Ordering::Acquire) == READY {
            return Ok(unsafe { (*self.key.get()).assume_init() });
        }

        // Try to claim the initialisation token.
        if self
            .state
            .compare_exchange(
                UNINITIALIZED,
                INITIALIZING,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
        {
            // We own the token — create the key.
            let mut raw_key: libc::pthread_key_t = 0;
            // Safety: `pthread_key_create` is thread-safe.
            let rc = unsafe { libc::pthread_key_create(&raw mut raw_key, None) };

            if rc == 0 {
                unsafe { (*self.key.get()).write(raw_key) };
                self.state.store(READY, Ordering::Release);
                Ok(raw_key)
            } else {
                // Rollback so retries are possible.
                self.state.store(UNINITIALIZED, Ordering::Release);
                Err(Error::Internal("pthread_key_create failed"))
            }
        } else {
            // Another thread is initialising — spin until ready or
            // rolled back.
            loop {
                match self.state.load(Ordering::Acquire) {
                    READY => {
                        return Ok(unsafe { (*self.key.get()).assume_init() });
                    }
                    UNINITIALIZED => {
                        // Initialisation failed — caller may retry.
                        return Err(Error::Internal(
                            "pthread TLS initialisation was attempted but failed",
                        ));
                    }
                    _ => {
                        core::hint::spin_loop();
                        unsafe {
                            libc::sched_yield();
                        }
                    }
                }
            }
        }
    }

    /// Return the key if already initialised, or `None`.
    pub fn get(&self) -> Option<libc::pthread_key_t> {
        if self.state.load(Ordering::Acquire) == READY {
            Some(unsafe { (*self.key.get()).assume_init() })
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Current-guard helper
// ---------------------------------------------------------------------------

/// RAII guard that pushes a value onto the TLS slot and restores the
/// previous value on drop.
pub struct CurrentGuard {
    key: libc::pthread_key_t,
    previous: *mut core::ffi::c_void,
}

impl CurrentGuard {
    /// Push `value` onto the per-thread slot.
    ///
    /// Returns `Err(OutOfMemory)` if `pthread_setspecific` fails.
    pub fn enter(key: libc::pthread_key_t, value: *mut core::ffi::c_void) -> Result<Self> {
        let previous = unsafe { libc::pthread_getspecific(key) };
        let rc = unsafe { libc::pthread_setspecific(key, value) };
        if rc != 0 {
            return Err(Error::OutOfMemory);
        }
        Ok(Self { key, previous })
    }
}

impl Drop for CurrentGuard {
    fn drop(&mut self) {
        let rc = unsafe { libc::pthread_setspecific(self.key, self.previous) };
        debug_assert_eq!(rc, 0, "pthread_setspecific restore failed");
    }
}
