//! Demo error type and OSAL error context helper.

use core::fmt;

use osal::prelude::Error;

/// Failure of a portable demo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemoError {
    /// An OSAL operation returned an error.
    Osal {
        /// Name of the OSAL operation that failed, e.g. `queue.send`.
        operation: &'static str,
        /// The error the backend returned.
        error: Error,
    },

    /// A demo invariant did not hold.
    Check(&'static str),

    /// A worker task reported a failure via its worker-error slot.
    Worker {
        /// Worker error code.
        code: u32,
    },
}

impl fmt::Display for DemoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DemoError::Osal { operation, error } => {
                write!(f, "OSAL operation `{operation}` failed: {error:?}")
            }
            DemoError::Check(reason) => write!(f, "check failed: {reason}"),
            DemoError::Worker { code } => write!(f, "worker failed: code={code}"),
        }
    }
}

/// Result alias for demo operations.
pub type DemoResult<T> = core::result::Result<T, DemoError>;

/// Attach the name of the failing OSAL operation to an OSAL error.
pub(crate) trait OsalResultExt<T> {
    fn demo_context(self, operation: &'static str) -> DemoResult<T>;
}

impl<T> OsalResultExt<T> for osal::prelude::Result<T> {
    fn demo_context(self, operation: &'static str) -> DemoResult<T> {
        self.map_err(|error| DemoError::Osal { operation, error })
    }
}
