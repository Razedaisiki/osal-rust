//! Prelude module for convenient importing.
//!
//! Exports the most commonly used and stable types. Users write:
//!
//! ```ignore
//! use osal::prelude::*;
//! ```
//!
//! Backend internals, shared implementation details, and experimental
//! types are intentionally excluded from the prelude.

pub use crate::error::{Error, Result};
pub use crate::time::Timeout;
pub use crate::types::{ExitCode, Handle, Priority, StackSize, TaskState, TimerMode};

pub use crate::traits::clock::Clock;
pub use crate::traits::mutex::Mutex;
pub use crate::traits::queue::Queue;
pub use crate::traits::semaphore::{BinarySemaphore, CountingSemaphore};
pub use crate::traits::system::System;
pub use crate::traits::task::{Task, TaskBuilder};
pub use crate::traits::timer::Timer;
