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

/// Configuration for a new POSIX thread.
pub struct PosixThreadConfig {
    /// Requested minimum stack size in bytes.  The backend rounds up
    /// to the platform minimum (`PTHREAD_STACK_MIN`) when lower.
    pub stack_size: usize,
}

/// An OS thread created via `pthread_create`.
///
/// On drop, if the handle has not been joined or explicitly consumed,
/// the thread is detached so the OS can reclaim resources.
pub struct PosixThread {
    thread: Option<libc::pthread_t>,
}

impl PosixThread {
    /// Spawn a new thread with the given config, entry, and argument.
    pub fn spawn(
        config: &PosixThreadConfig,
        entry: RawTaskEntry,
        arg: *mut c_void,
    ) -> Result<Self> {
        let stack = if config.stack_size < libc::PTHREAD_STACK_MIN {
            libc::PTHREAD_STACK_MIN
        } else {
            config.stack_size
        };

        unsafe {
            let mut attr: libc::pthread_attr_t = MaybeUninit::zeroed().assume_init();

            let mut rc = libc::pthread_attr_init(&raw mut attr);
            if rc != 0 {
                return Err(Error::Internal("pthread_attr_init failed"));
            }

            rc = libc::pthread_attr_setstacksize(&raw mut attr, stack);
            if rc != 0 {
                libc::pthread_attr_destroy(&raw mut attr);
                return Err(Error::InvalidParameter);
            }

            let mut tid = MaybeUninit::<libc::pthread_t>::uninit();
            rc = libc::pthread_create(tid.as_mut_ptr(), &attr, entry, arg);
            libc::pthread_attr_destroy(&raw mut attr);

            if rc != 0 {
                return Err(Error::OutOfMemory);
            }

            Ok(Self {
                thread: Some(tid.assume_init()),
            })
        }
    }

    /// Join the thread, consuming the handle on success.
    ///
    /// On failure the handle is preserved so the caller can retry.
    pub fn try_join(&mut self) -> Result<()> {
        let tid = self.thread.ok_or(Error::Internal("already joined"))?;
        let rc = unsafe { libc::pthread_join(tid, core::ptr::null_mut()) };

        if rc != 0 {
            return Err(Error::Internal("pthread_join failed"));
        }

        self.thread = None;
        Ok(())
    }
}

impl Drop for PosixThread {
    fn drop(&mut self) {
        if let Some(tid) = self.thread.take() {
            let rc = unsafe { libc::pthread_detach(tid) };
            debug_assert_eq!(rc, 0, "pthread_detach on owned handle failed");
        }
    }
}
