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
// Tick snapshot (ADR 0023 §1)
// ---------------------------------------------------------------------------

/// Coherent kernel tick snapshot — compatible with C `osal_freertos_tick_snapshot_t`.
#[derive(Debug, Clone, Copy)]
#[repr(C)]
pub struct TickSnapshot {
    pub overflow_count: u64,
    pub tick_count: u64,
}

// ---------------------------------------------------------------------------
// Delay status codes (ADR 0023 §5)
// ---------------------------------------------------------------------------

pub const DELAY_OK: u32 = 0;
pub const DELAY_INVALID_TICKS: u32 = 1;
pub const DELAY_SCHEDULER_STOPPED: u32 = 2;

/// Outcome of `delay_ticks()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelayStatus {
    Ok,
    InvalidTicks,
    SchedulerNotRunning,
    Unknown(u32),
}

impl DelayStatus {
    #[cfg(not(feature = "test-fixture"))]
    fn from_raw(raw: u32) -> Self {
        match raw {
            DELAY_OK => DelayStatus::Ok,
            DELAY_INVALID_TICKS => DelayStatus::InvalidTicks,
            DELAY_SCHEDULER_STOPPED => DelayStatus::SchedulerNotRunning,
            other => DelayStatus::Unknown(other),
        }
    }
}

// ---------------------------------------------------------------------------
// FFI declarations (native path only — fixture uses Rust stubs)
// ---------------------------------------------------------------------------

#[cfg(not(feature = "test-fixture"))]
unsafe extern "C" {
    fn osal_freertos_probe_capabilities() -> KernelCapabilities;
    fn osal_freertos_scheduler_state() -> u32;
    fn osal_freertos_tick_snapshot() -> TickSnapshot;
    fn osal_freertos_delay_ticks(ticks: u64) -> u32;
    fn osal_freertos_max_finite_delay_ticks() -> u64;
    fn osal_freertos_heap_free() -> u64;
    fn osal_freertos_enter_critical();
    fn osal_freertos_exit_critical();
    fn osal_freertos_max_semaphore_count() -> u64;
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
            tick_bits: TICK_BITS_FIXTURE.load(core::sync::atomic::Ordering::Relaxed),
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
        read_scheduler_state_fixture()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_scheduler_state() };
        SchedulerState::from_raw(raw)
    }
}

// ---------------------------------------------------------------------------
// Tick and delay safe wrappers (ADR 0023)
// ---------------------------------------------------------------------------

/// Capture a coherent tick + overflow count snapshot.
pub fn tick_snapshot() -> TickSnapshot {
    #[cfg(feature = "test-fixture")]
    {
        read_tick_fixture()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_tick_snapshot() }
    }
}

/// Request a delay of `ticks` kernel ticks.
///
/// Returns a [`DelayStatus`] indicating success or the reason for failure.
pub fn delay_ticks(ticks: u64) -> DelayStatus {
    #[cfg(feature = "test-fixture")]
    {
        use core::sync::atomic::Ordering;
        // Fixture: advance virtual tick counter by `ticks`, simulating
        // wrap at the configured tick width (modulo 2^bits).
        let bits = TICK_BITS_FIXTURE.load(Ordering::Relaxed);
        let modulus: u128 = 1u128 << (bits as u32);

        let current_overflow = TICK_OVERFLOW_FIXTURE.load(Ordering::Relaxed);
        let current_count = TICK_COUNT_FIXTURE.load(Ordering::Relaxed);

        let total: u128 = (current_count as u128)
            .checked_add(ticks as u128)
            .expect("fixture tick overflowed u128");

        let wrap_count = total / modulus;
        let new_count = (total % modulus) as u64;
        let new_overflow = current_overflow
            .checked_add(wrap_count as u64)
            .expect("fixture overflow count overflowed u64");

        TICK_COUNT_FIXTURE.store(new_count, Ordering::Relaxed);
        TICK_OVERFLOW_FIXTURE.store(new_overflow, Ordering::Relaxed);
        DelayStatus::Ok
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_delay_ticks(ticks) };
        DelayStatus::from_raw(raw)
    }
}

/// Return the maximum finite tick count for the platform.
///
/// This is `portMAX_DELAY - 1`, reserving the all-ones value as a
/// "forever" sentinel (ADR 0023 §5).
pub fn max_finite_delay_ticks() -> u64 {
    #[cfg(feature = "test-fixture")]
    {
        MAX_FINITE_DELAY_FIXTURE.load(core::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_max_finite_delay_ticks() }
    }
}

// ---------------------------------------------------------------------------
// Heap and critical-section safe wrappers (ADR 0024)
// ---------------------------------------------------------------------------

/// Return the current FreeRTOS kernel heap free space in bytes.
pub fn heap_free() -> u64 {
    #[cfg(feature = "test-fixture")]
    {
        HEAP_FREE_FIXTURE.load(core::sync::atomic::Ordering::Relaxed)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_heap_free() }
    }
}

/// Enter a critical section (nesting supported natively).
///
/// Each call must be paired with a matching [`exit_critical`].
pub fn enter_critical() {
    #[cfg(feature = "test-fixture")]
    {
        CRITICAL_DEPTH_FIXTURE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_enter_critical() };
    }
}

/// Exit a critical section.
///
/// # Safety (caller)
///
/// Must be paired with a prior [`enter_critical`].  Unbalanced exit
/// corrupts the kernel's nesting counter.
pub fn exit_critical() {
    #[cfg(feature = "test-fixture")]
    {
        let prev = CRITICAL_DEPTH_FIXTURE.fetch_sub(1, core::sync::atomic::Ordering::Relaxed);
        if prev == 0 {
            panic!("exit_critical() called without matching enter_critical()");
        }
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_exit_critical() };
    }
}

/// Return the current critical-section nesting depth (fixture only).
#[cfg(feature = "test-fixture")]
pub fn critical_depth() -> usize {
    CRITICAL_DEPTH_FIXTURE.load(core::sync::atomic::Ordering::Relaxed) as usize
}

// ---------------------------------------------------------------------------
// Semaphore range query (ADR 0026)
// ---------------------------------------------------------------------------

/// Return the maximum semaphore count supported by the native `UBaseType_t`.
///
/// Counting semaphore parameters (`max_count`, `initial_count`) must not
/// exceed this value.
pub fn max_semaphore_count() -> u64 {
    #[cfg(feature = "test-fixture")]
    {
        // Fixture: generous 32-bit range.
        u32::MAX as u64
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_max_semaphore_count() }
    }
}

// ---------------------------------------------------------------------------
// Fixture state (test-fixture only)
// ---------------------------------------------------------------------------

// Atomic fields for fixture-controlled tick and scheduler state.
// Individual atomics avoid dependence on portable-atomic's Atomic<T>.

#[cfg(feature = "test-fixture")]
static TICK_OVERFLOW_FIXTURE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[cfg(feature = "test-fixture")]
static TICK_COUNT_FIXTURE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

#[cfg(feature = "test-fixture")]
static SCHEDULER_STATE_FIXTURE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(SCHEDULER_RUNNING);

#[cfg(feature = "test-fixture")]
static HEAP_FREE_FIXTURE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(65536);

#[cfg(feature = "test-fixture")]
static CRITICAL_DEPTH_FIXTURE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Configurable tick width for wrap simulation (default: 32 bits).
#[cfg(feature = "test-fixture")]
static TICK_BITS_FIXTURE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(32);

/// Configurable max_finite_delay_ticks for forcing multi-chunk paths.
/// Default matches 32-bit: (1 << 32) - 2.
#[cfg(feature = "test-fixture")]
static MAX_FINITE_DELAY_FIXTURE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new((1u64 << 32) - 2);

#[cfg(feature = "test-fixture")]
fn read_tick_fixture() -> TickSnapshot {
    TickSnapshot {
        overflow_count: TICK_OVERFLOW_FIXTURE.load(core::sync::atomic::Ordering::Relaxed),
        tick_count: TICK_COUNT_FIXTURE.load(core::sync::atomic::Ordering::Relaxed),
    }
}

#[cfg(feature = "test-fixture")]
fn read_scheduler_state_fixture() -> SchedulerState {
    let raw = SCHEDULER_STATE_FIXTURE.load(core::sync::atomic::Ordering::Relaxed);
    SchedulerState::from_raw(raw)
}

/// Fixture control API — enables deterministic contract testing on host CI.
///
/// All items are only available behind `#[cfg(feature = "test-fixture")]`.
#[cfg(feature = "test-fixture")]
pub mod fixture {
    use core::sync::atomic::Ordering;

    /// Reset all fixture state to defaults.
    pub fn reset() {
        super::TICK_OVERFLOW_FIXTURE.store(0, Ordering::Relaxed);
        super::TICK_COUNT_FIXTURE.store(0, Ordering::Relaxed);
        super::SCHEDULER_STATE_FIXTURE.store(super::SCHEDULER_RUNNING, Ordering::Relaxed);
        super::HEAP_FREE_FIXTURE.store(65536, Ordering::Relaxed);
        super::CRITICAL_DEPTH_FIXTURE.store(0, Ordering::Relaxed);
        super::TICK_BITS_FIXTURE.store(32, Ordering::Relaxed);
        super::MAX_FINITE_DELAY_FIXTURE.store((1u64 << 32) - 2, Ordering::Relaxed);
    }

    /// Set the tick snapshot that `tick_snapshot()` will return.
    pub fn set_tick_snapshot(overflow_count: u64, tick_count: u64) {
        super::TICK_OVERFLOW_FIXTURE.store(overflow_count, Ordering::Relaxed);
        super::TICK_COUNT_FIXTURE.store(tick_count, Ordering::Relaxed);
    }

    /// Set the tick width (bits) for wrap simulation (16, 32, or 64).
    ///
    /// Must be called **before** `runtime::initialize()` so the
    /// capability probe caches the correct value.
    ///
    /// # Panics
    ///
    /// Panics if `bits` is not 16, 32, or 64.
    pub fn set_tick_bits(bits: u8) {
        assert!(
            matches!(bits, 16 | 32 | 64),
            "tick_bits must be 16, 32, or 64, got {bits}"
        );
        super::TICK_BITS_FIXTURE.store(bits, Ordering::Relaxed);
    }

    /// Set the value returned by `max_finite_delay_ticks()`.
    ///
    /// Use a small value (e.g. 7) to force multi-chunk delay paths in tests
    /// without waiting for enormous durations.
    ///
    /// # Panics
    ///
    /// Panics if `value` is less than 2 (must be at least
    /// `guard_tick + 1` so that `max_payload ≥ 1`).
    pub fn set_max_finite_delay_ticks(value: u64) {
        assert!(
            value >= 2,
            "max_finite_delay_ticks must be ≥ 2 (guard tick + min payload), got {value}"
        );
        super::MAX_FINITE_DELAY_FIXTURE.store(value, Ordering::Relaxed);
    }

    /// Set the scheduler state returned by `scheduler_state()`.
    pub fn set_scheduler_state(state: super::SchedulerState) {
        let raw: u32 = match state {
            super::SchedulerState::NotStarted => super::SCHEDULER_NOT_STARTED,
            super::SchedulerState::Running => super::SCHEDULER_RUNNING,
            super::SchedulerState::Suspended => super::SCHEDULER_SUSPENDED,
            super::SchedulerState::Unknown(v) => v,
        };
        super::SCHEDULER_STATE_FIXTURE.store(raw, Ordering::Relaxed);
    }

    /// Set the value returned by `heap_free()`.
    pub fn set_heap_free(bytes: u64) {
        super::HEAP_FREE_FIXTURE.store(bytes, Ordering::Relaxed);
    }

    /// Return the current critical-section nesting depth.
    pub fn critical_depth() -> usize {
        super::CRITICAL_DEPTH_FIXTURE.load(Ordering::Relaxed) as usize
    }

    /// Return the current tick count value (for test assertions).
    pub fn tick_count() -> u64 {
        super::TICK_COUNT_FIXTURE.load(Ordering::Relaxed)
    }

    /// Return the current overflow count value (for test assertions).
    pub fn tick_overflow_count() -> u64 {
        super::TICK_OVERFLOW_FIXTURE.load(Ordering::Relaxed)
    }
}
