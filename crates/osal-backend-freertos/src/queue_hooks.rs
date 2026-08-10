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

// ------------------------------------------------------------------
// Raw primitives (used by TimeoutHookGuard or directly)
// ------------------------------------------------------------------

fn arm() {
    GATE_ARMED.store(true, Ordering::Release);
}

fn release() {
    GATE_ARMED.store(false, Ordering::Release);
}

fn reset() {
    GATE_ARMED.store(false, Ordering::Release);
    AT_BOUNDARY.store(false, Ordering::Release);
}

/// Compute a total-tick value across overflow boundaries.
/// Matches the `total_ticks_diff` convention in the integration
/// harness: `(overflow_count << tick_bits) | tick_count`.
fn total_ticks(snap: sys::TickSnapshot, tick_bits: u8) -> u128 {
    ((snap.overflow_count as u128) << tick_bits) | (snap.tick_count as u128)
}

/// Wait (with bounded polling and correct tick-wrap handling)
/// until a helper has reached the timeout boundary and is paused.
///
/// `tick_bits` must match the FreeRTOS tick counter width declared
/// in the kernel capabilities (16, 32, or 64).
pub fn wait_at_boundary(deadline_ticks: u32, tick_bits: u8) -> bool {
    let start = sys::tick_snapshot();
    let start_total = total_ticks(start, tick_bits);
    loop {
        if AT_BOUNDARY.load(Ordering::Acquire) {
            return true;
        }
        let now = sys::tick_snapshot();
        let elapsed = total_ticks(now, tick_bits).saturating_sub(start_total);
        if elapsed >= deadline_ticks as u128 {
            return false;
        }
        if sys::delay_ticks(1) != sys::DelayStatus::Ok {
            return false;
        }
    }
}

/// Called from the Queue send/recv timeout path, AFTER
/// `wait_once()` returned `Unavailable` but BEFORE re-acquiring
/// `state_mutex`.  The caller does NOT hold any lock or semaphore
/// at this point.
///
/// When armed: sets AT_BOUNDARY, then polls/yields with tick delays
/// (so the controller task can run) until the gate is released.
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

// ------------------------------------------------------------------
// RAII guard — ensures the gate is always released on error paths.
// ------------------------------------------------------------------

/// RAII guard for the timeout-boundary hook.
///
/// ```ignore
/// let guard = TimeoutHookGuard::arm();
/// // spawn helper, wait for boundary, inject operation ...
/// guard.release();
/// // guard.drop() cleans up if any ?-path returned early
/// ```
pub struct TimeoutHookGuard {
    released: bool,
}

impl TimeoutHookGuard {
    /// Reset the hook state, arm the gate, and return a guard that
    /// will clean up on drop.
    pub fn arm() -> Self {
        reset();
        arm();
        Self { released: false }
    }

    /// Release the paused helper so it continues with race
    /// reconciliation.  Safe to call multiple times.
    pub fn release(&mut self) {
        if !self.released {
            release();
            self.released = true;
        }
    }
}

impl Drop for TimeoutHookGuard {
    fn drop(&mut self) {
        if !self.released {
            release();
        }
        reset();
    }
}
