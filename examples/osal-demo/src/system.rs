//! System demo — heap introspection and nested critical sections.
//!
//! `System` does not require the OSAL runtime lifecycle: `heap_free()`
//! and `enter_critical()` are always available. No output happens inside
//! the critical section — on FreeRTOS it disables interrupts.

use core::fmt;

use osal::prelude::*;

use crate::error::DemoResult;

/// Outcome of the system demo.
pub struct SystemReport {
    /// Free heap bytes as reported by the backend.
    pub heap_free: usize,
    /// Whether nested critical sections were entered and exited.
    pub nested_critical_ok: bool,
}

impl fmt::Display for SystemReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[system] heap_free={}\n[system] nested_critical_ok={}",
            self.heap_free, self.nested_critical_ok
        )
    }
}

/// Run the system demo against the active backend.
pub fn run() -> DemoResult<SystemReport> {
    let heap_free = System::heap_free();

    // Guards drop in reverse order, each exiting one nesting level.
    {
        let _outer = System::enter_critical();
        {
            let _inner = System::enter_critical();
        }
    }

    Ok(SystemReport {
        heap_free,
        nested_critical_ok: true,
    })
}
