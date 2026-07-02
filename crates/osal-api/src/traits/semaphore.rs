//! Semaphore traits — counting and binary semaphores.
//!
//! See [the behavior contract](../../../../docs/behavior-contract.md#10-semaphore-contract)
//! for the full behavioral specification.

use crate::error::Result;
use crate::time::Timeout;

// ---------------------------------------------------------------------------
// CountingSemaphore
// ---------------------------------------------------------------------------

/// A counting semaphore for resource management and task signaling.
///
/// Maintains an internal counter between 0 and `max_count`. Tasks call
/// [`acquire`](CountingSemaphore::acquire) to decrement the counter
/// (blocking if it is zero) and [`release`](CountingSemaphore::release)
/// to increment it (waking one blocked acquirer).
///
/// # ISR semantics
///
/// [`isr_acquire`](CountingSemaphore::isr_acquire) and
/// [`isr_release`](CountingSemaphore::isr_release) are non-blocking
/// variants safe to call from interrupt context.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// // Resource pool: at most 3 concurrent accesses
/// let pool = CountingSemaphore::new(3, 3)?;
/// pool.acquire(Timeout::Forever)?;
/// // ... use one resource slot ...
/// pool.release()?;
/// ```
pub trait CountingSemaphore: Sized {
    /// Create a semaphore with the given maximum and initial count.
    ///
    /// Returns `Error::InvalidParameter` if `initial > max` or
    /// `max == 0`. Returns `Error::OutOfMemory` on allocation failure.
    fn new(max_count: u32, initial_count: u32) -> Result<Self>;

    /// Decrement the counter, blocking according to `timeout`.
    ///
    /// | `timeout` | Behavior |
    /// |-----------|----------|
    /// | `NoWait`  | Return immediately; `Error::Timeout` if count is zero. |
    /// | `After(d)`| Block for at most `d`; `Error::Timeout` if no release occurs in time. |
    /// | `Forever` | Block until a release occurs. |
    fn acquire(&self, timeout: Timeout) -> Result<()>;

    /// Increment the counter, waking one blocked acquirer if any.
    ///
    /// Returns `Error::InvalidParameter` if `count` is already at
    /// `max_count` (the semaphore is full).
    fn release(&self) -> Result<()>;

    /// Non-blocking acquire, safe to call from ISR context.
    fn isr_acquire(&self) -> Result<()>;

    /// Non-blocking release, safe to call from ISR context.
    fn isr_release(&self) -> Result<()>;

    /// Return the maximum count configured at creation.
    fn max_count(&self) -> u32;

    /// Return the current count (a snapshot; may be stale immediately).
    fn count(&self) -> u32;
}

// ---------------------------------------------------------------------------
// BinarySemaphore
// ---------------------------------------------------------------------------

/// A binary semaphore for task-to-task signaling.
///
/// Equivalent to a [`CountingSemaphore`] with `max_count = 1`. Starts
/// with count 0 (unsignaled). A single [`release`](BinarySemaphore::release)
/// sets the semaphore to the signaled state; an
/// [`acquire`](BinarySemaphore::acquire) resets it to unsignaled.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// let ready = BinarySemaphore::new()?;
///
/// // Task A: wait for signal
/// ready.acquire(Timeout::Forever)?;
///
/// // Task B: send signal
/// ready.release()?;
/// ```
pub trait BinarySemaphore: Sized {
    /// Create a binary semaphore with count 0.
    fn new() -> Result<Self>;

    /// Decrement the counter (must be 1), blocking according to
    /// `timeout`. See [`CountingSemaphore::acquire`] for semantics.
    fn acquire(&self, timeout: Timeout) -> Result<()>;

    /// Increment the counter to 1 if currently 0. Returns
    /// `Error::InvalidParameter` if already signaled.
    fn release(&self) -> Result<()>;

    /// Return `true` if the semaphore is currently signaled (count == 1).
    fn is_acquired(&self) -> bool;

    /// Non-blocking acquire, safe to call from ISR context.
    fn isr_acquire(&self) -> Result<()>;

    /// Non-blocking release, safe to call from ISR context.
    fn isr_release(&self) -> Result<()>;
}
