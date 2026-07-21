//! OSAL runtime lifecycle API.
//!
//! Before creating any managed OSAL objects (Queue, Mutex, Timer,
//! Task, etc.), the runtime must be explicitly initialized.  When
//! all objects have been dropped, the runtime can be shut down.
//!
//! # Example
//!
//! ```ignore
//! fn main() -> osal::Result<()> {
//!     osal::initialize()?;
//!
//!     {
//!         let queue = osal::Queue::new(8, 32)?;
//!         // ...
//!     } // queue (and all other objects) dropped here
//!
//!     osal::shutdown()?;
//!     Ok(())
//! }
//! ```
//!
//! The runtime does **not** auto-initialize.  Creating a managed
//! object before `initialize()` returns `Error::NotInitialized`.

use osal_api::error::Result;
use osal_api::runtime::RuntimeState;

/// Initialize the OSAL runtime and all backend services.
///
/// Must be called before creating any managed objects (Queue, Mutex,
/// Timer, Task, etc.).  Idempotent: returns
/// `Error::AlreadyInitialized` if already [`Running`](RuntimeState::Running).
///
/// On failure the runtime auto-rolls back to
/// [`Uninitialized`](RuntimeState::Uninitialized).
pub fn initialize() -> Result<()> {
    #[cfg(feature = "backend-mock")]
    {
        osal_backend_mock::runtime::initialize()
    }
    #[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
    {
        osal_backend_posix::runtime::initialize()
    }
}

/// Shut down the OSAL runtime and all backend services.
///
/// Returns `Error::Busy` while any managed objects are still alive.
/// Returns `Error::NotInitialized` if the runtime is not
/// [`Running`](RuntimeState::Running).
///
/// On failure the runtime auto-rolls back to
/// [`Running`](RuntimeState::Running).
pub fn shutdown() -> Result<()> {
    #[cfg(feature = "backend-mock")]
    {
        osal_backend_mock::runtime::shutdown()
    }
    #[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
    {
        osal_backend_posix::runtime::shutdown()
    }
}

/// Return the current runtime state.
pub fn runtime_state() -> RuntimeState {
    #[cfg(feature = "backend-mock")]
    {
        osal_backend_mock::runtime::state()
    }
    #[cfg(all(feature = "backend-posix", not(feature = "backend-mock")))]
    {
        osal_backend_posix::runtime::state()
    }
}
