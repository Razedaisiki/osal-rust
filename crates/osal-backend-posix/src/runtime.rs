//! POSIX backend runtime hooks.
//!
//! Owns a backend-local [`RuntimeLifecycle`] (ADR 0019) and orchestrates
//! backend service startup / shutdown.  Currently the only managed
//! backend service is [`crate::timer_service`]; future services (event
//! loop, IO) will also be started and stopped here.
//!
//! # Initialization order
//!
//! ```text
//! begin_initialize()          // CAS Uninitialized → Initializing
//! → timer_service::initialize()
//! → commit()                  // CAS Initializing → Running
//! ```
//!
//! # Shutdown order
//!
//! ```text
//! begin_shutdown()            // CAS Running,0 → ShuttingDown,0
//! → timer_service::shutdown()
//! → commit()                  // CAS ShuttingDown → Uninitialized
//! ```
//!
//! On any error the transition guard drops and auto-rolls back to the
//! previous state (ADR 0016).
//!
//! Internal services (timer worker, control blocks, TLS slots) do **not**
//! hold [`RuntimeLease`]s — only user-visible logical objects contribute
//! to `active_objects()`.

use osal_api::error::Result;
use osal_api::runtime::RuntimeState;
use osal_shared::runtime::{RuntimeLease, RuntimeLifecycle};

// ---------------------------------------------------------------------------
// Backend-local lifecycle instance (ADR 0019 §1)
// ---------------------------------------------------------------------------

static RUNTIME: RuntimeLifecycle = RuntimeLifecycle::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise all backend services.
///
/// Idempotent: returns [`Error::AlreadyInitialized`] if the runtime is
/// already [`Running`](RuntimeState::Running).  On failure the runtime
/// auto-rolls back to [`Uninitialized`](RuntimeState::Uninitialized).
pub fn initialize() -> Result<()> {
    let transition = RUNTIME.begin_initialize()?;

    // Initialise backend-internal services.  Any error here must leave
    // the timer service stopped — the transition guard's Drop will
    // roll back to Uninitialized.
    crate::timer_service::initialize()?;

    transition.commit();
    Ok(())
}

/// Shut down all backend services.
///
/// Returns [`Error::Busy`] while any [`RuntimeLease`] is still alive.
/// Returns [`Error::NotInitialized`] if the runtime is not
/// [`Running`](RuntimeState::Running).  On failure the runtime
/// auto-rolls back to Running.
pub fn shutdown() -> Result<()> {
    // This atomically enters ShuttingDown and rejects shutdown
    // while any RuntimeLease is still alive (ADR 0016, ADR 0019 §5).
    let transition = RUNTIME.begin_shutdown()?;

    // Any returned error must leave the timer service running —
    // the transition guard's Drop will roll back to Running.
    crate::timer_service::shutdown()?;

    transition.commit();
    Ok(())
}

/// Return the current runtime state.
pub fn state() -> RuntimeState {
    RUNTIME.state()
}

/// Acquire a [`RuntimeLease`] for a managed object.
///
/// Only succeeds while the runtime is [`Running`](RuntimeState::Running).
/// The lease is released on drop, decrementing the active-object count.
#[allow(dead_code)] // used by managed-object constructors (P6B-6A)
pub(crate) fn acquire_object() -> Result<RuntimeLease<'static>> {
    RUNTIME.acquire()
}

/// Return the current active-object count (test-only).
#[cfg(feature = "testkit")]
pub fn active_objects() -> usize {
    RUNTIME.active_objects()
}
