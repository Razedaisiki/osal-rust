//! Mock system implementation.
//!
//! Provides deterministic, single-context implementations of
//! [`System`] operations for contract testing.
//!
//! # Critical sections
//!
//! Critical sections use an atomic nesting counter rather than a real
//! lock. Because Mock backend primitives use `Rc<RefCell<>>` and run
//! in a single execution context, thread-level mutual exclusion is
//! not needed — only the nesting contract matters.
//!
//! # heap_free()
//!
//! Returns `usize::MAX` (host virtual memory). Real heap introspection
//! is deferred to the BSP/resource phase.

use core::sync::atomic::{AtomicUsize, Ordering};

use osal_api::traits::system::System;

/// Global critical-section nesting depth.
///
/// An atomic counter is used rather than a real mutex because Mock
/// primitives operate in a single execution context. The counter
/// tracks nesting so that contract tests can verify the guard is
/// dropped the expected number of times.
static CRITICAL_DEPTH: AtomicUsize = AtomicUsize::new(0);

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Mock system — static methods only.
pub struct MockSystem;

/// RAII guard that decrements the critical-section nesting depth on drop.
///
/// Created by [`MockSystem::enter_critical`]. The private field
/// prevents external code from constructing a guard without going
/// through `enter_critical()`, which would underflow the counter.
pub struct MockCriticalSectionGuard {
    _private: (),
}

impl Drop for MockCriticalSectionGuard {
    fn drop(&mut self) {
        CRITICAL_DEPTH.fetch_sub(1, Ordering::SeqCst);
    }
}

// ---------------------------------------------------------------------------
// System trait
// ---------------------------------------------------------------------------

impl System for MockSystem {
    type CriticalSectionGuard = MockCriticalSectionGuard;

    fn heap_free() -> usize {
        // Virtual-memory hosts return MAX; real introspection is
        // deferred to the BSP/resource phase.
        usize::MAX
    }

    fn enter_critical() -> Self::CriticalSectionGuard {
        CRITICAL_DEPTH.fetch_add(1, Ordering::SeqCst);
        MockCriticalSectionGuard { _private: () }
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

#[cfg(feature = "testkit")]
pub struct MockSystemFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::SystemFactory for MockSystemFactory {
    type System = MockSystem;
}

// ---------------------------------------------------------------------------
// Test-only helpers
// ---------------------------------------------------------------------------

/// Return the current critical-section nesting depth.
///
/// Exposed for stabilisation tests that want to assert nesting
/// behaviour beyond what the basic contract covers.
#[cfg(feature = "testkit")]
pub fn critical_depth_for_test() -> usize {
    CRITICAL_DEPTH.load(Ordering::SeqCst)
}
