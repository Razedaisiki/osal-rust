//! Managed-object real-kernel validation cases (P7G Step 4).
//!
//! Cases are added incrementally as each primitive is validated:
//!
//!   Step 4A — mutex.rs    (Mutex real-kernel contracts)
//!   Step 4B — semaphore.rs (Counting/Binary Semaphore)
//!   Step 4C — queue.rs     (Queue blocking, close-drain, broadcast)
//!   Step 4D — task.rs      (Task lifecycle, join, self-delete, Idle cleanup)
//!   Step 4E — timer.rs     (Timer scheduling, callback reentry)

#[cfg(feature = "suite-aggregate")]
pub mod mutex;
#[cfg(feature = "suite-aggregate")]
pub mod semaphore;
#[cfg(feature = "suite-aggregate")]
pub mod queue;

#[cfg(feature = "suite-queue-blocking")]
pub mod queue_blocking;

#[cfg(feature = "suite-task")]
pub mod task_contracts;

#[cfg(feature = "suite-timer")]
pub mod timer_contracts;

#[cfg(feature = "suite-mixed")]
pub mod mixed;
