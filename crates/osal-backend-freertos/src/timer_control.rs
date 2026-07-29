//! Timer service control block — process-lifetime, restart-safe.
//!
//! A single process-lifetime `static` holds a mutex-protected
//! `ServiceSlot`.  The actual `TimerService` (timers, registry,
//! worker task) is created and destroyed inside the slot; the
//! control block itself persists across runtime restarts.

use alloc::sync::Arc;

use osal_api::error::Result;

use crate::timer_service::TimerService;

// ---------------------------------------------------------------------------
// Service slot
// ---------------------------------------------------------------------------

pub(crate) enum ServiceSlot {
    Stopped,
    Running {
        service: Arc<TimerService>,
        /// Lazy — `None` until first `start()`/`reset()`.
        worker: Option<osal_backend_freertos_sys::InternalTaskHandle>,
    },
    Stopping,
}

// ---------------------------------------------------------------------------
// Control block
// ---------------------------------------------------------------------------

struct TimerServiceControl {
    slot: spin::Mutex<ServiceSlot>,
}

impl TimerServiceControl {
    const fn new() -> Self {
        Self {
            slot: spin::Mutex::new(ServiceSlot::Stopped),
        }
    }
}

// ---------------------------------------------------------------------------
// Global control block
// ---------------------------------------------------------------------------

static CONTROL: TimerServiceControl = TimerServiceControl::new();

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run a closure with mutable access to the service slot.
pub(crate) fn with_slot<R>(f: impl FnOnce(&mut ServiceSlot) -> Result<R>) -> Result<R> {
    let mut guard = CONTROL.slot.lock();
    f(&mut guard)
}
