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

extern crate alloc;

// ---------------------------------------------------------------------------
// Opaque handle types (ADR 0022 §2)
// ---------------------------------------------------------------------------

/// Opaque FreeRTOS task handle.
pub type TaskHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS queue handle.
pub type QueueHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS timer handle.
pub type TimerHandle = *mut core::ffi::c_void;

/// Opaque FreeRTOS event group handle.
///
/// Created by `xEventGroupCreate()`.  Not `Copy` — the backend
/// inner owns the handle; `Drop` deletes it exactly once.
pub struct EventGroupHandle {
    pub(crate) raw: core::ptr::NonNull<core::ffi::c_void>,
}

// Safety: FreeRTOS handles may be sent and shared across tasks.
unsafe impl Send for EventGroupHandle {}
unsafe impl Sync for EventGroupHandle {}

// ---------------------------------------------------------------------------
// Synchronisation handle types (ADR 0026 §1)
// ---------------------------------------------------------------------------

/// Opaque FreeRTOS mutex handle.
///
/// Wraps a `SemaphoreHandle_t` created by `xSemaphoreCreateMutex()`.
/// Not `Copy` — the backend inner owns the handle; `Clone` on the
/// public type only increments the `Arc` reference count.
pub struct MutexHandle {
    pub(crate) raw: core::ptr::NonNull<core::ffi::c_void>,
}

/// Opaque FreeRTOS semaphore handle.
///
/// Wraps a `SemaphoreHandle_t` created by `xSemaphoreCreateCounting()`
/// or `xSemaphoreCreateBinary()`.  Not `Copy`.
pub struct SemaphoreHandle {
    pub(crate) raw: core::ptr::NonNull<core::ffi::c_void>,
}

// Safety: FreeRTOS handles may be sent and shared across tasks.
unsafe impl Send for MutexHandle {}
unsafe impl Sync for MutexHandle {}
unsafe impl Send for SemaphoreHandle {}
unsafe impl Sync for SemaphoreHandle {}

// ---------------------------------------------------------------------------
// Take / Give status enums
// ---------------------------------------------------------------------------

/// Outcome of a native take (mutex or semaphore acquire).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeStatus {
    Acquired,
    Timeout,
    Invalid,
}

impl TakeStatus {
    #[cfg(not(feature = "test-fixture"))]
    fn from_raw(raw: u32) -> Self {
        match raw {
            TAKE_ACQUIRED => TakeStatus::Acquired,
            TAKE_TIMEOUT => TakeStatus::Timeout,
            _ => TakeStatus::Invalid,
        }
    }
}

/// Outcome of a native give (mutex unlock or semaphore release).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GiveStatus {
    Ok,
    Full,
    Invalid,
}

impl GiveStatus {
    #[cfg(not(feature = "test-fixture"))]
    fn from_raw(raw: u32) -> Self {
        match raw {
            GIVE_OK => GiveStatus::Ok,
            GIVE_FULL => GiveStatus::Full,
            _ => GiveStatus::Invalid,
        }
    }
}

/// Outcome of a native event-group wait.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventGroupWaitStatus {
    Ok,
    Timeout,
    Invalid,
}

/// Outcome of a native task creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCreateStatus {
    Ok,
    OutOfMemory,
    Invalid,
}

// Take / Give raw constants (matching C #defines).
pub const TAKE_ACQUIRED: u32 = 0;
pub const TAKE_TIMEOUT: u32 = 1;
pub const TAKE_INVALID: u32 = 2;

pub const GIVE_OK: u32 = 0;
pub const GIVE_FULL: u32 = 1;
pub const GIVE_INVALID: u32 = 2;

// EventGroup constants (matching C #defines).
pub const EVENT_GROUP_OK: u32 = 0;
pub const EVENT_GROUP_TIMEOUT: u32 = 1;
pub const EVENT_GROUP_INVALID: u32 = 2;

// Task create constants (matching C #defines).
pub const TASK_CREATE_OK: u32 = 0;
pub const TASK_CREATE_OOM: u32 = 1;
pub const TASK_CREATE_INVALID: u32 = 2;

/// Task entry function type — `extern "C"` function pointer.
pub type TaskEntry = unsafe extern "C" fn(*mut core::ffi::c_void);

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
    // Task support (ADR 0028)
    pub minimal_stack_depth_words: u32,
    pub max_stack_depth_words: u32,
    pub tls_pointer_slots: u8,
    pub task_tls_index: u8,
    pub _reserved: [u8; 2],
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
    fn osal_freertos_mutex_create() -> *mut core::ffi::c_void;
    fn osal_freertos_mutex_take(handle: *mut core::ffi::c_void, ticks: u64) -> u32;
    fn osal_freertos_mutex_give(handle: *mut core::ffi::c_void) -> u32;
    fn osal_freertos_mutex_delete(handle: *mut core::ffi::c_void);
    fn osal_freertos_counting_semaphore_create(max: u32, initial: u32) -> *mut core::ffi::c_void;
    fn osal_freertos_binary_semaphore_create() -> *mut core::ffi::c_void;
    fn osal_freertos_semaphore_take(handle: *mut core::ffi::c_void, ticks: u64) -> u32;
    fn osal_freertos_semaphore_give(handle: *mut core::ffi::c_void) -> u32;
    fn osal_freertos_semaphore_count(handle: *mut core::ffi::c_void) -> u64;
    fn osal_freertos_semaphore_delete(handle: *mut core::ffi::c_void);

    // EventGroup (ADR 0028)
    fn osal_freertos_event_group_create() -> *mut core::ffi::c_void;
    fn osal_freertos_event_group_set_bits(handle: *mut core::ffi::c_void, bits: u32) -> u32;
    fn osal_freertos_event_group_wait_bits(
        handle: *mut core::ffi::c_void,
        bits: u32,
        clear_on_exit: u8,
        wait_for_all: u8,
        ticks: u64,
    ) -> u32;
    fn osal_freertos_event_group_delete(handle: *mut core::ffi::c_void);

    // Task (ADR 0028)
    fn osal_freertos_task_create(
        entry: TaskEntry,
        name: *const core::ffi::c_char,
        stack_depth_words: u32,
        parameter: *mut core::ffi::c_void,
        priority: u32,
    ) -> u32;
    fn osal_freertos_task_delete_current();
    fn osal_freertos_task_set_current_context(ptr: *mut core::ffi::c_void);
    fn osal_freertos_task_get_current_context() -> *mut core::ffi::c_void;

    // Internal task (P7F — Timer Service)
    fn osal_freertos_internal_task_create(
        entry: TaskEntry,
        name: *const core::ffi::c_char,
        stack_depth_words: u32,
        parameter: *mut core::ffi::c_void,
        priority: u32,
    ) -> *mut core::ffi::c_void;
    fn osal_freertos_task_get_current_handle() -> *mut core::ffi::c_void;
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
    // Task support (ADR 0028)
    pub minimal_stack_depth_words: u32,
    pub max_stack_depth_words: u32,
    pub tls_pointer_slots: u8,
    pub task_tls_index: u8,
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
        minimal_stack_depth_words: raw.minimal_stack_depth_words,
        max_stack_depth_words: raw.max_stack_depth_words,
        tls_pointer_slots: raw.tls_pointer_slots,
        task_tls_index: raw.task_tls_index,
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
            minimal_stack_depth_words: 64,
            max_stack_depth_words: 0xFFFFFFFFu32,
            tls_pointer_slots: 5,
            task_tls_index: 0,
            _reserved: [0; 2],
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
        // In Virtual mode, route through the notify bridge so blocked
        // waiters (e.g. Timer Service worker) are woken to recheck
        // deadlines against the new tick value.
        if fixture_sync::sync_wait_mode() == fixture_sync::FixtureWaitMode::Virtual {
            fixture_sync::advance_ticks_and_notify(ticks);
            return DelayStatus::Ok;
        }

        // Realtime mode: advance virtual tick counter by `ticks`,
        // simulating wrap at the configured tick width (modulo 2^bits).
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
// Mutex safe wrappers (ADR 0026)
// ---------------------------------------------------------------------------

/// Create a FreeRTOS native mutex.
///
/// Returns `None` on allocation failure (`NULL` from `xSemaphoreCreateMutex`).
pub fn mutex_create() -> Option<MutexHandle> {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::mutex_create()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_mutex_create() };
        core::ptr::NonNull::new(raw).map(|nn| MutexHandle { raw: nn })
    }
}

/// Attempt to acquire the mutex.
pub fn mutex_take(handle: &MutexHandle, ticks: u64) -> TakeStatus {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::mutex_take(handle, ticks)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_mutex_take(handle.raw.as_ptr(), ticks) };
        TakeStatus::from_raw(raw)
    }
}

/// Release the mutex.
pub fn mutex_give(handle: &MutexHandle) -> GiveStatus {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::mutex_give(handle)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_mutex_give(handle.raw.as_ptr()) };
        GiveStatus::from_raw(raw)
    }
}

/// Delete the mutex. The caller must ensure no task holds it.
pub fn mutex_delete(handle: MutexHandle) {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::mutex_delete(handle);
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_mutex_delete(handle.raw.as_ptr()) };
    }
}

// ---------------------------------------------------------------------------
// Semaphore safe wrappers (ADR 0026)
// ---------------------------------------------------------------------------

/// Create a counting semaphore.
pub fn counting_semaphore_create(max: u32, initial: u32) -> Option<SemaphoreHandle> {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::counting_semaphore_create(max, initial)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_counting_semaphore_create(max, initial) };
        core::ptr::NonNull::new(raw).map(|nn| SemaphoreHandle { raw: nn })
    }
}

/// Create a binary semaphore.
pub fn binary_semaphore_create() -> Option<SemaphoreHandle> {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::binary_semaphore_create()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_binary_semaphore_create() };
        core::ptr::NonNull::new(raw).map(|nn| SemaphoreHandle { raw: nn })
    }
}

/// Attempt to acquire the semaphore.
pub fn semaphore_take(handle: &SemaphoreHandle, ticks: u64) -> TakeStatus {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::semaphore_take(handle, ticks)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_semaphore_take(handle.raw.as_ptr(), ticks) };
        TakeStatus::from_raw(raw)
    }
}

/// Release the semaphore (wake one waiter).
pub fn semaphore_give(handle: &SemaphoreHandle) -> GiveStatus {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::semaphore_give(handle)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_semaphore_give(handle.raw.as_ptr()) };
        GiveStatus::from_raw(raw)
    }
}

/// Return the current semaphore count (snapshot).
pub fn semaphore_count(handle: &SemaphoreHandle) -> u64 {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::semaphore_count(handle)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_semaphore_count(handle.raw.as_ptr()) }
    }
}

/// Delete the semaphore.
pub fn semaphore_delete(handle: SemaphoreHandle) {
    #[cfg(feature = "test-fixture")]
    {
        fixture_sync::semaphore_delete(handle);
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_semaphore_delete(handle.raw.as_ptr()) };
    }
}

// ---------------------------------------------------------------------------
// EventGroup safe wrappers (ADR 0028)
// ---------------------------------------------------------------------------

/// Create a native FreeRTOS event group.
///
/// Returns `None` on allocation failure.
pub fn event_group_create() -> Option<EventGroupHandle> {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::event_group_create()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_event_group_create() };
        core::ptr::NonNull::new(raw).map(|nn| EventGroupHandle { raw: nn })
    }
}

/// Set bits in an event group.  Returns the status.
pub fn event_group_set_bits(handle: &EventGroupHandle, bits: u32) -> u32 {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::event_group_set_bits(handle, bits)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_event_group_set_bits(handle.raw.as_ptr(), bits) }
    }
}

/// Wait for bits in an event group.
pub fn event_group_wait_bits(
    handle: &EventGroupHandle,
    bits: u32,
    clear_on_exit: bool,
    wait_for_all: bool,
    ticks: u64,
) -> EventGroupWaitStatus {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::event_group_wait_bits(handle, bits, clear_on_exit, wait_for_all, ticks)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe {
            osal_freertos_event_group_wait_bits(
                handle.raw.as_ptr(),
                bits,
                clear_on_exit as u8,
                wait_for_all as u8,
                ticks,
            )
        };
        match raw {
            EVENT_GROUP_OK => EventGroupWaitStatus::Ok,
            EVENT_GROUP_TIMEOUT => EventGroupWaitStatus::Timeout,
            _ => EventGroupWaitStatus::Invalid,
        }
    }
}

/// Delete an event group.
pub fn event_group_delete(handle: EventGroupHandle) {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::event_group_delete(handle);
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_event_group_delete(handle.raw.as_ptr()) };
    }
}

// ---------------------------------------------------------------------------
// Task safe wrappers (ADR 0028)
// ---------------------------------------------------------------------------

/// Create a FreeRTOS task.  Returns a status code.
///
/// # Safety
///
/// The caller must ensure `parameter` points to a valid, properly-aligned
/// object that will remain alive for the duration of the task.
pub unsafe fn task_create(
    entry: TaskEntry,
    name: *const core::ffi::c_char,
    stack_depth_words: u32,
    parameter: *mut core::ffi::c_void,
    priority: u32,
) -> TaskCreateStatus {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::task_create(entry, name, stack_depth_words, parameter, priority)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe {
            osal_freertos_task_create(entry, name, stack_depth_words, parameter, priority)
        };
        match raw {
            TASK_CREATE_OK => TaskCreateStatus::Ok,
            TASK_CREATE_OOM => TaskCreateStatus::OutOfMemory,
            _ => TaskCreateStatus::Invalid,
        }
    }
}

/// Delete the current task.  Never returns.
pub fn task_delete_current() -> ! {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::task_delete_current()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_task_delete_current() };
        // The C function loops forever after vTaskDelete(NULL).
        loop {
            unsafe { core::arch::asm!("") }; // prevent compiler from assuming unreachable
        }
    }
}

/// Set the per-task TLS context pointer for the current task.
pub fn task_set_current_context(ptr: *mut core::ffi::c_void) {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::task_set_current_context(ptr);
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_task_set_current_context(ptr) };
    }
}

/// Get the per-task TLS context pointer for the current task.
pub fn task_current_context() -> *mut core::ffi::c_void {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::task_current_context()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        unsafe { osal_freertos_task_get_current_context() }
    }
}

// ---------------------------------------------------------------------------
// Internal task handle (P7F — Timer Service)
// ---------------------------------------------------------------------------

/// Opaque handle for a FreeRTOS task created by the OSAL runtime for
/// internal services (Timer Service, etc.).
///
/// Unlike public `FreeRtosTask`, internal tasks do NOT acquire a
/// `RuntimeLease`, do NOT set OSAL TLS identity, and do NOT increment
/// `Task::count()`.  They are invisible to the public `Task` API.
pub struct InternalTaskHandle {
    #[allow(dead_code)] // used by backend timer service
    pub(crate) raw: core::ptr::NonNull<core::ffi::c_void>,
}

// Safety: FreeRTOS task handles may be sent and shared across tasks.
unsafe impl Send for InternalTaskHandle {}
unsafe impl Sync for InternalTaskHandle {}

impl InternalTaskHandle {
    /// Return `true` if `other` identifies the same native task.
    ///
    /// Used for self-shutdown detection: compare the stored worker
    /// handle with `current_native_task_handle()`.
    pub fn matches(&self, other: &NativeTaskHandle) -> bool {
        core::ptr::eq(self.raw.as_ptr(), other.raw)
    }
}

/// Opaque token identifying the current native FreeRTOS task.
///
/// Obtained from `xTaskGetCurrentTaskHandle()`.  Used for identity
/// comparison only (e.g. detecting self-shutdown in the Timer Service).
#[derive(Clone, Copy)]
pub struct NativeTaskHandle {
    raw: *mut core::ffi::c_void,
}

impl PartialEq for NativeTaskHandle {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self.raw, other.raw)
    }
}

/// Create an internal FreeRTOS task.
///
/// Returns the native task handle on success, `None` on allocation
/// failure or invalid parameters.  The task is created via
/// `xTaskCreate` — identical to the public task path but without
/// RuntimeLease, TLS, or `Task::count()` side effects.
///
/// # Safety
///
/// The caller must ensure `parameter` points to a valid, properly-aligned
/// object that remains alive for the duration of the task.
pub unsafe fn internal_task_create(
    entry: TaskEntry,
    name: *const core::ffi::c_char,
    stack_depth_words: u32,
    parameter: *mut core::ffi::c_void,
    priority: u32,
) -> Option<InternalTaskHandle> {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::internal_task_create(entry, name, stack_depth_words, parameter, priority)
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe {
            osal_freertos_internal_task_create(entry, name, stack_depth_words, parameter, priority)
        };
        core::ptr::NonNull::new(raw).map(|nn| InternalTaskHandle { raw: nn })
    }
}

/// Return the current task's native FreeRTOS handle.
///
/// Returns `None` if called before the scheduler is started or from
/// a non-task context (e.g. an ISR).
pub fn current_native_task_handle() -> Option<NativeTaskHandle> {
    #[cfg(feature = "test-fixture")]
    {
        fixture_task::current_native_task_handle()
    }
    #[cfg(not(feature = "test-fixture"))]
    {
        let raw = unsafe { osal_freertos_task_get_current_handle() };
        if raw.is_null() {
            None
        } else {
            Some(NativeTaskHandle { raw })
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture modules (test-fixture only)
// ---------------------------------------------------------------------------

#[cfg(feature = "test-fixture")]
#[path = "sync_fixture.rs"]
mod fixture_sync;

#[cfg(not(feature = "test-fixture"))]
mod fixture_sync {
    // Stub — never compiled; all callers are cfg-gated.
}

#[cfg(feature = "test-fixture")]
#[path = "task_fixture.rs"]
mod fixture_task;

#[cfg(not(feature = "test-fixture"))]
mod fixture_task {
    // Stub — never compiled; all callers are cfg-gated.
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
        // Join internal task threads (including timer worker) BEFORE
        // clearing sync maps.  sync_notify_all unblocks threads waiting
        // on mutexes/semaphores/event-groups so they can observe
        // shutdown signals and exit cleanly.
        super::fixture_task::task_fixture_reset();
        super::fixture_sync::sync_reset();
    }

    /// Set the tick snapshot that `tick_snapshot()` will return.
    pub fn set_tick_snapshot(overflow_count: u64, tick_count: u64) {
        super::TICK_OVERFLOW_FIXTURE.store(overflow_count, Ordering::Relaxed);
        super::TICK_COUNT_FIXTURE.store(tick_count, Ordering::Relaxed);
        super::fixture_sync::tick_generation_inc();
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

    // ------------------------------------------------------------------
    // Sync fixture controls
    // ------------------------------------------------------------------

    /// Make the next `mutex_create()` return `None` (simulates OOM).
    pub fn set_fail_next_mutex_create(fail: bool) {
        super::fixture_sync::sync_set_fail_next_mutex_create(fail);
    }

    /// Make the next semaphore create return `None`.
    pub fn set_fail_next_semaphore_create(fail: bool) {
        super::fixture_sync::sync_set_fail_next_semaphore_create(fail);
    }

    /// Set the max finite wait ticks for mutex/semaphore take.
    ///
    /// Must be ≥ 2.
    pub fn set_max_finite_wait_ticks(ticks: u64) {
        super::fixture_sync::sync_set_max_finite_wait_ticks(ticks);
    }

    /// Number of mutex creates since last reset.
    pub fn mutex_create_count() -> usize {
        super::fixture_sync::sync_mutex_create_count()
    }

    /// Number of mutex deletes since last reset.
    pub fn mutex_delete_count() -> usize {
        super::fixture_sync::sync_mutex_delete_count()
    }

    /// Number of semaphore creates since last reset.
    pub fn sem_create_count() -> usize {
        super::fixture_sync::sync_sem_create_count()
    }

    /// Number of semaphore deletes since last reset.
    pub fn sem_delete_count() -> usize {
        super::fixture_sync::sync_sem_delete_count()
    }

    /// Ticks passed to take calls since last reset.
    pub fn take_call_ticks() -> alloc::vec::Vec<u64> {
        super::fixture_sync::sync_take_call_ticks()
    }

    /// Clear the recorded take call ticks.
    pub fn clear_take_call_ticks() {
        super::fixture_sync::sync_clear_take_call_ticks();
    }

    /// Number of give calls since last reset.
    pub fn give_call_count() -> usize {
        super::fixture_sync::sync_give_call_count()
    }

    /// Number of threads currently inside a Condvar wait.
    ///
    /// Use for test synchronization: poll until this reaches the
    /// expected value before performing a release/guard-drop.
    /// Incremented atomically immediately before `wait_timeout`,
    /// so a non-zero value guarantees the thread is actually blocked.
    pub fn blocked_count() -> u64 {
        super::fixture_sync::BLOCKED_COUNT.load(Ordering::Relaxed)
    }

    /// Number of threads blocked on a specific mutex's Condvar.
    ///
    /// Per-object variant of [`blocked_count`] — essential for Queue
    /// tests where multiple sync objects may have blocked waiters.
    pub fn mutex_blocked_count(handle: &super::MutexHandle) -> usize {
        super::fixture_sync::sync_mutex_blocked_count(handle)
    }

    /// Number of threads blocked on a specific semaphore's Condvar.
    ///
    /// Per-object variant of [`blocked_count`] — essential for Queue
    /// tests where sender and receiver wait on different semaphores.
    pub fn semaphore_blocked_count(handle: &super::SemaphoreHandle) -> usize {
        super::fixture_sync::sync_semaphore_blocked_count(handle)
    }

    // ------------------------------------------------------------------
    // Task fixture controls (ADR 0028)
    // ------------------------------------------------------------------

    /// Reset the task fixture state (called by `fixture::reset()`).
    pub fn task_fixture_reset() {
        super::fixture_task::task_fixture_reset();
    }

    /// Notify the task fixture of a scheduler state change.
    pub fn task_fixture_notify_scheduler_state(running: bool) {
        super::fixture_task::task_fixture_notify_scheduler_state(running);
    }

    /// Make the next `event_group_create()` return `None`.
    pub fn set_fail_next_event_group_create(fail: bool) {
        super::fixture_task::task_fixture_set_fail_next_event_group_create(fail);
    }

    /// Make the next `task_create()` return `OutOfMemory`.
    pub fn set_fail_next_task_create(fail: bool) {
        super::fixture_task::task_fixture_set_fail_next_task_create(fail);
    }

    /// Number of event group creates since last reset.
    pub fn event_group_create_count() -> usize {
        super::fixture_task::task_fixture_eg_create_count()
    }

    /// Number of event group deletes since last reset.
    pub fn event_group_delete_count() -> usize {
        super::fixture_task::task_fixture_eg_delete_count()
    }

    /// Number of task creates since last reset.
    pub fn task_create_count() -> usize {
        super::fixture_task::task_fixture_task_create_count()
    }

    /// Number of threads currently blocked on event group waits.
    pub fn task_blocked_count() -> u64 {
        super::fixture_task::task_fixture_blocked_count()
    }

    /// Per-event-group blocked count.
    pub fn event_group_blocked_count(handle: &super::EventGroupHandle) -> usize {
        super::fixture_task::task_fixture_eg_blocked_count(handle)
    }

    /// Last stack depth (in words) passed to `task_create()`.
    pub fn last_stack_depth_words() -> u32 {
        super::fixture_task::LAST_STACK_WORDS.load(Ordering::Relaxed)
    }

    /// Last native priority passed to `task_create()`.
    pub fn last_native_priority() -> u32 {
        super::fixture_task::LAST_NATIVE_PRIORITY.load(Ordering::Relaxed)
    }

    // ------------------------------------------------------------------
    // Internal task fixture controls (P7F)
    // ------------------------------------------------------------------

    /// Reset internal task fixture state.
    pub fn internal_task_fixture_reset() {
        super::fixture_task::internal_task_fixture_reset();
    }

    /// Make the next `internal_task_create()` return `None`.
    pub fn set_fail_next_internal_task_create(fail: bool) {
        super::fixture_task::set_fail_next_internal_task_create(fail);
    }

    /// Number of active internal task threads.
    pub fn active_internal_task_count() -> usize {
        super::fixture_task::active_internal_task_count()
    }

    /// Last stack depth (in words) passed to `internal_task_create()`.
    pub fn last_internal_stack_words() -> u32 {
        super::fixture_task::LAST_INTERNAL_STACK_WORDS.load(Ordering::Relaxed)
    }

    /// Last native priority passed to `internal_task_create()`.
    pub fn last_internal_priority() -> u32 {
        super::fixture_task::LAST_INTERNAL_PRIORITY.load(Ordering::Relaxed)
    }

    // ------------------------------------------------------------------
    // Virtual-tick wait mode controls (P7F-S2)
    // ------------------------------------------------------------------

    pub use super::fixture_sync::FixtureWaitMode;

    /// Set the fixture wait mode — `Realtime` (default) or `Virtual`.
    /// `Virtual` mode makes sync-object waits driven by virtual ticks
    /// rather than wall-clock time.  `reset()` restores `Realtime`.
    pub fn set_wait_mode(mode: FixtureWaitMode) {
        super::fixture_sync::sync_set_wait_mode(mode);
    }

    /// Query the current fixture wait mode.
    pub fn wait_mode() -> FixtureWaitMode {
        super::fixture_sync::sync_wait_mode()
    }

    /// Advance virtual ticks and notify all blocked sync-object waiters.
    /// In Virtual mode, blocked threads will recheck their conditions
    /// (deadline, semaphore count) against the new tick value.
    pub fn advance_ticks(ticks: u64) {
        super::fixture_sync::advance_ticks_and_notify(ticks);
    }

    /// Return the current tick generation counter.
    pub fn tick_generation() -> u64 {
        super::fixture_sync::tick_generation()
    }
}
