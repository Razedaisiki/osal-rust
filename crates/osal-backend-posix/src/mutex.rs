//! POSIX mutex implementation.
//!
//! Wraps [`PosixMutex`] (pthread `PTHREAD_MUTEX_RECURSIVE`) with
//! typed data storage, implementing the [`Mutex<T>`] trait.

use core::cell::UnsafeCell;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};
use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;

use crate::sys::condvar;
use crate::sys::mutex::PosixMutex;

// ---------------------------------------------------------------------------
// PosixMutexImpl
// ---------------------------------------------------------------------------

/// A recursive mutex protecting a value of type `T`.
///
/// Uses `pthread_mutex_t` (`PTHREAD_MUTEX_RECURSIVE`) for locking.
/// The data is stored alongside the mutex; access is gated by
/// successful lock acquisition.
pub struct PosixMutexImpl<T> {
    mutex: PosixMutex,
    data: UnsafeCell<T>,
}

// Safety: PosixMutex ensures mutual exclusion. The data in UnsafeCell
// is only accessed while the mutex is held.
unsafe impl<T: Send> Send for PosixMutexImpl<T> {}
unsafe impl<T: Send> Sync for PosixMutexImpl<T> {}

impl<T> PosixMutexImpl<T> {
    /// Create a new mutex containing `value`.
    pub fn new(value: T) -> Result<Self> {
        Ok(Self {
            mutex: PosixMutex::new()?,
            data: UnsafeCell::new(value),
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
    mutex: &'a PosixMutex,
    data: &'a UnsafeCell<T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Deref for PosixMutexGuardImpl<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // Safety: the guard only exists while the mutex is held.
        unsafe { &*self.data.get() }
    }
}

impl<T> DerefMut for PosixMutexGuardImpl<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // Safety: the guard provides exclusive access while the
        // mutex is held.
        unsafe { &mut *self.data.get() }
    }
}

impl<T> Drop for PosixMutexGuardImpl<'_, T> {
    fn drop(&mut self) {
        // Best-effort unlock. If the mutex was already destroyed
        // or is in an inconsistent state, we can't recover.
        let _ = self.mutex.unlock();
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
                self.mutex.try_lock()?;
            }
            Timeout::After(d) => {
                if d == Duration::ZERO {
                    // After(ZERO): try immediately, fail with Timeout
                    // if the lock is held by another thread.
                    match self.mutex.try_lock() {
                        Ok(()) => {}
                        Err(Error::LockFailed) => return Err(Error::Timeout),
                        Err(e) => return Err(e),
                    }
                } else {
                    let deadline = condvar::abs_deadline(d);
                    self.mutex.timed_lock(&deadline)?;
                }
            }
            Timeout::Forever => {
                self.mutex.lock()?;
            }
        }

        Ok(PosixMutexGuardImpl {
            mutex: &self.mutex,
            data: &self.data,
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
// Unit tests (no_std compatible)
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
    fn recursive_lock_two_levels() {
        let m = PosixMutexImpl::new(0u32).unwrap();
        let g1 = m.lock(Timeout::NoWait).unwrap();
        let g2 = m.lock(Timeout::NoWait).unwrap();
        assert_eq!(*g1, 0);
        assert_eq!(*g2, 0);
        drop(g2);
        drop(g1);
        let _g = m.lock(Timeout::NoWait).unwrap();
    }

    #[test]
    fn recursive_lock_three_levels() {
        let m = PosixMutexImpl::new(0u32).unwrap();
        let mut g1 = m.lock(Timeout::NoWait).unwrap();
        *g1 = 10;
        let mut g2 = m.lock(Timeout::NoWait).unwrap();
        assert_eq!(*g2, 10);
        *g2 = 20;
        let g3 = m.lock(Timeout::NoWait).unwrap();
        assert_eq!(*g3, 20);
        drop(g3);
        drop(g2);
        assert_eq!(*g1, 20);
        drop(g1);
    }
}
