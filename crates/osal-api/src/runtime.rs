//! Runtime lifecycle types.
//!
//! [`RuntimeState`] is the publicly observable state of an OSAL
//! runtime. The state machine and transition logic live in
//! `osal-shared::runtime::RuntimeLifecycle`.

/// Observable lifecycle state of an OSAL runtime.
///
/// The four states form a re-entrant cycle:
///
/// ```text
/// Uninitialized → Initializing → Running → ShuttingDown → Uninitialized
/// ```
///
/// `Uninitialized` is both the start and the end of the cycle,
/// allowing re-initialisation within the same process.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeState {
    /// No runtime services active.  Objects cannot be created.
    Uninitialized = 0,
    /// Initialisation in progress (may fail and roll back).
    Initializing = 1,
    /// Fully operational.  Objects can be created and used.
    Running = 2,
    /// Shutdown in progress (may fail and roll back).
    ShuttingDown = 3,
}
