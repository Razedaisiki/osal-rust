//! Contract tests for the `System` trait.
//!
//! These tests verify the behavioral contract defined in
//! `docs/behavior-contract.md` (System section).

use osal_api::traits::system::System as _;

use crate::factory::SystemFactory;

// ---------------------------------------------------------------------------
// Basic tests
// ---------------------------------------------------------------------------

/// `heap_free()` can be called and returns a value.
pub fn heap_free_is_callable<F: SystemFactory>(_factory: &F) {
    let _free = F::System::heap_free();
}

// ---------------------------------------------------------------------------
// Critical section tests
// ---------------------------------------------------------------------------

/// `enter_critical()` returns a guard.
pub fn enter_critical_returns_guard<F: SystemFactory>(_factory: &F) {
    let _guard = F::System::enter_critical();
}

/// The guard exits the critical section when dropped.
pub fn critical_section_guard_exits_on_drop<F: SystemFactory>(_factory: &F) {
    {
        let _guard = F::System::enter_critical();
    }
    // Re-entering after drop must succeed.
    let _g2 = F::System::enter_critical();
}

/// Critical sections support nesting.
pub fn critical_section_supports_nesting<F: SystemFactory>(_factory: &F) {
    let _outer = F::System::enter_critical();
    let _inner = F::System::enter_critical();
    // Both guards alive — nested critical section held.
}

/// Nested guards drop in reverse order without deadlocking.
pub fn nested_guards_drop_in_reverse_order<F: SystemFactory>(_factory: &F) {
    let outer = F::System::enter_critical();
    let inner = F::System::enter_critical();
    drop(inner);
    drop(outer);
}

// ---------------------------------------------------------------------------
// Grouped entry point
// ---------------------------------------------------------------------------

/// All system contract tests.
pub fn run_all<F: SystemFactory>(factory: &F) {
    heap_free_is_callable::<F>(factory);
    enter_critical_returns_guard::<F>(factory);
    critical_section_guard_exits_on_drop::<F>(factory);
    critical_section_supports_nesting::<F>(factory);
    nested_guards_drop_in_reverse_order::<F>(factory);
}
