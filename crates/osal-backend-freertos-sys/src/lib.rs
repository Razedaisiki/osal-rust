//! Low-level C FFI bindings for the FreeRTOS kernel.
//!
//! This crate is the **only** place where `extern "C"` and raw FFI
//! calls to FreeRTOS are permitted (ADR 0022).  All types exposed
//! to the backend crate are opaque handles or fixed-width C types.
//!
//! # Test fixture
//!
//! Enable `--features test-fixture` for host-compilable stub
//! capability data.  The fixture does **not** link against a real
//! FreeRTOS kernel.

#![no_std]

// ---------------------------------------------------------------------------
// Platform gate
// ---------------------------------------------------------------------------

#[cfg(not(any(
    feature = "test-fixture",
    // Future: target_os = "freertos",
)))]
compile_error!(
    "osal-backend-freertos-sys requires a FreeRTOS target or --features test-fixture. \
     See ADR 0022 §6."
);

// ---------------------------------------------------------------------------
// Opaque handle types (ADR 0022 §2)
// ---------------------------------------------------------------------------

/// Opaque FreeRTOS task handle.
pub type TaskHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS queue handle.
pub type QueueHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS semaphore handle.
pub type SemaphoreHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS timer handle.
pub type TimerHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS event group handle.
pub type EventGroupHandle = *mut core::ffi::c_void;

// ---------------------------------------------------------------------------
// Capability struct (ADR 0021 §2)
// ---------------------------------------------------------------------------

/// Kernel capabilities probed from `FreeRTOSConfig.h` at compile time.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KernelCapabilities {
    pub tick_rate_hz: u32,
    pub max_priorities: u32,
    pub max_task_name_len: u32,
    pub tick_bits: u8,
    pub stack_word_size: u8,
    pub dynamic_allocation: bool,
    pub software_timers: bool,
    pub scheduler_state: u32,
}

/// Scheduler state constants.
pub const SCHEDULER_NOT_STARTED: u32 = 0;
pub const SCHEDULER_RUNNING: u32 = 1;
pub const SCHEDULER_SUSPENDED: u32 = 2;

// ---------------------------------------------------------------------------
// FFI declarations
// ---------------------------------------------------------------------------

unsafe extern "C" {
    fn osal_freertos_probe_capabilities() -> KernelCapabilities;
    fn osal_freertos_scheduler_state() -> u32;
}

// ---------------------------------------------------------------------------
// Safe wrappers
// ---------------------------------------------------------------------------

/// Probe kernel capabilities.
///
/// # Test fixture
///
/// When `test-fixture` is enabled, returns fixed stub values so the
/// backend crate and its tests compile and run on a host without a
/// FreeRTOS toolchain.
pub fn probe_capabilities() -> KernelCapabilities {
    #[cfg(feature = "test-fixture")]
    {
        KernelCapabilities {
            tick_rate_hz: 1000,
            max_priorities: 8,
            max_task_name_len: 16,
            tick_bits: 32,
            stack_word_size: 4,
            dynamic_allocation: true,
            software_timers: true,
            scheduler_state: SCHEDULER_RUNNING,
        }
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_probe_capabilities() }
    }
}

/// Query the current FreeRTOS scheduler state.
pub fn scheduler_state() -> u32 {
    #[cfg(feature = "test-fixture")]
    {
        SCHEDULER_RUNNING
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_scheduler_state() }
    }
}
