//! OS-independent shared implementation layer.
//!
//! Provides common logic shared by all backends:
//!
//! - Runtime lifecycle state transitions (ADR 0016)
//! - Active-object lease accounting with linearizable acquire/shutdown
//! - Close-state tracking helpers
//! - Parameter validation helpers (`validate_queue_config`,
//!   `validate_task_config`, etc.)
//!
//! A global object ID registry and resource lookup table are
//! deferred (ADR 0006).  The MVP uses strongly typed handles.

#![no_std]

extern crate alloc;

#[cfg(test)]
extern crate std;

pub mod close_state;
pub mod runtime;
pub mod validation;
