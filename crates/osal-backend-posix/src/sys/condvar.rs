//! Thin wrapper around `pthread_cond_t`.

use core::time::Duration;

use osal_api::error::Result;

use crate::sys::errno;
use crate::sys::mutex::PosixMutexGuard;

// ---------------------------------------------------------------------------
// CondAttr — RAII wrapper for pthread_condattr_t
// ---------------------------------------------------------------------------

struct CondAttr {
    inner: libc::pthread_condattr_t,
}

impl CondAttr {
    fn new() -> Result<Self> {
        let mut attr = Self {
            inner: unsafe { core::mem::zeroed() },
        };
        errno::check_ret(unsafe { libc::pthread_condattr_init(&mut attr.inner) })?;
        Ok(attr)
    }
}

impl Drop for CondAttr {
    fn drop(&mut self) {
        unsafe {
            libc::pthread_condattr_destroy(&mut self.inner);
        }
    }
}

// ---------------------------------------------------------------------------
// PosixCondvar
// ---------------------------------------------------------------------------

/// Wrapper around `pthread_cond_t`.
///
/// Uses `CLOCK_MONOTONIC` so that `timed_wait` deadlines are
/// consistent with `clock_gettime(CLOCK_MONOTONIC)`.
pub struct PosixCondvar {
    inner: libc::pthread_cond_t,
}

impl PosixCondvar {
    /// Create and initialize a new condition variable with
    /// `CLOCK_MONOTONIC`.
    pub fn new() -> Result<Self> {
        let mut attr = CondAttr::new()?;

        errno::check_ret(unsafe {
            libc::pthread_condattr_setclock(&mut attr.inner, libc::CLOCK_MONOTONIC)
        })?;

        let mut c = Self {
            inner: unsafe { core::mem::zeroed() },
        };

        errno::check_ret(unsafe { libc::pthread_cond_init(&mut c.inner, &attr.inner) })?;

        Ok(c)
    }

    /// Wait on the condition variable.
    ///
    /// The guard must be locked. On return the guard is still locked
    /// (pthread_cond_wait atomically releases and reacquires the mutex).
    pub fn wait(&self, guard: &mut PosixMutexGuard<'_>) -> Result<()> {
        errno::check_ret(unsafe {
            libc::pthread_cond_wait(
                &raw const self.inner as *mut _,
                guard.raw_mutex_ptr(),
            )
        })
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
            libc::pthread_cond_timedwait(
                &raw const self.inner as *mut _,
                guard.raw_mutex_ptr(),
                abs_time,
            )
        })
    }

    /// Wake one waiter.
    pub fn signal(&self) -> Result<()> {
        errno::check_ret(unsafe { libc::pthread_cond_signal(&raw const self.inner as *mut _) })
    }

    /// Wake all waiters.
    pub fn broadcast(&self) -> Result<()> {
        errno::check_ret(unsafe { libc::pthread_cond_broadcast(&raw const self.inner as *mut _) })
    }
}

impl Drop for PosixCondvar {
    fn drop(&mut self) {
        unsafe {
            libc::pthread_cond_destroy(&mut self.inner);
        }
    }
}

unsafe impl Send for PosixCondvar {}
unsafe impl Sync for PosixCondvar {}

// ---------------------------------------------------------------------------
// Deadline helper
// ---------------------------------------------------------------------------

/// Compute an absolute deadline from a relative duration.
///
/// Returns a `timespec` representing `now + timeout` using
/// `CLOCK_MONOTONIC`, consistent with the condvar clock.
pub fn abs_deadline(timeout: Duration) -> libc::timespec {
    let mut ts = crate::sys::time::monotonic_now_raw();
    let sec = timeout.as_secs() as libc::time_t;
    let nsec = timeout.subsec_nanos() as libc::c_long;
    ts.tv_sec += sec;
    ts.tv_nsec += nsec;
    if ts.tv_nsec >= 1_000_000_000 {
        ts.tv_sec += 1;
        ts.tv_nsec -= 1_000_000_000;
    }
    ts
}
