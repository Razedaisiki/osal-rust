//! Monotonic time via `clock_gettime(CLOCK_MONOTONIC)`.

use core::time::Duration;

/// Return the current monotonic time as a `libc::timespec`.
pub fn monotonic_now_raw() -> libc::timespec {
    let mut ts: libc::timespec = unsafe { core::mem::zeroed() };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts
}

/// Return the current monotonic time as a `Duration`.
pub fn monotonic_now() -> Duration {
    let ts = monotonic_now_raw();
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}
