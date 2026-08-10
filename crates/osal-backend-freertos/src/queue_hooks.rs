//! Integration test hooks for Queue timeout-race contracts.
//!
//! Only compiled when `integration-test-hooks` is enabled (QEMU
//! integration firmware only).  Provides a deterministic rendezvous
//! at the timeout boundary so the controller can inject a concurrent
//! send/recv/close operation between the timeout firing and race
//! reconciliation.
//!
//! All hooks are quick atomic checks when not armed — zero syscalls.

use core::sync::atomic::{AtomicBool, Ordering};

use osal_backend_freertos_sys as sys;

/// Set by the test controller to request a pause at the next
/// timeout boundary.
static GATE_ARMED: AtomicBool = AtomicBool::new(false);

/// Set by the hook when it has entered the pause loop.  The
/// controller polls this to know when the helper is held.
static AT_BOUNDARY: AtomicBool = AtomicBool::new(false);

/// Arm the timeout-boundary hook.  The next send/recv that times
/// out will pause until [`release_timeout_boundary`] is called.
pub fn arm_timeout_boundary() {
    GATE_ARMED.store(true, Ordering::Release);
}

/// Wait (with bounded polling) until a helper has reached the
/// timeout boundary and is paused.
pub fn wait_at_boundary(deadline_ticks: u32) -> bool {
    let start = sys::tick_snapshot();
    loop {
        if AT_BOUNDARY.load(Ordering::Acquire) {
            return true;
        }
        let now = sys::tick_snapshot();
        if now.tick_count.wrapping_sub(start.tick_count) >= deadline_ticks as u64 {
            return false;
        }
        sys::delay_ticks(1);
    }
}

/// Release the paused helper so it continues with race
/// reconciliation.
pub fn release_timeout_boundary() {
    GATE_ARMED.store(false, Ordering::Release);
}

/// Reset to a clean state (called at test start/end).
pub fn reset_timeout_hook() {
    GATE_ARMED.store(false, Ordering::Release);
    AT_BOUNDARY.store(false, Ordering::Release);
}

/// Called from the Queue send/recv timeout path, AFTER
/// `wait_once()` returned `Unavailable` but BEFORE re-acquiring
/// `state_mutex`.  The caller does NOT hold any lock or semaphore
/// at this point.
///
/// When armed: sets AT_BOUNDARY, then busy-waits (with tick delays
/// so the controller task can run) until the gate is released.
/// When not armed: returns immediately (one `Relaxed` load).
pub fn on_timeout_boundary() {
    if !GATE_ARMED.load(Ordering::Relaxed) {
        return;
    }
    AT_BOUNDARY.store(true, Ordering::Release);
    while GATE_ARMED.load(Ordering::Acquire) {
        sys::delay_ticks(1);
    }
    AT_BOUNDARY.store(false, Ordering::Release);
}
