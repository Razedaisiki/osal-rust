//! Host task and EventGroup fixture for FreeRTOS task backend.
//!
//! Uses `std::thread::spawn` to simulate `xTaskCreate`, `std::sync::Mutex` +
//! `Condvar` for EventGroup wait, and `thread_local!` for TLS current context.
//! Only compiled when `test-fixture` is enabled.
//!
//! Full implementation in the next commit (Commit 3).

extern crate std;

use super::{EventGroupHandle, TaskCreateStatus, TaskEntry};

// ---------------------------------------------------------------------------
// EventGroup fixture — minimal stubs
// ---------------------------------------------------------------------------

pub fn event_group_create() -> Option<EventGroupHandle> {
    todo!("EventGroup fixture not yet implemented")
}

pub fn event_group_set_bits(_handle: &EventGroupHandle, _bits: u32) -> u32 {
    todo!("EventGroup fixture not yet implemented")
}

pub fn event_group_wait_bits(
    _handle: &EventGroupHandle,
    _bits: u32,
    _clear_on_exit: bool,
    _wait_for_all: bool,
    _ticks: u64,
) -> super::EventGroupWaitStatus {
    todo!("EventGroup fixture not yet implemented")
}

pub fn event_group_delete(_handle: EventGroupHandle) {
    todo!("EventGroup fixture not yet implemented")
}

// ---------------------------------------------------------------------------
// Task fixture — minimal stubs
// ---------------------------------------------------------------------------

pub fn task_create(
    _entry: TaskEntry,
    _name: *const core::ffi::c_char,
    _stack_depth_words: u32,
    _parameter: *mut core::ffi::c_void,
    _priority: u32,
) -> TaskCreateStatus {
    todo!("Task fixture not yet implemented")
}

pub fn task_delete_current() -> ! {
    todo!("Task fixture not yet implemented")
}

pub fn task_set_current_context(_ptr: *mut core::ffi::c_void) {
    todo!("Task fixture not yet implemented")
}

pub fn task_current_context() -> *mut core::ffi::c_void {
    todo!("Task fixture not yet implemented")
}
