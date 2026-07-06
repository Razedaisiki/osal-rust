//! Monotonic time via `clock_gettime(CLOCK_MONOTONIC)`.

use core::time::Duration;

/// Return the current monotonic time as a `libc::timespec`.
pub fn monotonic_now_raw() -> libc::timespec {
    let mut ts: libc::timespec = unsafe { core::mem::zeroed() };
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    // CLOCK_MONOTONIC on a valid timespec pointer should never fail.
    // If it does, the system is in an unrecoverable state.
    debug_assert_eq!(ret, 0, "clock_gettime(CLOCK_MONOTONIC) failed");
    ts
}

/// Return the current monotonic time as a `Duration`.
#[allow(dead_code)]
pub fn monotonic_now() -> Duration {
    let ts = monotonic_now_raw();
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

/// Sleep for at least `d` using `nanosleep`.
///
/// Restarts on `EINTR` (signal interruption) using the remaining time
/// reported by the kernel. Uses `CLOCK_MONOTONIC` so it is unaffected
/// by wall-clock changes.
pub fn nanosleep(d: Duration) {
    let mut remaining = libc::timespec {
        tv_sec: d.as_secs() as libc::time_t,
        tv_nsec: d.subsec_nanos() as libc::c_long,
    };
    loop {
        let mut rem: libc::timespec = unsafe { core::mem::zeroed() };
        let ret = unsafe { libc::nanosleep(&remaining, &mut rem) };
        if ret == 0 {
            return;
        }
        let err = unsafe { *libc::__errno_location() };
        if err == libc::EINTR {
            remaining = rem;
            continue;
        }
        // Other errors (EINVAL, EFAULT) — should not happen with valid input.
        return;
    }
}

/// Return `true` if `a >= b` (monotonic timespec comparison).
pub fn timespec_ge(a: &libc::timespec, b: &libc::timespec) -> bool {
    a.tv_sec > b.tv_sec || (a.tv_sec == b.tv_sec && a.tv_nsec >= b.tv_nsec)
}

/// Compute an absolute deadline: `now + timeout`, using `CLOCK_MONOTONIC`.
///
/// Normalizes nanosecond overflow. The caller is responsible for ensuring
/// the result does not overflow `time_t`.
pub fn abs_deadline(timeout: Duration) -> libc::timespec {
    let mut ts = monotonic_now_raw();
    let sec = timeout.as_secs() as libc::time_t;
    let nsec = timeout.subsec_nanos() as libc::c_long;
    ts.tv_sec = ts.tv_sec.saturating_add(sec);
    ts.tv_nsec += nsec;
    if ts.tv_nsec >= 1_000_000_000 {
        ts.tv_sec = ts.tv_sec.saturating_add(1);
        ts.tv_nsec -= 1_000_000_000;
    }
    ts
}
