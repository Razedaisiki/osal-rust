//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3A: minimal `#![no_std]` entry point.  No alloc, no OSAL,
//! no C shim — just C/Rust ABI validation.

#![no_std]

/// Called from the C bootstrap task after scheduler start.
///
/// Returns 0 on success.  Any non-zero value causes the C side
/// to emit a boot-failure marker and exit.
#[unsafe(no_mangle)]
pub extern "C" fn osal_rust_smoke_entry() -> i32 {
    0
}

/// Panic handler — required by `#![no_std]`.
///
/// Step 3A has no UART or platform output from Rust.  A panic traps
/// permanently here and is detected by the QEMU timeout.  A later
/// step will route panic diagnostics through the platform console.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
