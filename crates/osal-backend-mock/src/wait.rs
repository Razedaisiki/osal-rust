//! Deterministic wait outcome model for Mock primitives.
//!
//! Centralizes the timeout → result mapping used by MockQueue,
//! MockSemaphore, etc. Backends with real blocking (POSIX) implement
//! their own wait model; this module serves the Mock backend only.
//!
//! # Design
//!
//! Each operation (send, recv, acquire) produces a raw result from
//! the underlying data structure (`ByteQueue`, etc.). The wait model
//! maps `Timeout` + raw result → final result according to:
//!
//! | Timeout  | Raw result          | Final result      |
//! |----------|---------------------|-------------------|
//! | NoWait   | any                 | pass through      |
//! | After(_) | QueueFull/QueueEmpty | Error::Timeout    |
//! | After(_) | other               | pass through      |
//! | Forever  | QueueFull/QueueEmpty | Error::Unsupported |
//! | Forever  | other               | pass through      |

use osal_api::error::{Error, Result};
use osal_api::time::Timeout;

/// Outcome of an attempt with a given timeout.
pub enum WaitOutcome<T> {
    /// Operation succeeded immediately.
    Ready(T),
    /// Operation would block and timeout is After; return Timeout.
    TimedOut,
    /// Resource is closed.
    Closed,
    /// Operation would block and timeout is Forever; wait model
    /// not yet implemented.
    WouldBlock,
}

/// Map a raw `Result<T>` through the wait model for the given timeout.
///
/// `would_block_err` is the error variant that indicates the operation
/// would block (e.g. `Error::QueueFull` for send, `Error::QueueEmpty`
/// for recv/acquire).
pub fn apply_timeout<T>(
    timeout: Timeout,
    raw: Result<T>,
    would_block_err: Error,
) -> Result<T> {
    match timeout {
        Timeout::NoWait => raw,
        Timeout::After(_) => match raw {
            Err(e) if e == would_block_err => Err(Error::Timeout),
            other => other,
        },
        Timeout::Forever => match raw {
            Err(e) if e == would_block_err => Err(Error::Unsupported),
            other => other,
        },
    }
}
