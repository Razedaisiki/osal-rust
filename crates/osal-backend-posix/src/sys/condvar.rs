//! Thin wrapper around `pthread_cond_t`.

use core::time::Duration;

use osal_api::error::Result;

use crate::sys::errno;
use crate::sys::mutex::PosixMutex;

/// Wrapper around `pthread_cond_t`.
pub struct PosixCondvar {
    inner: libc::pthread_cond_t,
}

impl PosixCondvar {
    /// Create and initialize a new condition variable.
    pub fn new() -> Result<Self> {
        let mut c = Self {
            inner: unsafe { core::mem::zeroed() },
        };
        errno::check_ret(unsafe { libc::pthread_cond_init(&mut c.inner, core::ptr::null()) })?;
        Ok(c)
    }

    /// Wait on the condition variable. The mutex must be locked.
    pub fn wait(&self, mutex: &PosixMutex) -> Result<()> {
        errno::check_ret(unsafe {
            libc::pthread_cond_wait(
                &raw const self.inner as *mut _,
                &raw const mutex.inner as *mut _,
            )
        })
    }

    /// Timed wait. Returns `Ok(())` if signaled, or the error from
    /// the underlying call (typically `Error::Timeout` on timeout).
    pub fn timed_wait(&self, mutex: &PosixMutex, abs_time: &libc::timespec) -> Result<()> {
        errno::check_ret(unsafe {
            libc::pthread_cond_timedwait(
                &raw const self.inner as *mut _,
                &raw const mutex.inner as *mut _,
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

/// Compute an absolute deadline from a relative duration.
///
/// Returns a `timespec` representing `now + timeout`.
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
