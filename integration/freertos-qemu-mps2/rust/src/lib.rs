//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3B — runtime image sentinels + C shim capabilities +
//! scheduler state + delay/tick round-trip.

#![no_std]

use core::sync::atomic::{AtomicU32, Ordering};

use osal_backend_freertos_sys as sys;
use sys::{DelayStatus, SchedulerState, TickSnapshot};

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

    ShimDelay = 50,
    ShimTick  = 51,
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

    // 2. Probe capabilities — all 11 public fields matched against
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
    if caps.max_stack_depth_words != u32::MAX {
        return Err(SmokeFailure::CapMaxStack);
    }
    if caps.tls_pointer_slots != 1 {
        return Err(SmokeFailure::CapTlsSlots);
    }
    if caps.task_tls_index != 0 {
        return Err(SmokeFailure::CapTlsIndex);
    }

    // 3. Tick snapshot + shim delay round-trip — prove Rust → C shim
    //    → vTaskDelay → SysTick wake → C shim tick snapshot.
    let before = sys::tick_snapshot();

    if sys::delay_ticks(2) != DelayStatus::Ok {
        return Err(SmokeFailure::ShimDelay);
    }

    let after = sys::tick_snapshot();

    let ticks_before = total_ticks(before, caps.tick_bits);
    let ticks_after  = total_ticks(after, caps.tick_bits);

    if ticks_after <= ticks_before {
        return Err(SmokeFailure::ShimTick);
    }

    Ok(())
}

// ------------------------------------------------------------------
// Monotonic tick total — overflow_count << tick_bits | tick_count.
// ------------------------------------------------------------------

fn total_ticks(snapshot: TickSnapshot, tick_bits: u8) -> u128 {
    ((snapshot.overflow_count as u128) << tick_bits)
        | snapshot.tick_count as u128
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

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
