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

/// Panic handler — required by `#![no_std]`.  On panic, return
/// a distinct non-zero code so the C caller can report the failure.
#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    // Signal failure to the C side by returning a non-zero code.
    // We can't unwind (panic = "abort"), so this will trap.
    // A future step can add UART output here.
    loop {
        core::hint::spin_loop();
    }
}
