//! Error types for OSAL operations.
//!
//! All error variants are unit variants (no attached data other than
//! `Internal`), keeping the error type `Send + Sync + 'static` without
//! lifetime complexity.

/// Error conditions that can occur during OSAL operations.
///
/// # Design
///
/// Unlike C-style error codes or lifetime-parameterized errors, this type
/// uses simple unit variants. The `Internal` variant carries a static
/// string for debugging unexpected failures.
///
/// # Examples
///
/// ```ignore
/// use osal_api::error::{Error, Result};
///
/// fn do_work() -> Result<u32> {
///     Err(Error::Timeout)
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Insufficient memory to complete the operation.
    OutOfMemory,
    /// The operation timed out before completing.
    Timeout,
    /// Queue is full and cannot accept more items.
    QueueFull,
    /// Queue is empty, nothing to receive.
    QueueEmpty,
    /// Queue has been closed and cannot accept new operations.
    QueueClosed,
    /// The specified message size does not match the queue's message size.
    InvalidMessageSize,
    /// Failed to acquire the lock.
    LockFailed,
    /// Count or capacity overflow — a resource has reached its maximum.
    ///
    /// For example, signaling a semaphore that is already at `max_count`.
    Overflow,
    /// The requested resource was not found.
    NotFound,
    /// An invalid parameter was provided.
    InvalidParameter,
    /// The resource has already been initialized.
    AlreadyInitialized,
    /// The resource has not been initialized.
    NotInitialized,
    /// The operation cannot proceed because the runtime or resource is
    /// currently in use or a lifecycle transition is in progress.
    Busy,
    /// The operation is not supported on this backend.
    Unsupported,
    /// A general internal error occurred.
    Internal(&'static str),
}

/// Convenience type alias for `core::result::Result` with the OSAL `Error` type.
pub type Result<T> = core::result::Result<T, Error>;
