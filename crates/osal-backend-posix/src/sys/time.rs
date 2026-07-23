//! Monotonic time via `clock_gettime(CLOCK_MONOTONIC)`.

use core::time::Duration;

use osal_api::error::{Error, Result};

// ---------------------------------------------------------------------------
// errno helper — avoids non-portable __errno_location()
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
fn errno() -> libc::c_int {
    unsafe { *libc::__errno_location() }
}

#[cfg(not(target_os = "linux"))]
fn errno() -> libc::c_int {
    extern "C" {
        fn __error() -> *mut libc::c_int;
    }
    unsafe { *__error() }
}

// ---------------------------------------------------------------------------
// Duration ↔ timespec conversion
// ---------------------------------------------------------------------------

/// Convert a `Duration` to a `libc::timespec` for use with POSIX APIs.
///
/// Returns `Error::Overflow` if `tv_sec` would overflow `time_t` or if
/// sub-second nanos would overflow `c_long`.
#[allow(clippy::unnecessary_fallible_conversions)] // time_t may be i32
pub fn duration_to_timespec(d: Duration) -> Result<libc::timespec> {
    let secs = d.as_secs();
    let nsecs = d.subsec_nanos();
    // time_t is platform-dependent; reject values that don't fit.
    let tv_sec = libc::time_t::try_from(secs).map_err(|_| Error::Overflow)?;
    Ok(libc::timespec {
        tv_sec,
        tv_nsec: nsecs as libc::c_long,
    })
}

// ---------------------------------------------------------------------------
// Monotonic clock
// ---------------------------------------------------------------------------

/// Return the current monotonic time as a `libc::timespec`.
///
/// # Panics
///
/// Panics if `clock_gettime(CLOCK_MONOTONIC)` fails — this indicates
/// an unrecoverable platform state.
pub fn monotonic_now_raw() -> libc::timespec {
    let mut ts: libc::timespec = unsafe { core::mem::zeroed() };
    let ret = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    assert_eq!(
        ret, 0,
        "clock_gettime(CLOCK_MONOTONIC) failed unrecoverably"
    );
    ts
}

/// Return the current monotonic time as a `Duration`.
#[allow(dead_code)]
pub fn monotonic_now() -> Duration {
    let ts = monotonic_now_raw();
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

// ---------------------------------------------------------------------------
// Sleep
// ---------------------------------------------------------------------------

/// Maximum seconds representable in `time_t` for nanosleep.
#[cfg(target_pointer_width = "64")]
const MAX_SLEEP_SECS: i64 = libc::time_t::MAX;

#[cfg(not(target_pointer_width = "64"))]
const MAX_SLEEP_SECS: i64 = i32::MAX as libc::time_t;

/// Sleep for at least `d` using `nanosleep`.
///
/// Restarts on `EINTR` (signal interruption) using the remaining time
/// reported by the kernel. Uses `CLOCK_MONOTONIC` so it is unaffected
/// by wall-clock changes.
///
/// Splits durations that exceed `time_t` range into multiple segments,
/// honouring the contract that `delay(d)` sleeps for at least `d`.
pub fn nanosleep(d: Duration) {
    let mut remaining = d;
    loop {
        if remaining.is_zero() {
            return;
        }
        // Cap each segment to fit in time_t, but take at least one
        // segment even if the cap rounds to zero.
        let seg_secs = remaining.as_secs().min(MAX_SLEEP_SECS as u64);
        let seg = Duration::new(seg_secs, remaining.subsec_nanos());

        let ts = match duration_to_timespec(seg) {
            Ok(ts) => ts,
            Err(_) => return, // should not happen with capped value
        };

        let mut req = ts;
        loop {
            let mut rem: libc::timespec = unsafe { core::mem::zeroed() };
            let ret = unsafe { libc::nanosleep(&req, &mut rem) };
            if ret == 0 {
                break;
            }
            if errno() == libc::EINTR {
                req = rem;
                continue;
            }
            // Other errors (EINVAL, EFAULT) — should not happen.
            return;
        }

        remaining = remaining.saturating_sub(seg);
    }
}

// ---------------------------------------------------------------------------
// Timespec helpers
// ---------------------------------------------------------------------------

/// Return `true` if `a >= b` (monotonic timespec comparison).
pub fn timespec_ge(a: &libc::timespec, b: &libc::timespec) -> bool {
    a.tv_sec > b.tv_sec || (a.tv_sec == b.tv_sec && a.tv_nsec >= b.tv_nsec)
}

/// Compute an absolute deadline: `now + timeout`, using `CLOCK_MONOTONIC`.
///
/// Returns `Error::Overflow` if the result would overflow `time_t` or
/// the nanosecond field. The caller decides how to handle overflow
/// (e.g. fall back to `Forever`-equivalent or propagate the error).
pub fn checked_abs_deadline(timeout: Duration) -> Result<libc::timespec> {
    let now = monotonic_now_raw();
    let to = duration_to_timespec(timeout)?;

    let mut ts = now;
    ts.tv_sec = ts.tv_sec.checked_add(to.tv_sec).ok_or(Error::Overflow)?;
    ts.tv_nsec += to.tv_nsec;
    if ts.tv_nsec >= 1_000_000_000 {
        ts.tv_sec = ts.tv_sec.checked_add(1).ok_or(Error::Overflow)?;
        ts.tv_nsec -= 1_000_000_000;
    }
    Ok(ts)
}
