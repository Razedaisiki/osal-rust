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
    /// The factory is responsible for allocating the callback
    /// (which requires alloc), keeping the testkit crate itself
    /// alloc-free.
    fn create_timer(
        &self,
        name: &str,
        period: Duration,
        mode: TimerMode,
        callback: TimerCallback,
    ) -> Result<Self::Timer>;

    /// Create a no-op callback for use in creation/control tests.
    ///
    /// The factory handles allocation; the testkit crate itself
    /// remains alloc-free.
    fn dummy_callback(&self) -> TimerCallback {
        Box::new(|| {})
    }
}
