//! FreeRTOS clock — tick-based monotonic time via coherent kernel snapshots.
//!
//! Implements the OSAL [`Clock`] trait using `vTaskSetTimeOutState()`
//! (ADR 0023).  The tick counter may be 16-, 32-, or 64-bit depending
//! on the port configuration; the backend uses the kernel capability
//! probe to handle all widths uniformly.
//!
//! # Scheduler dependency
//!
//! - `now()` panics if the runtime has not been initialised (no cached
//!   capabilities) or the kernel returns an unexpected tick width.
//! - `delay(Duration::ZERO)` returns immediately in any scheduler state.
//! - `delay(d > 0)` requires the scheduler to be `Running` and the
//!   caller to be a FreeRTOS task.  It panics otherwise.

use core::time::Duration;

use osal_api::traits::clock::Clock;
use osal_portable::tick_time::{self, TickConfig, TickSnapshot};

use crate::runtime;
use osal_backend_freertos_sys as sys;

// ---------------------------------------------------------------------------
// Public type
// ---------------------------------------------------------------------------

/// FreeRTOS clock — monotonic tick counter backed by the kernel.
///
/// All methods are associated functions (no `self`).  The clock is a
/// process-wide singleton; the struct cannot be instantiated.
pub struct FreeRtosClock;

impl Clock for FreeRtosClock {
    fn now() -> Duration {
        let caps = runtime::capabilities().expect(
            "FreeRtosClock::now requires osal::initialize() to be called first",
        );

        let snap = sys::tick_snapshot();

        let config = TickConfig {
            rate_hz: caps.tick_rate_hz,
            bits: caps.tick_bits,
        };

        // Convert the coherent snapshot to a Duration.  Saturates at
        // Duration::MAX rather than wrapping (ADR 0023 §2).
        tick_time::snapshot_to_duration(
            TickSnapshot {
                overflow_count: snap.overflow_count,
                tick_count: snap.tick_count,
            },
            config,
        )
        .expect("tick snapshot → Duration conversion failed (bad capability data)")
    }

    fn delay(duration: Duration) {
        // Zero delay — return immediately (ADR 0023 §6).
        if duration.is_zero() {
            return;
        }

        // Non-zero delay requires a Running scheduler (ADR 0023 §6).
        let state = sys::scheduler_state();
        if state != sys::SchedulerState::Running {
            panic!(
                "FreeRtosClock::delay requires a running scheduler \
                 and task context (scheduler state: {state:?})"
            );
        }

        let caps = runtime::capabilities().expect(
            "FreeRtosClock::delay requires osal::initialize() to be called first",
        );

        // Ceiling conversion: non-zero duration → at least 1 tick.
        // Then add one guard tick (ADR 0023 §4).
        let ceil_ticks = tick_time::duration_to_ticks_ceil(duration, caps.tick_rate_hz)
            .expect("duration → ticks conversion overflowed");

        let total_ticks = ceil_ticks
            .checked_add(1)
            .expect("guard tick overflowed u128");

        // Chunk long delays to fit within portMAX_DELAY - 1 (ADR 0023 §5).
        let max_chunk = sys::max_finite_delay_ticks() as u128;
        let mut remaining = total_ticks;

        while remaining > 0 {
            let chunk = remaining.min(max_chunk);

            let status = sys::delay_ticks(chunk as u64);
            if status != sys::DelayStatus::Ok {
                panic!(
                    "FreeRtosClock::delay failed: {status:?} \
                     (chunk={chunk}, remaining={remaining})"
                );
            }

            remaining -= chunk;
        }
    }
}
