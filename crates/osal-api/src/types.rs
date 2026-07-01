//! Common type aliases for the OSAL framework.
//!
//! These types abstract over platform-specific integer sizes, allowing
//! each backend to define its actual representation.

/// Opaque handle to an OSAL resource.
///
/// Handles are lightweight, copyable identifiers. The underlying value
/// is backend-defined; portable code should treat handles as opaque.
pub type Handle = usize;

/// Task/thread priority value. Higher values indicate higher priority.
pub type Priority = u32;

/// Set of event flags represented as a bitmask.
pub type EventMask = u32;

/// Stack size in bytes.
pub type StackSize = usize;

// ---------------------------------------------------------------------------
// Exit code
// ---------------------------------------------------------------------------

/// Return code from a completed task.
///
/// Wraps a `u32` value. `ExitCode::SUCCESS` (code 0) indicates normal
/// termination. Non-zero codes are application-defined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExitCode(u32);

impl ExitCode {
    /// The canonical success code.
    pub const SUCCESS: ExitCode = ExitCode(0);

    /// Create an exit code from a raw value.
    pub const fn new(code: u32) -> Self {
        ExitCode(code)
    }

    /// Return the raw `u32` value.
    pub const fn code(&self) -> u32 {
        self.0
    }
}

// ---------------------------------------------------------------------------
// Task state
// ---------------------------------------------------------------------------

/// The current scheduling state of a task.
///
/// State transitions are backend-dependent. Portable code should query
/// state for diagnostic purposes only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskState {
    /// Task created and eligible to run.
    Ready,
    /// Task currently executing on a CPU.
    Running,
    /// Task waiting on a synchronization primitive.
    Blocked,
    /// Task explicitly suspended (backend-dependent).
    Suspended,
    /// Task entry function has returned.
    Finished,
}

// ---------------------------------------------------------------------------
// Timer mode
// ---------------------------------------------------------------------------

/// Determines whether a timer fires once or repeatedly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimerMode {
    /// Fire once, then stop.
    OneShot,
    /// Fire repeatedly at the configured period.
    Periodic,
}
