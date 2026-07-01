//! System trait — global system operations.
//!
//! See [the backend contract](../../../docs/behavior-contract.md)
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
/// The guard returned by [`enter_critical`](System::enter_critical)
/// automatically exits the critical section when dropped. This
/// prevents missed exits due to early returns or panics.
///
/// # Examples
///
/// ```ignore
/// use osal::prelude::*;
///
/// let free = System::heap_free();
/// println!("Heap free: {} bytes", free);
///
/// {
///     let _guard = System::enter_critical();
///     // ... short critical section ...
/// } // automatically exited here
/// ```
pub trait System {
    /// Guard that automatically exits a critical section on drop.
    ///
    /// Supports nesting: each [`enter_critical`](System::enter_critical)
    /// call produces a new guard; the critical section is fully exited
    /// only when the outermost guard is dropped.
    type CriticalSectionGuard: Drop;

    /// Return the number of free bytes in the heap.
    ///
    /// On virtual-memory systems (POSIX) this may return `usize::MAX`.
    fn heap_free() -> usize;

    /// Enter a critical section.
    ///
    /// Returns a guard that exits the critical section when dropped.
    /// Critical sections may be nested: each call produces a guard;
    /// dropping the outermost guard fully exits.
    fn enter_critical() -> Self::CriticalSectionGuard;
}
