//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3B diagnostic — delay-only smoke (no capability, no snapshot).

#![no_std]

use osal_backend_freertos_sys as sys;
use sys::{DelayStatus, SchedulerState};

// ------------------------------------------------------------------
// Error codes — distinct failure modes for diagnostic isolation.
// ------------------------------------------------------------------

#[repr(i32)]
enum SmokeFailure {
    DelayOk              = 0,   // not a failure — returned as success
    ShimSchedulerBefore  = 20,
    DelayInvalid         = 50,
    DelayScheduler       = 51,
    DelayUnknown         = 52,
    ShimSchedulerAfter   = 53,
}

// ------------------------------------------------------------------
// Delay-only smoke — no capabilities, no tick_snapshot, no sentinels.
// ------------------------------------------------------------------

fn validate_delay_only() -> Result<(), SmokeFailure> {
    // 1. Scheduler must be Running before delay.
    if sys::scheduler_state() != SchedulerState::Running {
        return Err(SmokeFailure::ShimSchedulerBefore);
    }

    // 2. Call delay_ticks — this is the target under test.
    match sys::delay_ticks(2) {
        sys::DelayStatus::Ok => {}
        sys::DelayStatus::InvalidTicks => {
            return Err(SmokeFailure::DelayInvalid);
        }
        sys::DelayStatus::SchedulerNotRunning => {
            return Err(SmokeFailure::DelayScheduler);
        }
        sys::DelayStatus::Unknown(_) => {
            return Err(SmokeFailure::DelayUnknown);
        }
    }

    // 3. Scheduler must still be Running after delay.
    if sys::scheduler_state() != SchedulerState::Running {
        return Err(SmokeFailure::ShimSchedulerAfter);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Public entry point.
// ------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn osal_rust_smoke_entry() -> i32 {
    match validate_delay_only() {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

// ------------------------------------------------------------------
// Panic handler.
// ------------------------------------------------------------------

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
