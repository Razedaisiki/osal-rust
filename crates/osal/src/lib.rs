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

/// Commonly used types, re-exported for convenience.
///
/// ```ignore
/// use osal::prelude::*;
/// ```
pub mod prelude {
    pub use osal_api::prelude::*;
}
