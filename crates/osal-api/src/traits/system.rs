//! System trait — global system operations.
//!
//! See [the backend contract](../../../docs/backend-contract.md)
//! for the full behavioral specification.

/// Global system-level operations.
///
/// Provides heap introspection and critical section entry/exit.
/// Backend-specific extensions (scheduler control, ISR yield) are
/// intentionally **not** part of this trait — they are documented as
/// backend-specific capabilities.
///
/// # Critical sections
///
/// Critical sections provide mutual exclusion for short, infrequent
/// operations. They may be nested. On real-time backends they may
/// disable interrupts; on host backends they use a process-local
/// recursive mutex.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// let free = PosixSystem::heap_free();
/// println!("Heap free: {} bytes", free);
///
/// PosixSystem::critical_enter();
/// // ... short critical section ...
/// PosixSystem::critical_exit();
/// ```
pub trait System {
    /// Return the number of free bytes in the heap.
    ///
    /// On virtual-memory systems (POSIX) this may return `usize::MAX`.
    fn heap_free() -> usize;

    /// Enter a critical section.
    ///
    /// May be nested; each call must be paired with a matching
    /// [`critical_exit`](System::critical_exit).
    fn critical_enter();

    /// Exit a critical section.
    fn critical_exit();
}
