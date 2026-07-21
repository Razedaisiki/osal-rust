//! Capability-based factory traits for creating OSAL primitives.
//!
//! Each trait focuses on one primitive type. Backends implement only
//! the factories for the primitives they support. This allows Mock
//! to start with `QueueFactory + ClockFactory` without implementing
//! `TaskFactory` or `TimerFactory`.
//!
//! # Backward compatibility
//!
//! [`BackendFactory`] combines all Phase A factories into a single
//! supertrait for existing contract tests. New code should use the
//! individual factories directly.

pub mod clock;
pub mod fault;
pub mod mutex;
pub mod queue;
pub mod runtime;
pub mod semaphore;
pub mod system;
pub mod task;
pub mod timer;

pub use clock::{ClockControl, ClockFactory, ControlledClockFactory};
pub use fault::FaultFactory;
pub use mutex::MutexFactory;
pub use queue::QueueFactory;
pub use runtime::RuntimeFactory;
pub use semaphore::SemaphoreFactory;
pub use system::SystemFactory;
pub use task::TaskFactory;
pub use timer::{ControlledTimerFactory, TimerFactory};

/// Combined Phase A factory for backward compatibility.
///
/// Prefer using individual factories (`QueueFactory`, `MutexFactory`,
/// `SemaphoreFactory`) in new contract code.
pub trait BackendFactory: QueueFactory + MutexFactory + SemaphoreFactory {}

impl<T> BackendFactory for T where T: QueueFactory + MutexFactory + SemaphoreFactory {}
