//! Factory trait for backend runtime lifecycle operations.

use osal_api::error::Result;
use osal_api::runtime::RuntimeState;

/// Factory for backend runtime lifecycle.
///
/// Each backend owns its own [`RuntimeLifecycle`] instance (ADR 0019).
/// Implementations delegate to the backend-local `static RUNTIME`.
///
/// Methods are associated functions (no `&self`) because the runtime is
/// process-global — there is exactly one active runtime per backend
/// per process.
pub trait RuntimeFactory {
    /// Initialise the backend runtime.
    fn initialize() -> Result<()>;

    /// Shut down the backend runtime.
    fn shutdown() -> Result<()>;

    /// Return the current runtime state.
    fn state() -> RuntimeState;
}
