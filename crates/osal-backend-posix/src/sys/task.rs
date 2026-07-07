//! Thin wrapper around `pthread_create` / `pthread_join`.
//!
//! POSIX task creation and joining, isolated from the higher-level
//! [`crate::task`] completion-state logic.
//!
//! # Portability note
//!
//! This module uses plain `pthread_join`, not `pthread_timedjoin_np`.
//! Timeout join is implemented in [`crate::task`] through a backend
//! completion state + `pthread_cond_timedwait`.

use core::ffi::c_void;
use core::mem::MaybeUninit;

use osal_api::error::{Error, Result};

/// The C ABI entry point for a spawned task.
pub type RawTaskEntry = extern "C" fn(*mut c_void) -> *mut c_void;

/// An OS thread created via `pthread_create`.
pub struct PosixThread {
    tid: libc::pthread_t,
}

impl PosixThread {
    /// Spawn a new thread with the given entry and argument.
    ///
    /// Uses default attributes (joinable, system-default stack size
    /// and scheduling).
    pub fn spawn(entry: RawTaskEntry, arg: *mut c_void) -> Result<Self> {
        unsafe {
            let mut tid = MaybeUninit::<libc::pthread_t>::uninit();

            let rc = libc::pthread_create(tid.as_mut_ptr(), core::ptr::null(), entry, arg);

            if rc != 0 {
                return Err(Error::OutOfMemory);
            }

            Ok(Self {
                tid: tid.assume_init(),
            })
        }
    }

    /// Join the thread.
    ///
    /// Must be called at most once — `pthread_join` is not repeatable.
    /// The higher-level [`crate::task::PosixTask`] guards this with a
    /// completion-state machine.
    pub fn join(&self) -> Result<()> {
        let rc = unsafe { libc::pthread_join(self.tid, core::ptr::null_mut()) };

        if rc == 0 {
            Ok(())
        } else {
            Err(Error::Internal("pthread_join failed"))
        }
    }
}
