//! POSIX system implementation.
//!
//! Critical sections use a process-local recursive `pthread_mutex_t`
//! initialised once via `pthread_once`. Nesting is supported — each
//! [`enter_critical`] call acquires the lock and each guard drop
//! releases one level.

use osal_api::traits::system::System;

use crate::sys::recursive_mutex::PosixRecursiveMutex;

// ---------------------------------------------------------------------------
// Global state — lazily initialised by pthread_once
// ---------------------------------------------------------------------------

static CRITICAL_MUTEX: PosixRecursiveMutex = PosixRecursiveMutex::uninit();
static mut CRITICAL_ONCE: libc::pthread_once_t = libc::PTHREAD_ONCE_INIT;

extern "C" fn init_critical_mutex() {
    // Safety: called exactly once by pthread_once.
    unsafe { CRITICAL_MUTEX.init() };
}

fn ensure_init() {
    unsafe {
        libc::pthread_once(core::ptr::addr_of_mut!(CRITICAL_ONCE), init_critical_mutex);
    }
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// POSIX system — static methods only.
pub struct PosixSystem;

/// RAII guard that unlocks the recursive mutex on drop.
///
/// Created by [`PosixSystem::enter_critical`].
pub struct PosixCriticalSectionGuard;

impl Drop for PosixCriticalSectionGuard {
    fn drop(&mut self) {
        CRITICAL_MUTEX.unlock();
    }
}

// ---------------------------------------------------------------------------
// System trait
// ---------------------------------------------------------------------------

impl System for PosixSystem {
    type CriticalSectionGuard = PosixCriticalSectionGuard;

    fn heap_free() -> usize {
        // Virtual-memory hosts return MAX; real introspection is
        // deferred to the BSP/resource phase.
        usize::MAX
    }

    fn enter_critical() -> Self::CriticalSectionGuard {
        ensure_init();
        CRITICAL_MUTEX.lock();
        PosixCriticalSectionGuard
    }
}

// ---------------------------------------------------------------------------
// Factory (testkit)
// ---------------------------------------------------------------------------

#[cfg(feature = "testkit")]
pub struct PosixSystemFactory;

#[cfg(feature = "testkit")]
impl osal_testkit::factory::SystemFactory for PosixSystemFactory {
    type System = PosixSystem;
}
