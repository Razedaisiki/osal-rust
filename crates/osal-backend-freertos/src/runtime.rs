//! FreeRTOS backend runtime hooks.
//!
//! Owns a backend-local [`RuntimeLifecycle`] (ADR 0019) and probes
//! FreeRTOS kernel capabilities at init time.  Does **not** start or
//! stop the FreeRTOS scheduler (ADR 0020 §1).

use osal_api::error::Result;
use osal_api::runtime::RuntimeState;
use osal_shared::runtime::{RuntimeLease, RuntimeLifecycle};

use osal_backend_freertos_sys::KernelCapabilities;

// ---------------------------------------------------------------------------
// Backend-local lifecycle instance (ADR 0019 §1)
// ---------------------------------------------------------------------------

static RUNTIME: RuntimeLifecycle = RuntimeLifecycle::new();

/// Cached kernel capabilities, populated during [`initialize`].
static mut CAPABILITIES: Option<KernelCapabilities> = None;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the FreeRTOS backend.
///
/// Probes kernel capabilities from the C shim and caches them.
/// Does **not** call `vTaskStartScheduler()`.
///
/// Returns [`Error::AlreadyInitialized`](osal_api::error::Error::AlreadyInitialized)
/// if the runtime is already [`Running`](RuntimeState::Running).
pub fn initialize() -> Result<()> {
    let transition = RUNTIME.begin_initialize()?;

    // Probe and cache kernel capabilities.
    let caps = osal_backend_freertos_sys::probe_capabilities();
    // SAFETY: single-threaded at init time (scheduler not yet started
    // or this is called from a single init path).
    unsafe {
        CAPABILITIES = Some(caps);
    }

    transition.commit();
    Ok(())
}

/// Shut down the FreeRTOS backend.
///
/// Returns [`Error::Busy`](osal_api::error::Error::Busy) while any
/// [`RuntimeLease`] is alive.  Does **not** call
/// `vTaskEndScheduler()`.
pub fn shutdown() -> Result<()> {
    let transition = RUNTIME.begin_shutdown()?;

    // Clear cached capabilities so re-initialisation re-probes.
    // SAFETY: shutdown requires no active leases; no concurrent
    // access to CAPABILITIES.
    unsafe {
        CAPABILITIES = None;
    }

    transition.commit();
    Ok(())
}

/// Return the current runtime state.
pub fn state() -> RuntimeState {
    RUNTIME.state()
}

/// Acquire a [`RuntimeLease`] for a managed object.
#[allow(dead_code)] // used by future primitive constructors (P7B+)
pub(crate) fn acquire_object() -> Result<RuntimeLease<'static>> {
    RUNTIME.acquire()
}

/// Return the current active-object count (test-only).
#[cfg(feature = "testkit")]
pub fn active_objects() -> usize {
    RUNTIME.active_objects()
}

/// Return a copy of the cached kernel capabilities.
///
/// Returns `None` if called before [`initialize`].
pub(crate) fn capabilities() -> Option<KernelCapabilities> {
    // SAFETY: CAPABILITIES is written once during initialize()
    // (single-threaded) and cleared during shutdown (after all
    // leases are released).  Reads during normal operation see
    // either None (uninitialised) or the cached value.
    unsafe { CAPABILITIES }
}
