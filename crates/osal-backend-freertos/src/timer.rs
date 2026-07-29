//! FreeRTOS timer — software timer backed by the ROUSSATL Timer Service.
//!
//! Follows the POSIX/Mock pattern: `Arc<InnerHandle { id, RuntimeLease }>`.
//! Callbacks live in the service registry, not in the handle.

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

struct FreeRtosTimerHandleInner {
    id: u64,
    /// Held for the lifetime of the logical timer object.  On drop,
    /// decrements the active-object count so `shutdown()` can proceed
    /// once all timers are released.
    _runtime: RuntimeLease<'static>,
}

impl Drop for FreeRtosTimerHandleInner {
    fn drop(&mut self) {
        let result = timer_service::deregister(self.id);
        debug_assert!(result.is_ok(), "live timer deregistration failed");
        // _runtime drops → active_objects decremented
    }
}

// ---------------------------------------------------------------------------
// FreeRtosTimer
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct FreeRtosTimer {
    inner: Arc<FreeRtosTimerHandleInner>,
}

impl FreeRtosTimer {
    pub fn new(
        _name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self> {
        // 1. Validate parameters (error precedence: params > runtime state).
        if period == Duration::ZERO {
            return Err(Error::InvalidParameter);
        }

        // 2. Acquire RuntimeLease.
        let runtime = crate::runtime::acquire_object()?;

        // 3. Register with the timer service.
        let id = timer_service::register(period, mode, callback)?;

        // 4. Construct Arc handle.
        Ok(Self {
            inner: Arc::new(FreeRtosTimerHandleInner {
                id,
                _runtime: runtime,
            }),
        })
    }
}

impl Timer for FreeRtosTimer {
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

pub struct FreeRtosTimerFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::TimerFactory for FreeRtosTimerFactory {
    type Timer = FreeRtosTimer;

    fn create_timer(
        &self,
        name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self::Timer> {
        FreeRtosTimer::new(name, period, mode, callback)
    }
}

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockFactory for FreeRtosTimerFactory {
    type Clock = crate::clock::FreeRtosClock;
}
