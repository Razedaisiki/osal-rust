//! Recursive POSIX mutex for system critical sections.
//!
//! This is separate from [`crate::sys::mutex::PosixMutex`] which uses
//! `PTHREAD_MUTEX_ERRORCHECK`. The [`System`] trait requires nested
//! critical sections, which need recursive locking semantics.
//!
//! # Safety
//!
//! This module provides a minimal wrapper sufficient for a process-local
//! critical-section mutex initialised once via `pthread_once`. It is NOT
//! a general-purpose mutex — there is no error mapping, no RAII guard,
//! and no timed lock.

use core::cell::UnsafeCell;
use core::mem::MaybeUninit;

/// A process-local recursive `pthread_mutex_t` for critical sections.
///
/// Callers must call [`init`](PosixRecursiveMutex::init) exactly once
/// (typically through `pthread_once`) before any [`lock`] / [`unlock`]
/// calls.
pub struct PosixRecursiveMutex {
    inner: UnsafeCell<MaybeUninit<libc::pthread_mutex_t>>,
}

// Safety: once initialised, pthread_mutex_lock / pthread_mutex_unlock
// are thread-safe. The mutex is never moved or dropped — it lives in a
// static.
unsafe impl Sync for PosixRecursiveMutex {}

impl PosixRecursiveMutex {
    /// Create an uninitialised mutex suitable for `static` placement.
    pub const fn uninit() -> Self {
        Self {
            inner: UnsafeCell::new(MaybeUninit::uninit()),
        }
    }

    /// One-time initialisation.
    ///
    /// # Safety
    ///
    /// Must be called exactly once. `pthread_once` is the canonical
    /// synchronisation mechanism.
    pub unsafe fn init(&self) {
        unsafe {
            let mut attr: libc::pthread_mutexattr_t = MaybeUninit::zeroed().assume_init();

            libc::pthread_mutexattr_init(&raw mut attr);
            libc::pthread_mutexattr_settype(&raw mut attr, libc::PTHREAD_MUTEX_RECURSIVE);
            libc::pthread_mutex_init((*self.inner.get()).as_mut_ptr(), &raw const attr);
            libc::pthread_mutexattr_destroy(&raw mut attr);
        }
    }

    /// Acquire the recursive lock. Blocks until acquired.
    pub fn lock(&self) {
        unsafe {
            libc::pthread_mutex_lock((*self.inner.get()).as_mut_ptr());
        }
    }

    /// Release one level of the recursive lock.
    pub fn unlock(&self) {
        unsafe {
            libc::pthread_mutex_unlock((*self.inner.get()).as_mut_ptr());
        }
    }
}
