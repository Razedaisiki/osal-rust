//! Thin wrapper around `pthread_cond_t`.

use core::cell::UnsafeCell;
use core::time::Duration;

use osal_api::error::Result;

use crate::sys::errno;
use crate::sys::mutex::PosixMutexGuard;

// ---------------------------------------------------------------------------
// CondAttr — RAII wrapper for pthread_condattr_t
// ---------------------------------------------------------------------------

struct CondAttr {
    inner: libc::pthread_condattr_t,
    initialized: bool,
}

impl CondAttr {
    fn new() -> Result<Self> {
        let mut attr = Self {
            inner: unsafe { core::mem::zeroed() },
            initialized: false,
        };
        errno::check_ret(unsafe { libc::pthread_condattr_init(&mut attr.inner) })?;
        attr.initialized = true;
        Ok(attr)
    }
}

impl Drop for CondAttr {
    fn drop(&mut self) {
        if self.initialized {
            unsafe {
                libc::pthread_condattr_destroy(&mut self.inner);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// PosixCondvar
// ---------------------------------------------------------------------------

/// Wrapper around `pthread_cond_t`.
///
/// Uses `CLOCK_MONOTONIC`. The inner FFI object is wrapped in
/// [`UnsafeCell`] because pthread operations mutate it through `&self`.
pub struct PosixCondvar {
    inner: UnsafeCell<libc::pthread_cond_t>,
}

impl PosixCondvar {
    /// Create and initialize a new condition variable with
    /// `CLOCK_MONOTONIC`.
    pub fn new() -> Result<Self> {
        let attr = CondAttr::new()?;

        errno::check_ret(unsafe {
            libc::pthread_condattr_setclock(&raw const attr.inner as *mut _, libc::CLOCK_MONOTONIC)
        })?;

        let c = Self {
            inner: UnsafeCell::new(unsafe { core::mem::zeroed() }),
        };

        errno::check_ret(unsafe { libc::pthread_cond_init(c.raw_ptr(), &attr.inner) })?;

        Ok(c)
    }

    /// Return a raw pointer to the inner condvar.
    fn raw_ptr(&self) -> *mut libc::pthread_cond_t {
        self.inner.get()
    }

    /// Wait on the condition variable.
    ///
    /// The guard must be locked. On return the guard is still locked
    /// (pthread_cond_wait atomically releases and reacquires the mutex).
    pub fn wait(&self, guard: &mut PosixMutexGuard<'_>) -> Result<()> {
        errno::check_ret(unsafe { libc::pthread_cond_wait(self.raw_ptr(), guard.raw_mutex_ptr()) })
    }

    /// Timed wait with absolute deadline.
    ///
    /// Returns `Error::Timeout` if the deadline expires before a signal.
    /// On any error, the guard is still locked.
    pub fn timed_wait(
        &self,
        guard: &mut PosixMutexGuard<'_>,
        abs_time: &libc::timespec,
    ) -> Result<()> {
        errno::check_ret(unsafe {
            libc::pthread_cond_timedwait(self.raw_ptr(), guard.raw_mutex_ptr(), abs_time)
        })
    }

    /// Wake one waiter.
    pub fn signal(&self) -> Result<()> {
        errno::check_ret(unsafe { libc::pthread_cond_signal(self.raw_ptr()) })
    }

    /// Wake all waiters.
    pub fn broadcast(&self) -> Result<()> {
        errno::check_ret(unsafe { libc::pthread_cond_broadcast(self.raw_ptr()) })
    }
}

impl Drop for PosixCondvar {
    fn drop(&mut self) {
        unsafe {
            libc::pthread_cond_destroy(self.raw_ptr());
        }
    }
}

unsafe impl Send for PosixCondvar {}
unsafe impl Sync for PosixCondvar {}

// ---------------------------------------------------------------------------
// Deadline helper — re-exported from sys::time
// ---------------------------------------------------------------------------

pub use crate::sys::time::abs_deadline;
