//! OSAL — Operating System Abstraction Layer
//!
//! This is the **facade crate**. Users depend on this single crate:
//!
//! ```toml
//! [dependencies]
//! osal = "0.1"
//! ```
//!
//! ```ignore
//! use osal::prelude::*;
//! use core::time::Duration;
//!
//! fn main() {
//!     let counter = Mutex::new(0u32);
//!     let mut guard = counter.lock(Timeout::Forever).unwrap();
//!     *guard += 1;
//! }
//! ```
//!
//! ## Backend Selection
//!
//! Choose a backend via Cargo features:
//!
//! ```toml
//! # POSIX (default — Linux, macOS, CI)
//! osal = "0.1"
//!
//! # Mock (testing, simulation)
//! osal = { version = "0.1", default-features = false, features = ["backend-mock"] }
//! ```
//!
//! Only one backend may be active at a time. The compilation fails
//! if zero or multiple backends are selected.

#![no_std]

extern crate alloc;

#[cfg(not(any(feature = "backend-posix", feature = "backend-mock")))]
compile_error!(
    "At least one OSAL backend must be enabled. \
     Enable the 'backend-posix' or 'backend-mock' feature."
);

#[cfg(all(feature = "backend-posix", feature = "backend-mock"))]
compile_error!("Only one OSAL backend may be enabled at a time.");

/// Re-export the public API types.
///
/// Users should prefer `use osal::prelude::*` for the most
/// common types.
pub use osal_api;

/// Backend type aliases — concrete types from the active backend.
pub mod backend;

/// Runtime lifecycle API — explicit init and shutdown.
pub mod runtime;

// Re-export at crate root so users can call osal::initialize() etc.
pub use runtime::{initialize, runtime_state, shutdown};

/// Commonly used types, re-exported for convenience.
///
/// ```ignore
/// use osal::prelude::*;
/// ```
pub mod prelude {
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::BinarySemaphore;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::Clock;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::CountingSemaphore;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::Mutex;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::Queue;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::System;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::Task;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::TaskBuilder;
    #[cfg(any(feature = "backend-mock", feature = "backend-posix"))]
    pub use crate::backend::Timer;
    pub use crate::runtime::{initialize, runtime_state, shutdown};
    pub use osal_api::error::{Error, Result};
    pub use osal_api::prelude::*;
    pub use osal_api::traits::clock::Clock as _;
    pub use osal_api::traits::mutex::Mutex as _;
    pub use osal_api::traits::queue::Queue as _;
    pub use osal_api::traits::semaphore::BinarySemaphore as _;
    pub use osal_api::traits::semaphore::CountingSemaphore as _;
    pub use osal_api::traits::system::System as _;
    pub use osal_api::traits::task::Task as _;
    pub use osal_api::traits::task::TaskBuilder as _;
    pub use osal_api::traits::timer::Timer as _;
}
