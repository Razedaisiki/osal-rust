//! Factory for clock access and control.

use core::time::Duration;

use osal_api::traits::clock::Clock;

/// Factory for accessing the system clock.
pub trait ClockFactory {
    /// Concrete clock type.
    type Clock: Clock;
}

/// Control interface for deterministic (fake) clocks.
///
/// Used by contract tests that need to advance time without
/// waiting for real time to pass. Only Mock implements this;
/// POSIX backends do not.
pub trait ClockControl {
    /// Advance the clock by `duration`.
    fn advance_clock(&self, duration: Duration);
}

/// Convenience bound: a clock that is both accessible and controllable.
///
/// Implemented automatically for any type that implements both
/// [`ClockFactory`] and [`ClockControl`].
pub trait ControlledClockFactory: ClockFactory + ClockControl {}

impl<T> ControlledClockFactory for T where T: ClockFactory + ClockControl {}
