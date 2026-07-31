//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3B-0: runtime image initialisation validation.
//! C/Rust ABI + `.data` copy + `.bss` zero sentinels.

#![no_std]

use core::sync::atomic::{AtomicU32, Ordering};

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
// Public entry point.
// ------------------------------------------------------------------

/// Called from the C bootstrap task after scheduler start.
///
/// Returns 0 on success.  Non-zero codes are `SmokeFailure` values.
#[unsafe(no_mangle)]
pub extern "C" fn osal_rust_smoke_entry() -> i32 {
    match validate_runtime_image() {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
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
