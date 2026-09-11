//! Semaphore demo — counting and binary semaphore roundtrips.

use core::fmt;

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};
use crate::runtime::with_runtime;

/// Outcome of the semaphore demo.
pub struct SemaphoreReport {
    /// Counting semaphore count at construction.
    pub initial_count: u32,
    /// Counting semaphore count after one acquire.
    pub after_acquire: u32,
    /// Counting semaphore count after one release.
    pub after_release: u32,
    /// Whether the binary semaphore completed its signal/acquire cycle.
    pub binary_roundtrip: bool,
}

impl fmt::Display for SemaphoreReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[semaphore] initial_count={}\n\
             [semaphore] after_acquire={}\n\
             [semaphore] after_release={}\n\
             [semaphore] binary_roundtrip={}",
            self.initial_count, self.after_acquire, self.after_release, self.binary_roundtrip
        )
    }
}

/// Run the semaphore demo against the active backend.
pub fn run() -> DemoResult<SemaphoreReport> {
    with_runtime(|| {
        let pool = CountingSemaphore::new(2, 1).demo_context("semaphore.counting.create")?;

        let initial_count = pool.count().demo_context("semaphore.counting.count")?;
        if initial_count != 1 {
            return Err(DemoError::Check("counting semaphore initial count"));
        }

        pool.acquire(Timeout::NoWait)
            .demo_context("semaphore.counting.acquire")?;

        let after_acquire = pool.count().demo_context("semaphore.counting.count")?;
        if after_acquire != 0 {
            return Err(DemoError::Check(
                "counting semaphore acquire did not decrement",
            ));
        }

        pool.release().demo_context("semaphore.counting.release")?;

        let after_release = pool.count().demo_context("semaphore.counting.count")?;
        if after_release != 1 {
            return Err(DemoError::Check(
                "counting semaphore release did not restore count",
            ));
        }

        let ready = BinarySemaphore::new().demo_context("semaphore.binary.create")?;

        if ready
            .is_signaled()
            .demo_context("semaphore.binary.is_signaled")?
        {
            return Err(DemoError::Check("binary semaphore started signaled"));
        }

        ready.release().demo_context("semaphore.binary.release")?;
        if !ready
            .is_signaled()
            .demo_context("semaphore.binary.is_signaled")?
        {
            return Err(DemoError::Check("binary semaphore release did not signal"));
        }

        ready
            .acquire(Timeout::NoWait)
            .demo_context("semaphore.binary.acquire")?;
        if ready
            .is_signaled()
            .demo_context("semaphore.binary.is_signaled")?
        {
            return Err(DemoError::Check("binary semaphore acquire did not clear"));
        }

        drop(pool);
        drop(ready);

        Ok(SemaphoreReport {
            initial_count,
            after_acquire,
            after_release,
            binary_roundtrip: true,
        })
    })
}
