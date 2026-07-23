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
///
/// Fields match the C `osal_freertos_capability_t` layout exactly.
/// Boolean-like fields are `u8` because C has no `bool` in FFI.
/// Use [`capabilities()`] to get a Rust-friendly view.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct KernelCapabilities {
    pub tick_rate_hz: u32,
    pub max_priorities: u32,
    pub max_task_name_len: u32,
    pub tick_bits: u8,
    pub stack_word_size: u8,
    pub dynamic_allocation: u8,
    pub software_timers: u8,
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

/// Rust-friendly capability view (converts C `u8` fields to `bool`).
#[derive(Debug, Clone, Copy)]
pub struct Capabilities {
    pub tick_rate_hz: u32,
    pub max_priorities: u32,
    pub max_task_name_len: u32,
    pub tick_bits: u8,
    pub stack_word_size: u8,
    pub dynamic_allocation: bool,
    pub software_timers: bool,
}

/// Probe kernel capabilities and return a Rust-friendly view.
pub fn capabilities() -> Capabilities {
    let raw = probe_capabilities();
    Capabilities {
        tick_rate_hz: raw.tick_rate_hz,
        max_priorities: raw.max_priorities,
        max_task_name_len: raw.max_task_name_len,
        tick_bits: raw.tick_bits,
        stack_word_size: raw.stack_word_size,
        dynamic_allocation: raw.dynamic_allocation != 0,
        software_timers: raw.software_timers != 0,
    }
}

/// Probe raw kernel capabilities (C ABI).
///
/// # Test fixture
///
/// When `test-fixture` is enabled, returns fixed stub values.
fn probe_capabilities() -> KernelCapabilities {
    #[cfg(feature = "test-fixture")]
    {
        KernelCapabilities {
            tick_rate_hz: 1000,
            max_priorities: 8,
            max_task_name_len: 16,
            tick_bits: 32,
            stack_word_size: 4,
            dynamic_allocation: 1,
            software_timers: 1,
        }
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_probe_capabilities() }
    }
}

/// Scheduler run-state (dynamic — never cached).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerState {
    NotStarted,
    Running,
    Suspended,
    Unknown(u32),
}

impl SchedulerState {
    fn from_raw(raw: u32) -> Self {
        match raw {
            SCHEDULER_NOT_STARTED => SchedulerState::NotStarted,
            SCHEDULER_RUNNING => SchedulerState::Running,
            SCHEDULER_SUSPENDED => SchedulerState::Suspended,
            other => SchedulerState::Unknown(other),
        }
    }
}

/// Query the current FreeRTOS scheduler state (dynamic).
pub fn scheduler_state() -> SchedulerState {
    #[cfg(feature = "test-fixture")]
    {
        SchedulerState::Running
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_scheduler_state() };
        SchedulerState::from_raw(raw)
    }
}
