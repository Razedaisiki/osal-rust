//! Rust integration staticlib for OSAL FreeRTOS QEMU MPS2 firmware.
//!
//! Step 3C — FreeRTOS-backed allocator smoke (no facade yet).

#![no_std]

extern crate alloc;

mod allocator;

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec::Vec;

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::{AtomicU32, Ordering};

use osal_backend_freertos_sys as sys;
use sys::{DelayStatus, SchedulerState, TickSnapshot};

use allocator::FreeRtosAllocator;

#[global_allocator]
static GLOBAL_ALLOCATOR: FreeRtosAllocator = FreeRtosAllocator;

// ------------------------------------------------------------------
// C bridges
// ------------------------------------------------------------------

use core::ffi::CStr;

unsafe extern "C" {
    fn osal_test_rust_fatal(reason: u32) -> !;
    fn osal_test_trace_u64(name: *const i8, value: u64);
}

fn trace_u64(name: &CStr, value: u64) {
    unsafe { osal_test_trace_u64(name.as_ptr(), value); }
}

// ------------------------------------------------------------------
// Error codes
// ------------------------------------------------------------------

#[repr(i32)]
enum SmokeFailure {
    // Runtime image (10–12)
    RustData     = 10,
    RustBss      = 11,
    RustBssWrite = 12,

    // C shim (20, 30–40, 50–51)
    ShimScheduler = 20,
    CapTickRate   = 30,
    CapPriorities = 31,
    CapTaskName   = 32,
    CapTickBits   = 33,
    CapStackWord  = 34,
    CapDynamicAllocation = 35,
    CapSoftwareTimers    = 36,
    CapMinimalStack      = 37,
    CapMaxStack          = 38,
    CapTlsSlots  = 39,
    CapTlsIndex  = 40,
    ShimDelay    = 50,
    ShimTick     = 51,

    // Allocator (60–69)
    BoxValue     = 60,
    Alignment    = 61,
    ArcCount     = 62,
    ArcDrop      = 63,
    VecValue     = 64,
    HeapDidNotDecrease = 65,
    AllocatorLeak = 66,
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
// C-shim validation (scheduler, capabilities, tick/delay)
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

    // Box
    let boxed = Box::new(0x1234_5678_u32);
    core::hint::black_box(boxed.as_ref());
    if *boxed != 0x1234_5678 { return Err(SmokeFailure::BoxValue); }

    // Over-aligned
    let aligned = Box::new(Aligned64 { bytes: [0xA5; 64] });
    core::hint::black_box(aligned.as_ref());
    let addr = &*aligned as *const Aligned64 as usize;
    if addr & 63 != 0 { return Err(SmokeFailure::Alignment); }

    let h1 = sys::heap_free();
    trace_u64(c"heap_allocator_live", h1);
    if h1 >= h0 { return Err(SmokeFailure::HeapDidNotDecrease); }

    // Arc
    let shared = Arc::new(0x55AA_u32);
    core::hint::black_box(shared.as_ref());
    let cloned = Arc::clone(&shared);
    if Arc::strong_count(&shared) != 2 { return Err(SmokeFailure::ArcCount); }
    drop(cloned);
    if Arc::strong_count(&shared) != 1 { return Err(SmokeFailure::ArcDrop); }

    // Vec growth (exercises realloc)
    let mut values = Vec::new();
    for v in 0u32..128 { values.push(v); }
    core::hint::black_box(values.as_slice());
    for (i, v) in values.iter().enumerate() {
        if *v != i as u32 { return Err(SmokeFailure::VecValue); }
    }

    // Drop everything and check heap recovery
    drop(values);
    drop(shared);
    drop(aligned);
    drop(boxed);

    let h2 = sys::heap_free();
    trace_u64(c"heap_after_allocator", h2);
    if h2 != h0 { return Err(SmokeFailure::AllocatorLeak); }

    Ok(())
}

// ------------------------------------------------------------------
// Main entry
// ------------------------------------------------------------------

fn run_smoke() -> Result<(), SmokeFailure> {
    validate_runtime_image()?;
    validate_shim()?;
    validate_allocator()?;
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
// Panic handler
// ------------------------------------------------------------------

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe { osal_test_rust_fatal(1); }
}
