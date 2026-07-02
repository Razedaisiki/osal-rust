//! Mutex trait — recursive mutual exclusion lock.
//!
//! See [the behavior contract](../../../../docs/behavior-contract.md#9-mutex-contract)
//! for the full behavioral specification.

use core::ops::{Deref, DerefMut};

use crate::error::Result;
use crate::time::Timeout;

/// A recursive mutual exclusion lock protecting a value of type `T`.
///
/// # Recursive semantics
///
/// The owning task may lock the same mutex multiple times without
/// deadlocking. Each call to [`lock`](Mutex::lock) must be paired with
/// exactly one drop of the returned guard. The mutex is fully released
/// only when the last guard is dropped.
///
/// # ISR safety
///
/// Mutex operations are **not** ISR-safe. Use [`Semaphore`] or
/// [`Queue::isr_send`] for interrupt-context signaling.
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
    /// Provides `&mut T` access via [`DerefMut`]. Releases one level of
    /// the lock when dropped.
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
    /// | `NoWait`  | Return immediately; `Error::LockFailed` if the mutex is held by another task. |
    /// | `After(d)`| Block for at most `d`; `Error::Timeout` if the mutex is not acquired in time. |
    /// | `Forever` | Block until the mutex is acquired. |
    ///
    /// The owning task may call `lock` again without blocking
    /// (recursive lock). Each `lock` call produces a new guard;
    /// dropping that guard releases one recursion level.
    fn lock(&self, timeout: Timeout) -> Result<Self::Guard<'_>>;
}
