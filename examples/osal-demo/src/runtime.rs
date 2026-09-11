//! Runtime lifecycle helper for demos that need the OSAL runtime.
//!
//! ```text
//! osal::initialize()
//!        ↓
//!      body()
//!        ↓
//! osal::shutdown()
//! ```
//!
//! `shutdown()` is always attempted. A body failure is reported as-is; a
//! body success followed by a shutdown failure is reported as the
//! shutdown failure.

use core::time::Duration;

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};

/// Upper bound on waiting for the runtime to become quiescent.
const SETTLE_TIMEOUT: Duration = Duration::from_secs(1);
/// Poll interval while waiting for quiescence.
const SETTLE_POLL: Duration = Duration::from_millis(2);

pub(crate) fn with_runtime<T>(body: impl FnOnce() -> DemoResult<T>) -> DemoResult<T> {
    osal::initialize().demo_context("runtime.initialize")?;

    let result = body();

    match result {
        Err(error) => {
            // Objects may still be live; a shutdown failure here is a
            // consequence of the body failure, so the body error wins.
            let _ = shutdown_settled();
            Err(error)
        }
        Ok(value) => {
            shutdown_settled()?;
            Ok(value)
        }
    }
}

/// Shut the runtime down, tolerating a short `Busy` window.
///
/// On FreeRTOS a task's trampoline publishes completion (waking the joiner)
/// *before* it drops the internal `Arc`s that carry the `RuntimeLease`
/// (ADR 0028 §6). A joiner that returns and immediately calls `shutdown()`
/// can therefore observe `Busy` for a few scheduler ticks even though every
/// public handle is already dropped. The POSIX backend hides this because
/// its join primitive does not return until the thread — and its trampoline
/// frame — has fully unwound.
///
/// Waiting for quiescence is the portable way to observe that reclamation.
/// It is a bounded wait, not a retry loop over a real failure: a genuine
/// lease leak still fails the demo.
fn shutdown_settled() -> DemoResult<()> {
    let deadline = Clock::now() + SETTLE_TIMEOUT;

    loop {
        match osal::shutdown() {
            Ok(()) => return Ok(()),
            Err(Error::Busy) if Clock::now() < deadline => {
                Clock::delay(SETTLE_POLL);
            }
            Err(error) => {
                return Err(DemoError::Osal {
                    operation: "runtime.shutdown",
                    error,
                });
            }
        }
    }
}
