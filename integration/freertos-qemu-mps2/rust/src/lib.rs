//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3C — allocator + facade + RuntimeLifecycle smoke.

#![no_std]

extern crate alloc;

mod allocator;
mod harness;
mod cases;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use core::ffi::CStr;
use core::sync::atomic::{AtomicU32, Ordering};

use osal_api::error::Error;
use osal_api::time::Timeout;
use osal_api::runtime::RuntimeState;
use osal_api::traits::mutex::Mutex;
use osal_backend_freertos_sys as sys;
use sys::{DelayStatus, SchedulerState, TickSnapshot};

use allocator::FreeRtosAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: FreeRtosAllocator = FreeRtosAllocator;

// ------------------------------------------------------------------
// C bridges
// ------------------------------------------------------------------

unsafe extern "C" {
    fn osal_test_rust_fatal(reason: u32) -> !;
    fn osal_test_trace_u64(name: *const i8, value: u64);
}

fn trace_u64(name: &CStr, value: u64) {
    unsafe { osal_test_trace_u64(name.as_ptr().cast::<i8>(), value); }
}

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------

#[repr(i32)]
enum SmokeFailure {
    RustData     = 10,
    RustBss      = 11,
    RustBssWrite = 12,

    ShimScheduler = 20,
    CapTickRate   = 30,
    CapPriorities = 31,
    CapTaskName   = 32,
    CapTickBits   = 33,
    CapStackWord  = 34,
    CapDynamicAllocation = 35,
    CapSoftwareTimers    = 36,
    CapMinimalStack = 37,
    CapMaxStack     = 38,
    CapTlsSlots  = 39,
    CapTlsIndex  = 40,
    ShimDelay    = 50,
    ShimTick     = 51,

    BoxValue     = 60,
    Alignment    = 61,
    ArcCount     = 62,
    ArcDrop      = 63,
    VecValue     = 64,
    AlignedValue = 67,
    HeapDidNotDecrease = 65,
    AllocatorLeak = 66,

    // Runtime lifecycle
    InitialState      = 70,
    ObjectBeforeInit  = 71,
    Initialize        = 72,
    AlreadyInitialized = 73,
    MutexCreate       = 74,
    MutexLock         = 75,
    MutexRelock       = 76,
    MutexValue        = 77,
    ShutdownBusy      = 78,
    BusyRollback      = 79,
    Shutdown          = 80,
    ShutdownState     = 81,
    NotInitialized    = 82,
    Reinitialize      = 83,
    Reshutdown        = 84,
    CycleInitialize   = 85,
    CycleMutex        = 86,
    CycleShutdown     = 87,
    CycleLeak         = 88,
    ObjectBeforeInitHeapChanged = 89,
    BusyHeapChanged             = 90,
    LifecycleLeak               = 91,
    ReinitializeLeak            = 92,
}

// ------------------------------------------------------------------
// Runtime-image sentinels
// ------------------------------------------------------------------

static RUST_DATA_SENTINEL: AtomicU32 = AtomicU32::new(0x2468_ACE0);
static RUST_BSS_SENTINEL: AtomicU32 = AtomicU32::new(0);

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
// C-shim validation
// ------------------------------------------------------------------

fn validate_shim() -> Result<(), SmokeFailure> {
    if sys::scheduler_state() != SchedulerState::Running {
        return Err(SmokeFailure::ShimScheduler);
    }

    let caps = sys::capabilities();
    if caps.tick_rate_hz != 1000 { return Err(SmokeFailure::CapTickRate); }
    if caps.max_priorities != 8 { return Err(SmokeFailure::CapPriorities); }
    if caps.max_task_name_len != 16 { return Err(SmokeFailure::CapTaskName); }
    if caps.tick_bits != 32 { return Err(SmokeFailure::CapTickBits); }
    if caps.stack_word_size != 4 { return Err(SmokeFailure::CapStackWord); }
    if !caps.dynamic_allocation { return Err(SmokeFailure::CapDynamicAllocation); }
    if caps.software_timers { return Err(SmokeFailure::CapSoftwareTimers); }
    if caps.minimal_stack_depth_words != 128 { return Err(SmokeFailure::CapMinimalStack); }
    if caps.max_stack_depth_words != u32::MAX { return Err(SmokeFailure::CapMaxStack); }
    if caps.tls_pointer_slots != 1 { return Err(SmokeFailure::CapTlsSlots); }
    if caps.task_tls_index != 0 { return Err(SmokeFailure::CapTlsIndex); }

    let before = sys::tick_snapshot();
    if sys::delay_ticks(2) != DelayStatus::Ok {
        return Err(SmokeFailure::ShimDelay);
    }
    let after = sys::tick_snapshot();
    let tb = total_ticks(before, caps.tick_bits);
    let ta = total_ticks(after, caps.tick_bits);
    if ta <= tb { return Err(SmokeFailure::ShimTick); }

    Ok(())
}

fn total_ticks(s: TickSnapshot, bits: u8) -> u128 {
    ((s.overflow_count as u128) << bits) | s.tick_count as u128
}

// ------------------------------------------------------------------
// Allocator smoke
// ------------------------------------------------------------------

#[repr(align(64))]
struct Aligned64 { bytes: [u8; 64], }

fn validate_allocator() -> Result<(), SmokeFailure> {
    let h0 = sys::heap_free();
    trace_u64(c"heap_baseline", h0);

    let boxed = Box::new(0x1234_5678_u32);
    core::hint::black_box(boxed.as_ref());
    if *boxed != 0x1234_5678 { return Err(SmokeFailure::BoxValue); }

    let aligned = Box::new(Aligned64 { bytes: [0xA5; 64] });
    if (&*aligned as *const Aligned64 as usize) & 63 != 0 {
        return Err(SmokeFailure::Alignment);
    }
    if aligned.bytes.iter().any(|b| *b != 0xA5) {
        return Err(SmokeFailure::AlignedValue);
    }
    core::hint::black_box(&aligned.bytes);

    let h1 = sys::heap_free();
    trace_u64(c"heap_alloc_live", h1);
    if h1 >= h0 { return Err(SmokeFailure::HeapDidNotDecrease); }

    let shared = Arc::new(0x55AA_u32);
    core::hint::black_box(shared.as_ref());
    let cloned = Arc::clone(&shared);
    if Arc::strong_count(&shared) != 2 { return Err(SmokeFailure::ArcCount); }
    drop(cloned);
    if Arc::strong_count(&shared) != 1 { return Err(SmokeFailure::ArcDrop); }

    let mut values = Vec::new();
    for v in 0u32..128 { values.push(v); }
    core::hint::black_box(values.as_slice());
    for (i, v) in values.iter().enumerate() {
        if *v != i as u32 { return Err(SmokeFailure::VecValue); }
    }

    drop(values);
    drop(shared);
    drop(aligned);
    drop(boxed);

    let h2 = sys::heap_free();
    trace_u64(c"heap_after_alloc", h2);
    if h2 != h0 { return Err(SmokeFailure::AllocatorLeak); }

    Ok(())
}

// ------------------------------------------------------------------
// Runtime lifecycle smoke
// ------------------------------------------------------------------

fn validate_lifecycle() -> Result<(), SmokeFailure> {
    let lifecycle_baseline = sys::heap_free();

    // Case 1: initial state
    if osal::runtime_state() != RuntimeState::Uninitialized {
        return Err(SmokeFailure::InitialState);
    }

    // Case 2: pre-init object creation must fail without changing free heap
    match osal::backend::Mutex::new(1u32) {
        Err(Error::NotInitialized) => {}
        _ => return Err(SmokeFailure::ObjectBeforeInit),
    }
    if sys::heap_free() != lifecycle_baseline {
        return Err(SmokeFailure::ObjectBeforeInitHeapChanged);
    }

    // Case 3: first initialize
    osal::initialize().map_err(|_| SmokeFailure::Initialize)?;
    if osal::runtime_state() != RuntimeState::Running {
        return Err(SmokeFailure::Initialize);
    }
    trace_u64(c"heap_after_init", sys::heap_free());

    // Case 4: repeat initialize
    match osal::initialize() {
        Err(Error::AlreadyInitialized) => {}
        _ => return Err(SmokeFailure::AlreadyInitialized),
    }

    // Case 5: create Mutex, lock, write, unlock, re-lock, read
    let mutex = osal::backend::Mutex::new(7u32)
        .map_err(|_| SmokeFailure::MutexCreate)?;
    trace_u64(c"heap_with_mutex", sys::heap_free());
    {
        let mut guard = mutex.lock(Timeout::NoWait)
            .map_err(|_| SmokeFailure::MutexLock)?;
        *guard = 11;
    }
    {
        let guard = mutex.lock(Timeout::NoWait)
            .map_err(|_| SmokeFailure::MutexRelock)?;
        if *guard != 11 { return Err(SmokeFailure::MutexValue); }
    }

    // Case 7: active object blocks shutdown, must be failure-atomic
    let heap_before_busy = sys::heap_free();
    match osal::shutdown() {
        Err(Error::Busy) => {}
        _ => return Err(SmokeFailure::ShutdownBusy),
    }
    if osal::runtime_state() != RuntimeState::Running {
        return Err(SmokeFailure::BusyRollback);
    }
    if sys::heap_free() != heap_before_busy {
        return Err(SmokeFailure::BusyHeapChanged);
    }

    // Case 8: drop mutex → shutdown succeeds → back to baseline
    drop(mutex);
    osal::shutdown().map_err(|_| SmokeFailure::Shutdown)?;
    if osal::runtime_state() != RuntimeState::Uninitialized {
        return Err(SmokeFailure::ShutdownState);
    }
    {
        let h = sys::heap_free();
        trace_u64(c"heap_after_shutdown", h);
        if h != lifecycle_baseline {
            return Err(SmokeFailure::LifecycleLeak);
        }
    }

    // Case 9: repeat shutdown
    match osal::shutdown() {
        Err(Error::NotInitialized) => {}
        _ => return Err(SmokeFailure::NotInitialized),
    }

    // Case 10: reinitialize + shutdown → back to baseline
    osal::initialize().map_err(|_| SmokeFailure::Reinitialize)?;
    osal::shutdown().map_err(|_| SmokeFailure::Reshutdown)?;
    if sys::heap_free() != lifecycle_baseline {
        return Err(SmokeFailure::ReinitializeLeak);
    }

    Ok(())
}

fn validate_lifecycle_cycles() -> Result<(), SmokeFailure> {
    const CYCLES: usize = 8;

    for cycle in 0..CYCLES {
        let baseline = sys::heap_free();

        osal::initialize().map_err(|_| SmokeFailure::CycleInitialize)?;
        let m = osal::backend::Mutex::new(cycle as u32)
            .map_err(|_| SmokeFailure::CycleMutex)?;
        drop(m);
        osal::shutdown().map_err(|_| SmokeFailure::CycleShutdown)?;

        let after = sys::heap_free();
        if after != baseline {
            return Err(SmokeFailure::CycleLeak);
        }
    }

    trace_u64(c"lifecycle_cycles", CYCLES as u64);
    Ok(())
}

// ------------------------------------------------------------------
// Main entry
// ------------------------------------------------------------------

fn run_smoke() -> Result<(), SmokeFailure> {
    validate_runtime_image()?;
    validate_shim()?;
    validate_allocator()?;
    validate_lifecycle()?;
    validate_lifecycle_cycles()?;
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "C" fn osal_rust_smoke_entry() -> i32 {
    match run_smoke() {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

// ------------------------------------------------------------------
// Object test entry (P7G Step 4)
// ------------------------------------------------------------------

#[unsafe(no_mangle)]
pub extern "C" fn osal_test_object_entry() -> i32 {
    let caps = sys::capabilities();
    match harness::run_harness_smoke(caps.tick_bits) {
        Ok(()) => 0,
        Err(e) => e as i32,
    }
}

// ------------------------------------------------------------------
// Panic handler
// ------------------------------------------------------------------

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { osal_test_rust_fatal(1); }
}
