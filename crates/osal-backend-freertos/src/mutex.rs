//! FreeRTOS mutex implementation with RAII guard.
//!
//! Uses a native FreeRTOS mutex (`xSemaphoreCreateMutex`) for task-level
//! mutual exclusion with priority inheritance, and an internal
//! `spin::Mutex<T>` for safe `&mut T` access (ADR 0026 §2-3).
//!
//! # Lock order
//!
//! 1. Acquire native FreeRTOS mutex  (kernel-level exclusion)
//! 2. `try_lock()` internal `spin::Mutex<T>` (Rust borrow safety)
//! 3. Return guard
//!
//! # Guard Drop order
//!
//! 1. Drop value guard (release `&mut T` borrow)
//! 2. Release native FreeRTOS mutex (allow next task to acquire)
//!
//! The native mutex is **non-recursive**.  FreeRTOS's native mutex
//! already enforces this — a task that holds the mutex cannot acquire
//! it again.

use alloc::rc::Rc;
use alloc::sync::Arc;
use core::marker::PhantomData;
use core::ops::{Deref, DerefMut};

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;
use osal_api::traits::mutex::Mutex;
use osal_shared::runtime::RuntimeLease;

use crate::wait::{self, WaitOutcome};
use osal_backend_freertos_sys as sys;

// ---------------------------------------------------------------------------
// Inner state
// ---------------------------------------------------------------------------

struct MutexInner<T> {
    native: Option<sys::MutexHandle>,
    value: spin::Mutex<T>,
    /// Held for the lifetime of the mutex (ADR 0019 §6).
    _lease: RuntimeLease<'static>,
}

impl<T> Drop for MutexInner<T> {
    fn drop(&mut self) {
        if let Some(h) = self.native.take() {
            sys::mutex_delete(h);
        }
    }
}

// Safety: native mutex provides mutual exclusion; spin::Mutex is
// always acquired after the native mutex, so there is never real
// contention on it.
unsafe impl<T: Send> Send for MutexInner<T> {}
unsafe impl<T: Send> Sync for MutexInner<T> {}

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// A non-recursive mutex protecting a value of type `T`.
///
/// Uses `Arc<MutexInner<T>>` internally; cloned handles share the
/// same native FreeRTOS mutex (ADR 0006).
pub struct FreeRtosMutex<T> {
    inner: Arc<MutexInner<T>>,
}

impl<T> Clone for FreeRtosMutex<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

impl<T> FreeRtosMutex<T> {
    /// Create a new mutex containing `value`.
    ///
    /// Constructor order (ADR 0019 §6):
    /// 1. Acquire [`RuntimeLease`]
    /// 2. Create native mutex
    /// 3. Construct inner
    pub fn new(value: T) -> Result<Self> {
        let lease = crate::runtime::acquire_object()?;
        let handle = sys::mutex_create().ok_or(Error::OutOfMemory)?;

        Ok(Self {
            inner: Arc::new(MutexInner {
                native: Some(handle),
                value: spin::Mutex::new(value),
                _lease: lease,
            }),
        })
    }
}

// ---------------------------------------------------------------------------
// Guard
// ---------------------------------------------------------------------------

/// RAII guard for [`FreeRtosMutex`].
///
/// Provides `&T` / `&mut T` access via [`Deref`] / [`DerefMut`].
/// Releases the native FreeRTOS mutex on drop.
///
/// `!Send + !Sync`: the guard must not be moved to another task
/// (ADR 0026 §4).
pub struct FreeRtosMutexGuard<'a, T> {
    native: &'a sys::MutexHandle,
    value_guard: Option<spin::MutexGuard<'a, T>>,
    _not_send: PhantomData<Rc<()>>,
}

impl<T> Deref for FreeRtosMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.value_guard.as_ref().expect("guard already dropped")
    }
}

impl<T> DerefMut for FreeRtosMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        self.value_guard.as_mut().expect("guard already dropped")
    }
}

impl<T> Drop for FreeRtosMutexGuard<'_, T> {
    fn drop(&mut self) {
        // 1. Release the Rust value borrow first.
        drop(self.value_guard.take());

        // 2. Release the native mutex.
        if sys::mutex_give(self.native) != sys::GiveStatus::Ok {
            panic!(
                "FreeRTOS mutex give failed after guard release — \
                 invariant violation"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Mutex trait
// ---------------------------------------------------------------------------

impl<T: 'static> Mutex<T> for FreeRtosMutex<T> {
    type Guard<'a>
        = FreeRtosMutexGuard<'a, T>
    where
        Self: 'a,
        T: 'a;

    fn new(value: T) -> Result<Self> {
        Self::new(value)
    }

    fn lock(&self, timeout: Timeout) -> Result<Self::Guard<'_>> {
        let native = self
            .inner
            .native
            .as_ref()
            .expect("mutex already deleted");

        let outcome = wait::wait_native(timeout, |ticks| {
            sys::mutex_take(native, ticks)
        })?;

        match outcome {
            WaitOutcome::Acquired => {
                // Native mutex acquired.  The internal spin lock
                // MUST be available — if not, a backend invariant
                // has been violated.
                let value_guard = self.inner.value.try_lock().expect(
                    "FreeRTOS mutex invariant violated: \
                     spin::Mutex held after native mutex acquire",
                );

                Ok(FreeRtosMutexGuard {
                    native,
                    value_guard: Some(value_guard),
                    _not_send: PhantomData,
                })
            }
            WaitOutcome::Unavailable => {
                // Map to the correct error per ADR 0025 §5.
                match timeout {
                    Timeout::NoWait => Err(Error::LockFailed),
                    Timeout::After(_) | Timeout::Forever => Err(Error::Timeout),
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

/// Factory for creating FreeRTOS mutexes in contract tests.
#[cfg(feature = "testkit")]
pub struct FreeRtosMutexFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::MutexFactory for FreeRtosMutexFactory {
    type Mutex = FreeRtosMutex<u32>;

    fn create_mutex(&self, value: u32) -> Result<Self::Mutex> {
        FreeRtosMutex::new(value)
    }
}
