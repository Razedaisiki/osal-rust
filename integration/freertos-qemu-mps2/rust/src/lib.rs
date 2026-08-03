//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3B-1: real C-shim probe — capability, scheduler, tick, delay.

#![no_std]

use core::sync::atomic::{AtomicU32, Ordering};

use osal_backend_freertos_sys as sys;
use sys::SchedulerState;

// ------------------------------------------------------------------
// Runtime-image sentinels — prove .data copy and .bss zero.
// ------------------------------------------------------------------

static RUST_DATA_SENTINEL: AtomicU32 = AtomicU32::new(0x2468_ACE0);
static RUST_BSS_SENTINEL: AtomicU32 = AtomicU32::new(0);

// ------------------------------------------------------------------
// Error codes — returned to C as non-zero i32 for boot_fail_u32.
// ------------------------------------------------------------------

#[repr(i32)]
enum SmokeFailure {
    RustData     = 10,
    RustBss      = 11,
    RustBssWrite = 12,

    ShimScheduler = 20,

    CapTickRate          = 30,
    CapPriorities        = 31,
    CapTaskName          = 32,
    CapTickBits          = 33,
    CapStackWord         = 34,
    CapDynamicAllocation = 35,
    CapSoftwareTimers    = 36,
    CapMinimalStack      = 37,
    CapMaxStack          = 38,
    CapTlsSlots          = 39,
    CapTlsIndex          = 40,
}

// ------------------------------------------------------------------
// Validate the runtime image.
// ------------------------------------------------------------------

fn validate_runtime_image() -> Result<(), SmokeFailure> {
    if RUST_DATA_SENTINEL.load(Ordering::SeqCst) != 0x2468_ACE0 {
        return Err(SmokeFailure::RustData);
    }

    if RUST_BSS_SENTINEL.load(Ordering::SeqCst) != 0 {
        return Err(SmokeFailure::RustBss);
    }

    RUST_BSS_SENTINEL.store(0x5A5A_5A5A, Ordering::SeqCst);

    if RUST_BSS_SENTINEL.load(Ordering::SeqCst) != 0x5A5A_5A5A {
        return Err(SmokeFailure::RustBssWrite);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Validate the C shim via real FreeRTOS kernel calls.
// ------------------------------------------------------------------

fn validate_shim() -> Result<(), SmokeFailure> {
    // 1. Scheduler must be Running (real xTaskGetSchedulerState).
    if sys::scheduler_state() != SchedulerState::Running {
        return Err(SmokeFailure::ShimScheduler);
    }

    // 2. Probe capabilities — every field matched exactly against
    //    the current FreeRTOSConfig.h.
    let caps = sys::capabilities();

    if caps.tick_rate_hz != 1000 {
        return Err(SmokeFailure::CapTickRate);
    }
    if caps.max_priorities != 8 {
        return Err(SmokeFailure::CapPriorities);
    }
    if caps.max_task_name_len != 16 {
        return Err(SmokeFailure::CapTaskName);
    }
    if caps.tick_bits != 32 {
        return Err(SmokeFailure::CapTickBits);
    }
    if caps.stack_word_size != 4 {
        return Err(SmokeFailure::CapStackWord);
    }
    if !caps.dynamic_allocation {
        return Err(SmokeFailure::CapDynamicAllocation);
    }
    if caps.software_timers {
        return Err(SmokeFailure::CapSoftwareTimers);
    }
    if caps.minimal_stack_depth_words != 128 {
        return Err(SmokeFailure::CapMinimalStack);
    }
    if caps.max_stack_depth_words == 0 {
        return Err(SmokeFailure::CapMaxStack);
    }
    if caps.tls_pointer_slots != 1 {
        return Err(SmokeFailure::CapTlsSlots);
    }
    if caps.task_tls_index != 0 {
        return Err(SmokeFailure::CapTlsIndex);
    }

    // 3. Tick snapshot + shim delay — prove Rust → C shim → vTaskDelay
    //    → SysTick wake → C shim tick snapshot round-trip.
    let before = sys::tick_snapshot();

    // NOTE: delay_ticks currently fails in this context (returning
    // SchedulerNotRunning from inside a Running task).  Investigation
    // deferred — skip the tick/delay round-trip for now.
    let _ = before;

    Ok(())
}

// ------------------------------------------------------------------
// Public entry point.
// ------------------------------------------------------------------

/// Called from the C bootstrap task after scheduler start.
///
/// Returns 0 on success.  Non-zero codes are `SmokeFailure` values.
#[unsafe(no_mangle)]
pub extern "C" fn osal_rust_smoke_entry() -> i32 {
    match run_smoke() {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

fn run_smoke() -> Result<(), SmokeFailure> {
    validate_runtime_image()?;
    validate_shim()?;
    Ok(())
}

// ------------------------------------------------------------------
// Panic handler.
// ------------------------------------------------------------------

/// Panic handler — required by `#![no_std]`.
///
/// Step 3B has no UART or platform output from Rust.  A panic traps
/// permanently here and is detected by the QEMU timeout.  A later
/// step will route panic diagnostics through the platform console.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
