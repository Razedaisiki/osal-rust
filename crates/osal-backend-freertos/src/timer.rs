//! FreeRTOS timer — software timer backed by the OSAL Timer Service.
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

// Re-export for integration tests.
#[cfg(feature = "testkit")]
pub use crate::timer_service::{
    fixture_clear_wake_wait_ticks, fixture_completion_waiter_count,
    fixture_fail_next_registry_reserve, fixture_registry_len, fixture_reset_timer_hooks,
    fixture_set_next_timer_id, fixture_shutdown_waiting, fixture_wake_wait_count,
    fixture_wake_wait_max_ticks, fixture_worker_exists, flush_timer_service, timer_flush_request,
};

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

#[cfg(feature = "testkit")]
impl osal_testkit::factory::ClockControl for FreeRtosTimerFactory {
    fn advance_clock(&self, duration: Duration) {
        let caps = crate::runtime::capabilities_for_test()
            .expect("capabilities must be available for controlled tests");
        let ticks = osal_portable::tick_time::duration_to_ticks_ceil(duration, caps.tick_rate_hz)
            .expect("tick overflow in advance_clock");
        // Advance ticks first, then request flush (signals worker to
        // scan and acknowledge at a quiescent point).
        if ticks > 0 {
            osal_backend_freertos_sys::delay_ticks(ticks as u64);
        }
        let target = crate::timer_service::timer_flush_request();
        crate::timer_service::flush_timer_service(target);
    }
}
