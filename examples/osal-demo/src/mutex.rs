//! Mutex demo — lock / guard / clone sharing.

use core::fmt;

use osal::prelude::*;

use crate::error::{DemoError, DemoResult, OsalResultExt};
use crate::runtime::with_runtime;

/// Outcome of the mutex demo.
pub struct MutexReport {
    /// Value read back through a cloned handle.
    pub final_value: u32,
}

impl fmt::Display for MutexReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[mutex] final_value={}\n[mutex] clone sharing OK",
            self.final_value
        )
    }
}

/// Run the mutex demo against the active backend.
pub fn run() -> DemoResult<MutexReport> {
    with_runtime(|| {
        let counter = Mutex::new(0u32).demo_context("mutex.create")?;

        {
            let mut guard = counter.lock(Timeout::Forever).demo_context("mutex.lock")?;
            *guard += 1;
        }

        let cloned = counter.clone();

        let value = {
            let guard = cloned
                .lock(Timeout::Forever)
                .demo_context("mutex.clone.lock")?;
            *guard
        };

        if value != 1 {
            return Err(DemoError::Check(
                "mutex clone did not share protected value",
            ));
        }

        drop(cloned);
        drop(counter);

        Ok(MutexReport { final_value: value })
    })
}
