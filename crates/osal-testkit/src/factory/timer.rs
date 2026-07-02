//! Factory for creating timer instances.

use alloc::boxed::Box;
use core::time::Duration;

use osal_api::error::Result;
use osal_api::traits::timer::{Timer, TimerCallback};
use osal_api::types::TimerMode;

/// Factory for creating timer instances in a backend-agnostic way.
pub trait TimerFactory {
    /// Concrete timer type.
    type Timer: Timer;

    /// Create a timer with the given configuration.
    ///
    /// This default implementation requires `alloc` (the testkit crate
    /// links `alloc` for `TimerCallback` support).
    fn create_timer(
        &self,
        name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self::Timer>;

    /// Create a no-op callback for use in creation/control tests.
    ///
    /// The default implementation requires `alloc`.
    fn dummy_callback(&self) -> TimerCallback {
        Box::new(|| {})
    }
}
