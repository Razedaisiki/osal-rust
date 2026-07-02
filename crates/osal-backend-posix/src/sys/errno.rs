//! errno → `osal_api::Error` mapping.

use osal_api::error::{Error, Result};

/// Map common `errno` values to OSAL errors.
///
/// Returns `Ok(())` if `ret == 0`, otherwise maps errno.
/// Use only with POSIX functions that return 0 on success.
pub fn check_ret(ret: i32) -> Result<()> {
    if ret == 0 {
        Ok(())
    } else {
        Err(map_errno())
    }
}

/// Map the current errno to an OSAL error.
fn map_errno() -> Error {
    let e = unsafe { *libc::__errno_location() };
    match e {
        libc::EAGAIN => Error::Timeout,
        libc::ETIMEDOUT => Error::Timeout,
        libc::ENOMEM => Error::OutOfMemory,
        libc::EINVAL => Error::InvalidParameter,
        libc::EBUSY => Error::LockFailed,
        libc::EDEADLK => Error::LockFailed,
        _ => Error::Internal("unexpected errno"),
    }
}
