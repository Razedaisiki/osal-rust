//! POSIX timer — software timer backed by `PosixTimerService`.

use alloc::sync::Arc;
use core::time::Duration;

use osal_api::error::{Error, Result};
use osal_api::traits::timer::{Timer, TimerCallback};
use osal_api::types::TimerMode;
use osal_shared::runtime::RuntimeLease;

use crate::timer_service;

// ---------------------------------------------------------------------------
// Handle inner — Drop deregisters from service, then releases RuntimeLease
// ---------------------------------------------------------------------------

struct PosixTimerHandleInner {
    id: u64,
    /// Held for the lifetime of the logical timer object.  On drop,
    /// decrements the active-object count so `shutdown()` can proceed
    /// once all timers are released (ADR 0019 §6).
    _runtime: RuntimeLease<'static>,
}

impl Drop for PosixTimerHandleInner {
    fn drop(&mut self) {
        let result = timer_service::deregister(self.id);
        debug_assert!(result.is_ok(), "live timer deregistration failed");
        // Fields drop after this Drop impl returns:
        // _runtime drops → active_objects decremented
    }
}

// ---------------------------------------------------------------------------
// PosixTimer
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct PosixTimer {
    inner: Arc<PosixTimerHandleInner>,
}

impl PosixTimer {
    pub fn new(
        _name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self> {
        // 1. Validate parameters first (error precedence: parameters >
        //    runtime state — ADR 0001, ADR 0019 §6).
        if period == Duration::ZERO {
            return Err(Error::InvalidParameter);
        }

        // 2. Acquire a runtime lease.  If the runtime is not Running,
        //    this returns NotInitialized.  The lease is held until the
        //    last clone drops.
        let runtime = crate::runtime::acquire_object()?;

        // 3. Register with the timer service.  If this fails, the
        //    local `runtime` lease drops — no active-object leak.
        let id = timer_service::register(period, mode, callback)?;

        // 4. Construct the inner handle.
        Ok(Self {
            inner: Arc::new(PosixTimerHandleInner {
                id,
                _runtime: runtime,
            }),
        })
    }
}

impl Timer for PosixTimer {
    fn new(name: &str, period: Duration, mode: TimerMode, callback: TimerCallback) -> Result<Self> {
        Self::new(name, period, mode, callback)
    }

    fn start(&self) -> Result<()> {
        timer_service::start(self.inner.id)
    }

    fn stop(&self) -> Result<()> {
        timer_service::stop(self.inner.id)
    }

    fn reset(&self) -> Result<()> {
        timer_service::reset(self.inner.id)
    }

    fn change_period(&self, new_period: Duration) -> Result<()> {
        if new_period == Duration::ZERO {
            return Err(Error::InvalidParameter);
        }
        timer_service::change_period(self.inner.id, new_period)
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

pub struct PosixTimerFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::TimerFactory for PosixTimerFactory {
    type Timer = PosixTimer;

    fn create_timer(
        &self,
        name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self::Timer> {
        PosixTimer::new(name, period, mode, callback)
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockFactory for PosixTimerFactory {
    type Clock = crate::clock::PosixClock;
}
