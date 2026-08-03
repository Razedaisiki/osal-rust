//! FreeRTOS-backed Rust global allocator (P7G Step 3C).
//!
//! Uses the over-allocation + header technique to satisfy arbitrary
//! alignment without depending on the underlying FreeRTOS heap to
//! provide aligned returns.

use core::alloc::{GlobalAlloc, Layout};
use core::mem::{align_of, size_of};
use core::ptr;

use osal_backend_freertos_sys as sys;

/// Header stored immediately before the user-visible allocation.
#[repr(C)]
struct AllocationHeader {
    /// The raw pointer returned by `sys::heap_alloc`, used at free
    /// time to release the correct base address.
    raw: *mut u8,
}

pub struct FreeRtosAllocator;

unsafe impl GlobalAlloc for FreeRtosAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // Defensive fallback.  GlobalAlloc callers must provide a
        // non-zero-sized Layout — the standard library never requests
        // a zero-sized allocation through the global allocator.
        if layout.size() == 0 {
            return ptr::null_mut();
        }

        let header_size = size_of::<AllocationHeader>();
        let effective_align =
            layout.align().max(align_of::<AllocationHeader>());

        // total = size + header + (effective_align - 1) for padding
        let total = match layout
            .size()
            .checked_add(header_size)
            .and_then(|v| v.checked_add(effective_align - 1))
        {
            Some(v) => v,
            None => return ptr::null_mut(),
        };

        let raw = unsafe { sys::heap_alloc(total) };
        if raw.is_null() {
            return ptr::null_mut();
        }

        // Align the user pointer within the block.
        let candidate = unsafe { raw.add(header_size) };
        let offset = candidate.align_offset(effective_align);

        if offset == usize::MAX {
            unsafe { sys::heap_dealloc(raw); }
            return ptr::null_mut();
        }

        let user = unsafe { candidate.add(offset) };
        let header = unsafe { user.sub(header_size).cast::<AllocationHeader>() };

        unsafe { header.write(AllocationHeader { raw }) };

        user
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        if pointer.is_null() {
            return;
        }

        let header_size = size_of::<AllocationHeader>();
        let header = unsafe {
            pointer.sub(header_size).cast::<AllocationHeader>()
        };
        let raw = unsafe { header.read().raw };

        unsafe { sys::heap_dealloc(raw) };
    }
}
