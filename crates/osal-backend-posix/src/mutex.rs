//! POSIX mutex implementation.
//!
//! Wraps the POSIX sys mutex (pthread `PTHREAD_MUTEX_ERRORCHECK`) with
//! typed data storage in an `Arc` for clone sharing, implementing
//! the [`Mutex<T>`] trait.

use alloc::sync::Arc;
use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;

use crate::sys::mutex::PosixMutex;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct PosixMutexInner<T> {
    raw: PosixMutex,
    data: UnsafeCell<T>,
}

// Safety: PosixMutex ensures mutual exclusion. The data is only
// accessed while the mutex is held.
unsafe impl<T: Send> Send for PosixMutexInner<T> {}
unsafe impl<T: Send> Sync for PosixMutexInner<T> {}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// A non-recursive mutex protecting a value of type `T`.
///
/// Uses `Arc<PosixMutexInner<T>>` internally; cloned handles share the
/// same backend resource (per ADR 0006).
pub struct PosixMutexImpl<T> {
    inner: Arc<PosixMutexInner<T>>,
}

impl<T> Clone for PosixMutexImpl<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> PosixMutexImpl<T> {
    /// Create a new mutex containing `value`.
    pub fn new(value: T) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(PosixMutexInner {
                raw: PosixMutex::new()?,
                data: UnsafeCell::new(value),
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

/// RAII guard for [`PosixMutexImpl`].
///
/// Provides `&T` / `&mut T` access via [`Deref`] / [`DerefMut`].
/// Unlocks the underlying pthread mutex on drop.
///
/// `!Send`: the guard must not be moved to another thread.
pub struct PosixMutexGuardImpl<'a, T> {
    inner: &'a PosixMutexInner<T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Deref for PosixMutexGuardImpl<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.inner.data.get() }
    }
}

impl<T> DerefMut for PosixMutexGuardImpl<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.inner.data.get() }
    }
}

impl<T> Drop for PosixMutexGuardImpl<'_, T> {
    fn drop(&mut self) {
        let _ = self.inner.raw.unlock();
    }
}

// ---------------------------------------------------------------------------
// Mutex trait
// ---------------------------------------------------------------------------

impl<T: 'static> Mutex<T> for PosixMutexImpl<T> {
    type Guard<'a>
        = PosixMutexGuardImpl<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn new(value: T) -> Result<Self> {
        Self::new(value)
    }

    fn lock(&self, timeout: Timeout) -> Result<Self::Guard<'_>> {
        match timeout {
            Timeout::NoWait => {
                self.inner.raw.try_lock()?;
            }
            Timeout::After(d) => {
                if d == Duration::ZERO {
                    match self.inner.raw.try_lock() {
                        Ok(()) => {}
                        Err(Error::LockFailed) => return Err(Error::Timeout),
                        Err(e) => return Err(e),
                    }
                } else {
                    self.inner.raw.timed_lock(d)?;
                }
            }
            Timeout::Forever => {
                self.inner.raw.lock()?;
            }
        }

        Ok(PosixMutexGuardImpl {
            inner: &self.inner,
            _not_send: PhantomData,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating POSIX mutexes.
pub struct PosixMutexFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::MutexFactory for PosixMutexFactory {
    type Mutex = PosixMutexImpl<u32>;

    fn create_mutex(&self, value: u32) -> Result<Self::Mutex> {
        PosixMutexImpl::new(value)
    }
}

// ---------------------------------------------------------------------------
// Unit tests (non-recursive)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_and_lock() {
        let m = PosixMutexImpl::new(42u32).unwrap();
        let guard = m.lock(Timeout::NoWait).unwrap();
        assert_eq!(*guard, 42);
    }

    #[test]
    fn guard_deref_mut() {
        let m = PosixMutexImpl::new(0u32).unwrap();
        {
            let mut guard = m.lock(Timeout::NoWait).unwrap();
            *guard += 1;
            assert_eq!(*guard, 1);
        }
        let guard = m.lock(Timeout::NoWait).unwrap();
        assert_eq!(*guard, 1);
    }

    #[test]
    fn lock_forever() {
        let m = PosixMutexImpl::new(100u32).unwrap();
        let guard = m.lock(Timeout::Forever).unwrap();
        assert_eq!(*guard, 100);
        drop(guard);
        let _g = m.lock(Timeout::Forever).unwrap();
    }

    #[test]
    fn no_second_guard() {
        let m = PosixMutexImpl::new(0u32).unwrap();
        let _g = m.lock(Timeout::NoWait).unwrap();
        // Second lock from same thread with ERRORCHECK — should fail.
        let result = m.lock(Timeout::NoWait);
        assert!(result.is_err());
    }

    #[test]
    fn clone_shares_state() {
        let m1 = PosixMutexImpl::new(0u32).unwrap();
        let m2 = m1.clone();
        {
            let mut guard = m1.lock(Timeout::NoWait).unwrap();
            *guard = 99;
        }
        let guard = m2.lock(Timeout::NoWait).unwrap();
        assert_eq!(*guard, 99);
    }

    #[test]
    fn drop_clone_keeps_alive() {
        let m1 = PosixMutexImpl::new(0u32).unwrap();
        let m2 = m1.clone();
        drop(m1);
        let guard = m2.lock(Timeout::NoWait).unwrap();
        assert_eq!(*guard, 0);
    }
}
