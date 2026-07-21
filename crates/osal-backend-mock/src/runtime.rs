//! Mock backend runtime hooks.
//!
//! Owns a backend-local [`RuntimeLifecycle`] (ADR 0019) and orchestrates
//! mock service startup / shutdown.  Mock services are in-memory and
//! deterministic: the timer/clock runtime, fault injection state, and
//! any future backend-internal registries.
//!
//! # Initialization order
//!
//! ```text
//! begin_initialize()          // CAS Uninitialized → Initializing
//! → reset time runtime, clear transient state
//! → commit()                  // CAS Initializing → Running
//! ```
//!
//! # Shutdown order
//!
//! ```text
//! begin_shutdown()            // CAS Running,0 → ShuttingDown,0
//! → detach all timers, reset clock to zero
//! → commit()                  // CAS ShuttingDown → Uninitialized
//! ```
//!
//! On any error the transition guard drops and auto-rolls back.

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

/// Initialise all mock backend services.
///
/// Resets the virtual clock to zero and clears transient state
/// (fault injection triggers).  Idempotent: returns
/// [`Error::AlreadyInitialized`] if already [`Running`](RuntimeState::Running).
pub fn initialize() -> Result<()> {
    let transition = RUNTIME.begin_initialize()?;

    // Reset restartable backend-internal state.  The time runtime
    // increments its epoch, detaches any stale timers, and resets
    // the clock to zero.  This must not clear state held by live
    // user objects — the RuntimeLease check in begin_shutdown()
    // enforces that.
    crate::time_runtime::reset_runtime();

    transition.commit();
    Ok(())
}

/// Shut down all mock backend services.
///
/// Detaches all timers (via epoch bump) and resets the clock.
/// Returns [`Error::Busy`] while any [`RuntimeLease`] is alive.
pub fn shutdown() -> Result<()> {
    let transition = RUNTIME.begin_shutdown()?;

    // Detach all timers and reset the virtual clock.
    crate::time_runtime::reset_runtime();

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
#[allow(dead_code)] // used by managed-object constructors (P6B-6A)
pub(crate) fn acquire_object() -> Result<RuntimeLease<'static>> {
    RUNTIME.acquire()
}

/// Return the current active-object count (test-only).
#[cfg(feature = "testkit")]
pub fn active_objects() -> usize {
    RUNTIME.active_objects()
}
