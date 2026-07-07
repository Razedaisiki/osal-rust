//! Test support helpers for mock backend.
//!
//! These helpers are only intended for contract/stabilization tests.
//! They serialize access to the process-global mock time runtime.

#[cfg(feature = "testkit")]
use spin::{Mutex, MutexGuard};

/// A global serialisation lock for tests that manipulate the mock time
/// runtime.  Because [`crate::time_runtime`] uses a process-global
/// `static RUNTIME`, parallel integration tests can interfere with each
/// other (one test resets while another is asserting callback counts).
/// Holding this guard for the duration of a time‑sensitive test ensures
/// deterministic, isolated behaviour.
#[cfg(feature = "testkit")]
static MOCK_TIME_TEST_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the global mock‑time test guard.
///
/// Call this at the top of every `#[test]` that calls
/// [`MockClockControl::reset`] or [`advance_and_dispatch`].
#[cfg(feature = "testkit")]
pub fn mock_time_test_guard() -> MutexGuard<'static, ()> {
    MOCK_TIME_TEST_LOCK.lock()
}
