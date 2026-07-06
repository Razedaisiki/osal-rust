//! Mutex trait — mutual exclusion lock.
//!
//! See [the behavior contract](../../../../docs/behavior-contract.md#9-mutex-contract)
//! for the full behavioral specification.

use core::ops::{Deref, DerefMut};

use crate::error::Result;
use crate::time::Timeout;

/// A mutual exclusion lock protecting a value of type `T`.
///
/// # Non-recursive
///
/// The mutex is **non-recursive**. The owning task cannot lock the same
/// mutex again while a guard is still alive. Attempting to do so returns
/// `Error::LockFailed` (for `NoWait`) or `Error::Timeout` (for `After`).
/// Recursive locking is deferred to a future `RecursiveMutex` type.
///
/// # ISR safety
///
/// Mutex operations are **not** ISR-safe. Use [`Semaphore`] or future
/// ISR extension traits for interrupt-context signaling.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// let counter = Mutex::new(0u32)?;
/// {
///     let mut guard = counter.lock(Timeout::Forever)?;
///     *guard += 1;
/// } // lock released here
/// ```
pub trait Mutex<T>: Sized {
    /// The guard type returned by a successful lock.
    ///
    /// Provides `&mut T` access via [`DerefMut`]. Releases the lock
    /// when dropped.
    type Guard<'a>: Deref<Target = T> + DerefMut<Target = T>
    where
        Self: 'a,
        T: 'a;

    /// Create a new mutex containing `value`.
    ///
    /// Returns `Error::OutOfMemory` if the underlying OS resource
    /// cannot be allocated.
    fn new(value: T) -> Result<Self>;

    /// Acquire the lock, blocking according to `timeout`.
    ///
    /// | `timeout` | Behavior |
    /// |-----------|----------|
    /// | `NoWait`  | Return immediately; `Error::LockFailed` if the mutex is held. |
    /// | `After(d)`| Block for at most `d`; `Error::Timeout` if not acquired in time. |
    /// | `Forever` | Block until the mutex is acquired. |
    ///
    /// Only one guard may exist at a time. Attempting to lock while
    /// already holding a guard returns `Error::LockFailed`.
    fn lock(&self, timeout: Timeout) -> Result<Self::Guard<'_>>;
}
