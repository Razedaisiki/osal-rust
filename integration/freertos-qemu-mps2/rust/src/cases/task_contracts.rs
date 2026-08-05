//! Task real-kernel contracts (P7G Step 4D).
//!
//! Cases are added incrementally across commits 2–5.
//! Commit 1 — skeleton only; no cases yet.

use crate::harness;

/// Error codes for Task contract cases.
#[repr(i32)]
pub enum TaskContractError {
    #[allow(dead_code)]
    Placeholder = 500,
}

/// Run all Task contract cases.
///
/// Cases are added in subsequent commits.
pub fn run_task_cases(_tick_bits: u8) -> Result<(), TaskContractError> {
    // Commit 1: no cases yet — harness case is run by the suite
    // dispatcher itself.
    let _ = harness::console_line; // keep import alive
    Ok(())
}
