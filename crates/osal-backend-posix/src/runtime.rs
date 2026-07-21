//! POSIX backend runtime hooks.
//!
//! Currently wraps the timer service lifecycle.  Future backend
//! services (event loop, IO) will also be started and stopped here.

use osal_api::error::Result;

/// Initialise all backend services.
pub fn initialize() -> Result<()> {
    crate::timer_service::initialize()
}

/// Shut down all backend services.
pub fn shutdown() -> Result<()> {
    crate::timer_service::shutdown()
}
