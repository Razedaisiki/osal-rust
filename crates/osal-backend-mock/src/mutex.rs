//! Mock mutex implementation.
//!
//! Uses `Rc` for shared ownership and `UnsafeCell` + `Cell` for
//! interior mutability. Non-recursive: only one guard at a time.
//!
//! # Capability boundary
//!
//! - Core contracts: supported (creation, lock/unlock)
//! - Blocking contracts: deferred (single execution context;
//!   cross-task contention not simulated)
//!
//! # Timeout semantics
//!
//! - `Timeout::NoWait`: succeeds if unlocked; `LockFailed` if locked.
//! - `Timeout::After(d)`: same as NoWait when `d > 0`; `Timeout` when
//!   `d == 0` and locked.
//! - `Timeout::Forever`: always succeeds (uncontended).

use alloc::rc::Rc;
use core::cell::{Cell, UnsafeCell};
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;
use osal_shared::runtime::RuntimeLease;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct MockMutexInner<T> {
    /// The protected data.
    data: UnsafeCell<T>,
    /// `true` when the mutex is currently held.
    locked: Cell<bool>,
    /// Held for the lifetime of the mutex (ADR 0019 §6).
    _runtime: RuntimeLease<'static>,
}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// A mock mutex for contract testing.
///
/// Uses `Rc` internally; cloned handles share the same backend resource.
pub struct MockMutex<T> {
    inner: Rc<MockMutexInner<T>>,
}

impl<T> Clone for MockMutex<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Rc::clone(&self.inner),
        }
    }
}

impl<T> MockMutex<T> {
    /// Create a new mock mutex containing `value`.
    pub fn new(value: T) -> Result<Self> {
        let runtime = crate::runtime::acquire_object()?;
        Ok(Self {
            inner: Rc::new(MockMutexInner {
                data: UnsafeCell::new(value),
                locked: Cell::new(false),
                _runtime: runtime,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

/// RAII guard for [`MockMutex`].
///
/// Provides `&T` / `&mut T` access via [`Deref`] / [`DerefMut`].
/// Sets the lock to released on drop.
///
/// `!Send`: the guard must not be sent to another thread.
pub struct MockMutexGuard<'a, T> {
    inner: &'a MockMutexInner<T>,
    _not_send: PhantomData<*const ()>,
}

impl<T> Deref for MockMutexGuard<'_, T> {
    type Target = T;

    fn deref(&self) -> &T {
        // Safety: the guard only exists when locked is true.
        unsafe { &*self.inner.data.get() }
    }
}

impl<T> DerefMut for MockMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        // Safety: only one guard exists at a time (non-recursive).
        unsafe { &mut *self.inner.data.get() }
    }
}

impl<T> Drop for MockMutexGuard<'_, T> {
    fn drop(&mut self) {
        debug_assert!(self.inner.locked.get(), "guard dropped on unlocked mutex");
        self.inner.locked.set(false);
    }
}

// ---------------------------------------------------------------------------
// Mutex trait
// ---------------------------------------------------------------------------

impl<T: 'static> Mutex<T> for MockMutex<T> {
    type Guard<'a>
        = MockMutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn new(value: T) -> Result<Self> {
        Self::new(value)
    }

    fn lock(&self, timeout: Timeout) -> Result<Self::Guard<'_>> {
        // Non-recursive: reject all re-lock attempts while a guard
        // is alive. In the single-context mock, a held lock always
        // means same-context re-entry.
        if self.inner.locked.get() {
            return match timeout {
                Timeout::NoWait => Err(Error::LockFailed),
                Timeout::After(_) => Err(Error::Timeout),
                Timeout::Forever => Err(Error::LockFailed),
            };
        }

        self.inner.locked.set(true);
        Ok(MockMutexGuard {
            inner: &self.inner,
            _not_send: PhantomData,
        })
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating mock mutexes.
pub struct MockMutexFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::MutexFactory for MockMutexFactory {
    type Mutex = MockMutex<u32>;

    fn create_mutex(&self, value: u32) -> Result<Self::Mutex> {
        MockMutex::new(value)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use core::time::Duration;

    #[test]
    fn after_nonzero_relock_returns_timeout() {
        let _ = crate::runtime::initialize(); // tolerate AlreadyInitialized
        let m = MockMutex::new(0u32).unwrap();
        let _guard = m.lock(Timeout::NoWait).unwrap();
        let result = m.lock(Timeout::After(Duration::from_millis(10)));
        assert!(matches!(result, Err(Error::Timeout)));
    }

    #[test]
    fn forever_relock_returns_lock_failed() {
        let _ = crate::runtime::initialize();
        let m = MockMutex::new(0u32).unwrap();
        let _guard = m.lock(Timeout::NoWait).unwrap();
        let result = m.lock(Timeout::Forever);
        assert!(matches!(result, Err(Error::LockFailed)));
    }

    #[test]
    fn failed_relock_does_not_release_original_guard() {
        let _ = crate::runtime::initialize();
        let m = MockMutex::new(0u32).unwrap();
        let guard = m.lock(Timeout::NoWait).unwrap();

        assert!(m.lock(Timeout::NoWait).is_err());
        assert!(m.lock(Timeout::After(Duration::from_millis(1))).is_err());
        assert!(m.lock(Timeout::Forever).is_err());

        drop(guard);
        assert!(m.lock(Timeout::NoWait).is_ok());
    }
}
